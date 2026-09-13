//! Readable collections, conflict detection, and recoverable atomic saves.
use anyhow::{Context, Result, bail};
use duckie_model::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

/// Schema version this build writes. Files marked with a higher version came from a newer
/// Duckie and must not be resaved, or the fields it understands would be silently discarded.
const SCHEMA_VERSION: u32 = 1;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub name: String,
    pub requests: Vec<String>,
    #[serde(default, flatten)]
    pub extra: Extensions,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretsFile {
    pub schema_version: u32,
    pub environments: BTreeMap<String, Values>,
}
impl Default for SecretsFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            environments: BTreeMap::new(),
        }
    }
}
#[derive(Clone)]
pub struct StoredRequest {
    pub definition: RequestDefinition,
    pub source: String,
}
#[derive(Clone)]
pub struct Collection {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub requests: Vec<StoredRequest>,
    pub environments: Vec<Environment>,
    pub secrets: SecretsFile,
    hashes: BTreeMap<String, Option<String>>,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn disk_hash(path: &Path) -> Result<Option<String>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(hash(&bytes))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub fn managed_path(root: &Path, relative: &str) -> Result<PathBuf> {
    resolve(&root.canonicalize()?, relative)
}
/// `managed_path` for a root already known to be canonical, which `Collection::root` always is.
/// Canonicalizing the root per file cost 60 ms per thousand on an open that resolves each file
/// more than once; the escape check below is unaffected and still runs for every path.
fn resolve(root: &Path, relative: &str) -> Result<PathBuf> {
    let rel = Path::new(relative);
    if rel.as_os_str().is_empty()
        || rel.components().any(|c| !matches!(c, Component::Normal(_)))
        || relative.contains(':')
    {
        bail!("Collection file must be relative and stay inside its folder: {relative}");
    }
    let path = root.join(rel);
    // Check every existing ancestor, including symlinks/junctions on Windows.
    let mut ancestor = path.as_path();
    while !ancestor.exists() {
        ancestor = ancestor.parent().context("Invalid collection path")?;
    }
    if !ancestor.canonicalize()?.starts_with(root) {
        bail!("Collection file escapes its folder: {relative}");
    }
    Ok(path)
}
/// Resolves and reads many managed files, preserving order.
///
/// Opening a collection is syscall-bound rather than CPU-bound: each file costs a path
/// validation and a read, and on Windows every open also pays antivirus filtering. Spreading
/// that across threads is what brings a thousand-request restore inside its budget. Each entry
/// keeps its own error so one bad file reports against its own name.
fn read_many(root: &Path, names: &[String]) -> Vec<Result<Vec<u8>>> {
    let read = |name: &String| -> Result<Vec<u8>> {
        let path = resolve(root, name)?;
        fs::read(&path).with_context(|| format!("Cannot read {}", path.display()))
    };
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(8);
    if names.len() < 64 || threads < 2 {
        return names.iter().map(read).collect();
    }
    let mut out = Vec::with_capacity(names.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = names
            .chunks(names.len().div_ceil(threads))
            .map(|chunk| scope.spawn(move || chunk.iter().map(read).collect::<Vec<_>>()))
            .collect();
        for handle in handles {
            // A panic in a reader is a bug, not a recoverable collection error.
            out.extend(handle.join().expect("collection reader panicked"));
        }
    });
    out
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("Cannot read {}", path.display()))?;
    serde_json::from_value(document(&bytes, path)?)
        .with_context(|| format!("Unexpected contents in {}", path.display()))
}
/// Size and schema-version validation, returning the document so a caller holding the bytes
/// can convert it without reading the file a second time.
fn document(bytes: &[u8], path: &Path) -> Result<serde_json::Value> {
    if bytes.len() > 20 * MIB as usize {
        bail!("Collection document exceeds 20 MiB: {}", path.display());
    }
    let v: serde_json::Value = serde_json::from_slice(bytes)
        .with_context(|| format!("Invalid JSON in {}", path.display()))?;
    if let Some(version) = v["schemaVersion"]
        .as_u64()
        .filter(|v| *v > SCHEMA_VERSION as u64)
    {
        bail!(
            "{} was saved by a newer version of Duckie (schema version {version}); update Duckie to open it",
            path.display()
        );
    }
    Ok(v)
}
fn json(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}
fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing parent directory")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(content)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("Cannot replace {}", path.display()))?;
    Ok(())
}
#[derive(Serialize, Deserialize)]
struct WriteEntry {
    path: String,
    before: Option<String>,
    bytes: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    entries: Vec<WriteEntry>,
}
fn recover(root: &Path) -> Result<()> {
    let journal_path = managed_path(root, ".duckie/pending-save.json")?;
    if !journal_path.exists() {
        return Ok(());
    }
    let journal: Journal = serde_json::from_slice(&fs::read(&journal_path)?)
        .context("Cannot read interrupted-save journal")?;
    // Verify the entire transaction before changing anything.
    for entry in &journal.entries {
        let current = disk_hash(&managed_path(root, &entry.path)?)?;
        if current != entry.before && current != Some(hash(&entry.bytes)) {
            bail!(
                "Interrupted save conflicts with {}; preserve your files and resolve .duckie/pending-save.json",
                entry.path
            );
        }
    }
    for entry in journal.entries {
        let path = managed_path(root, &entry.path)?;
        if disk_hash(&path)? != Some(hash(&entry.bytes)) {
            atomic_write(&path, &entry.bytes)?;
        }
    }
    fs::remove_file(journal_path)?;
    Ok(())
}
impl Collection {
    pub fn new(root: PathBuf, name: String) -> Result<Self> {
        fs::create_dir_all(&root)?;
        if root.join("duckie.json").exists() {
            bail!("A collection already exists here. Open it instead.");
        }
        Ok(Self {
            root: root.canonicalize()?,
            manifest: Manifest {
                schema_version: SCHEMA_VERSION,
                name,
                requests: vec![],
                extra: Extensions::new(),
            },
            requests: vec![],
            environments: vec![Environment::default()],
            secrets: SecretsFile::default(),
            hashes: BTreeMap::new(),
        })
    }
    pub fn open(root: &Path) -> Result<Self> {
        let root = root.canonicalize()?;
        recover(&root)?;
        let manifest: Manifest = read_json(&managed_path(&root, "duckie.json")?)?;
        let mut result = Self {
            root,
            manifest,
            requests: vec![],
            environments: vec![],
            secrets: SecretsFile::default(),
            hashes: BTreeMap::new(),
        };
        result.track("duckie.json")?;
        // Two passes: request files can be read at once, but the body and test files they
        // reference are only known after parsing, so they form a second batch.
        let names = result.manifest.requests.clone();
        let mut documents = Vec::with_capacity(names.len());
        let mut attachments = vec![];
        for (relative, bytes) in names.iter().zip(read_many(&result.root, &names)) {
            let bytes = bytes?;
            let value = document(&bytes, &result.path(relative)?)?;
            result.track_bytes(relative, &bytes);
            if let Some(file) = value["body"]["file"].as_str() {
                attachments.push(file.to_owned());
            }
            let tests = value["tests"]["file"].as_str().unwrap_or_default();
            if !tests.is_empty() {
                attachments.push(tests.to_owned());
            }
            documents.push(value);
        }
        let mut loaded = BTreeMap::new();
        for (relative, bytes) in attachments
            .iter()
            .zip(read_many(&result.root, &attachments))
        {
            let bytes = bytes?;
            let limit = if relative.starts_with("bodies/") {
                20 * MIB
            } else {
                MIB
            };
            if bytes.len() as u64 > limit {
                bail!(
                    "{relative} exceeds {} MiB; use File body mode or shorten the test",
                    limit / MIB
                );
            }
            result.track_bytes(relative, &bytes);
            loaded.insert(
                relative.clone(),
                String::from_utf8_lossy(&bytes).into_owned(),
            );
        }
        let mut ids = std::collections::HashSet::new();
        for mut value in documents {
            if let Some(file) = value["body"]["file"].as_str().map(str::to_owned) {
                let text = loaded.get(&file).cloned().unwrap_or_default();
                value["body"]["text"] = text.into();
                value["body"].as_object_mut().unwrap().remove("file");
            }
            let definition: RequestDefinition = serde_json::from_value(value)?;
            if !ids.insert(definition.id.clone()) {
                bail!("Duplicate request ID {}", definition.id);
            }
            let source = loaded
                .get(&definition.tests.file)
                .cloned()
                .unwrap_or_default();
            result.requests.push(StoredRequest { definition, source });
        }
        let env_dir = result.path("environments")?;
        if env_dir.exists() {
            for entry in fs::read_dir(env_dir)? {
                let entry = entry?;
                if entry.path().extension().is_some_and(|x| x == "json") {
                    let relative = format!("environments/{}", entry.file_name().to_string_lossy());
                    let bytes = result.read_tracked(&relative)?;
                    let path = result.path(&relative)?;
                    result
                        .environments
                        .push(serde_json::from_value(document(&bytes, &path)?)?);
                }
            }
        }
        if result.environments.is_empty() {
            result.environments.push(Environment::default());
        }
        let secret_path = result.path(".duckie/secrets.json")?;
        if secret_path.exists() {
            result.secrets = read_json(&secret_path)?;
        }
        result.track(".duckie/secrets.json")?;
        result.track("secrets.example.json")?;
        result.track(".gitignore")?;
        Ok(result)
    }
    fn path(&self, relative: &str) -> Result<PathBuf> {
        resolve(&self.root, relative)
    }
    fn track(&mut self, relative: &str) -> Result<()> {
        self.hashes
            .insert(relative.into(), disk_hash(&self.path(relative)?)?);
        Ok(())
    }
    /// Records the hash of content already read. Re-reading a file purely to hash it doubled
    /// both the I/O and the path validation performed by every open.
    fn track_bytes(&mut self, relative: &str, bytes: &[u8]) {
        self.hashes.insert(relative.into(), Some(hash(bytes)));
    }
    /// Reads a managed file once and records its hash from the same bytes.
    fn read_tracked(&mut self, relative: &str) -> Result<Vec<u8>> {
        let path = self.path(relative)?;
        let bytes = fs::read(&path).with_context(|| format!("Cannot read {}", path.display()))?;
        self.track_bytes(relative, &bytes);
        Ok(bytes)
    }
    pub fn changed_on_disk(&self) -> Result<Vec<String>> {
        let mut changed = vec![];
        for (name, saved) in &self.hashes {
            if &disk_hash(&self.path(name)?)? != saved {
                changed.push(name.clone());
            }
        }
        Ok(changed)
    }
    pub fn save(&mut self) -> Result<()> {
        let changed = self.changed_on_disk()?;
        if !changed.is_empty() {
            bail!(
                "Changed on disk: {}. Reload the collection, or save your draft to a new folder.",
                changed.join(", ")
            );
        }
        if self.path(".duckie/pending-save.json")?.exists() {
            bail!(
                "An interrupted save is pending. Reopen the collection to recover it before saving again."
            );
        }
        let mut writes = Vec::<(String, Vec<u8>)>::new();
        let mut paths = vec![];
        let mut example = SecretsFile::default();
        for req in &mut self.requests {
            // Stable, filesystem-safe IDs determine file names; imported/user IDs are validated as paths.
            let relative = format!("requests/{}.request.json", req.definition.id);
            resolve(&self.root, &relative)?;
            if req.definition.tests.file.is_empty() {
                req.definition.tests.file = format!("tests/{}.test.js", req.definition.id);
            }
            writes.push((
                req.definition.tests.file.clone(),
                req.source.as_bytes().to_vec(),
            ));
            let mut value = serde_json::to_value(&req.definition)?;
            if matches!(req.definition.body, Body::Json { .. } | Body::Text { .. }) {
                let extension = if matches!(req.definition.body, Body::Json { .. }) {
                    "json"
                } else {
                    "txt"
                };
                let body_path = format!("bodies/{}.{extension}", req.definition.id);
                let text = value["body"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                value["body"].as_object_mut().unwrap().remove("text");
                value["body"]["file"] = body_path.clone().into();
                writes.push((body_path, text.into_bytes()));
            }
            writes.push((relative.clone(), json(&value)?));
            paths.push(relative);
        }
        for env in &self.environments {
            if env.name.is_empty()
                || env
                    .name
                    .chars()
                    .any(|c| !(c.is_alphanumeric() || c == '-' || c == '_'))
            {
                bail!("Environment names can contain letters, numbers, '-' and '_' only");
            }
            writes.push((format!("environments/{}.json", env.name), json(env)?));
            let keys = example.environments.entry(env.name.clone()).or_default();
            for req in &self.requests {
                if let Some(b) = &req.definition.auth.bearer {
                    keys.insert(b.secret.clone(), String::new());
                }
                if let Some(k) = &req.definition.auth.api_key {
                    keys.insert(k.secret.clone(), String::new());
                }
            }
        }
        writes.push(("secrets.example.json".into(), json(&example)?));
        // Only values explicitly remembered by the UI enter this structure.
        if !self.secrets.environments.is_empty() {
            writes.push((".duckie/secrets.json".into(), json(&self.secrets)?));
        }
        let ignore_path = self.path(".gitignore")?;
        let mut ignore = fs::read_to_string(&ignore_path).unwrap_or_default();
        if !ignore.lines().any(|s| s == "/.duckie/") {
            if !ignore.is_empty() && !ignore.ends_with('\n') {
                ignore.push('\n');
            }
            ignore.push_str("/.duckie/\n");
        }
        writes.push((".gitignore".into(), ignore.into_bytes()));
        let mut manifest = self.manifest.clone();
        manifest.requests = paths;
        writes.push(("duckie.json".into(), json(&manifest)?)); // Commit manifest last.
        let mut entries = vec![];
        let mut seen = std::collections::HashSet::new();
        for (relative, bytes) in writes {
            if !seen.insert(relative.clone()) {
                bail!("Two managed resources use {relative}");
            }
            let path = self.path(&relative)?;
            let before = disk_hash(&path)?;
            if !self.hashes.contains_key(&relative) && before.is_some() {
                bail!("Refusing to overwrite an existing untracked file: {relative}");
            }
            entries.push(WriteEntry {
                path: relative,
                before,
                bytes,
            });
        }
        let journal = Journal { entries };
        let journal_path = self.path(".duckie/pending-save.json")?;
        atomic_write(&journal_path, &json(&journal)?)?;
        recover(&self.root)?;
        self.manifest = manifest;
        for entry in journal.entries {
            self.hashes.insert(entry.path, Some(hash(&entry.bytes)));
        }
        Ok(())
    }
}
pub fn load_secrets(path: &Path) -> Result<SecretsFile> {
    read_json(path)
}
pub fn export_secrets(path: &Path, secrets: &SecretsFile) -> Result<()> {
    atomic_write(path, &json(secrets)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_separation_and_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Collection::new(dir.path().into(), "Example".into()).unwrap();
        let mut r = RequestDefinition {
            body: Body::Json {
                text: "{\"ok\":true}".into(),
            },
            ..Default::default()
        };
        r.auth.bearer = Some(SecretBinding {
            secret: "token".into(),
        });
        r.extra.insert("x-team".into(), "test".into());
        c.requests.push(StoredRequest {
            definition: r,
            source: "test('ok',()=>{});".into(),
        });
        c.secrets.environments.insert(
            "dev".into(),
            Values::from([("token".into(), "private-value".into())]),
        );
        c.save().unwrap();
        let loaded = Collection::open(dir.path()).unwrap();
        assert_eq!(loaded.requests[0].source, "test('ok',()=>{});");
        assert!(loaded.requests[0].definition.extra.contains_key("x-team"));
        let request = fs::read_to_string(dir.path().join(&loaded.manifest.requests[0])).unwrap();
        assert!(!request.contains("private-value"));
        assert!(
            !fs::read_to_string(dir.path().join("secrets.example.json"))
                .unwrap()
                .contains("private-value")
        );
        fs::write(dir.path().join("duckie.json"), "external edit").unwrap();
        assert!(c.save().is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("duckie.json")).unwrap(),
            "external edit"
        );
    }
    #[test]
    fn paths_cannot_escape() {
        let d = tempfile::tempdir().unwrap();
        for p in ["../secret", "C:\\outside", "requests/../../bad", "/abs"] {
            assert!(managed_path(d.path(), p).is_err());
        }
    }
    #[test]
    fn interrupted_transaction_recovers() {
        let d = tempfile::tempdir().unwrap();
        let journal = Journal {
            entries: vec![WriteEntry {
                path: "tests/a.test.js".into(),
                before: None,
                bytes: b"// safe source".to_vec(),
            }],
        };
        atomic_write(
            &d.path().join(".duckie/pending-save.json"),
            &json(&journal).unwrap(),
        )
        .unwrap();
        recover(d.path()).unwrap();
        assert_eq!(
            fs::read(d.path().join("tests/a.test.js")).unwrap(),
            b"// safe source"
        );
    }
    #[test]
    fn extensions_roundtrip_through_save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let nested = serde_json::json!({
            "list": [1, 2, {"deep": true}],
            "map": {"a": "b"},
        });
        let mut c = Collection::new(dir.path().into(), "Example".into()).unwrap();
        c.manifest
            .extra
            .insert("manifestExt".into(), nested.clone());
        let mut req = RequestDefinition::default();
        req.query.push(Row::new("q", "1"));
        req.query[0].extra.insert("rowExt".into(), nested.clone());
        req.extra.insert("requestExt".into(), nested.clone());
        c.requests.push(StoredRequest {
            definition: req,
            source: String::new(),
        });
        c.environments[0]
            .extra
            .insert("envExt".into(), nested.clone());
        c.save().unwrap();
        let loaded = Collection::open(dir.path()).unwrap();
        assert_eq!(loaded.manifest.extra.get("manifestExt"), Some(&nested));
        assert_eq!(
            loaded.requests[0].definition.extra.get("requestExt"),
            Some(&nested)
        );
        assert_eq!(
            loaded.requests[0].definition.query[0].extra.get("rowExt"),
            Some(&nested)
        );
        assert_eq!(loaded.environments[0].extra.get("envExt"), Some(&nested));
    }
    #[test]
    fn future_schema_version_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Collection::new(dir.path().into(), "Example".into()).unwrap();
        c.save().unwrap();
        let path = dir.path().join("duckie.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["schemaVersion"] = serde_json::json!(SCHEMA_VERSION + 1);
        fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        match Collection::open(dir.path()) {
            Ok(_) => panic!("a newer schema version must not open"),
            Err(e) => assert!(e.to_string().contains("duckie.json"), "{e}"),
        }
    }
}
