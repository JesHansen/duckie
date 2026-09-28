# Duckie

<p align="center">
  <img src="assets/duckie.png" alt="Duckie" width="280">
</p>

Duckie is a fast, local HTTP workbench for Windows. Build requests, keep them in plain files, inspect responses, and write JavaScript assertions without creating an account or sending your work to a cloud service.

It is useful for exploring an API, keeping repeatable requests beside a project, and turning a response check into a small executable test.

For example, send a request to a local service:

```http
GET http://127.0.0.1:8787/echo?message=hello
```

Then check its response in Duckie:

```javascript
test("returns the message", () => {
  expect(response.status).toBe(200);
  expect(response.json().query).toEqual([["message", "hello"]]);
});
```

## What you can do

- Compose HTTP requests with query parameters, headers, bodies, bearer tokens, and API keys.
- Paste supported cURL commands into editable drafts and copy requests as redacted POSIX or PowerShell cURL commands.
- Organize requests into local collections and environments that are easy to inspect and version.
- Import Swagger 2.0 and OpenAPI 3.x JSON or YAML specifications, including multi-file specifications.
- Inspect text, JSON, binary, compressed, and large responses without leaving the app.
- Navigate bounded JSON responses as a tree, copy values or RFC 6901 JSON Pointers, and store a selected value as a request Template variable.
- Run JavaScript assertions against a response, or rerun them without sending the request again.
- Keep credentials session-only by default, with an explicit option to save them in a gitignored secrets file.

Duckie has no account, telemetry, update checks, cloud service, or automatic background API traffic. Collections stay on your disk.

## Run Duckie

Download the portable Windows x64 ZIP from [GitHub Releases](../../releases), extract it, and run `duckie.exe`. Each release lists the ZIP's SHA-256 checksum and carries a GitHub build-provenance attestation.

To build from source on Windows x64, you need Rust 1.95 or newer with the MSVC toolchain, plus Visual Studio C++ Build Tools and the Windows SDK.

```powershell
cargo build --workspace --release
.\target\release\duckie.exe
```

The app requires OpenGL 3 or newer. Keep `duckie-test-worker.exe` beside `duckie.exe` and `duckie-cli.exe`; building the whole workspace as shown above produces all three executables. To create a portable ZIP containing the desktop app, CLI, and test worker, run:

```powershell
.\scripts\package.ps1
```

## Take a quick tour

Start the bundled example server (Node.js is required for this example):

```powershell
node scripts/dev-server.mjs
```

Then open the `examples/local-api` collection in Duckie. It exercises echo, error-status, redirect, delayed, and compressed-response endpoints on `127.0.0.1:8787`.

On first launch, paste an HTTP or HTTPS URL and press **Ctrl+Enter** to send it. Use the request tabs to edit parameters, authentication, headers, the body, tests, and transport settings.

Query parameters and headers offer **Bulk edit**: enter `name=value` or `Name: value`
per line. The first delimiter splits the line; blank lines are ignored. Duplicate
names retain their enabled flags by occurrence. Header names are trimmed and one
space after `:` is removed; query text and templates are preserved. Values must be
single-line; existing multiline rows stay in the row editor. Apply returns to rows.
Malformed text is kept and must be fixed before saving, sending, or exporting.

In Settings, **Folder / group** lets you choose an existing folder, choose
**Ungrouped**, or type a new name. Leading/trailing whitespace is trimmed when
the field loses focus; names differing in case remain separate. The sidebar's
**Move to folder** action uses the same picker.

**New request** inherits the selected folder. A leading `{{env.baseUrl}}` reference
is retained (including its configured base path); other leading environment
references are retained when they resolve to an HTTP(S) base without credentials,
query, or fragment. Concrete URLs fall back to their origin (scheme, host, port),
because Duckie cannot infer an API base path. Other URLs start empty. New requests
keep default name/method and do not copy authentication, query values, headers,
bodies, or tests. Opening a response Location still creates a fresh GET request.

Press **F1** for the searchable keyboard reference, also available under Help.

