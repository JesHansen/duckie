# V1 implementation status

This document owns current decisions, delivery status, and remaining work. [README.md](README.md) describes using the application; [PERFORMANCE.md](PERFORMANCE.md) owns measurements and benchmark commands. The [architecture](ARCHITECTURE.md) and [UI design](UI_DESIGN.md) remain the original proposals, with the decisions below taking precedence.

## Owner decisions and accepted scope

Recorded on 13 September 2026:

- **Memory:** below 500 MB for v1, using 500,000,000 bytes as the conservative ceiling. The original 80 MiB proposal is superseded. Include the desktop and any active test worker when assessing workload peaks.
- **License:** MIT, copyright Jes Bak Hansen; see [LICENSE](LICENSE). Dependency review and notices are described in the README.
- **Delivery:** an unsigned portable Windows ZIP, with no installer planned. Signing was declined for this single-user tool; it is not an outstanding v1 gate.
- **Windows scope:** Narrator, IME, keyboard-only accessibility walkthroughs, corporate root certificates, and PAC/WPAD or authenticated enterprise proxies are outside v1 validation. AccessKit, native Windows TLS, and system-proxy support remain in the build but are uncertified for those uses.
- **Owner acceptance:** Omnissa Horizon, 150%/200% display scale with monitor changes, post-reboot cold launch, and running the packaged ZIP on a machine without a toolchain were reported acceptable. These are owner judgements, not captured benchmark measurements. The OpenGL renderer remains selected.
- **Version:** bumped from 0.1.0 to 1.0.0 on 13 September 2026, once Tiers B, C and D were all closed. This is a version number change only; it does not itself rebuild or revalidate the release binaries or portable ZIP, and area statuses below are unaffected by it.

## Validation snapshot

The previous checkpoint at commit `fa0231c` recorded 64 passing tests, successful formatting and strict Clippy checks, and a matching `dist/Duckie-0.1.0-windows-x64.zip`. These are historical results from 13 September 2026, not verification of subsequent changes. Re-run relevant checks before claiming a newer build or package is validated.

