use anyhow::{Context, Result, bail};
use duckie_app::HeadlessReport;
use duckie_model::{EnvironmentSnapshot, Outcome};
use duckie_storage::Collection;
use serde::Serialize;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[derive(PartialEq, Clone, Copy)]
enum Format {
    Text,
    Json,
}
struct Options {
    collection: PathBuf,
    request: Option<String>,
    environment: String,
    format: Format,
}
fn result_code(reports: &[HeadlessReport], cancelled: bool) -> i32 {
    if cancelled {
        3
    } else if reports.iter().any(HeadlessReport::execution_failed) {
        2
    } else if reports.iter().any(HeadlessReport::assertion_failed) {
        1
    } else {
        0
    }
}

fn options(args: impl IntoIterator<Item = String>) -> Result<Options> {
    let mut args = args.into_iter();
    let _ = args.next();
    if args.next().as_deref() != Some("run") {
        bail!(
            "Usage: duckie-cli run --collection PATH [--request ID_OR_NAME] --environment NAME [--format text|json]"
        );
    }
    let mut collection = None;
    let mut request = None;
    let mut environment = None;
    let mut format = Format::Text;
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .with_context(|| format!("{arg} requires a value"))
        };
        match arg.as_str() {
            "--collection" => collection = Some(PathBuf::from(value()?)),
            "--request" => request = Some(value()?),
            "--environment" => environment = Some(value()?),
            "--format" => {
                format = match value()?.as_str() {
                    "text" => Format::Text,
                    "json" => Format::Json,
                    other => bail!("Unknown output format: {other}"),
                }
            }
            "--suite" => {}
            other => bail!("Unknown option: {other}"),
        }
    }
    Ok(Options {
        collection: collection.context("--collection is required")?,
        request,
        environment: environment.context("--environment is required")?,
        format,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Output<'a> {
    collection: &'a str,
    environment: &'a str,
    cancelled: bool,
    results: &'a [HeadlessReport],
}

#[tokio::main]
async fn main() {
    let code = match run().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("duckie-cli: {error:#}");
            2
        }
    };
    std::process::exit(code);
}

async fn run() -> Result<i32> {
    let options = options(std::env::args())?;
    let mut collection = Collection::open(&options.collection).context("Cannot open collection")?;
    let env = collection
        .environments
        .iter()
        .find(|e| e.name == options.environment)
        .cloned()
        .with_context(|| format!("Environment {:?} does not exist", options.environment))?;
    let mut secrets = collection
        .secrets
        .environments
        .get(&env.name)
        .cloned()
        .unwrap_or_default();
    for (key, value) in std::env::vars() {
        if let Some(name) = key.strip_prefix("DUCKIE_SECRET_") {
            secrets.insert(name.replace("__", "."), value);
        }
    }
    let ids: Vec<String> = collection
        .requests
        .iter()
        .filter(|r| {
            options
                .request
                .as_ref()
                .is_none_or(|needle| &r.definition.id == needle || &r.definition.name == needle)
        })
        .map(|r| r.definition.id.clone())
        .collect();
    if ids.is_empty() {
        bail!("No request matched the selection");
    }
    for id in &ids {
        collection.ensure_loaded(id)?;
    }
    let cancel = CancellationToken::new();
    let interrupt = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            interrupt.cancel();
        }
    });
    let requests = ids
        .into_iter()
        .map(|id| {
            let stored = collection
                .requests
                .iter()
                .find(|r| r.definition.id == id)
                .unwrap();
            (stored.definition.clone(), stored.source.clone())
        })
        .collect();
    let snapshot = EnvironmentSnapshot {
        name: env.name.clone(),
        values: env.values.clone(),
        secrets,
    };
    let reports = duckie_app::execute_headless_suite(
        &duckie_http::HttpEngine::default(),
        requests,
        snapshot,
        cancel.clone(),
    )
    .await;
    let cancelled =
        cancel.is_cancelled() || reports.iter().any(|r| r.outcome == Outcome::Cancelled);
    match options.format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&Output {
                collection: &collection.manifest.name,
                environment: &env.name,
                cancelled,
                results: &reports
            })?
        ),
        Format::Text => {
            for report in &reports {
                let state = if report.execution_failed() {
                    "ERROR"
                } else if report.assertion_failed() {
                    "FAIL"
                } else {
                    "PASS"
                };
                println!(
                    "{state} {} [{}] {} ms",
                    report.request_name,
                    report.status.map_or("-".into(), |s| s.to_string()),
                    report.duration_ms
                );
                if let Some(error) = &report.error {
                    println!("  {error}");
                }
                if let Some(tests) = &report.tests {
                    for test in &tests.tests {
                        println!(
                            "  {} {}",
                            if test.passed { "PASS" } else { "FAIL" },
                            test.name
                        );
                    }
                }
            }
        }
    }
    Ok(result_code(&reports, cancelled))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_explicit_inputs() {
        assert!(options(["duckie-cli".into(), "run".into()]).is_err())
    }
    #[test]
    fn parses_selection() {
        let o = options(
            [
                "x",
                "run",
                "--collection",
                "c",
                "--request",
                "r",
                "--environment",
                "dev",
                "--format",
                "json",
            ]
            .map(str::to_string),
        )
        .unwrap();
        assert_eq!(o.request.as_deref(), Some("r"));
        assert!(o.format == Format::Json)
    }
    #[test]
    fn exit_codes_are_stable() {
        let report = |outcome, error, tests| HeadlessReport {
            request_id: "r".into(),
            request_name: "R".into(),
            outcome,
            status: Some(200),
            duration_ms: 1,
            tests,
            error,
        };
        assert_eq!(
            result_code(&[report(Outcome::Complete, None, None)], false),
            0
        );
        assert_eq!(
            result_code(
                &[report(
                    Outcome::Complete,
                    None,
                    Some(duckie_model::TestReport {
                        tests: vec![duckie_model::TestCase {
                            name: "x".into(),
                            passed: false,
                            error: None,
                            line: None
                        }],
                        ..Default::default()
                    })
                )],
                false
            ),
            1
        );
        assert_eq!(
            result_code(
                &[report(
                    Outcome::ConnectionError,
                    Some("failed".into()),
                    None
                )],
                false
            ),
            2
        );
        assert_eq!(result_code(&[], true), 3);
    }
}