<!-- shortcuts:start -->
| Shortcut | Action | Context |
| --- | --- | --- |
| **F1** | Open searchable keyboard reference | Anywhere |
| **Ctrl+Shift+O** | Import an OpenAPI specification | Workspace; unsaved-changes guard applies |
| **Ctrl+O** | Open a collection | Workspace; unsaved-changes guard applies |
| **Ctrl+N** | Create a request in the current folder/API | Workspace |
| **Ctrl+S** | Save the whole collection | Workspace |
| **Ctrl+Shift+Enter** | Run tests against the retained response | Retained response; no active run |
| **Ctrl+Enter** | Send the current request | No active request or suite |
| **Ctrl+L** | Focus the URL field | Workspace |
| **Ctrl+T** | Paste clipboard as bearer token, or focus token field | Workspace |
| **Ctrl+B** | Toggle request sidebar | Workspace |
| **Ctrl+K** | Focus request search and show sidebar | Workspace |
| **Ctrl+F** | Find in focused Body/Tests editor, otherwise response body | Workspace |
| **Escape** | Close keyboard help, environment/import dialog, or stop active work | Workspace |
| **ArrowDown** | Select next matching request | Request search focused |
| **ArrowUp** | Select previous matching request | Request search focused |
| **Enter** | Open matching request and focus URL without sending | Request search focused |
| **Escape** | Clear request search | Request search focused |
| **ArrowDown** | Select next template completion | URL template completion open |
| **ArrowUp** | Select previous template completion | URL template completion open |
| **Enter** | Insert selected template completion | URL template completion open |
| **Escape** | Dismiss template completion | URL template completion open |
| **Enter** | Move name to value; append row from populated final value | Query/header/form/variable row focused |
| **Enter** | Reveal next find match | Find field focused |
| **Shift+Enter** | Reveal previous find match | Find field focused |
| **Enter** | Move focus to Read and review | Import URL field focused; no active I/O |
| **Enter** | Save and continue | Unsaved-changes dialog |
| **Alt+S** | Save and continue | Unsaved-changes dialog |
| **Ctrl+S** | Save and continue | Unsaved-changes dialog |
| **Alt+D** | Discard and continue | Unsaved-changes dialog |
| **Escape** | Cancel switching/closing | Unsaved-changes dialog |
| **Alt+C** | Cancel switching/closing | Unsaved-changes dialog |
| **ArrowDown** | Select next JSON value | JSON tree focused |
| **ArrowUp** | Select previous JSON value | JSON tree focused |
| **ArrowLeft** | Collapse container or select parent | JSON tree focused; filter keeps ancestors open |
| **ArrowRight** | Expand container or select first child | JSON tree focused |
| **Home** | Select first visible JSON value | JSON tree focused |
| **End** | Select last visible JSON value | JSON tree focused |
<!-- shortcuts:end -->

Sending does not save automatically. Duckie retains dirty drafts, and **Ctrl+S** saves the entire collection together.

Use **Request → Preview prepared request** to resolve the current draft without sending it. The preview shows Duckie-prepared URL, headers, body representation, environment, and template-value sources. Secrets are masked. File bodies show bounded content and metadata.

Use **File → Paste cURL from clipboard** to replace the current draft with a reviewable import. Duckie supports URLs, methods, repeated headers, textual `--data` forms, URL-encoded bodies, multipart fields, and file references. Unsupported cURL options stop the import. **Request → Copy as cURL** offers POSIX and PowerShell syntax; exports redact common credential headers unless you deliberately choose a “with credentials” action.

New requests allow 10 minutes by default, including connection setup and response transfer. You can change the timeout and the encoded and decoded response-size limits per request under **Settings**.

## Collections and secrets

A collection is a normal folder containing a `duckie.json` manifest and separate files for requests, bodies, tests, and environments. That makes collections suitable for ordinary source control and collaboration.

**File → Recent collections** lists the last eight successfully opened or saved
collection roots. Switching uses the unsaved-changes prompt; a missing collection
leaves your current workspace intact. The list stays in local preferences.

