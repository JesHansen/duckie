# Performance notes

## V1 budget

The [current owner decisions](IMPLEMENTATION_STATUS.md#owner-decisions-and-accepted-scope) set the v1 memory ceiling at **below 500 MB** (500,000,000 bytes), superseding the proposal's 80 MiB figure. Account for the desktop and any active test worker when validating workload peaks.

Targets below come from [ARCHITECTURE.md](ARCHITECTURE.md#9-performance-acceptance-targets), with later owner decisions taking precedence. Owner acceptance is distinct from measured coverage: a CPU-only frame measurement or an unrecorded cold-launch walkthrough does not prove every original target passes.

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

The fixture generator creates deterministic 1,000- and 10,000-request collections and a 10+ MiB OpenAPI document under the gitignored `fixtures/` directory. Reuse byte-identical fixtures for before/after comparisons. Measure each target from the baseline it names; collection restore is timed after the first usable frame, not from process start.

## Results, 13 September 2026

| Measurement | Target | Measured | |
| --- | --- | --- | --- |
| Warm launch to first frame, empty workspace, 30 trials | p95 ≤ 300 ms | p95 **167 ms** | pass |
| Restored 1,000-request collection, first frame, 20 trials | p95 ≤ 300 ms | p95 **180 ms** | pass |
| Restored 1,000-request collection searchable, 20 trials | p95 ≤ 500 ms **after first usable frame** | p95 **439 ms** (min 339, median 413) | pass |
| Idle private bytes, empty workspace, no retained response | ≤ 80 MiB proposed; 500 MB v1 ceiling | **94.0 MiB** (working set 64.5 MiB) | over proposal, well inside v1 |
| Idle CPU, 60 s sampled after 30 s without input | ≤ 0.1% of machine | **0.0016%** | pass |
| Frame construction under input, 1,000 requests and a coloured 64 KiB response | p95 input-to-paint ≤ 16 ms | p95 **7.0 ms** (median 6.4) | CPU portion measured; full target not measured |
| Warm local request dispatch | p95 ≤ 5 ms | p95 **0.62 ms**, including a loopback round trip | pass |
| Cold test worker overhead, 1 KiB response, trivial test | p95 ≤ 150 ms | p95 **19 ms** (median 15) | pass |
| 50 MiB response, every shape below | ≤ 32 MiB over settled baseline | **3.8–25.0 MiB** | pass |
| Cold launch | p95 ≤ 1 s | owner-confirmed acceptable after a reboot; no figure captured | judgement, not a measurement |

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
| Trickled in 64 KiB pieces over 12.6 s | 14.4 MiB |

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

### Deferred body/test loading, 13 September 2026

`Collection::open` used to read every body and test file into memory regardless of whether the request was ever viewed. It now hashes and discards them during open (for conflict detection) and reads the real content again from disk only when a request is first selected or sent (`ensure_loaded`). This is a single manual before/after comparison, not the p95 protocol above: one bench-instrumented trial per configuration, opening `fixtures/collection-10k` (10,000 requests, 2,000 of them with a 2+ KiB JSON body and a small test, matching the `hasExtra` fixture split).

| Configuration | Peak private bytes | Private bytes after a 300 ms settle |
| --- | ---: | ---: |
| Eager (every request hydrated at open, simulating the prior behaviour) | 151.7 MiB | 151.7 MiB |
| Deferred (this change) | 143.8 MiB | 143.5 MiB |
| Delta | **~8.0 MiB** | **~8.3 MiB** |

The first attempt at this comparison showed almost no difference (~3 MiB), which turned out to be a real finding about the measurement, not the feature: the attachment-reading pass batched every file's bytes into one `Vec` before discarding any of them, so the transient peak was close to the eager case regardless of what was kept afterward. `read_many` now applies its per-file transform (a hash, for this pass) at the point each file is read, so a large batch is never resident all at once — this is what the table above measures, and it is also a cheaper API when a caller genuinely never needs the bytes back.

Two caveats: 20% of requests carrying an extra file is this fixture's shape, not a general ratio, and process-level peak/current-after-settle readings carry more run-to-run noise than the p95 figures above, which average many trials. Restore *time* is essentially unaffected — the same files are still read and hashed at open either way — matching the existing caution that this work targets memory, not the restore-time budget.

### Bounded session history, 15 September 2026

The session-history implementation shares each response's existing `BodyHandle` rather than copying its body, retains at most 20 MiB of history bodies, and keeps at most 100 metadata entries. The regression suite directly verifies both limits and that the oldest body is evicted while its metadata remains.

A single release/bench trial exercised the existing 50 MiB response path after history capture was added. It measured 108,273,664 settled private bytes, 134,541,312 peak private bytes, and a **25.05 MiB** delta, remaining inside the 32 MiB response delta target and the 500 MB product ceiling. Raw output is in [`artifacts/history-retention-2026-09-15.json`](artifacts/history-retention-2026-09-15.json). This was one focused comparison, not the full multi-trial p95 protocol, and does not measure a history filled to its 20 MiB aggregate cap.

### What the frame number does and does not include

Frame construction is CPU time to produce a frame, measured headlessly with a keystroke on every other frame, a 1,000-request sidebar, and a syntax-coloured response with an active find. The size is the worst case that still colours, just under the 64 KiB cap; a 1 MiB page draws plain and costs 3.6 ms. Colouring is what makes the difference — the same megabyte cost 73 ms per frame when coloured, which is why the cap exists: 25 KiB measured 3.7 ms, 50 KiB 7.2 ms, 100 KiB 12.9 ms and 200 KiB 28.6 ms. It excludes texture upload, present and the compositor, so the reported p95 7.0 ms is a lower bound on input-to-paint, not proof that the full 16 ms target passes. The notes also report a 20.9 ms maximum attributed to first-frame font-atlas construction.

## Renderer selection

| Renderer | Sampled idle private memory | Result |
| --- | ---: | --- |
| wgpu | 374.3 MiB | Under revised ceiling; higher baseline |
| glow / OpenGL | 94.0 MiB | Selected; confirmed working in an Omnissa Horizon session |

An earlier glow sample read 146.3 MiB; the current build measures 94.0 MiB on an empty scratch workspace. Raw samples from the first session are in `artifacts/idle-*.json` and predate that improvement.

## Binary sizes

Built without the development screenshot and bench features: desktop 13,631,488 bytes, worker 1,401,856 bytes. The packaged ZIP from `scripts/package.ps1`, which also emits `THIRD_PARTY_NOTICES.md`, is 6,547,262 bytes. The desktop binary grew by roughly 460 KB when the icon was added: a 221 KB multi-resolution `.ico` resource plus 64 KB of raw window-icon pixels.

## Caveats on the numbers above

- **Cold launch has no figure.** Defeating the Windows file cache needs a reboot or standby-list tooling wanting administrator rights, and copying the binary does not help because writing it leaves it cached. The owner confirmed a post-reboot launch was acceptable, which closes the gate as a judgement; `scripts/benchmark.ps1` with a bench build will produce a number if one is ever wanted.
- **Frame construction is a lower bound on input-to-paint**, excluding upload, present and the compositor.
- Figures here cover the GUI process. Test workers are separate processes and are not included in any peak above.
