//! Single-run orchestration. Callers receive response and tests as independent events.
use anyhow::{Result, bail};
use duckie_model::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadlessReport {
    pub request_id: String,
    pub request_name: String,
    pub environment: String,
    pub revision: u64,
    pub outcome: Outcome,
    pub status: Option<u16>,
    pub duration_ms: u64,
    pub response_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_snippet: Option<String>,
    pub tests: Option<TestReport>,
    pub error: Option<String>,
}
impl HeadlessReport {
    pub fn assertion_failed(&self) -> bool {
        self.tests
            .as_ref()
            .is_some_and(|r| r.suite_error.is_some() || r.tests.iter().any(|t| !t.passed))
    }
    pub fn execution_failed(&self) -> bool {
        self.error.is_some() || self.outcome != Outcome::Complete
    }
}

/// Shared, renderer-free request path used by command-line automation.
pub async fn execute_headless(
    http: &duckie_http::HttpEngine,
    request: &RequestDefinition,
    source: String,
    environment: EnvironmentSnapshot,
    cancel: CancellationToken,
    include_response_snippet: bool,
    revision: u64,
) -> HeadlessReport {
    let failure = |error: anyhow::Error| HeadlessReport {
        request_id: request.id.clone(),
        request_name: request.name.clone(),
        environment: environment.name.clone(),
        revision,
        outcome: Outcome::ConnectionError,
        status: None,
        duration_ms: 0,
        response_bytes: 0,
        response_snippet: None,
        tests: None,
        error: Some(error.to_string()),
    };
    let prepared = match prepare(request, &environment, &RunBindings::default(), revision) {
        Ok(value) => value,
        Err(error) => return failure(error),
    };
    let result = match http.execute(prepared, cancel.clone()).await {
        Ok(value) => value,
        Err(error) => return failure(error),
    };
    let mut report = HeadlessReport {
        request_id: request.id.clone(),
        request_name: request.name.clone(),
        environment: environment.name.clone(),
        revision: result.summary.revision,
        outcome: result.outcome.clone(),
        status: result.status,
        duration_ms: result.duration_ms,
        response_bytes: result.body.len(),
        response_snippet: if include_response_snippet {
            result.body.read(0, 4096).ok().map(|b| {
                let mut text = String::from_utf8_lossy(&b).into_owned();
                for secret in environment.secrets.values().filter(|v| !v.is_empty()) {
                    text = text.replace(secret, "[redacted]");
                }
                text
            })
        } else {
            None
        },
        tests: None,
        error: result.body_error.clone(),
    };
    if result.outcome == Outcome::Complete && request.tests.enabled && !source.trim().is_empty() {
        match duckie_tests::TestInput::from_result(source, &result, environment.values) {
            Ok(input) => match duckie_tests::evaluate(input, cancel).await {
                Ok(tests) => report.tests = Some(tests),
                Err(error) => report.error = Some(error.to_string()),
            },
            Err(error) => report.error = Some(error.to_string()),
        }
    }
    report
}

/// Runs frozen inputs sequentially in their supplied order and stops starting new work after
/// cancellation. A fresh environment snapshot is cloned for each request so execution cannot
/// mutate the selected collection environment.
pub async fn execute_headless_suite(
    http: &duckie_http::HttpEngine,
    requests: Vec<(RequestDefinition, String, u64)>,
    environment: EnvironmentSnapshot,
    cancel: CancellationToken,
    stop_on_failure: bool,
    include_response_snippets: bool,
) -> Vec<HeadlessReport> {
    let mut reports = Vec::with_capacity(requests.len());
    for (request, source, revision) in requests {
        if cancel.is_cancelled() {
            break;
        }
        reports.push(
            execute_headless(
                http,
                &request,
                source,
                environment.clone(),
                cancel.clone(),
                include_response_snippets,
                revision,
            )
            .await,
        );
        if stop_on_failure
            && reports
                .last()
                .is_some_and(|r| r.execution_failed() || r.assertion_failed())
        {
            break;
        }
    }
    reports
}

