//! File and execution contracts. No GUI, transport, or JavaScript dependencies.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::Arc,
};

pub const MIB: u64 = 1024 * 1024;
pub type Values = BTreeMap<String, String>;
pub type Extensions = BTreeMap<String, serde_json::Value>;
pub fn new_id() -> String {
    format!("req_{}", uuid::Uuid::new_v4().simple())
}
fn yes() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    #[serde(default = "yes")]
    pub enabled: bool,
    pub name: String,
    pub value: String,
    /// Original encoded query component; discarded only when that row is edited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(default, flatten)]
    pub extra: Extensions,
}
impl Row {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            enabled: true,
            name: name.into(),
            value: value.into(),
            raw: None,
            extra: Extensions::new(),
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SecretBinding {
    pub secret: String,
}
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiKey {
    pub header: String,
    pub secret: String,
}
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Auth {
    pub bearer: Option<SecretBinding>,
    pub api_key: Option<ApiKey>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub enabled: bool,
    pub name: String,
    pub value: String,
    pub file: bool,
}
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Body {
    #[default]
    None,
    Json {
        text: String,
    },
    Text {
        text: String,
    },
    Form {
        rows: Vec<Row>,
    },
    Multipart {
        parts: Vec<Part>,
    },
    File {
        path: String,
        #[serde(rename = "contentType")]
        content_type: String,
    },
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct TestDefinition {
    pub enabled: bool,
    pub file: String,
}
impl Default for TestDefinition {
    fn default() -> Self {
        Self {
            enabled: true,
            file: String::new(),
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum ProxyMode {
    #[default]
    System,
    Direct,
    Explicit {
        url: String,
        bypass: String,
    },
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RequestDefinition {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub folder: String,
    pub method: String,
    pub url: String,
    pub variables: Values,
    pub query: Vec<Row>,
    pub headers: Vec<Row>,
    pub auth: Auth,
    pub body: Body,
    pub tests: TestDefinition,
    pub timeout_ms: u64,
    pub encoded_limit: u64,
    pub decoded_limit: u64,
    pub proxy: ProxyMode,
    /// Import features that must be corrected before the request can run.
    pub blockers: Vec<String>,
    #[serde(flatten)]
    pub extra: Extensions,
}
impl Default for RequestDefinition {
    fn default() -> Self {
        Self {
            schema_version: 1,
            id: new_id(),
            name: "Untitled request".into(),
            folder: String::new(),
            method: "GET".into(),
            url: String::new(),
            variables: Values::new(),
            query: vec![],
            headers: vec![],
            auth: Auth::default(),
            body: Body::None,
            tests: TestDefinition::default(),
            timeout_ms: 30_000,
            encoded_limit: 50 * MIB,
            decoded_limit: 50 * MIB,
            proxy: ProxyMode::System,
            blockers: vec![],
            extra: Extensions::new(),
        }
    }
}
impl RequestDefinition {
    pub fn address(&self) -> String {
        let query: Vec<_> = self
            .query
            .iter()
            .filter(|r| r.enabled)
            .map(|r| {
                r.raw
                    .clone()
                    .unwrap_or_else(|| encode_pair(&r.name, &r.value))
            })
            .collect();
        if query.is_empty() {
            self.url.clone()
        } else {
            format!("{}?{}", self.url, query.join("&"))
        }
    }
    pub fn set_address(&mut self, address: &str) {
        // Braces are accepted in templates. Split at a literal '?' only; fragments are validated at preparation.
        if let Some((base, query)) = address.split_once('?') {
            self.url = base.into();
            self.query = query
                .split('&')
                .filter(|p| !p.is_empty())
                .map(|raw| {
                    let (name, value) = url::form_urlencoded::parse(raw.as_bytes())
                        .next()
                        .unwrap_or_default();
                    let mut row = Row::new(name, value);
                    row.raw = Some(raw.into());
                    row
                })
                .collect();
        } else {
            self.url = address.into();
            self.query.clear();
        }
    }
}
pub fn encode_pair(name: &str, value: &str) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .append_pair(name, value)
        .finish()
}
pub fn resolve_location(base: &str, location: &str) -> Result<String, String> {
    let url = url::Url::parse(base)
        .and_then(|u| u.join(location))
        .map_err(|_| "Invalid redirect Location".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Redirect Location must be an HTTP(S) URL without credentials".into());
    }
    Ok(url.into())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub schema_version: u32,
    pub name: String,
    pub values: Values,
    #[serde(default, flatten)]
    pub extra: Extensions,
}
impl Default for Environment {
    fn default() -> Self {
        Self {
            schema_version: 1,
            name: "dev".into(),
            values: Values::new(),
            extra: Extensions::new(),
        }
    }
}
/// Deliberately no Debug/Serialize: resolved credentials must not enter logs or collection files.
#[derive(Clone, Default)]
pub struct EnvironmentSnapshot {
    pub name: String,
    pub values: Values,
    pub secrets: Values,
}
#[derive(Clone, Default)]
pub struct RunBindings(pub Values);
pub fn interpolate(
    input: &str,
    request: &Values,
    env: &EnvironmentSnapshot,
    bindings: &RunBindings,
) -> Result<String> {
    let mut remaining = input;
    let mut out = String::new();
    while let Some(start) = remaining.find("{{") {
        out.push_str(&remaining[..start]);
        let end = remaining[start + 2..]
            .find("}}")
            .ok_or_else(|| anyhow::anyhow!("Unclosed variable reference"))?
            + start
            + 2;
        let key = remaining[start + 2..end].trim();
        let (ns, name) = key
            .split_once('.')
            .ok_or_else(|| anyhow::anyhow!("Use env, request, or secret namespace"))?;
        let value = match ns {
            "env" => env.values.get(name),
            "request" => bindings.0.get(name).or_else(|| request.get(name)),
            "secret" => env.secrets.get(name),
            _ => None,
        };
        match value.filter(|v| !v.is_empty()) {
            Some(value) => out.push_str(value),
            None => bail!("No value for {key} in {}", env.name),
        }
        remaining = &remaining[end + 2..];
    }
    out.push_str(remaining);
    Ok(out)
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSummary {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub environment: String,
    pub revision: u64,
    pub timeout_ms: u64,
}
#[derive(Clone)]
pub struct PreparedRequest {
    pub id: String,
    pub run_id: String,
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Body,
    pub timeout_ms: u64,
    pub encoded_limit: u64,
    pub decoded_limit: u64,
    pub proxy: ProxyMode,
    pub summary: RequestSummary,
}
pub fn prepare(
    req: &RequestDefinition,
    env: &EnvironmentSnapshot,
    bindings: &RunBindings,
    revision: u64,
) -> Result<PreparedRequest> {
    if !req.blockers.is_empty() {
        bail!(
            "Resolve import issues in Settings before Send: {}",
            req.blockers.join("; ")
        );
    }
    if req.timeout_ms == 0 || req.timeout_ms > 3_600_000 {
        bail!("Timeout must be between 1 ms and 1 hour");
    }
    if req.encoded_limit == 0
        || req.decoded_limit == 0
        || req.encoded_limit > 1024 * MIB
        || req.decoded_limit > 1024 * MIB
    {
        bail!("Response limits must be between 1 byte and 1 GiB");
    }
    let resolve = |s: &str| interpolate(s, &req.variables, env, bindings);
    let base = resolve(&req.url)?;
    let mut parsed = url::Url::parse(&base)
        .map_err(|_| anyhow::anyhow!("Enter a valid absolute HTTP(S) URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        bail!("Only HTTP and HTTPS URLs are supported");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("URL credentials are unsupported; use Auth");
    }
    if parsed.fragment().is_some() {
        bail!("Remove the URL fragment (#...) before Send");
    }
    if parsed.query().is_some() && !req.query.is_empty() {
        bail!("A URL variable contains a query; move its parameters into Params");
    }
    if !req.query.is_empty() {
        let mut query = Vec::new();
        for row in req.query.iter().filter(|r| r.enabled) {
            // Re-encode templated rows after substitution, preserving untouched literal bytes.
            if let Some(raw) = &row.raw
                && !raw.contains("{{")
                && !row.name.contains("{{")
                && !row.value.contains("{{")
            {
                query.push(raw.clone());
            } else {
                query.push(encode_pair(&resolve(&row.name)?, &resolve(&row.value)?));
            }
        }
        if !query.is_empty() {
            parsed.set_query(Some(&query.join("&")));
        }
    }
    let mut headers = Vec::new();
    for row in req
        .headers
        .iter()
        .filter(|r| r.enabled && !r.name.is_empty())
    {
        headers.push((resolve(&row.name)?, resolve(&row.value)?));
    }
    let mut sensitive_headers = vec![
        "authorization".to_string(),
        "proxy-authorization".into(),
        "cookie".into(),
    ];
    let mut add_auth = |name: &str, key: &str, bearer: bool| -> Result<()> {
        if headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
            bail!("Auth conflicts with header {name}; remove the duplicate binding/header");
        }
        let value = env
            .secrets
            .get(key)
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("No value for {key} in {}", env.name))?;
        let value = value.trim();
        let value = if bearer {
            let token = if value
                .get(..7)
                .is_some_and(|p| p.eq_ignore_ascii_case("Bearer "))
            {
                value[7..].trim()
            } else {
                value
            };
            if token.is_empty() {
                bail!("Bearer token is empty");
            }
            format!("Bearer {token}")
        } else {
            value.into()
        };
        sensitive_headers.push(name.to_ascii_lowercase());
        headers.push((name.into(), value));
        Ok(())
    };
    if let Some(binding) = &req.auth.bearer {
        add_auth("Authorization", &binding.secret, true)?;
    }
    if let Some(binding) = &req.auth.api_key {
        add_auth(&binding.header, &binding.secret, false)?;
    }
    let mut body = req.body.clone();
    match &mut body {
        Body::Json { text } | Body::Text { text } => *text = resolve(text)?,
        Body::Form { rows } => {
            for row in rows.iter_mut().filter(|r| r.enabled) {
                row.name = resolve(&row.name)?;
                row.value = resolve(&row.value)?;
            }
        }
        Body::Multipart { parts } => {
            for part in parts.iter_mut().filter(|p| p.enabled) {
                part.name = resolve(&part.name)?;
                part.value = resolve(&part.value)?;
                if part.file && !PathBuf::from(&part.value).is_file() {
                    bail!("A selected multipart file is missing");
                }
            }
        }
        Body::File { path, .. } => {
            if !PathBuf::from(&*path).is_file() {
                bail!("Select an existing body file");
            }
        }
        Body::None => {}
    }
    let content_type = match &body {
        Body::Json { .. } => Some("application/json"),
        Body::Text { .. } => Some("text/plain; charset=utf-8"),
        Body::Form { .. } => Some("application/x-www-form-urlencoded"),
        Body::File { content_type, .. } if !content_type.is_empty() => Some(content_type.as_str()),
        _ => None,
    };
    if let Some(ct) = content_type
        && !headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("content-type"))
    {
        headers.push(("Content-Type".into(), ct.into()));
    }
    if matches!(body, Body::Multipart { .. })
        && headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("content-type"))
    {
        bail!("Remove the Content-Type header so Multipart can generate its boundary");
    }
    let redact = |value: &str| {
        let mut value = value.to_string();
        for secret in env.secrets.values().filter(|v| !v.is_empty()) {
            value = value.replace(secret, "[redacted]");
            let encoded =
                url::form_urlencoded::byte_serialize(secret.as_bytes()).collect::<String>();
            value = value.replace(&encoded, "[redacted]");
        }
        value
    };
    let summary = RequestSummary {
        method: req.method.clone(),
        url: redact(parsed.as_str()),
        headers: headers
            .iter()
            .map(|(n, v)| {
                (
                    n.clone(),
                    if sensitive_headers.iter().any(|s| n.eq_ignore_ascii_case(s)) {
                        "[redacted]".into()
                    } else {
                        redact(v)
                    },
                )
            })
            .collect(),
        environment: env.name.clone(),
        revision,
        timeout_ms: req.timeout_ms,
    };
    Ok(PreparedRequest {
        id: req.id.clone(),
        run_id: uuid::Uuid::new_v4().to_string(),
        url: parsed.into(),
        method: req.method.clone(),
        headers,
        body,
        timeout_ms: req.timeout_ms,
        encoded_limit: req.encoded_limit,
        decoded_limit: req.decoded_limit,
        proxy: req.proxy.clone(),
        summary,
    })
}

#[derive(Clone)]
pub enum BodyHandle {
    Memory(Arc<Vec<u8>>),
    File {
        path: Arc<tempfile::TempPath>,
        len: u64,
    },
}
impl Default for BodyHandle {
    fn default() -> Self {
        Self::Memory(Arc::new(Vec::new()))
    }
}
impl BodyHandle {
    pub fn len(&self) -> u64 {
        match self {
            Self::Memory(v) => v.len() as u64,
            Self::File { len, .. } => *len,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn read(&self, offset: u64, limit: u64) -> std::io::Result<Vec<u8>> {
        let n = self.len().saturating_sub(offset).min(limit) as usize;
        match self {
            Self::Memory(v) => Ok(v
                .get(offset as usize..offset as usize + n)
                .unwrap_or_default()
                .to_vec()),
            Self::File { path, .. } => {
                let mut f = std::fs::File::open(path.as_ref())?;
                f.seek(SeekFrom::Start(offset))?;
                let mut v = vec![0; n];
                f.read_exact(&mut v)?;
                Ok(v)
            }
        }
    }
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        match self {
            Self::Memory(v) => std::fs::write(path, v.as_ref()),
            Self::File { path: source, .. } => std::fs::copy(source.as_ref(), path).map(|_| ()),
        }
    }
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum Outcome {
    Complete,
    Cancelled,
    Timeout,
    ConnectionError,
    TlsError,
    DnsError,
    BodyError,
    EncodedLimit,
    DecodedLimit,
}
impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Complete => "Complete",
                Self::Cancelled => "Cancelled — the server may already have processed this request",
                Self::Timeout => "Request timed out",
                Self::ConnectionError => "Connection failed",
                Self::TlsError => "TLS connection failed",
                Self::DnsError => "DNS lookup failed",
                Self::BodyError => "Response body could not be read or decoded",
                Self::EncodedLimit => "Response incomplete: encoded body limit reached",
                Self::DecodedLimit => "Response incomplete: decoded body limit reached",
            }
        )
    }
}
#[derive(Clone)]
pub struct ExecutionResult {
    pub request_id: String,
    pub run_id: String,
    pub summary: RequestSummary,
    pub outcome: Outcome,
    pub status: Option<u16>,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: BodyHandle,
    pub encoded_bytes: u64,
    pub duration_ms: u64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReport {
    pub tests: Vec<TestCase>,
    pub suite_error: Option<String>,
    pub console: Vec<String>,
    pub duration_ms: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct TestCase {
    pub name: String,
    pub passed: bool,
    pub error: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_preserves_literals_and_duplicates() {
        let mut r = RequestDefinition::default();
        r.set_address("https://example.test/a?x=%2f&x=a+b&bare");
        assert_eq!(r.address(), "https://example.test/a?x=%2f&x=a+b&bare");
        let p = prepare(
            &r,
            &EnvironmentSnapshot::default(),
            &RunBindings::default(),
            1,
        )
        .unwrap();
        assert_eq!(p.url, r.address());
    }
    #[test]
    fn substitution_is_one_pass_and_bindings_cannot_override_secrets() {
        let mut e = EnvironmentSnapshot::default();
        e.values.insert("base".into(), "{{secret.key}}".into());
        e.secrets.insert("key".into(), "token".into());
        let bindings = RunBindings(Values::from([("key".into(), "wrong".into())]));
        assert_eq!(
            interpolate("{{env.base}}/{{secret.key}}", &Values::new(), &e, &bindings).unwrap(),
            "{{secret.key}}/token"
        );
    }
    #[test]
    fn credentials_are_normalized_redacted_and_collisions_rejected() {
        let mut r = RequestDefinition {
            url: "https://example.test".into(),
            ..Default::default()
        };
        r.auth.bearer = Some(SecretBinding {
            secret: "token".into(),
        });
        let mut e = EnvironmentSnapshot::default();
        e.secrets.insert("token".into(), "Bearer abc".into());
        let p = prepare(&r, &e, &RunBindings::default(), 1).unwrap();
        assert_eq!(p.headers[0].1, "Bearer abc");
        assert_eq!(p.summary.headers[0].1, "[redacted]");
        r.headers.push(Row::new("authorization", "manual"));
        assert!(prepare(&r, &e, &RunBindings::default(), 1).is_err());
        assert!(
            prepare(
                &r,
                &EnvironmentSnapshot::default(),
                &RunBindings::default(),
                1
            )
            .is_err()
        );
    }
}
