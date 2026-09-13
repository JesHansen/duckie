use duckie_model::*;
use duckie_tests::{TestInput, evaluate};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn input(source: &str) -> TestInput {
    let result = ExecutionResult {
        request_id: "test".into(),
        run_id: "test-run".into(),
        summary: RequestSummary {
            method: "GET".into(),
            url: "http://localhost".into(),
            headers: vec![],
            environment: "dev".into(),
            revision: 3,
            timeout_ms: 30_000,
        },
        outcome: Outcome::Complete,
        status: Some(500),
        status_text: "Internal Server Error".into(),
        headers: vec![],
        body: BodyHandle::Memory(Arc::new(b"{}".to_vec())),
        encoded_bytes: 2,
        duration_ms: 5,
    };
    TestInput::from_result(source.into(), &result, Values::new()).unwrap()
}
#[tokio::test]
async fn actual_worker_roundtrip_and_cancel() {
    // Cargo builds the real worker executable for this integration target.
    assert!(std::path::Path::new(env!("CARGO_BIN_EXE_duckie-test-worker")).exists());
    let report = evaluate(
        input("test('500 is expected',()=>expect(response.status).toBe(500));"),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(report.suite_error.is_none(), "{:?}", report.suite_error);
    assert!(report.tests[0].passed);
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        token.cancel();
    });
    let started = std::time::Instant::now();
    let error = evaluate(input("while(true){}"), cancel)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("stopped"));
    assert!(started.elapsed().as_secs() < 2);
}
#[tokio::test]
async fn excessive_allocation_fails_without_killing_parent_and_next_run_works() {
    let report = evaluate(
        input("const huge = []; while(true) huge.push(new Array(100000).fill(42));"),
        CancellationToken::new(),
    )
    .await;
    assert!(report.is_err() || report.unwrap().suite_error.is_some());
    let report = evaluate(
        input("test('recovered',()=>expect(1).toBe(1));"),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(report.tests[0].passed);
}
