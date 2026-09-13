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
| 50 MiB response, 80-column lines, tests disabled | ≤ 32 MiB over settled baseline | **+4.2 MiB** | pass |
| 50 MiB response, single line, tests disabled | ≤ 32 MiB over settled baseline | **+25.1 MiB**, from 213.7 MiB | pass |
| Restored 1,000-request collection, first frame, 20 trials | p95 ≤ 300 ms | p95 180 ms (min 162, median 172) | pass |
| Restored 1,000-request collection searchable, 20 trials | p95 ≤ 500 ms | **p95 618 ms** (min 533, median 566), from 995 ms | **still misses** |
| Idle private bytes, empty workspace (earlier smoke sample) | ≤ 80 MiB proposed; 500 MB v1 ceiling | 146.3 MiB | misses proposal, within v1 ceiling |

The 10,000-request stress collection, which has no stated target, became searchable in 8.9–9.0 s before the change below.

### Restore: 2.6× faster in storage, still short of target

The window itself appears on time — first frame is 180 ms even with a 1,000-request collection on the command line, because loading runs as a background job. What misses is the point at which the sidebar is populated and searchable.

The obvious suspect was eager body and test loading, which item 5 calls out. **Profiling said otherwise.** For the 1,000-request fixture, of 725 ms inside `Collection::open`:

| Step | Cost |
| --- | ---: |
| `managed_path` × 1,000 | 178 ms |
| … of which re-canonicalizing the already-canonical root | 60 ms |
| … of which the symlink-escape check on the file | 71 ms |
| Reading 1,000 request files | 70 ms |
| Reading 400 body and test files | **28 ms** |
| Parsing 1,000 request files | 2 ms |
| SHA-256 of bytes already in memory | 0 ms |

Eager loading was 4% of the time. Lazy loading would have bought almost nothing. The real costs were that `open` resolved and **read every file twice** — once for content, once more inside `track` to hash it — and that path validation re-canonicalized the collection root on every call.

Three changes, in `duckie-storage`:

1. Hash the bytes already in hand rather than re-reading each file.
2. Resolve paths against the root canonicalized once at open. The escape check still runs for every path; only the redundant root syscall is gone.
3. Read files in parallel. Opening a collection is syscall-bound, not CPU-bound, and on Windows every file open also pays antivirus filtering, so this is where the remaining time was.

`Collection::open` went from 725 ms to **280 ms** for 1,000 requests and from 8,180 ms to **2,833 ms** for 10,000. End to end, restore went from p95 995 ms to **p95 618 ms**.

That still misses the 500 ms target, and the residual is not yet explained. `Collection::open` accounts for 280 ms and the first frame for about 172 ms, and the job starts before the first frame, so roughly 180 ms is unaccounted for. Two plausible causes were measured and **both ruled out**: building the 1,000 drafts on the UI thread costs 542 µs, and body/test loading is the 28 ms above. Finding the rest needs in-app instrumentation of the background job rather than more guessing. Lazy body/test loading remains worth doing for memory on large collections, but it is not the restore bottleneck and should not be sold as one.

Fixtures come from `node scripts/make-fixtures.mjs` and are deterministic, so a before-and-after comparison is not polluted by fixture churn. They load cleanly through `Collection::open`, and `fixtures/openapi-10mb.json` (10,939,528 bytes) imports to 13,459 operations with no diagnostics.

### Long lines: fixed by sizing the preview page to its content

A 50 MiB body split into 80-column lines settles 4.0 MiB above baseline. The same 50 MiB as one unbroken line costs 213.7 MiB. The body itself is not the cost — holding response size constant and varying it from 1 MiB to 50 MiB leaves the delta flat:

| Body size, single line | 1 MiB | 4 MiB | 16 MiB | 50 MiB |
| --- | ---: | ---: | ---: | ---: |
| Delta over baseline | 216.3 MiB | 213.6 MiB | 213.5 MiB | 213.5 MiB |

That flatness is the useful result: **streaming, the 2 MiB memory-to-file spill and the 1 MiB preview paging all work as designed — there is no full-body copy.** The cost is entirely in laying out one preview page, and it is paid whether the body is 1 MiB or 50 MiB. At roughly 213 bytes per character it is the per-glyph layout and tessellated mesh egui builds for a 1,048,576-character row.

Absolute peak in the failing case is about 314 MiB, so the v1 500 MB ceiling still holds — but the delta budget exists specifically to prove bounded buffers, and on this shape it does not.

This matters more than the synthetic fixture suggests: `pretty` is only computed for bodies of
1 MiB or less, so **every larger response renders raw**. A big minified JSON reply — the ordinary
shape of an API response — is therefore exactly one enormous line.

The preview page is now sized to its content (`page_size` in `state.rs`): a body whose longest
line is under 4 KiB keeps the full 1 MiB page, and anything with longer lines pages in 128 KiB
steps instead. The response view already paged at 1 MiB with `Showing bytes X–Y of Z` and
Previous/Next, so this reuses controls that were already on screen, and each page is still a
single text widget — drag-selection and copy across a page are unchanged.

That took the single-line case from 213.7 MiB to **25.1 MiB**, inside the budget, leaving the
multi-line case untouched at 4.2 MiB. Paging always advances by exactly the number of bytes
shown, so a body that changes shape partway through still pages without gaps or overlap.

Full **virtual text pages** — keeping a 1 MiB page while rendering only the visible window — is
no longer needed for the budget and remains an optional improvement in IMPLEMENTATION_STATUS.md
item 6. It would break drag-selection across a page, so it needs a design that restores it.

Disabling wrapping in the code editors is not the cause and mildly helped: before this change,
wrapped cost 228.9 MiB against 213.7 MiB unwrapped.

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
