# Duckie

A small, local HTTP workbench for Windows. **0.1.0 is the first development preview of v1**, based on [ARCHITECTURE.md](ARCHITECTURE.md) and [UI_DESIGN.md](UI_DESIGN.md).

Compose a request, paste credentials, send once, inspect the response, and run JavaScript assertions. No Duckie account, telemetry, update checks, cloud service, or background API traffic. Collection files stay on your disk.

## Documentation

| Document | Purpose |
| --- | --- |
| This README | Run, use, and verify Duckie; licensing. |
| [Implementation status](IMPLEMENTATION_STATUS.md) | Current owner decisions, accepted scope, prioritized backlog, and known defects. |
| [Performance](PERFORMANCE.md) | Benchmark commands, measured results, and limitations. |
| [Architecture](ARCHITECTURE.md) / [UI design](UI_DESIGN.md) | Original design proposals; later decisions in the status document take precedence. |
| [Contributor and agent guidance](AGENTS.md) | Provider-neutral working instructions and documentation ownership. |

## Run

Requirements for development: Windows x64, Rust 1.95 or newer with the MSVC toolchain, and Visual Studio C++ Build Tools with the Windows SDK. Node.js is needed only for the local development server and the two end-to-end tests that drive it.

```powershell
cargo build --workspace --release
.\target\release\duckie.exe
```

Duckie draws with OpenGL 3 or newer. If the graphics driver cannot provide that — which can happen in a virtual desktop session without acceleration — it reports the failure in a dialog and exits rather than starting invisibly. The build compiles the application icon into `duckie.exe`, so Explorer and a pinned taskbar shortcut show it, and the running window sets the same artwork on itself. Both come from `assets/duckie.png` via `./scripts/make-icon.ps1`, which is only worth rerunning when the artwork changes — its output is committed. Keep `duckie-test-worker.exe` beside `duckie.exe`. Building only the desktop crate does not build the test worker. A portable ZIP can be produced with `./scripts/package.ps1`.

The sidebar groups requests under their folder, in the order the collection stores them. Click a folder to collapse it; the state is remembered between launches. Requests with no folder appear under **Ungrouped**. Right-click a request to move it up or down within its folder — ordering is written to the manifest on Save. Searching looks inside collapsed folders, so a match is never hidden. Reopening a collection restores the request and environment you were last on, along with the sidebar width and the request/response split. These are matched by name, so opening a different collection simply starts at its first request.

On first launch, paste an HTTP(S) URL and press **Ctrl+Enter**. Auth supports a bearer token, a header API key, or both. Change request tabs to edit parameters, headers, body, tests, and transport settings. **Ctrl+T** jumps straight to the Auth tab and focuses the bearer token field — enabling bearer auth first if the request has none — so pasting a freshly copied token is Ctrl+T, Ctrl+V. **Ctrl+S saves the whole collection**, including its dirty drafts and local environment changes. Sending never saves automatically.

The body, test and response-body views are monospace editors with a line-number gutter, JSON and JavaScript colouring, and bracket pairing that tints the bracket under the caret and its partner. Colouring stops above 64 KiB, where the cost of drawing thousands of separately coloured runs would show; a larger response draws plain and says so. They do not wrap; long lines scroll sideways under the gutter. **Ctrl+F** opens find for whichever editor holds the caret, and otherwise for the response body. Enter and Shift+Enter step forward and back through matches, the current match is highlighted more strongly than the rest, and the counter reads `3 of 12`. Matching is case-sensitive. A large body is shown a page at a time; pages are 1 MiB, or 128 KiB when the content has very long lines, since a single unbroken line of a million characters is expensive to lay out.

In the response body's **Raw** view, find searches the whole body, not just the visible page: the scan runs in the background in bounded chunks, so a body that only exists on disk is never loaded whole, and stepping to a match loads the page holding it and selects it. The **Pretty** view is a reformatted copy whose offsets do not map back to the body, so find there stays page-local and says so. Scans stop after 50,000 matches, and replacing the query abandons a scan already running.

Response text follows a supported `charset` declared in `Content-Type`, including common legacy encodings such as Windows-1252. If the charset is unknown or undeclared bytes are not valid UTF-8, Duckie keeps treating the response as binary but offers explicit UTF-8 and Windows-1252 preview choices. Those choices affect display and find only: Save body and response tests continue to use the original bytes. Unsupported or stacked `Content-Encoding` values stop body processing and report a sanitized, actionable diagnostic without echoing arbitrary header data.

## Try locally

```powershell
node scripts/dev-server.mjs
```

Then open the `examples/local-api` collection in Duckie, or import `examples/openapi.json`. The server binds only to `127.0.0.1:8787`. It has echo, error-status, redirect, delayed, and compressed-response endpoints. The protected OpenAPI URL is `http://127.0.0.1:8787/protected/openapi.json`; its illustrative development token is `duckie-local-demo`.

A spec split across files imports too. References to other documents are followed relative to the document holding them — sibling files for a file import, same-origin URLs for a URL import — and onward from those, up to 50 documents and 20 MiB. Cross-origin references are **not** fetched: the credentials you gave for the spec would travel with them. Anything not retrieved is reported and still blocks the request that needs it. A relative `servers` entry resolves against the document that declared it, so a path item pulled in from another file honors its own server override rather than the root document's. An example's `externalValue` content is fetched the same bounded, same-origin way.

A `multipart/form-data` body is generated from its schema: a `format: binary` property becomes a file part awaiting your selection, everything else a generated text value. A whole-body binary schema, or `application/octet-stream`, becomes a body file awaiting selection instead of blocking the operation.

