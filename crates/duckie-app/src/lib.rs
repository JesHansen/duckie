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
    pub outcome: Outcome,
    pub status: Option<u16>,
    pub duration_ms: u64,
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
) -> HeadlessReport {
    let failure = |error: anyhow::Error| HeadlessReport {
        request_id: request.id.clone(),
        request_name: request.name.clone(),
        outcome: Outcome::ConnectionError,
        status: None,
        duration_ms: 0,
        tests: None,
        error: Some(error.to_string()),
    };
    let prepared = match prepare(request, &environment, &RunBindings::default(), 0) {
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
        outcome: result.outcome.clone(),
        status: result.status,
        duration_ms: result.duration_ms,
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
    requests: Vec<(RequestDefinition, String)>,
    environment: EnvironmentSnapshot,
    cancel: CancellationToken,
) -> Vec<HeadlessReport> {
    let mut reports = Vec::with_capacity(requests.len());
    for (request, source) in requests {
        if cancel.is_cancelled() {
            break;
        }
        reports.push(
            execute_headless(http, &request, source, environment.clone(), cancel.clone()).await,
        );
    }
    reports
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
