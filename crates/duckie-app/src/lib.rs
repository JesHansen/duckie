//! Single-run orchestration. Callers receive response and tests as independent events.
use anyhow::{Result, bail};
use duckie_model::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

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
    ) -> Result<CancellationToken> {
        if self.busy.swap(true, Ordering::AcqRel) {
            bail!("A request or test evaluation is already running");
        }
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let http = self.http.clone();
        let sender = self.sender.clone();
        let wake = self.wake.clone();
        let busy = self.busy.clone();
        self.runtime.spawn(async move {
            let request_id = request.id.clone();
            match http.execute(request, token.clone()).await {
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
        Ok(cancel)
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