Credentials are session-only unless you select **Remember in secrets file**. Remembered values go into `.duckie/secrets.json`, while shareable request definitions contain only secret-key bindings. The generated `.gitignore` excludes `/.duckie/`; do not share that directory. **Environments → Export secrets** deliberately creates a separate plaintext file, so handle that export accordingly.

Duckie detects external edits and prevents an older in-app copy from silently overwriting them. It also preserves fields it does not recognize when saving compatible collection files.

## OpenAPI import

Duckie imports Swagger 2.0 and OpenAPI 3.0, 3.1, and 3.2 JSON or YAML from a file or protected URL. A specification and its referenced documents may mix `.json`, `.yaml`, and `.yml`. Duckie follows contained local references and same-origin remote references, supports common parameter serialization styles, and creates file inputs for binary and multipart bodies.

YAML imports accept one bounded, JSON-compatible document. Duplicate mapping keys, unsupported custom tags, non-string mapping keys, excessive aliases/nesting, and normalized documents over 20 MiB are rejected rather than interpreted ambiguously. YAML comments are not retained because import normalizes the document into the same JSON value tree used by the existing review pipeline.

Use **File → Update from spec…** to compare an imported collection with its source. Duckie shows changed, new, and removed operations before applying your choices, preserves existing request IDs and tests, and waits for you to save the collection.

Cross-origin references are not fetched because credentials supplied for the specification must not be forwarded to another origin.

## Response tests

Tests use a small synchronous JavaScript API:

```javascript
test("returns a successful JSON response", () => {
  expect(response.status).toBe(200);
  expect(response.header("content-type")).toContain("application/json");
  expect(response.json()).toBeType("object");
});
```

`expect(value)` supports `toBe`, `toEqual`, `toContain`, `toBeType`, `toBeLessThan`, `toBeLessThanOrEqual`, `toBeGreaterThan`, and `toBeGreaterThanOrEqual`. The `response` object exposes `status`, `header(name)`, `headers(name)`, `text()`, `json()`, `bodySize`, and `durationMs`.

Tests run in a fresh native worker process with no filesystem, network, module loader, or Node.js API. Async callbacks are not supported. Process isolation is not an operating-system security sandbox.

Response JSON and header rows can insert editable assertion snippets into the Tests tab. Generated JSON assertions use RFC 6901 pointers and include that path in mismatch output. Buttons that use observed status, value, array length, or duration label the resulting expectation so you can decide whether it is a real contract.

Use **Request → Run requests…** to review and sequentially execute the selected request, its folder, or the collection. Choose whether to stop at the first failure; cancellation stops the active operation and prevents later requests from starting. The final table distinguishes assertion and execution failures and can export JSON, JUnit XML, or standalone HTML.

The response **Compare** tab compares the current result with another retained result or an explicitly saved baseline. It reports status, header, bounded text, and structural JSON changes. JSON object order is insignificant, array order remains significant, and comma-separated RFC 6901 pointers can ignore volatile fields. Baseline files contain response data and are limited to 2 MiB.

In **JSON tree**, select a value, enter a Template-variable name, and choose **Set**. Duckie creates or updates that variable on the current request, switches the request pane to **Params**, and makes the request dirty so the value follows the normal collection Save flow. Reference it as `{{request.name}}` in the URL, parameters, headers, or body. Strings are stored as their text; other JSON values use compact JSON. Because this is an ordinary request variable, previews, direct sends, and suites all use it.

Response **Request details** shows the negotiated HTTP version, total duration, time until response headers arrived, and the combined body-transfer/decoding interval. Lower-level DNS, TCP, TLS, proxy, upload, server-wait, connection-reuse, and separate transfer/decoding measurements are labelled unavailable because the current transport cannot observe them reliably.

Use **Request → Session history…** to review direct sends from the current application session. Each entry keeps its captured editable input, masked effective request, environment, result, duration, test outcome, and response while available. History is limited to 100 entries and 20 MiB of response bodies; older bodies are visibly evicted before metadata. **Open as new draft** creates an unsaved editable copy without sending it. Values supplied through secret bindings are not copied into history and resolve from the currently selected environment on a later send; literal values authored directly in a request remain part of its captured input.

