# Performance notes

## V1 budget

On 13 September 2026, the user changed the v1 memory requirement to **below 500 MB**. Use 500,000,000 bytes as the conservative ceiling. The proposal's 80 MiB figure is superseded for v1. Account for the desktop and any active test worker when validating workload peaks.

The per-measurement targets referenced below are the release gates proposed in ARCHITECTURE.md. Where a measurement misses its target it is recorded as a miss, not relaxed.

## Measurement setup

Windows 11 Pro 10.0.26200; AMD Ryzen 7 5800X, 16 logical processors; NVIDIA GTX 1080 Ti, driver 32.0.15.8180; 125% display scaling; glow (OpenGL) renderer. Release binaries built with `--features duckie-desktop/bench`, run outside a debugger.

Milestones are written by the app itself as absolute epoch milliseconds; `scripts/benchmark.ps1` subtracts the OS-reported process creation time, so loader cost before `main()` is included. Reproduce with:

```powershell
cargo build --workspace --release --features duckie-desktop/bench
./scripts/benchmark.ps1
```

Peak memory is the kernel's `PeakPagefileUsage` for the GUI process, read at exit. Reading the kernel's own peak avoids the sampling error an external poller would have over a run lasting a few hundred milliseconds.

## Results, 13 September 2026

| Measurement | Target | Measured | |
| --- | --- | --- | --- |
| Warm launch to first frame, empty workspace, 30 trials | p95 ≤ 300 ms | **p95 167 ms** (min 156, median 159, max 173) | pass |
| 50 MiB response, 80-column lines, tests disabled | ≤ 32 MiB over settled baseline | **+4.0 MiB** | pass |
| 50 MiB response, single line, tests disabled | ≤ 32 MiB over settled baseline | **+213.7 MiB** | **miss, 6.7×** |
| Restored 1,000-request collection, first frame, 20 trials | p95 ≤ 300 ms | p95 180 ms (min 162, median 172) | pass |
| Restored 1,000-request collection searchable, 20 trials | p95 ≤ 500 ms | **p95 995 ms** (min 922, median 957) | **miss, 2×** |
| Idle private bytes, empty workspace (earlier smoke sample) | ≤ 80 MiB proposed; 500 MB v1 ceiling | 146.3 MiB | misses proposal, within v1 ceiling |

The 10,000-request stress collection, which has no stated target, becomes searchable in 8.9–9.0 s over three trials at a peak of 154.0 MB private.

### Collections load eagerly, so restore misses by 2×

The window itself appears on time — first frame is 180 ms even with a 1,000-request collection on the command line, because loading runs as a background job. What misses is the point at which the sidebar is populated and searchable.

`Collection::open` reads every referenced body and test file up front. The 1,000-request fixture carries 200 bodies and 200 test files, so restore pays 1,401 file reads before the list appears; the 10,000-request fixture pays 14,001 and takes nine seconds. The architecture's target states the remedy in the same breath as the number — "load bodies/tests lazily" — and that is the open item in IMPLEMENTATION_STATUS.md item 5.

Fixtures come from `node scripts/make-fixtures.mjs` and are deterministic, so a before-and-after comparison is not polluted by fixture churn. They load cleanly through `Collection::open`, and `fixtures/openapi-10mb.json` (10,939,528 bytes) imports to 13,459 operations with no diagnostics.

### The single-line response is the one real failure

A 50 MiB body split into 80-column lines settles 4.0 MiB above baseline. The same 50 MiB as one unbroken line costs 213.7 MiB. The body itself is not the cost — holding response size constant and varying it from 1 MiB to 50 MiB leaves the delta flat:

| Body size, single line | 1 MiB | 4 MiB | 16 MiB | 50 MiB |
| --- | ---: | ---: | ---: | ---: |
| Delta over baseline | 216.3 MiB | 213.6 MiB | 213.5 MiB | 213.5 MiB |

That flatness is the useful result: **streaming, the 2 MiB memory-to-file spill and the 1 MiB preview paging all work as designed — there is no full-body copy.** The cost is entirely in laying out one preview page, and it is paid whether the body is 1 MiB or 50 MiB. At roughly 213 bytes per character it is the per-glyph layout and tessellated mesh egui builds for a 1,048,576-character row.

Absolute peak in the failing case is about 314 MiB, so the v1 500 MB ceiling still holds — but the delta budget exists specifically to prove bounded buffers, and on this shape it does not.

The fix is the **virtual text pages** item in IMPLEMENTATION_STATUS.md item 6: hand the text widget only the visible window rather than a whole page. The measurement above is the regression test for it.

Disabling wrapping in the code editors, added the same day, is not the cause and mildly helps: wrapped, the same case costs 228.9 MiB against 213.7 MiB unwrapped.

## Renderer selection

| Renderer | Sampled idle private memory | Result |
| --- | ---: | --- |
| wgpu | 374.3 MiB | Under revised ceiling; higher baseline |
| glow / OpenGL | 146.3 MiB | Selected |

The glow sample was 153,415,680 private bytes and roughly 119.7 MiB working set, taken on an empty scratch workspace with no response or test worker. Raw samples are in `artifacts/idle-start.json`, `artifacts/idle-cpu-start.json` and `artifacts/idle-result.json`. These predate the day's editor, storage and import changes and have not been retaken.

## Binary sizes

Built without the development screenshot and bench features: desktop 13,111,808 bytes, worker 1,401,856 bytes. The packaged ZIP from `scripts/package.ps1`, which also emits `THIRD_PARTY_NOTICES.md`, is 6,304,963 bytes. The earlier 13,130,752-byte desktop figure included the screenshot feature; packaging now refuses a binary containing either development marker.

## Still unmeasured

These remain open against the ARCHITECTURE.md table and must not be treated as passing:

- **Cold launch** p95 ≤ 1 s over 20 trials. Needs controlled cache eviction between trials; the harness does not yet do this.
- **Input-to-paint latency** p95 ≤ 16 ms, and **GPU allocations**. Neither is instrumented.
- **Warm local request dispatch** p95 ≤ 5 ms, and **cold test worker overhead** p95 ≤ 150 ms.
- **Idle CPU** ≤ 0.1% over 60 s without input.
- Remaining response shapes from the protocol: binary, compressed, truncated and slow-streaming. Only uncompressed single-line and 80-column text have been measured.
- Peak memory covers the GUI process only. Test workers are separate processes and are not included in any figure here.
