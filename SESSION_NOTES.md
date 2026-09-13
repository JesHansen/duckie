# Checkpoint — 13 September 2026 (second session)

After you have read all relevant files here in the repo root, you may continue automatically.

## User decisions

- Build Duckie v1 from ARCHITECTURE.md and UI_DESIGN.md.
- The user explicitly relaxed the initial memory requirement: **keep v1 below 500 MB**. Treat 500,000,000 bytes as the conservative ceiling. The original 80 MiB proposal is no longer the v1 requirement.
- The project is **MIT licensed** (decided 13 September 2026), copyright Jes Bak Hansen. Dependency licenses were reviewed then: all 254 linked crates are permissive, none copyleft.
- **Gate 2 narrowed for a single-user tool** (decided 13 September 2026). Out of scope for v1: accessibility (Narrator, IME, keyboard-only walkthroughs), corporate root certificates, and enterprise proxies — the owner does not need them for planned use. The features stay in the build and are simply uncertified. **Omnissa Horizon was tested on 13 September 2026 and works**, with startup subjectively comparable to local, so the OpenGL renderer choice holds in the environment the owner actually uses. Only display scaling remains open in gate 2.
- Token economy is at a premium. Act as an orchestrator and make heavy use of sub agents for implementation, if this saves on token cost / leaves more of my 5h window usable. For regular implementation tasks, spawn a Sonnet agent, for harder tasks implement them yourself without delegating to a sub agent.

## Current state

- Eight-crate Rust desktop workspace in `crates/`, with Cargo.lock. The original design documents remain unmodified.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` all pass. **42 tests**, up from 24 at the previous checkpoint.
- `dist/Duckie-0.1.0-windows-x64.zip` (6,303,547 bytes) was produced by `scripts/package.ps1` from a clean `--release --locked` build without the screenshot feature. Desktop 13,111,808 bytes; worker 1,401,856 bytes.
- The repository was initialized on `main` and the work committed. No remote is configured and nothing has been pushed. No public API request was made; all transport tests use loopback servers.

## What this session changed

1. **Verified the previous session's final UI edit.** Formatting, lints and the whole suite were clean; nothing needed correcting.
2. **`crates/duckie-test-worker/tests/end_to_end.rs` (new).** Two tests start `scripts/dev-server.mjs` on an ephemeral port and drive the real path — storage load, interpolation, HTTP, assertion evaluation in the real worker. One runs the shipped `examples/local-api` collection; the other covers the documented protected OpenAPI URL (401 unauthenticated, then import and send of every operation). These fixtures had never been exercised together before.
3. **Packaging ran end to end for the first time.** `scripts/package.ps1` now refuses to package a `duckie.exe` that still contains the `DUCKIE_CAPTURE_PATH` marker; the guard was verified by deliberately building with the feature and watching it reject.
4. **Assertion failure reporting (Sonnet agent).** QuickJS's `Error.prototype.stack` carries no message, so every failure previously showed a bare stack with the reason discarded. The test source now evaluates as `tests.js` (the harness as `duckie-harness.js`) via `EvalOptions::filename`, failures carry the message above the stack, and `TestCase.line` reports the first `tests.js` frame.
5. **`crates/duckie-desktop/src/editor.rs` (new).** Pinned line-number gutter, find with highlighted and navigable matches shared by the request editors and the response preview, Ctrl+F routed to the focused editor, and **Go to line N** from a failed assertion into the Tests editor. Editors no longer wrap, which is what makes the gutter's one-number-per-line mapping correct.
6. **OpenAPI array parameters.** Previously any array parameter blocked Send. Scalar-item arrays now serialize for `form` (exploded to one row per item, or joined), `spaceDelimited`, `pipeDelimited` and `simple`. `examples/openapi.json` gained an exploded `tags` array and the end-to-end test asserts it reaches the wire as `tags=duck&tags=yellow`.
7. **Storage hardening (Sonnet agent).** `duckie_http::sweep_stale_spool()` removes spool files untouched for over 24 hours and is spawned at startup; a document whose `schemaVersion` exceeds what this build writes is refused at load instead of being silently resaved; extension round-trips through save and reload are covered for nested objects and arrays.

## Next useful steps

1. Continue the prioritized backlog in IMPLEMENTATION_STATUS.md. The largest untouched areas are the **performance gate** (item 1), **Windows display-scale validation** (item 2, now nearly closed), and **lazy collection loading plus disk-change detection with a comparison UI** (item 5).
2. Remaining editor polish: bracket pairing and incremental syntax colors, F6 region traversal, and **grouped/collapsible sidebar folders with user ordering**. The sidebar is a virtualized flat list (`show_rows`, uniform 38 px); grouping means flattening to a display list of header and request entries so virtualization survives, plus a collapsed-folder set in `Duckie`.
3. Remaining OpenAPI work: relative and remote references, object parameters (`deepObject`, exploded `form` objects), `label`/`matrix` path styles, richer media handling and reimport diffs. Keep the existing rule — unsupported cases stay blocked rather than guessed.
4. Re-measure memory and startup: the PERFORMANCE.md samples predate this session's changes.
5. The ZIP has not been tried on a clean Windows machine without a toolchain, and is unsigned.

## Implementation cautions

- The two end-to-end tests **require Node.js on `PATH`** and fail loudly rather than skipping when it is missing. CI (`windows-latest`) has it. `repository_root()` in that file is deliberately not canonicalized: Node's module loader cannot resolve the `\\?\` verbatim prefix Windows canonicalization adds.
- `Save` writes the entire collection, not just the selected request. UI editing is disabled while saves are pending; in-memory drafts survive errors.
- The spool sweep's 24-hour threshold is a safety property, not hygiene: Windows opens files with `FILE_SHARE_DELETE`, so deleting a spool still open in another running instance would succeed. Do not shorten it without replacing the guard.
- Tests use fresh worker processes per evaluation instead of 30-second idle reuse, so idle test-worker count is zero.
- Find is case-sensitive and stops after 5,000 matches on one preview page. The response preview's editor id carries the request id so each request keeps its own scroll position.
- Egui 0.36 uses `App::ui`, `Panel::top/left/bottom`, and `Context::run_ui`, unlike older APIs. Headless tests must explicitly clear `output.textures_delta` when not rendering it. `TextEdit::show` returns `TextEditOutput` whose `response` field is an `AtomLayoutResponse`, so the `Response` is `output.response.response`.
- `DUCKIE_CAPTURE_PATH`, `DUCKIE_CAPTURE_THEME` and `DUCKIE_CAPTURE_VIEW` only work with `--features duckie-desktop/screenshot`; `DUCKIE_CAPTURE_VIEW=editor` seeds a draft so a capture shows the gutter and find highlighting. The capture process needs explicit termination. Production packaging must omit this feature and the script now enforces that.
- Response find navigates and highlights, but UI error “Go to code” navigates only for test cases that carry a line.
