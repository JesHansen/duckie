# V1 implementation status

This document owns current decisions, delivery status, and remaining work. [README.md](README.md) describes using the application; [PERFORMANCE.md](PERFORMANCE.md) owns measurements and benchmark commands. The [architecture](ARCHITECTURE.md) and [UI design](UI_DESIGN.md) remain the original proposals, with the decisions below taking precedence.

## Owner decisions and accepted scope

Recorded on 13 September 2026:

- **Memory:** below 500 MB for v1, using 500,000,000 bytes as the conservative ceiling. The original 80 MiB proposal is superseded. Include the desktop and any active test worker when assessing workload peaks.
- **License:** MIT, copyright Jes Bak Hansen; see [LICENSE](LICENSE). Dependency review and notices are described in the README.
- **Delivery:** an unsigned portable Windows ZIP, with no installer planned. Signing was declined for this single-user tool; it is not an outstanding v1 gate.
- **Windows scope:** Narrator, IME, keyboard-only accessibility walkthroughs, corporate root certificates, and PAC/WPAD or authenticated enterprise proxies are outside v1 validation. AccessKit, native Windows TLS, and system-proxy support remain in the build but are uncertified for those uses.
- **Owner acceptance:** Omnissa Horizon, 150%/200% display scale with monitor changes, post-reboot cold launch, and running the packaged ZIP on a machine without a toolchain were reported acceptable. These are owner judgements, not captured benchmark measurements. The OpenGL renderer remains selected.

## Validation snapshot

The previous checkpoint at commit `fa0231c` recorded 64 passing tests, successful formatting and strict Clippy checks, and a matching `dist/Duckie-0.1.0-windows-x64.zip`. These are historical results from 13 September 2026, not verification of subsequent changes. Re-run relevant checks before claiming a newer build or package is validated.

Path-value fix validated on 13 September 2026 in the working tree: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` passed (71 tests passed; four performance measurements intentionally ignored). Both Node-backed end-to-end tests passed. The release binaries and portable ZIP were not rebuilt or revalidated for this change.

| Area | Recorded status | Remaining scope or qualification |
| --- | --- | --- |
| 1. Performance | Accepted for the agreed v1 scope | See PERFORMANCE.md. Cold launch has owner acceptance without a measured p95; CPU frame construction is only a lower bound on input-to-paint; reported GUI memory peaks exclude workers. |
| 2. Windows validation | Accepted for the agreed v1 scope | Horizon and display-scale walkthroughs accepted; excluded enterprise/accessibility cases remain uncertified. |
| 3. OpenAPI completeness | Open | Media handling, relative servers, source provenance, and reviewed reimport. |
| 4. Editor polish | Accepted for the agreed v1 scope | Line numbers, syntax colours, bracket matching, grouped ordering, find navigation, and assertion-line navigation implemented. F6 traversal was dropped with accessibility. |
| 5. Storage hardening | Open | Multi-process transaction locking and lazy body/test loading. |
| 6. Transport polish | Partially complete | Path-value encoding fixed; charset/binary options and safe diagnostic detail remain. |
| 7. Packaging | Accepted for the agreed v1 scope | MIT license and dependency review recorded; portable package accepted on a clean machine. Keep development features out of later packages. |

## Implemented foundation

- Eight Rust crates separate the model, storage, HTTP transport, OpenAPI import, assertion client/worker, orchestration, and native desktop UI.
- Scratch and saved requests support custom methods, ordered duplicate query/header rows, namespaced interpolation, run bindings, bearer/API-key authentication, and redacted request summaries. Secrets are environment-scoped and separate from ordinary request files.
- JSON/text, URL-encoded form, multipart fields/files, and streamed file bodies work with native Windows TLS, HTTP/1.1 and HTTP/2, proxy selection, timeout and cancellation. Requests have no automatic retries or redirect following.
- Responses use bounded streaming decompression with separate encoded/decoded caps and a 2 MiB spill threshold. Preview pages adapt to long lines; at most three responses are retained. Raw-body find scans bounded chunks off the UI thread. Progress distinguishes received and decoded bytes.
- Send captures immutable input. One active run includes automatic tests; responses publish before evaluation finishes, and tests can rerun without HTTP. Fresh QuickJS worker processes have engine limits, watchdogs and Windows job limits. Assertion errors identify `tests.js` lines separately from harness frames.
- Collections use explicit saves, stable IDs, atomic replacement, recovery journals, content-hash conflict detection, focus-time external-change reporting, newer-schema rejection, and nested unknown-field roundtrips. Selection and split state restore per collection. Abandoned spool files are swept after 24 hours.
- OpenAPI 3.0/3.1/3.2 JSON imports from local files or protected URLs into a review draft. Internal, relative local and same-origin remote references resolve transitively with rebasing, bounded at 50 documents and 20 MiB. Import handles examples, read-only omission, server precedence, security alternatives/combined requirements, `allOf` merges, and selectable `oneOf`/`anyOf` bodies.
- Parameter serialization covers query `form`/`deepObject` objects and arrays, `spaceDelimited`/`pipeDelimited` arrays, and path `simple`/`label`/`matrix`, honoring supported `explode` combinations. Undefined forms such as arrays of objects remain blocked. URL path substitutions encode structural characters while preserving existing percent escapes and style punctuation; dot traversal segments are rejected before parsing.
- The native UI includes dark/light/system themes, file dialogs, retained dirty drafts, grouped/collapsible folders and ordering, line-numbered editors, bounded syntax colouring, bracket matching, focused-editor find, and response snapshot labels. Editor undo buffers are not persisted.

## Remaining work, in priority order

### Tier B — correctness edges

1. **Exact path-value encoding (area 6): completed 13 September 2026.** See the resolution and regression coverage below.
2. **Charset and binary preview options (area 6).** Non-UTF-8 text is currently classified as binary and offered for saving. Unknown or stacked content encodings produce an incomplete-body error. Improve safe source diagnostics where current errors lack actionable detail.
3. **Multi-process transaction locking (area 5).** Save-time content-hash checks already catch external changes; locking should coordinate the transaction itself across instances.

### Tier C — import completeness (area 3)

4. Multipart and richer request media handling.
5. Relative server URLs resolved against source identity, including multi-file specs.
6. Sanitized source provenance and reviewed reimport/update diffs that preserve local edits and tests.
7. External example diagnostics.

### Tier D — smaller follow-up

8. **Lazy test/body loading (area 5).** Investigate for memory savings on large collections. Earlier profiling attributed only 28 ms of a 725 ms open to this work; do not claim it is the restore bottleneck without new measurements.

### Deferred or excluded from v1

Cross-origin reference fetching is refused under the current credential-forwarding policy; reconsider only with explicit origin selection and credential isolation. Virtualized text pages were dropped after adaptive page sizing met the response-memory delta target and to preserve drag-selection across a page. Configurable connect timeout, proxy-secret bindings, structured-query editing around opaque templates, and F6 traversal are also outside the agreed remaining work.

Scenario execution, plugin marketplace, login, token refresh, shared cookie sessions, cloud sync, and automatic spec polling remain outside v1.

## Resolved: path values can change URL structure

The prior checkpoint reported these results when substituting into `https://api.test/items/{{request.id}}/detail`:

