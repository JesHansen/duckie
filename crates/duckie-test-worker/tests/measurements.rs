//! Measurements for the acceptance table in ARCHITECTURE.md that are cheapest to take in-process.
//!
//! Ignored by default: these are timing runs, not assertions about behaviour, and they would add
//! seconds to every `cargo test`. Take them with:
//!
//! ```powershell
//! cargo test -p duckie-test-worker --release --test measurements -- --ignored --nocapture
//! ```
use duckie_app::{ExecutionService, RunEvent};
use duckie_model::*;
use std::{
    io::Read,
    net::TcpListener,
    sync::Arc,
    time::{Duration, Instant},
};

/// Nearest-rank p95: the gates are stated against observed runs, so an interpolated value between
/// two samples would be a number nobody measured.
fn p95(mut samples: Vec<u128>) -> u128 {
    samples.sort_unstable();
    samples[samples
        .len()
        .saturating_mul(95)
        .div_ceil(100)
        .saturating_sub(1)
        .min(samples.len() - 1)]
}
fn report(name: &str, samples: Vec<u128>, target: &str) {
    let mut sorted = samples.clone();
    sorted.sort_unstable();
    println!(
        "{name}: min {} median {} p95 {} max {} (target {target})",
        sorted[0],
        sorted[sorted.len() / 2],
        p95(samples),
        sorted[sorted.len() - 1],
    );
}

#[test]
#[ignore = "measurement"]
fn cold_test_worker_overhead() {
    let result = ExecutionResult {
        request_id: "m".into(),
        run_id: "m".into(),
        summary: RequestSummary {
            method: "GET".into(),
            url: "http://localhost/".into(),
            headers: vec![],
            environment: "dev".into(),
            revision: 1,
            timeout_ms: 30_000,
        },
        outcome: Outcome::Complete,
        status: Some(200),
        status_text: "OK".into(),
        headers: vec![("content-type".into(), "application/json".into())],
        body_error: None,
        body: BodyHandle::Memory(Arc::new(vec![b'x'; 1024])),
        encoded_bytes: 1024,
        duration_ms: 1,
        diagnostics: ResponseDiagnostics::default(),
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut samples = vec![];
    for _ in 0..20 {
        let input = duckie_tests::TestInput::from_result(
            "test('trivial', () => expect(response.status).toBe(200));".into(),
            &result,
            Values::new(),
        )
        .unwrap();
        let started = Instant::now();
        let report = runtime
            .block_on(duckie_tests::evaluate(input, Default::default()))
            .unwrap();
        samples.push(started.elapsed().as_millis());
        assert!(report.tests[0].passed);
    }
    report("cold test worker", samples, "p95 <= 150 ms");
}

#[test]
#[ignore = "measurement"]
fn warm_local_request_dispatch() {
    // A listener that accepts and answers immediately, so the measurement is dominated by the
    // time Duckie takes to turn a Send into bytes on the wire rather than by any server.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer);
            let _ = std::io::Write::write_all(
                &mut stream,
                b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok",
            );
        }
    });
    let (service, mut events) = ExecutionService::new(|| {}).unwrap();
    let snapshot = EnvironmentSnapshot::default();
    let mut samples = vec![];
    for _ in 0..30 {
        let definition = RequestDefinition {
            url: format!("http://127.0.0.1:{port}/"),
            ..Default::default()
        };
        let prepared = prepare(&definition, &snapshot, &RunBindings::default(), 1).unwrap();
        let started = Instant::now();
        service
            .send(prepared, String::new(), false, Values::new())
            .unwrap();
        let mut dispatched = None;
        while let Some(event) = events.blocking_recv() {
            match event {
                // The response event is the first point the caller learns anything; it bounds
                // dispatch from above, since it includes a full loopback round trip.
                RunEvent::Response(_) => dispatched = Some(started.elapsed()),
                RunEvent::Finished => break,
                _ => {}
            }
        }
        samples.push(dispatched.unwrap().as_micros());
    }
    report(
        "send to loopback response (microseconds)",
        samples,
        "dispatch alone p95 <= 5 ms",
    );
}

#[test]
#[ignore = "measurement"]
fn whole_body_search_over_fifty_megabytes() {
    let body = BodyHandle::Memory(Arc::new(vec![b'.'; 50 * MIB as usize]));
    let never = || false;
    let mut samples = vec![];
    for _ in 0..5 {
        let started = Instant::now();
        let found = body.find_all(b"needle", 50_000, &never).unwrap();
        samples.push(started.elapsed().as_millis());
        assert!(found.is_empty());
    }
    report("whole-body search of 50 MiB", samples, "no stated target");
    std::thread::sleep(Duration::from_millis(1));
}
