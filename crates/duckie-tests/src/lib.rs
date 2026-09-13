//! Bounded test protocol and parent watchdog. No JavaScript engine in the GUI process.
use anyhow::{Context, Result, bail};
use duckie_model::*;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

pub const MAX_FRAME: usize = 24 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestInput {
    pub protocol: u32,
    pub source: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub body_size: u64,
    pub duration_ms: u64,
    pub environment: Values,
    pub request: RequestSummary,
}
impl TestInput {
    pub fn from_result(
        source: String,
        result: &ExecutionResult,
        environment: Values,
    ) -> Result<Self> {
        if result.outcome != Outcome::Complete {
            bail!("Tests are skipped because the response is incomplete");
        }
        let body = if result.body.len() <= 10 * MIB {
            Some(String::from_utf8_lossy(&result.body.read(0, 10 * MIB)?).into_owned())
        } else {
            None
        };
        Ok(Self {
            protocol: 1,
            source,
            status: result.status.context("No HTTP response")?,
            headers: result.headers.clone(),
            body,
            body_size: result.body.len(),
            duration_ms: result.duration_ms,
            environment,
            request: result.summary.clone(),
        })
    }
}
pub fn worker_path() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let directory = exe.parent().context("No executable directory")?;
    let name = if cfg!(windows) {
        "duckie-test-worker.exe"
    } else {
        "duckie-test-worker"
    };
    let direct = directory.join(name);
    if direct.is_file() {
        return Ok(direct);
    }
    if directory.file_name().is_some_and(|n| n == "deps") {
        let parent = directory.parent().unwrap().join(name);
        if parent.is_file() {
            return Ok(parent);
        }
    }
    bail!(
        "Test worker is missing. Build the workspace or place duckie-test-worker.exe beside Duckie."
    )
}
pub async fn evaluate(input: TestInput, cancel: CancellationToken) -> Result<TestReport> {
    if input.source.len() > MIB as usize {
        bail!("Test source exceeds 1 MiB");
    }
    let bytes = serde_json::to_vec(&input)?;
    if bytes.len() > MAX_FRAME {
        bail!("Test input exceeds protocol limit");
    }
    let mut command = tokio::process::Command::new(worker_path()?);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command.spawn().context("Cannot launch test worker")?;
    #[cfg(windows)]
    let _job = job::Job::attach(&child)?;
    let mut stdin = child.stdin.take().context("Missing worker input")?;
    let mut stdout = child.stdout.take().context("Missing worker output")?;
    let operation = async {
        let mut ready = [0u8; 4];
        tokio::time::timeout(Duration::from_secs(3), stdout.read_exact(&mut ready))
            .await
            .context("Worker startup timed out")??;
        if &ready != b"DK01" {
            bail!("Incompatible test worker protocol");
        }
        let evaluation = async {
            stdin.write_u32_le(bytes.len() as u32).await?;
            stdin.write_all(&bytes).await?;
            stdin.shutdown().await?;
            let len = stdout
                .read_u32_le()
                .await
                .context("Worker stopped unexpectedly (possible memory limit)")?
                as usize;
            if len > 256 * 1024 {
                bail!("Worker report exceeds protocol limit");
            }
            let mut report = vec![0; len];
            stdout.read_exact(&mut report).await?;
            let report: TestReport =
                serde_json::from_slice(&report).context("Invalid worker report")?;
            Ok(report)
        };
        tokio::time::timeout(Duration::from_millis(2500), evaluation)
            .await
            .context("Tests timed out (2 second evaluation limit)")?
    };
    let result =
        tokio::select! {_=cancel.cancelled()=>Err(anyhow::anyhow!("Tests stopped")),r=operation=>r};
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}
#[cfg(windows)]
mod job {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::*,
    };
    pub struct Job(HANDLE);
    // The handle is owned and only closed on drop. Job API calls are thread-safe.
    unsafe impl Send for Job {}
    impl Job {
        pub fn attach(child: &tokio::process::Child) -> Result<Self> {
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return Err(std::io::Error::last_os_error().into());
                }
                let job = Self(handle);
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags =
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
                limits.ProcessMemoryLimit = 128 * 1024 * 1024;
                if SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of_val(&limits) as u32,
                ) == 0
                {
                    return Err(std::io::Error::last_os_error().into());
                }
                let process = child.raw_handle().context("Missing worker handle")?;
                if AssignProcessToJobObject(handle, process) == 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                Ok(job)
            }
        }
    }
    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
