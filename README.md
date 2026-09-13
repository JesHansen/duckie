# Duckie

A small, local HTTP workbench for Windows. **0.1.0 is the first development preview of v1**, based on [ARCHITECTURE.md](ARCHITECTURE.md) and [UI_DESIGN.md](UI_DESIGN.md).

Compose a request, paste credentials, send once, inspect the response, and run JavaScript assertions. No Duckie account, telemetry, update checks, cloud service, or background API traffic. Collection files stay on your disk.

## Run

Requirements for development: Windows x64, Rust 1.95 or newer with the MSVC toolchain, and Visual Studio C++ Build Tools with the Windows SDK. Node.js is needed only for the local development server and the two end-to-end tests that drive it.

```powershell
cargo build --workspace --release
.\target\release\duckie.exe
```

Keep `duckie-test-worker.exe` beside `duckie.exe`. Building only the desktop crate does not build the test worker. A portable ZIP can be produced with `./scripts/package.ps1`.

On first launch, paste an HTTP(S) URL and press **Ctrl+Enter**. Auth supports a bearer token, a header API key, or both. Change request tabs to edit parameters, headers, body, tests, and transport settings. **Ctrl+S saves the whole collection**, including its dirty drafts and local environment changes. Sending never saves automatically.

The body, test and response-body views are monospace editors with a line-number gutter. They do not wrap; long lines scroll sideways under the gutter. **Ctrl+F** opens find for whichever editor holds the caret, and otherwise for the response body. Enter and Shift+Enter step forward and back through matches, the current match is highlighted more strongly than the rest, and the counter reads `3 of 12`. Matching is case-sensitive and stops counting after 5,000 hits on one preview page.

## Try locally

```powershell
node scripts/dev-server.mjs
```

Then open the `examples/local-api` collection in Duckie, or import `examples/openapi.json`. The server binds only to `127.0.0.1:8787`. It has echo, error-status, redirect, delayed, and compressed-response endpoints. The protected OpenAPI URL is `http://127.0.0.1:8787/protected/openapi.json`; its illustrative development token is `duckie-local-demo`.

`node scripts/make-fixtures.mjs` generates the fixtures the performance targets in [ARCHITECTURE.md](ARCHITECTURE.md#9-performance-acceptance-targets) call for: a 1,000-request collection, a 10,000-request stress collection, and a 10+ MiB OpenAPI document, all under `fixtures/` (gitignored). Output is deterministic, so re-running it for a benchmark pass produces byte-identical fixtures.

## Local files and secrets

A collection contains `duckie.json`, `requests/`, `bodies/`, `tests/`, and `environments/`. Request IDs are stable; the manifest orders request-file references. JSON is UTF-8 with two-space indentation and LF endings. Save uses atomic file replacement, a recovery journal, and content-hash conflict checks. An external edit blocks overwrite; reload or save the draft into a new folder. Fields Duckie does not recognize are preserved through save and reload, and a document carrying a higher `schemaVersion` than this build writes refuses to open rather than being resaved without the parts it cannot read.

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

**Run tests / Ctrl+Shift+Enter** evaluates current source against the retained response without another HTTP request. One run, including tests, occupies the app's run slot. Test code runs in a fresh native worker process with a 2-second engine deadline, 64 MiB JS heap, 512 KiB stack, a parent watchdog, and a Windows Job Object with a 128 MiB process-memory cap. No filesystem, network, module loader, or Node.js API is exposed. Async callbacks are unsupported. Process isolation is not an OS security sandbox.

## Verify

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Performance targets are measured separately, against release binaries built with the development `bench` feature:

```powershell
cargo build --workspace --release --features duckie-desktop/bench
./scripts/benchmark.ps1
```

Tests use deterministic loopback servers. They cover transport semantics, compressed-body limits, secret resolution, collection conflicts/recovery, import conversion, native UI rendering, real worker IPC, cancellation, and allocation failure. Two end-to-end tests need Node.js on `PATH`: they start `scripts/dev-server.mjs` on an ephemeral port and drive the shipped `examples/local-api` collection and the protected OpenAPI import all the way through send and assertion evaluation. See [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md) for the remaining v1 work and [PERFORMANCE.md](PERFORMANCE.md) for measurements and release gates.

The UI uses [eframe/egui](https://docs.rs/eframe/0.36.2/eframe/); assertions use [rquickjs](https://docs.rs/rquickjs/0.13.0/rquickjs/). Dependencies are fixed by `Cargo.lock`. Packaging gathers dependency metadata and available license notices into `THIRD_PARTY_NOTICES.md`.

## License

MIT — see [LICENSE](LICENSE).

Every one of the 254 third-party crates linked into the binaries declares a permissive license, and none is copyleft. Where a crate offers a choice, Duckie takes the permissive option: `self_cell` is used under Apache-2.0 rather than GPL-2.0-only. `epaint_default_fonts` embeds typefaces under OFL-1.1 and the Ubuntu Font Licence, which allow redistribution inside an application but not sale of the fonts by themselves. Re-check `THIRD_PARTY_NOTICES.md` whenever `Cargo.lock` changes.
