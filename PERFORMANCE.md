# Performance notes

## V1 budget

On 13 September 2026 the owner changed the v1 memory requirement to **below 500 MB**. Use 500,000,000 bytes as the conservative ceiling. The proposal's 80 MiB figure is superseded for v1. Account for the desktop and any active test worker when validating workload peaks.

Targets below are the release gates proposed in ARCHITECTURE.md. Where a measurement misses, it is recorded as a miss, not relaxed.

## Measurement setup

Windows 11 Pro 10.0.26200; AMD Ryzen 7 5800X, 16 logical processors; NVIDIA GTX 1080 Ti, driver 32.0.15.8180; 125% display scaling; glow (OpenGL) renderer. Release binaries, outside a debugger.

Launch and response figures come from a build with the development `bench` feature, which writes epoch milestones the harness pairs with the OS-reported process creation time, so loader cost before `main()` is included. Peak memory is the kernel's `PeakPagefileUsage`, read at exit, which avoids the sampling error an external poller has over a run lasting a few hundred milliseconds.

```powershell
node scripts/make-fixtures.mjs                                       # fixtures the runs need
cargo build --workspace --release --features duckie-desktop/bench
./scripts/benchmark.ps1                                              # launch and restore
cargo test -p duckie-test-worker --release --test measurements -- --ignored --nocapture
cargo test -p duckie-desktop --release -- --ignored --nocapture      # frame construction
```

## Results, 13 September 2026

| Measurement | Target | Measured | |
| --- | --- | --- | --- |
| Warm launch to first frame, empty workspace, 30 trials | p95 ≤ 300 ms | p95 **167 ms** | pass |
| Restored 1,000-request collection, first frame, 20 trials | p95 ≤ 300 ms | p95 **180 ms** | pass |
| Restored 1,000-request collection searchable, 20 trials | p95 ≤ 500 ms **after first usable frame** | p95 **439 ms** (min 339, median 413) | pass |
| Idle private bytes, empty workspace, no retained response | ≤ 80 MiB proposed; 500 MB v1 ceiling | **94.0 MiB** (working set 64.5 MiB) | over proposal, well inside v1 |
| Idle CPU, 60 s sampled after 30 s without input | ≤ 0.1% of machine | **0.0016%** | pass |
| Frame construction under input, 1,000 requests and a 1 MiB response | p95 input-to-paint ≤ 16 ms | p95 **2.7 ms** (median 2.4) | pass as a lower bound, see below |
| Warm local request dispatch | p95 ≤ 5 ms | p95 **0.62 ms**, including a loopback round trip | pass |
| Cold test worker overhead, 1 KiB response, trivial test | p95 ≤ 150 ms | p95 **19 ms** (median 15) | pass |
| 50 MiB response, every shape below | ≤ 32 MiB over settled baseline | **3.8–25.0 MiB** | pass |

Supporting numbers: a whole-body find across 50 MiB completes in 23 ms; GPU texture uploads over 40 frames total 267 KB, which is the font atlas and nothing else.

### Response shapes

All against the 32 MiB delta budget, body 50 MiB unless stated.

| Shape | Delta over baseline |
| --- | ---: |
| Multi-line text | 3.8 MiB |
| Single unbroken line | 25.0 MiB |
| gzip-encoded | 11.7 MiB |
| Binary, NUL bytes throughout | 4.1 MiB |
| 60 MiB against a 50 MiB cap, truncated | 3.9 MiB |

Compressed bodies decode as a bounded stream rather than into memory, and a body that exceeds its cap stops at the limit without a full-body copy. The single-line case is the expensive one, discussed below; it was 213.7 MiB before the preview page was sized to its content.

### A correction: restore was measured against the wrong baseline

Restore was reported as missing at p95 995 ms, then 618 ms, both measured from **process start**. The target reads "p95 ≤ 500 ms **after first usable frame**". Measured as specified it is **439 ms, a pass**.

The optimisation work was not wasted — `Collection::open` alone took 725 ms before it, which missed on either baseline — but the status reported after that work was wrong, and wrong because of how it was measured rather than anything in the code.

Phases of a restore, median of 12 trials:

| Phase | Median |
| --- | ---: |
| Process start to the background job starting | 151 ms |
| `Collection::open` itself | 289 ms |
| Open finishing to the first searchable frame | 108 ms |
| Total from process start | 545 ms |

The first 151 ms is the window and GL context coming up before `Duckie::new` runs, which is also why the first frame lands at 156 ms. Loading cannot start earlier without restructuring startup to begin the read before `eframe::run_native` — possible, but not needed to meet the target.

### Long lines are the expensive shape

Holding the shape single-line and varying the body from 1 MiB to 50 MiB left the delta flat at about 213 MiB, which proved the cost was laying out one preview page rather than anything holding the body. This is not a synthetic case: `pretty` is only computed for bodies of 1 MiB or less, so every larger response renders raw, and a large minified JSON reply is exactly one enormous line.

The preview page is now sized to its content — under 4 KiB per line keeps the full 1 MiB page, longer lines page in 128 KiB steps — which brought the case to 25.1 MiB, inside the budget, leaving multi-line untouched. Each page is still one text widget, so selection and copy across a page are unchanged.

### What the frame number does and does not include

Frame construction is CPU time to produce a frame, measured headlessly with a keystroke on every other frame, a 1,000-request sidebar, and a 1 MiB response with an active find. It excludes texture upload, present and the compositor, so it is a lower bound on input-to-paint rather than the thing the target names. At p95 2.7 ms it leaves roughly 13 ms of headroom inside a 16 ms budget. The 20.9 ms maximum is the first frame, which builds the font atlas.

## Renderer selection

| Renderer | Sampled idle private memory | Result |
| --- | ---: | --- |
| wgpu | 374.3 MiB | Under revised ceiling; higher baseline |
| glow / OpenGL | 94.0 MiB | Selected; confirmed working in an Omnissa Horizon session |

An earlier glow sample read 146.3 MiB; the current build measures 94.0 MiB on an empty scratch workspace. Raw samples from the first session are in `artifacts/idle-*.json` and predate that improvement.

## Binary sizes

Built without the development screenshot and bench features: desktop 13,575,168 bytes, worker 1,401,856 bytes. The packaged ZIP from `scripts/package.ps1`, which also emits `THIRD_PARTY_NOTICES.md`, is 6,525,037 bytes. The desktop binary grew by roughly 460 KB when the icon was added: a 221 KB multi-resolution `.ico` resource plus 64 KB of raw window-icon pixels.

## Still unmeasured

- **Cold launch**, p95 ≤ 1 s over 20 controlled trials. Defeating the Windows file cache needs either a reboot or standby-list tooling wanting administrator rights, and copying the binary does not help because writing it leaves it cached. A single genuine cold launch after a reboot is the practical substitute and has not been taken.
- **Slow-streaming peak.** The fixture server's `/slow` endpoint exists and cancellation is covered by the transport tests, but no memory figure has been taken for a response that trickles in.
- Figures here cover the GUI process. Test workers are separate processes and are not included in any peak above.