## Headless execution

Build the workspace, then run one saved request by ID or exact name:

```powershell
.\target\release\duckie-cli.exe run --collection .\examples\local-api --request "Echo a request" --environment dev
```

Omit `--request` (or pass `--suite`) to run every request sequentially in manifest order. Add `--format json` for machine-readable output. `--report result.json`, `--report result.xml`, or `--report result.html` writes a portable JSON, JUnit, or HTML artifact. Reports contain summaries by default; `--include-response-snippets` explicitly adds bounded snippets, which can contain sensitive response data. Known collection secrets are redacted from those snippets. Duckie does not change collection files during a CLI run. It reads the selected environment and local secrets file; a process environment variable named `DUCKIE_SECRET_name` overrides secret `name`, and `DUCKIE_SECRET_group__name` addresses `group.name` without putting its value in command-line arguments.

Exit code 0 means success, 1 means an assertion or suite failure, 2 means configuration or execution failure, and 3 means cancellation. Keep `duckie-test-worker.exe` beside `duckie-cli.exe` when requests have tests.

## Development

Run the standard checks before submitting a change:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Shortcut bindings and reference text live in
`crates/duckie-desktop/src/shortcuts.tsv`. After changing that table, run
`python scripts/generate-shortcuts.py` to update this README; use `--check` to
verify it without writing. Workspace tests also check that the generated reference
matches the binding table.

Node.js is needed for the local development server, fixture generator, and the two end-to-end tests that use it.

