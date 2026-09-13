use duckie_model::TestReport;
use duckie_tests::{MAX_FRAME, TestInput};
use rquickjs::{Context, Function, Object, Runtime, context::EvalOptions};
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};

fn evaluate(input: TestInput) -> TestReport {
    let started = Instant::now();
    let run = || -> Result<TestReport, String> {
        if input.protocol != 1 {
            return Err("Unsupported protocol version".into());
        }
        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        runtime.set_memory_limit(64 * 1024 * 1024);
        runtime.set_max_stack_size(512 * 1024);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            started.elapsed() > Duration::from_secs(2)
        })));
        let context = Context::full(&runtime).map_err(|e| e.to_string())?;
        context.with(|ctx| {
            let payload = serde_json::to_string(&input).map_err(|e| e.to_string())?;
            ctx.globals()
                .set("__duckieInput", payload)
                .map_err(|e| e.to_string())?;
            let mut harness_options = EvalOptions::default();
            harness_options.filename = Some("duckie-harness.js".into());
            let harness: Object = ctx
                .eval_with_options(include_str!("harness.js"), harness_options)
                .map_err(|e| format!("Cannot initialize assertion API: {e}"))?;
            let mut source_options = EvalOptions::default();
            source_options.filename = Some("tests.js".into());
            if let Err(error) =
                ctx.eval_with_options::<(), _>(input.source.as_bytes(), source_options)
            {
                if started.elapsed() > Duration::from_secs(2) {
                    return Err("Tests timed out (2 second evaluation limit)".into());
                }
                let caught = ctx.catch();
                if caught.is_null() || caught.is_undefined() {
                    return Err(
                        "Memory limit exceeded or script could not produce an exception".into(),
                    );
                }
                let fail: Function = harness.get("fail").map_err(|e| e.to_string())?;
                fail.call::<_, ()>((caught,))
                    .map_err(|_| format!("Script error or memory limit exceeded: {error}"))?;
            }
            let report: Function = harness.get("report").map_err(|e| e.to_string())?;
            let json: String = report
                .call(())
                .map_err(|e| format!("Memory limit exceeded or report unavailable: {e}"))?;
            serde_json::from_str(&json).map_err(|e| e.to_string())
        })
    };
    let mut report = run().unwrap_or_else(|e| TestReport {
        suite_error: Some(e),
        ..Default::default()
    });
    report.duration_ms = started.elapsed().as_millis() as u64;
    report
}
fn main() {
    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(b"DK01")
        .and_then(|_| stdout.flush())
        .is_err()
    {
        return;
    }
    let mut stdin = std::io::stdin().lock();
    let mut size = [0u8; 4];
    if stdin.read_exact(&mut size).is_err() {
        return;
    }
    let size = u32::from_le_bytes(size) as usize;
    if size > MAX_FRAME {
        return;
    }
    let mut bytes = vec![0; size];
    if stdin.read_exact(&mut bytes).is_err() {
        return;
    }
    let Ok(input) = serde_json::from_slice::<TestInput>(&bytes) else {
        return;
    };
    drop(bytes);
    let report = evaluate(input);
    let Ok(bytes) = serde_json::to_vec(&report) else {
        return;
    };
    if bytes.len() > 256 * 1024 {
        return;
    }
    let _ = stdout
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .and_then(|_| stdout.write_all(&bytes))
        .and_then(|_| stdout.flush());
}
#[cfg(test)]
mod tests {
    use super::*;
    fn input(source: &str) -> TestInput {
        TestInput {
            protocol: 1,
            source: source.into(),
            status: 500,
            headers: vec![("X-Test".into(), "a".into()), ("X-Test".into(), "b".into())],
            body: Some("{\"ok\":true}".into()),
            body_size: 11,
            duration_ms: 5,
            environment: Default::default(),
            request: duckie_model::RequestSummary {
                method: "GET".into(),
                url: "http://localhost".into(),
                headers: vec![],
                environment: "dev".into(),
                revision: 1,
                timeout_ms: 30000,
            },
        }
    }
    #[test]
    fn assertions_continue_and_status_is_independent() {
        let r = evaluate(input(
            "test('status',()=>expect(response.status).toBe(500));test('fail',()=>expect(1).toBe(2));test('json',()=>expect(response.json()).toEqual({ok:true}));test('headers',()=>expect(response.headers('x-test')).toEqual(['a','b']));",
        ));
        assert!(r.suite_error.is_none(), "{:?}", r.suite_error);
        assert_eq!(r.tests.len(), 4);
        assert_eq!(r.tests.iter().filter(|t| t.passed).count(), 3);
    }
    #[test]
    fn script_async_and_body_errors_are_visible() {
        assert!(evaluate(input("const = broken")).suite_error.is_some());
        let r = evaluate(input("test('async',async()=>{});"));
        assert!(!r.tests[0].passed);
        let mut i = input(
            "test('body',()=>response.text());test('metadata',()=>expect(response.status).toBe(500));",
        );
        i.body = None;
        let r = evaluate(i);
        assert!(!r.tests[0].passed);
        assert!(r.tests[1].passed);
    }
    #[test]
    fn infinite_loop_is_interrupted() {
        let r = evaluate(input("while(true){}"));
        assert!(r.suite_error.unwrap().contains("timed out"));
    }
    #[test]
    fn no_node_or_network_api() {
        let r = evaluate(input(
            "test('isolated',()=>{expect(typeof fetch).toBe('undefined');expect(typeof require).toBe('undefined');expect(typeof process).toBe('undefined');});",
        ));
        assert!(r.tests[0].passed);
    }
    #[test]
    fn failed_assertion_reports_message_and_source_line() {
        let r = evaluate(input("test('x', () => {\n  expect(1).toBe(2);\n});"));
        let error = r.tests[0].error.as_ref().unwrap();
        assert!(error.contains("Assertion failed (toBe)"), "{error}");
        assert!(error.contains("Expected:"), "{error}");
        assert!(error.contains("Received:"), "{error}");
        assert_eq!(r.tests[0].line, Some(2));
    }
    #[test]
    fn thrown_error_reports_helper_throw_site_line() {
        let r = evaluate(input(
            "function helper() {\n  throw new Error('boom');\n}\ntest('helper', () => {\n  helper();\n});",
        ));
        assert_eq!(r.tests[0].line, Some(2));
    }
    #[test]
    fn passing_test_has_no_line() {
        let r = evaluate(input("test('ok', () => expect(1).toBe(1));"));
        assert!(r.tests[0].passed);
        assert_eq!(r.tests[0].line, None);
    }
}