**File > Update from spec…** re-reads a source and compares it against the open collection, matching operations by path, method, and `operationId`. Review shows what changed, what's new, and what the spec no longer defines, and you choose what to apply. Applying always keeps the existing request's id and tests; everything else is replaced from the fresh import, same as import always does — just scoped to the one operation and shown before it happens. Nothing is written to disk until you save.

Parameters are expanded following the specification's supported style and `explode` rules, including object and array parameters in a query and the `label` and `matrix` path styles. Combinations with no defined form — an array of objects, for instance — are blocked rather than guessed. Variables substituted into a URL path encode structural characters such as `/`, `?`, `#`, and backslash, while preserving existing percent escapes and style punctuation. Base URL templates remain supported. Whole `.` and `..` path segments, including percent-encoded forms, are rejected before Send because URL parsing would silently normalize them.

## Local files and secrets

A collection contains `duckie.json`, `requests/`, `bodies/`, `tests/`, and `environments/`. Request IDs are stable; the manifest orders request-file references. JSON is UTF-8 with two-space indentation and LF endings. Save uses atomic file replacement, a recovery journal, and content-hash conflict checks. An external edit blocks overwrite; reload or save the draft into a new folder. Bringing Duckie back to the foreground re-checks the collection's files, so an edit made elsewhere is reported when you return rather than when you next save: the dialog lists what was added, removed or modified and which request each file belongs to, and offers to reload or to keep what you have. Fields Duckie does not recognize are preserved through save and reload, and a document carrying a higher `schemaVersion` than this build writes refuses to open rather than being resaved without the parts it cannot read.

Tokens default to the current session. **Remember in secrets file** opts a value into `.duckie/secrets.json` on the next Save. Normal request definitions contain only a secret key binding. Secrets are scoped by environment; switching environments never reuses another environment's credentials. `secrets.example.json` contains empty keys and `.gitignore` excludes `/.duckie/`.

Share the public collection files through your normal tools, excluding `.duckie/`. **Environments → Export secrets** deliberately writes a separate plaintext file. Loading secrets creates session values until individually remembered. If a saved credential is replaced without remembering the replacement, the previously saved value remains on disk. File-upload paths are absolute and need reselection on another machine.

## Response tests

```javascript
test("returns a successful response", () => {
  expect(response.status).toBe(200);
  expect(response.header("content-type")).toContain("application/json");
  expect(response.json()).toBeType("object");
});
```

`test(name, callback)` continues after assertion failures. `expect(value)` supports `toBe`, `toEqual`, `toContain`, `toBeType`, `toBeLessThan`, `toBeLessThanOrEqual`, `toBeGreaterThan`, and `toBeGreaterThanOrEqual`. `toEqual` compares JSON values without depending on object key order. `toBeType` recognizes JavaScript types plus `array` and `null`.

`response` exposes `status`, `header(name)`, `headers(name)`, `text()`, `json()`, `bodySize`, and `durationMs`. `environment` contains public environment values; `request` is the redacted request summary. JSON parsing is lazy. Body access above 10 MiB reports a limit error; metadata assertions still work. HTTP 4xx/5xx are valid completed responses and can pass assertions.

A failure reports the assertion message above the stack, and your test file evaluates as `tests.js` so its frames are distinguishable from the harness's. **Test results** turns the first `tests.js` frame into a **Go to line** button that selects that line in the Tests editor.

While a request is in flight the bar above the request shows elapsed time and bytes received, with a progress bar when the server declared a content length. A compressed body reports received and decoded separately, since those differ. Cancel stops it at any point.

**Run tests / Ctrl+Shift+Enter** evaluates current source against the retained response without another HTTP request. One run, including tests, occupies the app's run slot. Test code runs in a fresh native worker process with a 2-second engine deadline, 64 MiB JS heap, 512 KiB stack, a parent watchdog, and a Windows Job Object with a 128 MiB process-memory cap. No filesystem, network, module loader, or Node.js API is exposed. Async callbacks are unsupported. Process isolation is not an OS security sandbox.

## Verify

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Performance targets are measured separately against release binaries with the development `bench` feature. [PERFORMANCE.md](PERFORMANCE.md#measurement-setup) contains fixture generation and the full benchmark commands.

Tests use deterministic loopback servers. They cover transport semantics, compressed-body limits, secret resolution, collection conflicts/recovery, import conversion, native UI rendering, real worker IPC, cancellation, and allocation failure. Two end-to-end tests need Node.js on `PATH`: they start `scripts/dev-server.mjs` on an ephemeral port and drive the shipped `examples/local-api` collection and the protected OpenAPI import all the way through send and assertion evaluation. See [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md) for the remaining v1 work and [PERFORMANCE.md](PERFORMANCE.md) for measurements and release gates.

The UI uses [eframe/egui](https://docs.rs/eframe/0.36.2/eframe/); assertions use [rquickjs](https://docs.rs/rquickjs/0.13.0/rquickjs/). Dependencies are fixed by `Cargo.lock`. Packaging gathers dependency metadata and available license notices into `THIRD_PARTY_NOTICES.md`.

## License

MIT — see [LICENSE](LICENSE).

Every third-party crate linked into the binaries declares a permissive license, and none is copyleft. Where a crate offers a choice, Duckie takes the permissive option: `self_cell` is used under Apache-2.0 rather than GPL-2.0-only. `epaint_default_fonts` embeds typefaces under OFL-1.1 and the Ubuntu Font Licence, which allow redistribution inside an application but not sale of the fonts by themselves. Re-check the generated `THIRD_PARTY_NOTICES.md` whenever `Cargo.lock` changes.
