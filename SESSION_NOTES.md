# Checkpoint — 13 September 2026

After you have read the other documents in this repo root, you may continue automatically.

## Owner decisions

- Build Duckie v1 from ARCHITECTURE.md and UI_DESIGN.md. Those two documents have never been edited.
- **Memory ceiling is below 500 MB** for v1. Treat 500,000,000 bytes as the limit. The proposal's 80 MiB figure is superseded.
- **MIT licensed**, copyright Jes Bak Hansen. Dependency licenses were reviewed: all 254 linked crates are permissive, none copyleft.
- **The build stays unsigned.** A certificate is a recurring cost that buys nothing for a single-user tool, and a self-signed one would not quiet SmartScreen. No installer; the portable ZIP is the delivery.
- **Out of scope for v1:** accessibility (Narrator, IME, keyboard-only walkthroughs), corporate root certificates, and PAC/WPAD or authenticated enterprise proxies. Those code paths remain in the build and are simply uncertified.
- **Verified by the owner on a real machine:** Omnissa Horizon (the environment they actually work in), 150%/200% display scale with monitor changes, a post-reboot cold launch, and the packaged ZIP running on a machine with no toolchain. All are owner judgements, not captured measurements.
- Token economy matters. Act as an orchestrator and delegate ordinary implementation to Sonnet subagents where that saves budget; do harder work directly.

## Current state

- Git repository on `main`, no remote, everything committed. Working tree clean.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` all pass. **64 tests.**
- `dist/Duckie-0.1.0-windows-x64.zip` matches `main` as of this checkpoint.
- **Four of seven release gates are closed:** performance (1), Windows validation (2), editor polish (4) and packaging (7). What remains is gates 3, 5 and 6, all feature work.

## Remaining work, in priority order

Fuller descriptions live in IMPLEMENTATION_STATUS.md. This is the running order agreed with the owner.

**Tier B — correctness edges (next up)**

1. **Exact path-value encoding rules** (gate 6). **Confirmed broken** — see the finding below. This is the item to start with.
2. **Charset and binary preview options** (gate 6). Non-UTF-8 text is currently classed as binary and offered for saving rather than mangled, so this is a gap rather than a bug.
3. **Strict multi-process transaction locking** (gate 5). Content-hash conflict detection already blocks the damaging case; locking would make the failure earlier and clearer.

**Tier C — import completeness (gate 3), only felt when a spec uses one**

4. Multipart and richer media handling.
5. Relative servers resolved against source identity — more relevant now that multi-file specs import.
6. Sanitized source provenance and reviewed reimport/update diffs, so an updated spec can be re-imported without losing edits.
7. External example diagnostics.

**Tier D — small leftovers**

8. Lazy test/body loading (gate 5), for memory on large collections only. Measured at 28 ms of a 725 ms open, so it carries no speed claim.

**Decided against, or deliberately dropped:** cross-origin `$ref` fetching (the spec's credentials would travel with it; revisitable behind an explicit opt-in) · virtual text pages (the memory budget was closed by page sizing instead, and this would break drag-selection across a page) · configurable connect timeout · proxy-secret binding (drops with enterprise proxies out of scope) · structured-query editing around opaque templates · F6 region traversal (dropped with accessibility).

## Open finding: path values can silently change the request

Probed by substituting values into `https://api.test/items/{{request.id}}/detail`:

| Value | Result | |
| --- | --- | --- |
| `a/b` | `/items/a/b/detail` | becomes an extra path segment |
| `a?b` | `/items/a?b/detail` | **`?` starts a query**; the path truncates to `/items/a` |
| `..` | `/detail` | the segment is resolved away |
| `a#b` | rejected | an existing guard catches fragments |
| `a b`, `ü` | `%20`, `%C3%BC` | already correct |
| `a%2Fb` | passes through | pre-encoded values work today and must keep working |

The first three are silent: a wrong request is sent with no error.