| Value | Observed result | Consequence |
| --- | --- | --- |
| `a/b` | `/items/a/b/detail` | Adds a path segment. |
| `a?b` | `/items/a?b/detail` | Starts a query; path truncates to `/items/a`. |
| `..` | `/detail` | Removes the preceding segment. |
| `a#b` | Rejected | Existing fragment guard catches it. |
| `a b`, `ü` | `%20`, `%C3%BC` | Already encoded correctly. |
| `a%2Fb` | Preserved | Existing pre-encoded values must continue to work. |

The first three cases previously sent a different request without an error. Regression coverage now verifies slash/question/hash/backslash encoding and rejection of dot traversal before transport, including exact HTTP request targets received by a loopback server.

`prepare` now determines substitution position after expanding preceding URL parts. Path values are encoded regardless of namespace; base URL and authority substitutions remain supported. Query, header, and body interpolation retain their previous behavior. Existing percent escapes and OpenAPI label/matrix punctuation are preserved. Import-to-prepare regressions cover simple, label, and matrix styles.

The URL parser does normalize percent-encoded dot segments. Encoding dots therefore cannot preserve them safely: complete `.` and `..` path segments are rejected, including literal, encoded, and composed forms. This also applies to literal URLs and base URL values. Path value whitespace and control bytes are encoded rather than silently stripped. These checks concern the URL Duckie sends; server-side decoding behavior remains server-defined.

URLs must start with `http://` or `https://` (case-insensitive). Parser shorthand such as `https:host/path` is rejected so it cannot bypass path-position checks.

## Implementation cautions

- **Save covers the collection.** All dirty drafts save together. Reordering changes the manifest and must mark it dirty even if no request content changed. Deletion removes the manifest entry on Save but retains unreferenced files. Body/test files currently load eagerly on a background job.
- **The 24-hour spool threshold protects other instances.** Windows permits deletion of files opened with `FILE_SHARE_DELETE`; a file can still be in use by another Duckie instance. Do not shorten the threshold without a replacement ownership guard.
- **Raw and Pretty searches have different offsets.** Raw find scans retained body bytes; Pretty is a reformatted copy, so its search remains page-local and labelled accordingly.
- **Reference rebasing is essential.** An external document's internal `#/components/...` references must point into its embedded copy after inlining, rather than the root document's components. `as_url` also distinguishes Windows drive paths from URL schemes.
- **Serialization order is deterministic.** `serde_json` sorts object members; no supported parameter style relies on authored object-key order.
- **Syntax colouring is capped at 64 KiB** by `editor::MAX_COLOURED`. Larger text draws plain. A performance fixture must have the correct content type to exercise colouring; see PERFORMANCE.md for the measurements and baseline rules.
- **Egui 0.36 APIs differ from older versions.** Use `App::ui`, `Panel::top/left/bottom`, and `Context::run_ui`. Headless tests must clear unapplied `output.textures_delta`. `TextEdit::show` yields an `AtomLayoutResponse`; its inner response is `output.response.response`.
- **Development dependencies are explicit.** The two end-to-end tests and fixture generator require Node.js on `PATH` and fail rather than silently skipping. The icon script needs PowerShell's unary comma when returning an array; preserve it when editing. Prefer structured patches to shell-generated source that can corrupt escapes.
- **Development features must not ship.** Screenshot variables (`DUCKIE_CAPTURE_PATH`, `DUCKIE_CAPTURE_THEME`, `DUCKIE_CAPTURE_VIEW`) and `DUCKIE_BENCH_PATH` belong to development builds. Packaging rejects binaries containing the capture/bench markers. Rebuild without those features before packaging.
- **Fresh workers are intentional.** Each test evaluation starts and stops its worker instead of retaining it for 30 seconds. Redirect navigation also intentionally creates a fresh GET draft without inherited credentials.