Path-value fix validated on 13 September 2026 in the working tree: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` passed (71 tests passed; four performance measurements intentionally ignored). Both Node-backed end-to-end tests passed. The release binaries and portable ZIP were not rebuilt or revalidated for this change.

Charset/binary preview and content-encoding diagnostics validated on 13 September 2026: formatting, strict workspace Clippy, and all 78 non-measurement workspace tests passed; four performance measurements remained intentionally ignored. Both Node-backed end-to-end tests passed. Release binaries were rebuilt without development features, the 262 third-party dependency license expressions were reviewed as permissive, and `scripts/package.ps1` produced a refreshed portable ZIP. Performance measurements were not rerun because this chunk does not change the established release gates.

Multi-process transaction locking validated on 13 September 2026 in the working tree: formatting, strict workspace Clippy, and all 79 non-measurement workspace tests passed (one new test added); four performance measurements remained intentionally ignored. Both Node-backed end-to-end tests passed. Release binaries and the portable ZIP were not rebuilt or revalidated for this change.

OpenAPI import completeness (Tier C, all four items) validated on 13 September 2026 in the working tree: formatting, strict workspace Clippy, and all 87 non-measurement workspace tests passed (eight new tests in `duckie-openapi`); four performance measurements remained intentionally ignored. Both Node-backed end-to-end tests passed. The desktop binary was also launched directly (debug build, `DUCKIE_CAPTURE_PATH`) and rendered its first frame without error, as a smoke check on the changed import dialog; this did not exercise the File-menu "Update from spec…" entry interactively. Release binaries and the portable ZIP were not rebuilt or revalidated for this change.

Lazy body/test loading (Tier D, area 5) validated on 13 September 2026 in the working tree: formatting, strict workspace Clippy, and all 91 non-measurement workspace tests passed (five new tests, three in `duckie-storage` and two in `duckie-desktop`); four performance measurements remained intentionally ignored. Both Node-backed end-to-end tests passed, the second updated to load its two requests' content explicitly before sending, since that content is no longer loaded automatically. A manual before/after memory comparison is recorded in [PERFORMANCE.md](PERFORMANCE.md#deferred-bodytest-loading-13-september-2026); it is not part of the p95 benchmark protocol. Release binaries and the portable ZIP were not rebuilt or revalidated for this change.

Version bump to 1.0.0 validated on 13 September 2026 in the working tree: formatting, strict workspace Clippy, and all 91 non-measurement workspace tests passed, unchanged from the prior checkpoint; four performance measurements remained intentionally ignored. `Cargo.lock` was regenerated so all eight workspace crates show 1.0.0. `scripts/package.ps1` now reads the packaged version from `cargo metadata` instead of a hardcoded string, so the dist folder and zip name follow future bumps automatically; the script was run end-to-end and produced `dist/Duckie-1.0.0-windows-x64.zip`, confirming the new naming. Dependency license review and the earlier owner walkthroughs (Horizon, display scale, clean-machine launch) were not repeated for this change.

Security-review hardening validated on 13 September 2026 in the working tree: `cargo fmt --all -- --check`, strict workspace Clippy, and all 109 non-measurement workspace tests passed; four performance measurements remained intentionally ignored. Both Node-backed end-to-end tests passed. `cargo audit 0.22.2` checked all 473 locked packages against RustSec database commit `b50980aad8b8f14f77e25a97b32dd94bf008b0af` (updated 9 September 2026) and reported no vulnerabilities or warnings. Release binaries, the portable ZIP, and performance gates were not rebuilt or remeasured for this change.

| Area | Recorded status | Remaining scope or qualification |
| --- | --- | --- |
| 1. Performance | Accepted for the agreed v1 scope | See PERFORMANCE.md. Cold launch has owner acceptance without a measured p95; CPU frame construction is only a lower bound on input-to-paint; reported GUI memory peaks exclude workers. |
| 2. Windows validation | Accepted for the agreed v1 scope | Horizon and display-scale walkthroughs accepted; excluded enterprise/accessibility cases remain uncertified. |
| 3. OpenAPI completeness | Accepted for the agreed v1 scope | Multipart/binary body handling, relative server URLs, contained and bounded external acquisition, opaque external examples, generated-example budgets, and scoped reimport implemented; see below. |
| 4. Editor polish | Accepted for the agreed v1 scope | Line numbers, syntax colours, bracket matching, grouped ordering, find navigation, and assertion-line navigation implemented. F6 traversal was dropped with accessibility. |
| 5. Storage hardening | Accepted for the agreed v1 scope | Multi-process transaction locking, lazy body/test loading, and bounded managed-file reads implemented; see below. |
| 6. Transport polish | Accepted for the agreed v1 scope | Path-value encoding, literal response-derived redirect queries, charset-aware/binary preview choices, and safe content-encoding diagnostics implemented. |
| 7. Packaging | Accepted for the agreed v1 scope | MIT license and dependency review recorded; portable package accepted on a clean machine. Keep development features out of later packages. |

## Implemented foundation

- Eight Rust crates separate the model, storage, HTTP transport, OpenAPI import, assertion client/worker, orchestration, and native desktop UI.
- Scratch and saved requests support custom methods, ordered duplicate query/header rows, namespaced interpolation, run bindings, bearer/API-key authentication, and redacted request summaries. Secrets are environment-scoped and separate from ordinary request files.
- JSON/text, URL-encoded form, multipart fields/files, and streamed file bodies work with native Windows TLS, HTTP/1.1 and HTTP/2, proxy selection, timeout and cancellation. Requests have no automatic retries or redirect following.
- Responses use bounded streaming decompression with separate encoded/decoded caps and a 2 MiB spill threshold. Preview pages adapt to long lines; at most three responses are retained. Raw-body find scans bounded chunks off the UI thread. Progress distinguishes received and decoded bytes.
- Send captures immutable input. One active run includes automatic tests; responses publish before evaluation finishes, and tests can rerun without HTTP. Fresh QuickJS worker processes have engine limits, watchdogs and Windows job limits. Assertion errors identify `tests.js` lines separately from harness frames.
- Collections use explicit saves, stable IDs, atomic replacement, recovery journals, content-hash conflict detection, focus-time external-change reporting, bounded managed-file reads, newer-schema rejection, and nested unknown-field roundtrips. Selection and split state restore per collection. Abandoned spool files are swept after 24 hours.
- OpenAPI 3.0/3.1/3.2 JSON imports from local files or protected URLs into a review draft. Internal, contained local and same-origin remote references resolve transitively with rebasing. External acquisition is bounded at 50 attempts and 20 MiB; URL imports also share cancellation and a 60-second deadline. Import handles opaque examples, bounded placeholder generation, read-only omission, server precedence, security alternatives/combined requirements, `allOf` merges, and selectable `oneOf`/`anyOf` bodies.
- Parameter serialization covers query `form`/`deepObject` objects and arrays, `spaceDelimited`/`pipeDelimited` arrays, and path `simple`/`label`/`matrix`, honoring supported `explode` combinations. Undefined forms such as arrays of objects remain blocked. URL path substitutions encode structural characters while preserving existing percent escapes and style punctuation; dot traversal segments are rejected before parsing.
- The native UI includes dark/light/system themes, file dialogs, retained dirty drafts, grouped/collapsible folders and ordering, line-numbered editors, bounded syntax colouring, bracket matching, focused-editor find, and response snapshot labels. Editor undo buffers are not persisted.
- **Ctrl+T** reads the clipboard directly (Win32 `OpenClipboard`/`GetClipboardData`, no clipboard crate) and pastes it into the request's bearer token secret, enabling bearer auth first if it has none — copy a token, press Ctrl+T, done. An empty or non-text clipboard falls back to switching to the Auth tab and focusing the token field for a manual paste.

## Remaining work, in priority order

### Tier B — correctness edges

1. **Exact path-value encoding (area 6): completed 13 September 2026.** See the resolution and regression coverage below.
2. **Charset and binary preview options (area 6): completed 13 September 2026.** See the resolution below.
3. **Multi-process transaction locking (area 5): completed 13 September 2026.** See the resolution below. Tier B is now closed; lazy body/test loading remains open as Tier D item 8.

### Tier C — import completeness (area 3): completed 13 September 2026

4. **Multipart and richer request media handling.** See the resolution below.
5. **Relative server URLs resolved against source identity, including multi-file specs.** See the resolution below.
6. **Sanitized source provenance and reviewed reimport/update diffs that preserve local edits and tests.** See the resolution below. Scoped, by owner decision, to whole-operation replace: a reviewed diff decides which operations to touch, not which fields within one.
7. **External example diagnostics.** Folded into item 6's resolution: `externalValue` content is now fetched, not just flagged as unfetched.

### Tier D — smaller follow-up: completed 13 September 2026

8. **Lazy test/body loading (area 5): completed 13 September 2026.** See the resolution below. As the prior note anticipated, this is a memory result, not a restore-time one: the same files are still read and hashed at open either way.

Tier D is now closed. Every numbered backlog item is resolved; what remains is the owner-decided scope in this document and the deliberately deferred/excluded items below.

### Deferred or excluded from v1

Cross-origin reference fetching is refused under the current credential-forwarding policy; reconsider only with explicit origin selection and credential isolation. Virtualized text pages were dropped after adaptive page sizing met the response-memory delta target and to preserve drag-selection across a page. Configurable connect timeout, proxy-secret bindings, structured-query editing around opaque templates, and F6 traversal are also outside the agreed remaining work.

Scenario execution, plugin marketplace, login, token refresh, shared cookie sessions, cloud sync, and automatic spec polling remain outside v1.

## Resolved: security review hardening

The 13 September 2026 review identified four source-supported findings; focused reproductions confirmed all four. A response `Location` query could decode `{{secret.name}}` and gain template semantics when opened and sent as a new request. Response-derived query components now retain literal provenance through save/reload and preparation; editing a parameter deliberately restores normal template behavior. The fresh request still has no inherited authentication binding.

External example inlining recursively visited the value it had just inserted, so a self-referential payload could grow until the desktop process exhausted its stack. Authored and fetched example values are now opaque, the remaining metadata walk has a depth ceiling, and cycle-shaped payloads are covered. Schema placeholder generation had a separate exponential-amplification path through reused branching schemas; one candidate set now has a shared 50,000-node and 4 MiB scalar/key-data budget, including authored values before they are cloned.

A local specification could read a relative, absolute, symlinked, or junction-backed target outside the selected specification folder. Every local target is now canonicalized and checked against the canonical import root before a bounded read. Remote acquisition now counts unique attempts and received bytes regardless of HTTP status or JSON validity, remembers failures, shares cached bytes when one URI serves as both a document and an example, propagates cancellation, and stops at one 60-second deadline. Managed collection JSON and body reads are similarly capped at 20 MiB and test sources at 1 MiB, including deferred reload after a file grows; conflict hashes stream in 64 KiB chunks.

The remaining review observations did not establish additional vulnerabilities. Standard gzip/deflate and Brotli decoding have format/library window bounds, and the vendored Zstandard 1.5.7 decoder's default `windowLog` ceiling is 27 (128 MiB); Duckie's encoded/decoded caps, request deadline, and cancellation remain around decoder reads. This is a source-level bound assessment, not an adversarial peak-memory benchmark. Test scripts still run with ordinary user permissions inside a fresh worker rather than an OS sandbox, as the README states; no escape was found, and current QuickJS-NG 0.16.2 is newer than the patched versions in the upstream advisories reviewed. Plaintext opted-in secret files, exports, response spools, and explicitly selected upload paths remain documented local artifacts; no ACL bypass or automatic disclosure path was found.

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

## Resolved: response charset and content-encoding diagnostics

Response previews now honor supported `charset` labels in `Content-Type`; the retained body stays byte-for-byte unchanged. Undeclared invalid UTF-8, NUL-bearing content, and unknown declared charsets remain classified as binary. The body view offers explicit UTF-8 and Windows-1252 overrides, clearly labels an override, and can return to automatic detection. Whole-body find encodes its query with the displayed charset so match offsets still refer to the retained bytes.

`Content-Encoding` is parsed across repeated fields and comma-separated values. Duckie accepts identity or one supported gzip, deflate, Brotli, or Zstandard coding. Unknown and stacked codings stop body processing with actionable detail; malformed, non-ASCII, empty, or oversized values receive a fixed diagnostic. Only short validated HTTP token characters are ever echoed, so arbitrary header data, response bytes, URLs, and credentials cannot enter the diagnostic.

## Resolved: multi-process transaction locking

Save-time content-hash checks already caught changes made by another instance *before* a save began, but the save transaction itself — the disk-change check, writing `.duckie/pending-save.json`, and applying it — was not coordinated across processes. Two instances saving the same collection at once could interleave those steps: one instance's atomic journal write could be overwritten by the other's before either applied it, so `recover` could apply the wrong entries.

`Collection::save` and the recovery step in `Collection::open` now hold an OS-level exclusive lock on `.duckie/lock` for the duration of that transaction. The lock is per-open-handle (`std::fs::File::lock`, stable since Rust 1.89), not a marker file the code has to clean up itself: Windows and the kernel release it the instant the holding process exits or crashes, the same guarantee the 24-hour spool sweep already relies on for `FILE_SHARE_DELETE`. A second instance's save blocks until the first finishes, then proceeds through its own content-hash check as before, so a genuine conflict (both instances editing the same collection) still surfaces as the existing "Changed on disk" error rather than silent corruption.

Regression coverage takes the lock directly from two threads to verify the second blocks until the first releases it, standing in for two instances racing to save.

## Resolved: OpenAPI import completeness

**Multipart and richer request media handling.** A `multipart/form-data` (or `multipart/mixed`) request body is now generated from its schema rather than blocked: a `format: binary` property (or an array of them) becomes a file part flagged "Needs input", exactly like a missing path or query value; every other property becomes a generated text part. A whole-body `{type: string, format: binary}` schema, or `application/octet-stream`, becomes a `Body::File` awaiting a manual file selection instead of a hard blocker — Send is still available and fails with the existing "Select an existing body file" message until one is chosen, the same soft-diagnostic pattern already used for an unfilled path variable. Other media types this build cannot generate (arbitrary XML, custom binary formats) remain a manual-configuration blocker; generating XML from a JSON Schema was judged out of scope rather than attempted partially.

**Relative server URLs resolved against source identity, including multi-file specs.** `servers` entries were previously used as literal strings. They are now resolved against the identity of the document that declared them: the root document's `base` for the top-level `servers` array, and — since OpenAPI 3.1 lets a path item be a `$ref` into another file — that file's own URI for a path item's or operation's `servers` override, tracked via `document_keys` (the same `dN` numbering `inline_external` embeds documents under) and `origin_document`. A relative override in a multi-file spec now produces a request Duckie can send rather than an unresolved template fragment.

**External example diagnostics, extended to actually fetch them.** `externalValue` content (an example's literal value kept in a separate file or URL, distinct from `$ref`) is now collected (`external_examples`) and fetched the same deliberately bounded, same-origin way as referenced documents, then inlined (`inline_examples`) before import runs. JSON content is parsed; anything else is kept as raw text, which is what a non-JSON example becomes as a body value. An example that could not be retrieved is still reported by name rather than silently dropped.

**Sanitized source provenance and reviewed reimport/update diffs.** By owner decision, reimport is whole-operation replace, not a field-level merge: every generated request now carries its source (`sanitize_source` strips credentials, query, and fragment before it is recorded) alongside the existing `path`/`method`/`operationId` identity in `x-openapi`. **File > Update from spec…** re-reads a source, matches its operations against the open collection's requests by that identity, and presents a reviewable diff — Changed, New, and No-longer-in-the-spec — before touching anything. Applying a change always keeps the existing request's `id` and its `tests` (file and source) untouched; every other field is replaced wholesale from the fresh import, the same regeneration a first import already does, just scoped to one operation and shown before it happens. A request with no `x-openapi` provenance (hand-written, or imported from a different spec) is never proposed for removal. Nothing is written to disk until the collection is saved, same as every other change in Duckie.

## Resolved: lazy body/test loading

`Collection::open` used to read every body and test file's content into memory for every request, whether or not it was ever going to be viewed. It now only hashes and discards that content during open — the hash is still what conflict detection compares against — and reads the real text again from disk the first time a request is actually needed: `Collection::ensure_loaded(id)` fills in `StoredRequest::source` and any file-backed `body` text, matched by request id so it stays correct across reordering, additions and removals rather than assuming index alignment.

`Draft` carries the same idea into the desktop app as a `pending` flag: `use_collection` sets it for a request `open` deferred, the per-frame check in `Duckie::ui` loads the selected draft the moment it is shown, and `send` loads it defensively as a second line of defence. The one place this needed real care is `save_collection`, which writes every draft back to a file regardless of whether the user ever opened it: it now hydrates every remaining pending draft before it builds the write set, and refuses to save at all if a hydration fails, rather than risk writing an unloaded placeholder over real content. `StoredRequest::new` gives every external caller (import, the reimport apply step, tests) a fully-loaded request, so nothing outside this crate can construct a deferred one by accident.

The first attempt at measuring this showed almost no memory difference, which was a real finding about the *reading* code, not the deferral itself: the attachment pass batched every file's bytes into one `Vec` before any were discarded, so the transient peak barely differed from reading everything eagerly regardless of what got kept afterward. `read_many` now takes a per-file transform applied where each file is read, before its bytes cross back to the caller, so hashing 2,000 attachments during open no longer means holding all 2,000 at once. See [PERFORMANCE.md](PERFORMANCE.md#deferred-bodytest-loading-13-september-2026) for the measured comparison — restore time is unaffected, since the same files are still read and hashed at open either way; only memory changes.

## Implementation cautions

- **Save covers the collection.** All dirty drafts save together. Reordering changes the manifest and must mark it dirty even if no request content changed. Deletion removes the manifest entry on Save but retains unreferenced files. Body and test content loads lazily, per request, on first selection or Send; see below.
- **A `pending` draft's body/source are placeholders, not empty content.** Any new code path that reads `Draft.request.body` or `Draft.source` — not just the ones that already do — must either go through a draft that is not `pending`, or call `Duckie::ensure_loaded` first. `save_collection`'s pre-save hydration loop is the backstop, but do not add a second way to reach `Collection::save` that skips it.
- **The 24-hour spool threshold protects other instances.** Windows permits deletion of files opened with `FILE_SHARE_DELETE`; a file can still be in use by another Duckie instance. Do not shorten the threshold without a replacement ownership guard.
- **Raw and Pretty searches have different offsets.** Raw find scans retained body bytes; Pretty is a reformatted copy, so its search remains page-local and labelled accordingly.
- **Reference rebasing is essential.** An external document's internal `#/components/...` references must point into its embedded copy after inlining, rather than the root document's components. `as_url` also distinguishes Windows drive paths from URL schemes.
- **External import limits count work, not successful parses.** Keep one visited/cache entry per resolved URI, charge bytes before parsing, reuse the cached raw bytes when a URI serves as both `$ref` and `externalValue`, and pass the root cancellation/deadline through every remote fetch. Local references must canonicalize inside the selected specification folder before opening.
- **Serialization order is deterministic.** `serde_json` sorts object members; no supported parameter style relies on authored object-key order.
- **Syntax colouring is capped at 64 KiB** by `editor::MAX_COLOURED`. Larger text draws plain. A performance fixture must have the correct content type to exercise colouring; see PERFORMANCE.md for the measurements and baseline rules.
- **Egui 0.36 APIs differ from older versions.** Use `App::ui`, `Panel::top/left/bottom`, and `Context::run_ui`. Headless tests must clear unapplied `output.textures_delta`. `TextEdit::show` yields an `AtomLayoutResponse`; its inner response is `output.response.response`.
- **Development dependencies are explicit.** The two end-to-end tests and fixture generator require Node.js on `PATH` and fail rather than silently skipping. The icon script needs PowerShell's unary comma when returning an array; preserve it when editing. Prefer structured patches to shell-generated source that can corrupt escapes.
- **Development features must not ship.** Screenshot variables (`DUCKIE_CAPTURE_PATH`, `DUCKIE_CAPTURE_THEME`, `DUCKIE_CAPTURE_VIEW`) and `DUCKIE_BENCH_PATH` belong to development builds. Packaging rejects binaries containing the capture/bench markers. Rebuild without those features before packaging.
- **Fresh workers are intentional.** Each test evaluation starts and stops its worker instead of retaining it for 30 seconds. Redirect navigation also intentionally creates a fresh GET draft without inherited credentials, and its response-derived query rows remain literal until the user edits them.