fn xml(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
pub fn junit(reports: &[HeadlessReport]) -> String {
    let tests: usize = reports
        .iter()
        .map(|r| r.tests.as_ref().map_or(1, |t| t.tests.len().max(1)))
        .sum();
    let failures: usize = reports
        .iter()
        .map(|r| {
            if r.execution_failed() {
                1
            } else {
                r.tests.as_ref().map_or(0, |t| {
                    t.tests.iter().filter(|c| !c.passed).count()
                        + usize::from(t.suite_error.is_some())
                })
            }
        })
        .sum();
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"Duckie\" tests=\"{tests}\" failures=\"{failures}\">\n"
    );
    for r in reports {
        if let Some(t) = &r.tests {
            if t.tests.is_empty() {
                out.push_str(&format!(
                    "  <testcase classname=\"{}\" name=\"request\" time=\"{:.3}\">",
                    xml(&r.request_name),
                    r.duration_ms as f64 / 1000.0
                ));
                if let Some(e) = t.suite_error.as_ref().or(r.error.as_ref()) {
                    out.push_str(&format!("<failure message=\"{}\" />", xml(e)));
                }
                out.push_str("</testcase>\n");
            }
            for c in &t.tests {
                out.push_str(&format!(
                    "  <testcase classname=\"{}\" name=\"{}\" time=\"{:.3}\">",
                    xml(&r.request_name),
                    xml(&c.name),
                    t.duration_ms as f64 / 1000.0
                ));
                if !c.passed {
                    out.push_str(&format!(
                        "<failure message=\"{}\" />",
                        xml(c.error.as_deref().unwrap_or("Assertion failed"))
                    ));
                }
                out.push_str("</testcase>\n");
            }
        } else {
            out.push_str(&format!(
                "  <testcase classname=\"{}\" name=\"request\" time=\"{:.3}\">",
                xml(&r.request_name),
                r.duration_ms as f64 / 1000.0
            ));
            if let Some(e) = &r.error {
                out.push_str(&format!("<failure message=\"{}\" />", xml(e)));
            }
            out.push_str("</testcase>\n");
        }
    }
    out.push_str("</testsuite>\n");
    out
}
pub fn html(reports: &[HeadlessReport]) -> String {
    let mut rows = String::new();
    for r in reports {
        let state = if r.execution_failed() {
            "ERROR"
        } else if r.assertion_failed() {
            "FAIL"
        } else {
            "PASS"
        };
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{} ms</td></tr>",
            xml(state),
            xml(&r.request_name),
            xml(&r.environment),
            r.status.map_or("-".into(), |s| s.to_string()),
            r.duration_ms
        ));
    }
    format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Duckie report</title><style>body{{font:14px system-ui;margin:2rem}}table{{border-collapse:collapse}}td,th{{padding:.5rem;border:1px solid #ccc}}</style><h1>Duckie run report</h1><table><tr><th>Result</th><th>Request</th><th>Environment</th><th>Status</th><th>Duration</th></tr>{rows}</table>"
    )
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub status: Option<(Option<u16>, Option<u16>)>,
    pub headers: Vec<String>,
    pub differences: Vec<String>,
    pub truncated: bool,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Baseline {
    pub status: Option<u16>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl Baseline {
    pub fn from_result(result: &ExecutionResult) -> Result<Self> {
        if result.body.len() > 2 * MIB {
            bail!("Baseline bodies are limited to 2 MiB")
        }
        Ok(Self {
            status: result.status,
            headers: result.headers.clone(),
            body: result.body.read(0, 2 * MIB)?,
        })
    }
    pub fn load(path: &std::path::Path) -> Result<Self> {
        use std::io::Read;
        let mut bytes = vec![];
        std::fs::File::open(path)?
            .take(12 * MIB + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > 12 * MIB {
            bail!("Baseline file exceeds 12 MiB")
        }
        let value: Self = serde_json::from_slice(&bytes)?;
        if value.body.len() as u64 > 2 * MIB {
            bail!("Baseline body exceeds 2 MiB")
        }
        Ok(value)
    }
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}
pub fn compare_baseline(
    left: &ExecutionResult,
    right: &Baseline,
    ignored: &[String],
) -> Comparison {
    let synthetic = ExecutionResult {
        request_id: "baseline".into(),
        run_id: "baseline".into(),
        summary: left.summary.clone(),
        outcome: Outcome::Complete,
        status: right.status,
        status_text: String::new(),
        headers: right.headers.clone(),
        body_error: None,
        body: BodyHandle::Memory(Arc::new(right.body.clone())),
        encoded_bytes: right.body.len() as u64,
        duration_ms: 0,
        diagnostics: ResponseDiagnostics::default(),
    };
    compare(left, &synthetic, ignored)
}
pub fn compare(left: &ExecutionResult, right: &ExecutionResult, ignored: &[String]) -> Comparison {
    let status = (left.status != right.status).then_some((left.status, right.status));
    let mut headers = vec![];
    let normalize = |values: &[(String, String)]| {
        let mut v = values
            .iter()
            .map(|(n, v)| (n.to_ascii_lowercase(), v.clone()))
            .collect::<Vec<_>>();
        v.sort();
        v
    };
    let lh = normalize(&left.headers);
    let rh = normalize(&right.headers);
    for item in lh.iter().filter(|h| !rh.contains(h)) {
        headers.push(format!("- {}: {}", item.0, item.1));
    }
    for item in rh.iter().filter(|h| !lh.contains(h)) {
        headers.push(format!("+ {}: {}", item.0, item.1));
    }
    let cap = 2 * MIB;
    let lb = left.body.read(0, cap).unwrap_or_default();
    let rb = right.body.read(0, cap).unwrap_or_default();
    let truncated = left.body.len() > cap || right.body.len() > cap;
    let mut differences = vec![];
    if let (Ok(mut l), Ok(mut r)) = (
        serde_json::from_slice::<serde_json::Value>(&lb),
        serde_json::from_slice::<serde_json::Value>(&rb),
    ) {
        for p in ignored {
            if let Some(slot) = l.pointer_mut(p) {
                *slot = serde_json::Value::Null
            }
            if let Some(slot) = r.pointer_mut(p) {
                *slot = serde_json::Value::Null
            }
        }
        fn walk(path: &str, l: &serde_json::Value, r: &serde_json::Value, out: &mut Vec<String>) {
            if out.len() >= 500 {
                return;
            }
            match (l, r) {
                (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
                    for key in a
                        .keys()
                        .chain(b.keys())
                        .collect::<std::collections::BTreeSet<_>>()
                    {
                        let p = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                        match (a.get(key), b.get(key)) {
                            (Some(x), Some(y)) => walk(&p, x, y, out),
                            (Some(_), None) => out.push(format!("- {p}")),
                            (None, Some(_)) => out.push(format!("+ {p}")),
                            _ => {}
                        }
                    }
                }
                (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
                    for i in 0..a.len().max(b.len()) {
                        let p = format!("{path}/{i}");
                        match (a.get(i), b.get(i)) {
                            (Some(x), Some(y)) => walk(&p, x, y, out),
                            (Some(_), None) => out.push(format!("- {p}")),
                            (None, Some(_)) => out.push(format!("+ {p}")),
                            _ => {}
                        }
                    }
                }
                _ if l != r => out.push(format!(
                    "~ {}: {} → {}",
                    if path.is_empty() { "/" } else { path },
                    bounded(l),
                    bounded(r)
                )),
                _ => {}
            }
        }
        fn bounded(v: &serde_json::Value) -> String {
            let mut s = serde_json::to_string(v).unwrap_or_default();
            if s.len() > 160 {
                s.truncate(160);
                s.push('…')
            }
            s
        }
        walk("", &l, &r, &mut differences);
    } else if lb != rb {
        let l = String::from_utf8_lossy(&lb);
        let r = String::from_utf8_lossy(&rb);
        for (i, (a, b)) in l.lines().zip(r.lines()).enumerate() {
            if a != b {
                differences.push(format!(
                    "line {}: {} → {}",
                    i + 1,
                    a.chars().take(160).collect::<String>(),
                    b.chars().take(160).collect::<String>()
                ));
                if differences.len() >= 200 {
                    break;
                }
            }
        }
        if differences.is_empty() {
            differences.push("Body length or trailing lines changed".into());
        }
    }
    Comparison {
        status,
        headers,
        differences,
        truncated,
    }
}

pub enum RunEvent {
    Response(ExecutionResult),
    Tests {
        request_id: String,
        run_id: String,
        revision: u64,
        report: TestReport,
    },
    Error {
        request_id: String,
        message: String,
    },
    Finished,
}
pub struct ExecutionService {
    runtime: tokio::runtime::Runtime,
    http: Arc<duckie_http::HttpEngine>,
    busy: Arc<AtomicBool>,
    sender: mpsc::Sender<RunEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl ExecutionService {
    pub fn new(
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<(Self, mpsc::Receiver<RunEvent>)> {
        let (sender, receiver) = mpsc::channel(8);
        Ok((
            Self {
                runtime: tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()?,
                http: Arc::new(duckie_http::HttpEngine::default()),
                busy: Arc::new(AtomicBool::new(false)),
                sender,
                wake: Arc::new(wake),
            },
            receiver,
        ))
    }
    pub fn runtime(&self) -> &tokio::runtime::Runtime {
        &self.runtime
    }
    pub fn http(&self) -> Arc<duckie_http::HttpEngine> {
        self.http.clone()
    }
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }
    pub fn send(
        &self,
        request: PreparedRequest,
        source: String,
        auto_test: bool,
        environment: Values,
    ) -> Result<(CancellationToken, duckie_http::Progress)> {
        if self.busy.swap(true, Ordering::AcqRel) {
            bail!("A request or test evaluation is already running");
        }
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let progress = duckie_http::Progress::default();
        let watched = progress.clone();
        let http = self.http.clone();
        let sender = self.sender.clone();
        let wake = self.wake.clone();
        let busy = self.busy.clone();
        self.runtime.spawn(async move {
            let request_id = request.id.clone();
            match http.execute_watched(request, token.clone(), watched).await {
                Ok(result) => {
                    let evaluate = auto_test
                        && !source.trim().is_empty()
                        && result.outcome == Outcome::Complete;
                    let _ = sender.send(RunEvent::Response(result.clone())).await;
                    wake();
                    if evaluate {
                        let id = result.request_id.clone();
                        let run_id = result.run_id.clone();
                        let revision = result.summary.revision;
                        let input = tokio::task::spawn_blocking(move || {
                            duckie_tests::TestInput::from_result(source, &result, environment)
                        })
                        .await;
                        let report = match input {
                            Ok(Ok(input)) => duckie_tests::evaluate(input, token).await,
                            Ok(Err(e)) => Err(e),
                            Err(e) => Err(e.into()),
                        };
                        let report = report.unwrap_or_else(|e| TestReport {
                            suite_error: Some(e.to_string()),
                            ..Default::default()
                        });
                        let _ = sender
                            .send(RunEvent::Tests {
                                request_id: id,
                                run_id,
                                revision,
                                report,
                            })
                            .await;
                        wake();
                    }
                }
                Err(e) => {
                    let _ = sender
                        .send(RunEvent::Error {
                            request_id,
                            message: e.to_string(),
                        })
                        .await;
                    wake();
                }
            }
            busy.store(false, Ordering::Release);
            let _ = sender.send(RunEvent::Finished).await;
            wake();
        });
        Ok((cancel, progress))
    }
    pub fn rerun_tests(
        &self,
        result: ExecutionResult,
        source: String,
        revision: u64,
        environment: Values,
    ) -> Result<CancellationToken> {
        if self.busy.swap(true, Ordering::AcqRel) {
            bail!("A request or test evaluation is already running");
        }
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let sender = self.sender.clone();
        let wake = self.wake.clone();
        let busy = self.busy.clone();
        self.runtime.spawn(async move {
            let request_id = result.request_id.clone();
            let run_id = result.run_id.clone();
            let input = tokio::task::spawn_blocking(move || {
                duckie_tests::TestInput::from_result(source, &result, environment)
            })
            .await;
            let report = match input {
                Ok(Ok(input)) => duckie_tests::evaluate(input, token).await,
                Ok(Err(e)) => Err(e),
                Err(e) => Err(e.into()),
            };
            let report = report.unwrap_or_else(|e| TestReport {
                suite_error: Some(e.to_string()),
                ..Default::default()
            });
            let _ = sender
                .send(RunEvent::Tests {
                    request_id,
                    run_id,
                    revision,
                    report,
                })
                .await;
            wake();
            busy.store(false, Ordering::Release);
            let _ = sender.send(RunEvent::Finished).await;
            wake();
        });
        Ok(cancel)
    }
}

#[cfg(test)]
mod feature_tests {
    use super::*;
    fn result(status: u16, body: &str, headers: Vec<(String, String)>) -> ExecutionResult {
        ExecutionResult {
            request_id: "r".into(),
            run_id: "run".into(),
            summary: RequestSummary {
                method: "GET".into(),
                url: "http://local/".into(),
                headers: vec![],
                environment: "dev".into(),
                revision: 3,
                timeout_ms: 1000,
            },
            outcome: Outcome::Complete,
            status: Some(status),
            status_text: String::new(),
            headers,
            body_error: None,
            body: BodyHandle::Memory(Arc::new(body.as_bytes().to_vec())),
            encoded_bytes: body.len() as u64,
            duration_ms: 4,
            diagnostics: ResponseDiagnostics::default(),
        }
    }
    fn report(failed: bool) -> HeadlessReport {
        HeadlessReport {
            request_id: "r".into(),
            request_name: "A & B".into(),
            environment: "dev".into(),
            revision: 2,
            outcome: Outcome::Complete,
            status: Some(200),
            duration_ms: 4,
            response_bytes: 2,
            response_snippet: None,
            tests: Some(TestReport {
                tests: vec![TestCase {
                    name: "works".into(),
                    passed: !failed,
                    error: failed.then(|| "bad < value".into()),
                    line: None,
                }],
                ..Default::default()
            }),
            error: None,
        }
    }
    #[test]
    fn comparison_ignores_object_order_and_selected_pointers() {
        let left = result(
            200,
            r#"{"stable":1,"time":"a"}"#,
            vec![("X".into(), "1".into())],
        );
        let right = result(
            201,
            r#"{"time":"b","stable":1}"#,
            vec![("X".into(), "2".into())],
        );
        let c = compare(&left, &right, &["/time".into()]);
        assert!(c.status.is_some());
        assert_eq!(c.differences, Vec::<String>::new());
        assert_eq!(c.headers.len(), 2);
    }
    #[test]
    fn reports_are_deterministic_and_escape_markup() {
        let reports = vec![report(true)];
        let xml = junit(&reports);
        assert!(xml.contains("failures=\"1\""));
        assert!(xml.contains("A &amp; B"));
        assert!(xml.contains("bad &lt; value"));
        let page = html(&reports);
        assert!(page.contains("A &amp; B"));
        assert!(page.contains("FAIL"));
    }
    #[test]
    fn baseline_round_trips_all_compared_content() {
        let original = result(200, "body", vec![("X".into(), "one".into())]);
        let baseline = Baseline::from_result(&original).unwrap();
        let bytes = serde_json::to_vec(&baseline).unwrap();
        let loaded: Baseline = serde_json::from_slice(&bytes).unwrap();
        let c = compare_baseline(&original, &loaded, &[]);
        assert!(c.status.is_none() && c.headers.is_empty() && c.differences.is_empty());
    }
}
