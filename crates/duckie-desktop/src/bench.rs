//! Development-only measurement hooks for the benchmark protocol in ARCHITECTURE.md.
//!
//! Never compiled into a packaged build: `scripts/package.ps1` refuses a binary containing
//! the marker below. Milestones are absolute epoch milliseconds so the harness can subtract
//! the OS-reported process creation time and capture loader cost the app cannot see itself.
use crate::state::*;
use eframe::egui;
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub const MARKER: &str = "DUCKIE_BENCH_PATH";

/// Epoch milliseconds bracketing the background collection open. Globals rather than fields so
/// the job closure in `state.rs` does not have to carry a handle purely for measurement.
pub static OPEN_STARTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static OPEN_FINISHED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn mark(slot: &std::sync::atomic::AtomicU64) {
    slot.store(now_ms() as u64, std::sync::atomic::Ordering::Release);
}
fn taken(slot: &std::sync::atomic::AtomicU64) -> Option<u64> {
    Some(slot.load(std::sync::atomic::Ordering::Acquire)).filter(|v| *v > 0)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}
/// Peak private bytes and peak working set as tracked by the kernel for this process.
/// Reading the peak avoids the sampling error an external poller would have on a short run.
fn peak_memory() -> (u64, u64) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::{ProcessStatus::*, Threading::GetCurrentProcess};
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) != 0 {
            return (
                counters.PeakPagefileUsage as u64,
                counters.PeakWorkingSetSize as u64,
            );
        }
    }
    (0, 0)
}

pub struct Bench {
    path: PathBuf,
    url: Option<String>,
    frames: u32,
    first_frame: Option<u128>,
    searchable: Option<u128>,
    responded: Option<u128>,
    sent: bool,
    settled: Option<u64>,
}
impl Bench {
    /// Enabled only by `DUCKIE_BENCH_PATH`. `DUCKIE_BENCH_URL` additionally sends one request
    /// once the app is ready, which is how the 50 MiB response budget gets exercised.
    pub fn from_env() -> Option<Self> {
        Some(Self {
            path: std::env::var_os(MARKER).map(PathBuf::from)?,
            url: std::env::var("DUCKIE_BENCH_URL")
                .ok()
                .filter(|u| !u.is_empty()),
            frames: 0,
            first_frame: None,
            searchable: None,
            responded: None,
            sent: false,
            settled: None,
        })
    }
    fn write(&self, requests: usize) {
        let (peak_private, peak_working_set) = peak_memory();
        let report = serde_json::json!({
            "firstFrameEpochMs": self.first_frame,
            "searchableEpochMs": self.searchable,
            "openStartedEpochMs": taken(&OPEN_STARTED),
            "openFinishedEpochMs": taken(&OPEN_FINISHED),
            "respondedEpochMs": self.responded,
            "requests": requests,
            "settledPrivateBytes": self.settled,
            "peakPrivateBytes": peak_private,
            "peakWorkingSetBytes": peak_working_set,
            "url": self.url,
        });
        if let Ok(bytes) = serde_json::to_vec_pretty(&report) {
            let _ = std::fs::write(&self.path, bytes);
        }
    }
}
impl Duckie {
    /// Advances the measurement state machine once per frame and closes the window when the
    /// run's terminal milestone is reached.
    pub fn bench_frame(&mut self, ctx: &egui::Context) {
        let Some(mut bench) = self.bench.take() else {
            return;
        };
        bench.frames += 1;
        if bench.first_frame.is_none() {
            bench.first_frame = Some(now_ms());
        }
        // A collection populates the sidebar from a background job, so "searchable" is the
        // first frame after that job lands rather than anything observable at startup.
        if bench.searchable.is_none() && self.collection.is_some() && !self.io_busy {
            bench.searchable = Some(now_ms());
        }
        let ready = self.collection.is_none() || bench.searchable.is_some();
        if let Some(url) = bench.url.clone() {
            if !bench.sent && ready && bench.frames > 2 {
                // Record the baseline the response budget is measured against before sending.
                bench.settled = Some(peak_memory().0);
                self.drafts[self.selected].request.set_address(&url);
                self.drafts[self.selected].request.tests.enabled = false;
                self.send();
                bench.sent = true;
            }
            if bench.sent && bench.responded.is_none() && self.active.is_none() {
                let id = &self.drafts[self.selected].request.id;
                if self.responses.contains_key(id) {
                    bench.responded = Some(now_ms());
                }
            }
        }
        let done = match (&bench.url, self.collection.is_some()) {
            (Some(_), _) => bench.responded.is_some(),
            (None, true) => bench.searchable.is_some(),
            (None, false) => bench.frames >= 2,
        };
        if done {
            bench.write(self.drafts.len());
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
        self.bench = Some(bench);
    }
}
