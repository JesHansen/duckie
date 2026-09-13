//! Drives the shipped `examples/local-api` collection against `scripts/dev-server.mjs`:
//! storage load, interpolation, real HTTP, and assertion evaluation in the real worker.
use duckie_app::{ExecutionService, RunEvent};
use duckie_model::*;
use duckie_storage::Collection;
use std::{
    io::Read,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// Deliberately not canonicalized: Node's module loader cannot resolve the verbatim
/// `\\?\` prefix that Windows canonicalization adds, and `Collection::open` canonicalizes anyway.
fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
/// Kills the fixture server even when an assertion unwinds the test.
struct Server {
    child: Child,
    port: u16,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Server {
    fn start(root: &Path) -> Option<Self> {
        let port = free_port();
        let child = Command::new(if cfg!(windows) { "node.exe" } else { "node" })
            .arg(root.join("scripts/dev-server.mjs"))
            .env("DUCKIE_TEST_PORT", port.to_string())
            .current_dir(root)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        let mut server = Self { child, port };
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some(server);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // Report why node gave up instead of leaving a bare timeout.
        let _ = server.child.kill();
        let mut reason = String::new();
        if let Some(mut stderr) = server.child.stderr.take() {
            let _ = stderr.read_to_string(&mut reason);
        }
        panic!("Fixture server did not accept connections on port {port}: {reason}");
    }
}
/// Runs one request through the same orchestration the desktop uses and drains its events.
/// The report is absent whenever the request carries no test source.
fn run(
    service: &ExecutionService,
    receiver: &mut tokio::sync::mpsc::Receiver<RunEvent>,
    request: PreparedRequest,
    source: String,
    environment: Values,
) -> (ExecutionResult, Option<TestReport>) {
    service
        .send(request, source, true, environment)
        .expect("run slot was free");
    let (mut result, mut report) = (None, None);
    while let Some(event) = receiver.blocking_recv() {
        match event {
            RunEvent::Response(r) => result = Some(r),
            RunEvent::Tests { report: r, .. } => report = Some(r),
            RunEvent::Error { message, .. } => panic!("Execution failed: {message}"),
            RunEvent::Finished => break,
        }
    }
    (result.expect("a response event"), report)
}
#[test]
fn shipped_example_collection_passes_against_the_fixture_server() {
    let root = repository_root();
    let Some(server) = Server::start(&root) else {
        panic!(
            "Node.js is required for this end-to-end test; install Node or run it on a machine that has it"
        );
    };
    let collection = Collection::open(&root.join("examples/local-api")).unwrap();
    assert_eq!(
        collection.requests.len(),
        2,
        "fixture request count changed"
    );

    // The checked-in environment points at the documented default port; retarget the ephemeral one.
    let mut environment = collection
        .environments
        .iter()
        .find(|e| e.name == "dev")
        .expect("a dev environment")
        .clone();
    environment.values.insert(
        "baseUrl".into(),
        format!("http://127.0.0.1:{}", server.port),
    );
    let snapshot = EnvironmentSnapshot {
        name: environment.name.clone(),
        values: environment.values.clone(),
        secrets: Values::new(),
    };

    let (service, mut receiver) = ExecutionService::new(|| {}).unwrap();
    for stored in &collection.requests {
        let prepared = prepare(&stored.definition, &snapshot, &RunBindings::default(), 1)
            .unwrap_or_else(|e| panic!("{} could not be prepared: {e}", stored.definition.id));
        let (result, report) = run(
            &service,
            &mut receiver,
            prepared,
            stored.source.clone(),
            environment.values.clone(),
        );
        let id = &stored.definition.id;
        let report = report.expect("a test report event");
        assert_eq!(result.outcome, Outcome::Complete, "{id} did not complete");
        assert_eq!(
            report.suite_error, None,
            "{id} assertions did not run: {:?}",
            report.suite_error
        );
        assert!(!report.tests.is_empty(), "{id} declared no tests");
        for case in &report.tests {
            assert!(case.passed, "{id} / {}: {:?}", case.name, case.error);
        }
    }
}

/// Mirrors the desktop's protected-URL acquisition: a prepared GET with a bearer secret,
/// executed by the real HTTP engine, whose body is handed to the importer.
fn acquire_spec(
    service: &ExecutionService,
    receiver: &mut tokio::sync::mpsc::Receiver<RunEvent>,
    url: &str,
    bearer: Option<&str>,
) -> ExecutionResult {
    let mut definition = RequestDefinition {
        url: url.into(),
        ..Default::default()
    };
    let mut snapshot = EnvironmentSnapshot::default();
    if let Some(bearer) = bearer {
        definition.auth.bearer = Some(SecretBinding {
            secret: "importBearer".into(),
        });
        snapshot
            .secrets
            .insert("importBearer".into(), bearer.into());
    }
    let prepared = prepare(&definition, &snapshot, &RunBindings::default(), 0).unwrap();
    run(service, receiver, prepared, String::new(), Values::new()).0
}
#[test]
fn protected_openapi_url_imports_and_the_imported_requests_run() {
    let root = repository_root();
    let Some(server) = Server::start(&root) else {
        panic!("Node.js is required for this end-to-end test");
    };
    let base = format!("http://127.0.0.1:{}", server.port);
    let spec_url = format!("{base}/protected/openapi.json");
    let (service, mut receiver) = ExecutionService::new(|| {}).unwrap();

    let refused = acquire_spec(&service, &mut receiver, &spec_url, None);
    assert_eq!(refused.outcome, Outcome::Complete);
    assert_eq!(
        refused.status,
        Some(401),
        "the protected spec must reject an unauthenticated read"
    );

    // The token documented in README.md for the local fixture server.
    let allowed = acquire_spec(
        &service,
        &mut receiver,
        &spec_url,
        Some("duckie-local-demo"),
    );
    assert_eq!(allowed.status, Some(200));
    let draft = duckie_openapi::import_json(&allowed.body.read(0, 20 * MIB).unwrap()).unwrap();
    assert_eq!(draft.name, "Duckie local API");
    assert_eq!(
        draft.version, "3.1.2",
        "draft.version reports the OpenAPI version"
    );
    assert_eq!(draft.servers, vec!["http://127.0.0.1:8787".to_string()]);
    assert_eq!(draft.operations.len(), 4, "operation count changed");

    // Import leaves the root server as {{env.baseUrl}}, so a retargeted environment is enough.
    let mut values = Values::new();
    values.insert("baseUrl".into(), base.clone());
    let snapshot = EnvironmentSnapshot {
        name: "dev".into(),
        values: values.clone(),
        secrets: Values::new(),
    };
    for operation in &draft.operations {
        let definition = operation.finish();
        assert!(
            definition.blockers.is_empty(),
            "{} was imported with blockers: {:?}",
            definition.url,
            definition.blockers
        );
        // /large gzip fixtures are exercised by the transport tests; keep this to the small ones.
        if definition.url.contains("/large") {
            continue;
        }
        let prepared = prepare(&definition, &snapshot, &RunBindings::default(), 1).unwrap();
        let expected = if definition.url.contains("/status/") {
            500
        } else {
            200
        };
        let (result, _) = run(
            &service,
            &mut receiver,
            prepared,
            String::new(),
            values.clone(),
        );
        assert_eq!(result.outcome, Outcome::Complete, "{}", definition.url);
        assert_eq!(result.status, Some(expected), "{}", definition.url);
        // The echo fixture returns the query it received, which proves the exploded array
        // parameter reached the wire as one repeated name rather than a joined value.
        if definition.method == "GET" && definition.url.ends_with("/echo") {
            let body: serde_json::Value =
                serde_json::from_slice(&result.body.read(0, MIB).unwrap()).unwrap();
            assert_eq!(
                body["query"],
                serde_json::json!([
                    ["message", "Hello, Duckie"],
                    ["tags", "duck"],
                    ["tags", "yellow"]
                ]),
                "echoed query for {}",
                definition.url
            );
        }
    }
}