**The fix, and the trap in it.** `prepare` interpolates variables into the URL as a string and then parses it, so it cannot tell a path parameter from a base URL. Blanket-escaping every substitution would destroy `{{env.baseUrl}}/echo`, which every example collection and every OpenAPI import uses.

The namespace looks like the right seam: escape `request.` values (path parameters, which is what the importer emits for `{id}`) and leave `env.` and `secret.` alone (base URLs). Keep the escape narrow — `/`, `?`, `#`, and a value that is exactly `.` or `..` — rather than full percent-encoding, so the `label` and `matrix` path styles keep working; those deliberately carry `.`, `;`, `=` and `,`. Leave `%` alone so pre-encoded values still work.

**Still unverified:** whether `Url::parse` normalises `%2E%2E` back to `..`, which would defeat the traversal part of the fix. Check that before relying on it.

## Implementation cautions

- **Measure against the baseline the target names.** Restore was reported as missing twice because it was measured from process start when ARCHITECTURE.md says "after first usable frame". PERFORMANCE.md records the correction.
- **Syntax colouring is capped at 64 KiB** (`editor::MAX_COLOURED`). It turns one run of text into thousands of formatted sections; a megabyte cost 73 ms per frame against a 16 ms budget. Past the cap the text draws plain and the UI says so. The frame measurement defaults to the worst case that still colours, so a regression is visible — but only if the fixture actually colours: an early version measured nothing because the test response carried no `content-type`.
- `Save` writes the entire collection, not just the selected request. Order lives in the manifest, so reordering marks it dirty even though no request changed.
- The spool sweep's 24-hour threshold is a safety property, not hygiene: Windows opens files with `FILE_SHARE_DELETE`, so deleting a spool still open in another running instance would succeed. Do not shorten it without replacing the guard.
- Response find covers the whole body in the **Raw** view only. Pretty is a reformatted copy whose offsets do not map onto body bytes, so find stays page-local there and says so.
- External OpenAPI documents are inlined into the root before import. A fetched document's own `#/components/...` references must be rebased into its embedded copy, or after splicing they silently address the root's components.
- Object members expand in key order, not the order written, because serde_json sorts them. JSON gives member order no meaning and no parameter style depends on it.
- `Url::parse` reads a Windows drive letter as a one-character scheme, which turned `C:/specs/api.json` into a URL and lower-cased it. `as_url` in duckie-openapi guards against that.
- Egui 0.36 uses `App::ui`, `Panel::top/left/bottom` and `Context::run_ui`. Headless tests must clear `output.textures_delta` when not rendering it. `TextEdit::show` returns `TextEditOutput` whose `response` field is an `AtomLayoutResponse`, so the `Response` is `output.response.response`.
- The two end-to-end tests and the fixture generator need Node.js on `PATH`. They fail loudly rather than skipping.
- `DUCKIE_CAPTURE_PATH`, `DUCKIE_CAPTURE_THEME`, `DUCKIE_CAPTURE_VIEW` and `DUCKIE_BENCH_PATH` only exist with `--features duckie-desktop/screenshot` or `bench`. Packaging refuses a binary containing either marker.
- PowerShell unrolls an array returned from a function; `scripts/make-icon.ps1` needs the unary comma, and silently produced a 176-byte icon without it. Python heredocs that patch Rust mangle backslash escapes — write the patch to a file, or use the editing tools.

## Reproducing the measurements

```powershell
node scripts/make-fixtures.mjs                                       # 1k/10k collections, 10 MiB spec
cargo build --workspace --release --features duckie-desktop/bench
./scripts/benchmark.ps1                                              # launch and restore
cargo test -p duckie-test-worker --release --test measurements -- --ignored --nocapture
cargo test -p duckie-desktop --release -- --ignored --nocapture      # frame construction
```

Fixtures are gitignored and deterministic, so a before-and-after comparison is not polluted by fixture churn. `scripts/make-icon.ps1` regenerates the icon from `assets/duckie.png`; its output is committed and only needs rerunning when the artwork changes.