The UI uses [eframe/egui](https://docs.rs/eframe/0.36.2/eframe/), and response assertions use [rquickjs](https://docs.rs/rquickjs/0.13.0/rquickjs/). Dependencies are fixed by `Cargo.lock`.

## Product contract and scope

This section is the canonical record of current owner decisions. Historical proposals and completed implementation plans are deliberately omitted.

- Duckie is a Windows-only, local HTTP workbench. It has no account, hosted service, telemetry, activation, automatic update check, cloud synchronization, or unsolicited network activity. Network access occurs only when the user sends a request or explicitly imports a specification URL.
- Collections are ordinary local files. Secrets stay in session memory unless the user explicitly remembers or exports them. Remembered secrets, response spool files, selected uploads, and exported secret files are local sensitive artifacts; OS paging, backups, and other programs remain outside Duckie's deletion guarantees.
- Scenario execution, schedules, load testing, login, automatic token acquisition or refresh, shared cookie sessions, plugin discovery, GraphQL schema tooling, gRPC, WebSockets, and cloud collaboration are outside the accepted v1 scope. Manual response-value extraction writes an ordinary request Template variable; it does not make suites response-driven.
- Authentication is a single choice among none, bearer, or one header API key. Users can author other headers manually, but Duckie does not treat literal credentials as secret bindings. Basic authentication, client certificates, OAuth flows, and authenticated enterprise proxies are unsupported.
- Duckie sends each request once. Automatic retries and redirect following are disabled. A 3xx response remains inspectable and can create a fresh GET draft; credentials are not inherited. There is no cookie jar.
- One desktop request or test evaluation runs at a time. Sending captures an immutable request, environment, and test-source revision. Editing can continue while it runs, and the result stays attached to the captured revision.
- Test scripts receive a completed response and cannot access the filesystem, network, shell, modules, or Node.js APIs. Each evaluation uses a fresh worker process. The worker has engine, watchdog, and Windows job limits, but it runs with the user's ordinary OS permissions and is not an OS security sandbox.
- OpenAPI imports can fetch same-origin remote references during the explicit import action. Cross-origin fetching is refused under the current credential-forwarding policy. Reimport is reviewed at whole-operation granularity: selected changes retain request identity and tests and replace the other generated fields.
- The v1 memory ceiling is 500,000,000 bytes for the desktop plus any active test worker. Performance claims must identify the measured process, fixture, build features, trial count, and baseline. A focused or CPU-only measurement is not a full release-gate result.
- Release version changes do not prove that binaries or a portable ZIP were rebuilt. A release claim must identify the commit, checks, build configuration, package, and validation performed on that exact artifact.

### Current limits

| Area | Limit or behavior |
| --- | --- |
| Request deadline | 10 minutes by default; configurable from 1 ms to 1 hour. Connection setup and transfer share it. |
| Response capture | Separate encoded and decoded limits, 50 MiB by default and configurable up to 1 GiB. Limit hits retain a visibly incomplete body and skip automatic tests. |
| Response storage | Bodies spill from memory after 2 MiB. Preview pages use 1 MiB for ordinary lines and 128 KiB for very long lines. |
| JSON tree | Complete valid JSON only, at most 2 MiB, 128 levels, and 10,000 laid-out rows. Raw body view remains available. |
| Compare | First 2 MiB of each body; at most 500 structural JSON differences or 200 text differences. Omitted results and body clipping are reported separately. |
| Tests | Body access is bounded; metadata assertions remain available when body access is unavailable. Async callbacks are unsupported. |
| Retained responses | At most three response panes. Session history keeps 100 metadata entries and 20 MiB of shared response bodies, evicting old bodies before metadata. |
| Managed files | Collection JSON and body reads are capped at 20 MiB; test sources at 1 MiB. Unknown compatible fields round-trip. |
| OpenAPI acquisition | At most 50 external attempts and 20 MiB received, under one 10-minute deadline. Local targets must canonicalize inside the selected specification folder. |
| Editor and search | Syntax coloring stops after 64 KiB; editor find is capped at 5,000 hits and whole-body search at 50,000 hits. |

## Architecture and implementation invariants

The Cargo workspace keeps model, storage, transport, import, test execution, orchestration, CLI, and desktop presentation separate. The domain model must not depend on egui, reqwest, or the JavaScript engine. Import produces a reviewable draft; tests consume a response and do not control transport.

Future changes must preserve these invariants:

- Prepared requests own fully resolved immutable input. Secret-bearing types must not expose values through default debug output, reports, history, or collection files.
- `RunBindings` remain an explicit ephemeral override mechanism for future run-scoped callers. Ordinary desktop and suite execution currently pass an empty map; response-value extraction writes the current request's saved Template variables instead.
- URL interpolation is single-pass. Path substitutions encode structural characters, preserve valid existing percent escapes and supported OpenAPI style punctuation, and reject complete dot-traversal segments before URL parsing. Response-derived redirect query rows remain literal until edited.
- Header and query rows preserve order and duplicate names. Untouched query values retain literal provenance; editing restores normal template behavior. Serialization must never guess an undefined OpenAPI shape.
- Response decoding is streamed with encoded and decoded caps around every supported decoder. Do not buffer a compressed body before applying limits. Unknown, malformed, or stacked content encodings stop processing with bounded safe diagnostics.
- Save covers the whole collection. Dirty reordering changes the manifest. A deferred draft's body and test source are placeholders; any code that reads them must call `ensure_loaded`, and saving must hydrate every remaining deferred draft before building the write set.
- Collection saves use recoverable atomic replacement, content-hash conflict detection, and cross-process transaction locking. Managed paths remain within the collection root. Preserve nested unknown fields and reject newer schema versions.
- Response spools are application-owned and swept only after 24 hours because another Duckie process can still hold a Windows handle with delete sharing. Do not shorten this threshold without replacement ownership tracking.
- OpenAPI acquisition counts attempted work and received bytes even when retrieval or parsing fails. Cache one result per resolved URI, share bytes used as both a document and an example, propagate cancellation and the common deadline, and rebase an external document's internal pointers into its embedded copy.
- Authored and fetched example payloads are opaque data. Metadata recursion and generated placeholders remain depth, node, and byte bounded.
- Fresh test workers are intentional. Each evaluation starts and stops its worker; do not retain one across runs or broaden its host API casually.
- UI work stays off the network and file-I/O paths. Background work communicates through bounded channels and explicit repaint requests. Avoid unconditional frame loops, per-request worker threads, and repeated response-body copies.
- Development screenshot and benchmark features must never ship. `scripts/package.ps1` checks their binary markers. Package only ordinary release builds.
- egui 0.36 APIs differ from older examples: use `App::ui`, `Panel::top/left/bottom`, and `Context::run_ui`. Headless tests must clear unapplied `output.textures_delta`; `TextEdit::show` returns its response through `output.response.response`.

## Open work

The following items are unresolved. They are not authorization to begin unrelated work; the current task and owner direction still control scope.

### Confirmed gaps and validation work

- Full request chaining remains outside v1. If real usage justifies it, prefer declared response captures into run-scoped bindings over scripts mutating live environments. Preserve immutable snapshots, expose capture provenance, define missing-value behavior, and keep collection files unchanged during execution.
- Accessibility and enterprise-network coverage are incomplete. Narrator, keyboard-only operation, 200% scaling, IME, remote desktop/display changes, corporate certificate chains, PAC/WPAD, and authenticated proxies are not certified.
- Cold launch is owner-accepted but has no recorded p95. The 7.0 ms frame result measures CPU frame construction only, excluding upload, presentation, and compositor time. GUI memory measurements exclude test workers.
- The latest source changes have not produced a newly validated release package. Before publishing, build from a tagged commit without development features, run the full checks, package it, validate the ZIP on a clean Windows x64 machine, and record a checksum and provenance.
- The CLI needs a more discoverable front door: bare invocation and `--help`/`-h`/`--version` should succeed, help should name exit codes, and a sole environment could be inferred. Folder/name filtering remains a separate candidate.
- `crates/duckie-desktop/src/ui.rs` remains a large mixed-responsibility module. Split coherent response, request, sidebar, and top-level view units only when doing adjacent work, preserving shared state and avoiding cosmetic churn.

### Review candidates awaiting an owner decision

These source-supported review ideas are retained for future evaluation, not accepted backlog:

- Add Basic authentication inside the secret-binding model. Client certificates and OAuth client credentials require larger transport and chaining decisions; authorization-code/device flows conflict with the no-background-network contract unless explicitly user-initiated and short lived.
- Make redirect policy visible and optionally configurable without forwarding authorization across origins. Consider an opt-in, inspectable, environment-scoped cookie jar only if session-cookie workflows justify the state and security surface.
- Offer an external-editor action for managed body and test files, then keep the embedded editor limited to quick edits, line navigation, and find.
- Explain JSON-tree fallback more directly and keep pointer/assertion workflows useful when the tree is unavailable. Do not add more tree features without a different parsing need.
- Reassess standalone HTML reports and persisted comparison baselines if maintenance outweighs demonstrated use. JSON and JUnit have defined machine consumers; retained-response comparison remains useful for live inspection.
- Avoid expanding bulk-edit syntax or cURL option coverage speculatively. Keep bulk editing as paste assistance and cURL conversion strict, with credential-redacted export as the default.
- Improve limit labels or add coarse presets only where observed use shows that raw millisecond/byte fields cause mistakes. Runtime bounds remain mandatory even if presentation changes.

Previously rejected QOL suggestions are closed unless new evidence changes the decision: session-history search, header-name completion, remembered file-dialog directories, adjacent-request shortcuts, inferred response filenames, title-bar dirty state, persistent sidebar result badges, drag-and-drop routing, and speculative response-clone optimization.

## Performance measurement

The accepted ceiling is below 500 MB for the desktop and active test worker together. The recorded measurements below used Windows 11 Pro 10.0.26200, Ryzen 7 5800X, GTX 1080 Ti, 125% scaling, the glow/OpenGL renderer, release builds outside a debugger, and deterministic fixtures. Launch milestones include loader time; peak memory uses Windows `PeakPagefileUsage`.

Reproduce the benchmark suite with:

```powershell
node scripts/make-fixtures.mjs
cargo build --workspace --release --features duckie-desktop/bench
.\scripts\benchmark.ps1
cargo test -p duckie-test-worker --release --test measurements -- --ignored --nocapture
cargo test -p duckie-desktop --release -- --ignored --nocapture
```

Reuse byte-identical fixtures and the baseline named by each target. Collection restore is measured after the first usable frame, not from process start.

| Measurement | Recorded result | Qualification |
| --- | ---: | --- |
| Warm launch to first frame, empty workspace, 30 trials | p95 167 ms | Target ≤ 300 ms; pass. |
| Restored 1,000-request collection, first frame, 20 trials | p95 180 ms | Target ≤ 300 ms; pass. |
| Restored collection searchable after first usable frame, 20 trials | p95 439 ms | Target ≤ 500 ms; pass. Earlier process-start comparisons were the wrong baseline. |
| Empty idle private bytes | 94.0 MiB | Desktop only; inside revised ceiling. |
| Idle CPU over 60 seconds | 0.0016% of machine | Pass. |
| Frame construction under input | p95 7.0 ms | CPU-only lower bound, not full input-to-paint. |
| Warm loopback request dispatch | p95 0.62 ms | Includes loopback round trip; pass. |
| Cold test-worker overhead | p95 19 ms | 1 KiB response and trivial test; pass. |
| 50 MiB response-memory delta | 3.8–25.0 MiB | Multi-line, long-line, compressed, binary, truncated, and slow-stream shapes; target ≤ 32 MiB. |
| Cold launch | No figure | Post-reboot owner judgement only. |

The 50 MiB response deltas were 3.8 MiB for multi-line text, 25.0 MiB for a single long line, 11.7 MiB for gzip, 4.1 MiB for binary, 3.9 MiB for a capped 60 MiB stream, and 14.4 MiB for a slow stream. Long-line layout was previously 213.7 MiB before adaptive 128 KiB preview pages.

One manual 10,000-request comparison measured deferred body/test loading at about 8 MiB below eager loading (143.8 MiB versus 151.7 MiB peak). This was one trial per configuration and measures memory, not restore speed. One focused session-history trial measured a 25.05 MiB response delta; it was not the full multi-trial protocol.

Glow was selected over wgpu after sampled idle private memory measured 94.0 MiB versus 374.3 MiB. The selected renderer was also confirmed in an Omnissa Horizon session. These figures are snapshots, not guarantees for other machines or drivers.

## Contributor guide

- Follow the user's current task. An open item or checkpoint does not authorize starting another task.
- Inspect the working tree before editing and preserve unrelated changes. Keep work inside relevant crate boundaries and the local-only product contract.
- Update this README instead of creating another status log, proposal, release report, agent instruction file, or duplicate backlog. Keep current behavior, durable decisions, open work, and reproducible measurement context; remove completed plans rather than accumulating a ledger.
- Distinguish measurements, owner acceptance, review candidates, and hypotheses. Date validation snapshots and state what was not measured or rebuilt.
- Use `rg` for repository searches. Prefer bounded background work and preserve deterministic behavior.
- For documentation-only changes, verify accuracy, anchors, generated shortcut content, repository references, and the diff. Full application builds are usually unnecessary.

Run the standard checks for code changes:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python scripts/generate-shortcuts.py --check
```

To publish a release, bump `version` under `[workspace.package]` in `Cargo.toml`, push it to `main`, and run the **Release** workflow from the Actions tab. It builds the current head of `main`, runs the standard checks, packages with `scripts/package.ps1`, and creates tag `v<version>` with the ZIP, a `.sha256` file, and a provenance attestation. It refuses to run if that tag already exists. It does not validate the ZIP on a clean machine.

The Node-backed tests and fixture generator require Node.js on `PATH` and fail rather than silently skipping. Benchmark tests are intentionally ignored during ordinary workspace tests.

The latest full validation was 22 September 2026: formatting, strict workspace Clippy, shortcut generation, and all 162 non-measurement workspace tests passed; four performance measurements remained ignored. Both Node-backed end-to-end tests passed. This did not rebuild or validate a release ZIP and did not rerun performance measurements.

## License

Duckie is available under the [MIT License](LICENSE). The packaging script generates a third-party license notice for the dependencies included in a build.
