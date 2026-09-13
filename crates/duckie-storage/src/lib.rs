//! Readable collections, conflict detection, and recoverable atomic saves.
use anyhow::{Context, Result, bail};
use duckie_model::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
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
    /// False when `source` and any file-backed body text are placeholders rather than the
    /// file's real content. `Collection::open` defers reading every body and test file so a
    /// large collection's restore does not hold all of them in memory at once; `ensure_loaded`
    /// fills in the real content for one request on first real use (viewing or sending it).
    pub loaded: bool,
    /// The body file this request's `body` text was read from, if any, kept only so
    /// `ensure_loaded` can find it again — parsing already removes `file` from `body` itself.
    body_file: Option<String>,
}
impl StoredRequest {
    /// A request whose `definition` and `source` are already the real content — the shape every
    /// caller outside this crate needs: a freshly imported or edited request has no file to defer
    /// reading from yet.
    pub fn new(definition: RequestDefinition, source: String) -> Self {
        Self {
            definition,
            source,
            loaded: true,
            body_file: None,
        }
    }
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
/// What each tracked file hashed to when the collection was last read or written.
#[derive(Clone)]
pub struct Watcher {
    root: PathBuf,
    hashes: BTreeMap<String, Option<String>>,
}
/// How a tracked file differs from what the collection last saw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    Modified,
    Added,
    Removed,
}
impl Watcher {
    /// Paths whose contents no longer match, cheapest first: a file that has not changed costs
    /// one read and one hash.
    pub fn changed(&self) -> Result<Vec<String>> {
        Ok(self.compare()?.into_iter().map(|(path, _)| path).collect())
    }
    pub fn compare(&self) -> Result<Vec<(String, Change)>> {
        let mut changed = vec![];
        for (name, saved) in &self.hashes {
            let now = disk_hash(&resolve(&self.root, name)?)?;
            if &now == saved {
                continue;
            }
            changed.push((
                name.clone(),
                match (saved, &now) {
                    (Some(_), None) => Change::Removed,
                    (None, Some(_)) => Change::Added,
                    _ => Change::Modified,
                },
            ));
        }
        Ok(changed)
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn disk_hash(path: &Path) -> Result<Option<String>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut digest = Sha256::new();
    let mut chunk = [0; 64 * 1024];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        digest.update(&chunk[..read]);
    }
    Ok(Some(format!("{:x}", digest.finalize())))
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
/// Resolves, reads and transforms many managed files in parallel, preserving order. `f` runs
/// right where each file is read, before its bytes cross back to the caller — so a caller that
/// only needs a hash, say, never has the whole batch's raw bytes resident at once just to
/// discard them a moment later, the way collecting every `Vec<u8>` first and mapping afterward
/// would.
///
/// Opening a collection is syscall-bound rather than CPU-bound: each file costs a path
/// validation and a read, and on Windows every open also pays antivirus filtering. Spreading
/// that across threads is what brings a thousand-request restore inside its budget. Each entry
/// keeps its own error so one bad file reports against its own name.
fn read_many<T: Send>(
    root: &Path,
    names: &[String],
    limit: impl Fn(&str) -> u64 + Sync,
    f: impl Fn(Vec<u8>) -> T + Sync,
) -> Vec<Result<T>> {
    let read = |name: &String| -> Result<T> {
        let path = resolve(root, name)?;
        let bytes = read_limited(&path, limit(name))?;
        Ok(f(bytes))
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
fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = File::open(path).with_context(|| format!("Cannot read {}", path.display()))?;
    read_limited_from(file, path, limit)
}
fn read_limited_from(reader: impl Read, path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .with_context(|| format!("Cannot read {}", path.display()))?;
    if bytes.len() as u64 > limit {
        bail!("{} exceeds {} MiB", path.display(), limit.div_ceil(MIB));
    }
    Ok(bytes)
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = read_limited(path, 20 * MIB)?;
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
/// Serializes a save transaction (disk-change check, journal write, journal recovery) across
/// processes and instances sharing this collection. An OS-level lock is used rather than a
/// plain marker file so a crashed holder cannot wedge every other instance: Windows and the
/// kernel release the lock the moment the holding process exits, the same guarantee the 24-hour
/// spool sweep already relies on. Blocks until acquired; a save transaction is brief, so the
/// wait is bounded by however long the other instance takes to finish its own save.
fn lock_transaction(root: &Path) -> Result<File> {
    let path = resolve(root, ".duckie/lock")?;
    fs::create_dir_all(path.parent().context("Missing parent directory")?)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("Cannot open {}", path.display()))?;
    file.lock()
        .with_context(|| format!("Cannot lock {}", path.display()))?;
    Ok(file)
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
        {
            let _lock = lock_transaction(&root)?;
            recover(&root)?;
        }
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
        // reference are only known after parsing, so they form a second batch. That second
        // batch is read here only to validate size and hash it for conflict detection — the
        // decoded text is dropped rather than kept, so opening a large collection does not
        // hold every body and test in memory before anything has actually been viewed.
        // `ensure_loaded` reads a given request's body and test again, from disk, on demand.
        let names = result.manifest.requests.clone();
        let mut documents = Vec::with_capacity(names.len());
        let mut attachments = vec![];
        for (relative, value) in
            names
                .iter()
                .zip(read_many(&result.root, &names, |_| 20 * MIB, |bytes| bytes))
        {
            let bytes = value?;
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
        // Hashed and discarded within the same read, one file at a time per thread: a large
        // batch of attachments is never resident all at once just to be thrown away afterward.
        for (relative, hashed) in attachments.iter().zip(read_many(
            &result.root,
            &attachments,
            |relative| {
                if relative.starts_with("bodies/") {
                    20 * MIB
                } else {
                    MIB
                }
            },
            |bytes| hash(&bytes),
        )) {
            result.hashes.insert(relative.clone(), Some(hashed?));
        }
        let mut ids = std::collections::HashSet::new();
        for mut value in documents {
            let body_file = value["body"]["file"].as_str().map(str::to_owned);
            if body_file.is_some() {
                // A placeholder the deserializer accepts; `ensure_loaded` fills in the real text.
                value["body"]["text"] = String::new().into();
                value["body"].as_object_mut().unwrap().remove("file");
            }
            let definition: RequestDefinition = serde_json::from_value(value)?;
            if !ids.insert(definition.id.clone()) {
                bail!("Duplicate request ID {}", definition.id);
            }
            let loaded = body_file.is_none() && definition.tests.file.is_empty();
            result.requests.push(StoredRequest {
                definition,
                source: String::new(),
                loaded,
                body_file,
            });
        }
        let env_dir = result.path("environments")?;
        if env_dir.exists() {
            for entry in fs::read_dir(env_dir)? {
                let entry = entry?;
                if entry.path().extension().is_some_and(|x| x == "json") {
                    let relative = format!("environments/{}", entry.file_name().to_string_lossy());
                    let bytes = result.read_tracked(&relative, 20 * MIB)?;
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
    fn read_tracked(&mut self, relative: &str, limit: u64) -> Result<Vec<u8>> {
        let path = self.path(relative)?;
        let bytes = read_limited(&path, limit)?;
        self.track_bytes(relative, &bytes);
        Ok(bytes)
    }
    pub fn changed_on_disk(&self) -> Result<Vec<String>> {
        self.watcher().changed()
    }
    /// Reads the real body text and test source for one request, if `open` deferred them. A
    /// plain read, not `read_tracked`: the hash captured at `open` is what conflict detection
    /// still compares against, so this never masks an external edit — it only fills in the
    /// content to show or send. Harmless to call on an already-loaded request.
    pub fn ensure_loaded(&mut self, id: &str) -> Result<()> {
        let Some(index) = self.requests.iter().position(|r| r.definition.id == id) else {
            return Ok(());
        };
        if self.requests[index].loaded {
            return Ok(());
        }
        let tests_file = self.requests[index].definition.tests.file.clone();
        let body_file = self.requests[index].body_file.clone();
        let read = |relative: &str, limit| -> Result<String> {
            let bytes = read_limited(&self.path(relative)?, limit)
                .with_context(|| format!("Cannot read {relative}"))?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        };
        let source = if tests_file.is_empty() {
            String::new()
        } else {
            read(&tests_file, MIB)?
        };
        let body_text = body_file
            .as_deref()
            .map(|relative| read(relative, 20 * MIB))
            .transpose()?;
        let req = &mut self.requests[index];
        req.source = source;
        if let Some(text) = body_text {
            match &mut req.definition.body {
                Body::Json { text: t } | Body::Text { text: t } => *t = text,
                _ => {}
            }
        }
        req.loaded = true;
        Ok(())
    }
    /// A handle that re-checks the tracked files without copying the collection's contents.
    /// Cloning a whole collection to answer "did anything change" would copy every body and test.
    pub fn watcher(&self) -> Watcher {
        Watcher {
            root: self.root.clone(),
            hashes: self.hashes.clone(),
        }
    }
    pub fn save(&mut self) -> Result<()> {
        let _lock = lock_transaction(&self.root)?;
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
    fn bounded_reads_stop_after_the_limit_probe() {
        struct CountingReader {
            bytes_read: usize,
        }
        impl Read for CountingReader {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                buffer.fill(b'x');
                self.bytes_read += buffer.len();
                Ok(buffer.len())
            }
        }

        let mut reader = CountingReader { bytes_read: 0 };
        let error = read_limited_from(&mut reader, Path::new("oversized"), 8).unwrap_err();
        assert!(error.to_string().contains("exceeds"), "{error}");
        assert_eq!(reader.bytes_read, 9, "the reader must stop at limit + 1");
    }

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
        c.requests
            .push(StoredRequest::new(r, "test('ok',()=>{});".into()));
        c.secrets.environments.insert(
            "dev".into(),
            Values::from([("token".into(), "private-value".into())]),
        );
        c.save().unwrap();
        let mut loaded = Collection::open(dir.path()).unwrap();
        assert!(
            !loaded.requests[0].loaded,
            "content is deferred until asked for"
        );
        let id = loaded.requests[0].definition.id.clone();
        loaded.ensure_loaded(&id).unwrap();
        assert!(loaded.requests[0].loaded);
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
        c.requests.push(StoredRequest::new(req, String::new()));
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
    fn the_watcher_classifies_what_changed_under_the_collection() {
        let dir = tempfile::tempdir().unwrap();
        let mut collection = Collection::new(dir.path().to_path_buf(), "watched".into()).unwrap();
        collection.requests.push(StoredRequest::new(
            RequestDefinition {
                id: "one".into(),
                body: Body::Json { text: "{}".into() },
                ..Default::default()
            },
            "test('a', () => {});".into(),
        ));
        collection.save().unwrap();
        let watcher = collection.watcher();
        assert!(watcher.compare().unwrap().is_empty(), "nothing changed yet");

        // Modified, removed and added are told apart rather than lumped together.
        let requests = dir.path().join("requests/one.request.json");
        let body = dir.path().join("bodies/one.json");
        fs::write(&requests, fs::read_to_string(&requests).unwrap() + " ").unwrap();
        fs::remove_file(&body).unwrap();
        let mut changed = watcher.compare().unwrap();
        changed.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            changed
                .iter()
                .map(|(path, change)| (path.as_str(), *change))
                .collect::<Vec<_>>(),
            [
                ("bodies/one.json", Change::Removed),
                ("requests/one.request.json", Change::Modified),
            ]
        );
        // A file the collection never tracked is not its business.
        fs::write(dir.path().join("unrelated.txt"), "x").unwrap();
        assert_eq!(watcher.compare().unwrap().len(), 2);

        // The watcher holds hashes only, so it keeps working after the collection is gone.
        drop(collection);
        assert_eq!(watcher.changed().unwrap().len(), 2);
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
    #[test]
    fn save_transactions_are_serialized_across_instances() {
        let dir = tempfile::tempdir().unwrap();
        Collection::new(dir.path().into(), "Example".into())
            .unwrap()
            .save()
            .unwrap();
        let root = dir.path().canonicalize().unwrap();

        // Stands in for another instance mid-transaction: `Collection::save` and the
        // recovery step in `Collection::open` both take this same lock.
        let held = lock_transaction(&root).unwrap();

        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (unblocked_tx, unblocked_rx) = std::sync::mpsc::channel::<()>();
        let other_root = root.clone();
        let handle = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let _second = lock_transaction(&other_root).unwrap();
            unblocked_tx.send(()).unwrap();
        });
        started_rx.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            unblocked_rx.try_recv().is_err(),
            "a second transaction acquired the lock while the first still held it"
        );
        drop(held);
        unblocked_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the second transaction should acquire the lock once released");
        handle.join().unwrap();
    }
    #[test]
    fn lazy_body_and_test_content_loads_on_demand() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Collection::new(dir.path().into(), "Example".into()).unwrap();
        c.requests.push(StoredRequest::new(
            RequestDefinition {
                id: "with-extras".into(),
                body: Body::Json {
                    text: "{\"n\":1}".into(),
                },
                ..Default::default()
            },
            "test('x', () => {});".into(),
        ));
        c.save().unwrap();

        let mut opened = Collection::open(dir.path()).unwrap();
        let with_extras = |c: &Collection| {
            c.requests
                .iter()
                .find(|r| r.definition.id == "with-extras")
                .unwrap()
                .clone()
        };
        let deferred = with_extras(&opened);
        assert!(!deferred.loaded, "content is deferred until asked for");
        assert!(deferred.source.is_empty());
        match &deferred.definition.body {
            Body::Json { text } => assert!(text.is_empty(), "body text deferred too"),
            other => panic!("expected Json, {}", serde_json::to_string(other).unwrap()),
        }

        // The hash needed to catch an external edit was still captured at open, unaffected by
        // deferring the content itself.
        fs::write(dir.path().join("bodies/with-extras.json"), "{}").unwrap();
        assert_eq!(
            opened.changed_on_disk().unwrap(),
            ["bodies/with-extras.json"]
        );
        // `save` writes the body's raw text as-is, with no reformatting, so this restores the
        // exact original bytes rather than an equivalent-but-differently-printed document.
        fs::write(dir.path().join("bodies/with-extras.json"), "{\"n\":1}").unwrap();
        assert!(opened.changed_on_disk().unwrap().is_empty());

        opened.ensure_loaded("with-extras").unwrap();
        let loaded = with_extras(&opened);
        assert!(loaded.loaded);
        assert_eq!(loaded.source, "test('x', () => {});");
        match &loaded.definition.body {
            Body::Json { text } => assert_eq!(text, "{\"n\":1}"),
            other => panic!("expected Json, {}", serde_json::to_string(other).unwrap()),
        }

        // Idempotent, and an unknown id is a no-op rather than an error.
        opened.ensure_loaded("with-extras").unwrap();
        opened.ensure_loaded("does-not-exist").unwrap();
    }
    #[test]
    fn lazy_loading_rechecks_attachment_size_with_a_bounded_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Collection::new(dir.path().into(), "Example".into()).unwrap();
        c.requests.push(StoredRequest::new(
            RequestDefinition {
                id: "growing".into(),
                body: Body::Json { text: "{}".into() },
                ..Default::default()
            },
            String::new(),
        ));
        c.save().unwrap();

        let mut opened = Collection::open(dir.path()).unwrap();
        let body = File::options()
            .write(true)
            .open(dir.path().join("bodies/growing.json"))
            .unwrap();
        body.set_len(20 * MIB + 1).unwrap();
        drop(body);

        let error = opened.ensure_loaded("growing").unwrap_err();
        assert!(format!("{error:#}").contains("exceeds 20 MiB"), "{error:#}");
        assert!(!opened.requests[0].loaded);
    }

    #[test]
    fn a_request_with_no_body_file_and_no_tests_needs_nothing_deferred() {
        // Duckie's own `save` always assigns a test file, so this shape — inline body text, no
        // tests file at all — only arises from a hand-authored or externally produced request.
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("requests")).unwrap();
        fs::write(
            dir.path().join("requests/bare.request.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "id": "bare",
                "name": "Bare",
                "folder": "",
                "method": "GET",
                "url": "",
                "body": {"kind": "text", "text": "inline"},
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("duckie.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "name": "Example",
                "requests": ["requests/bare.request.json"],
            }))
            .unwrap(),
        )
        .unwrap();
        let opened = Collection::open(dir.path()).unwrap();
        let bare = &opened.requests[0];
        assert!(bare.loaded, "no file-backed body or tests to defer");
        match &bare.definition.body {
            Body::Text { text } => assert_eq!(text, "inline"),
            other => panic!("expected Text, {}", serde_json::to_string(other).unwrap()),
        }
    }
}
