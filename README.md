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
- Navigate bounded JSON responses as a tree and copy values or RFC 6901 JSON Pointers.
- Run JavaScript assertions against a response, or rerun them without sending the request again.
- Keep credentials session-only by default, with an explicit option to save them in a gitignored secrets file.

Duckie has no account, telemetry, update checks, cloud service, or automatic background API traffic. Collections stay on your disk.

## Run Duckie

Duckie currently builds from source on Windows x64. You need Rust 1.95 or newer with the MSVC toolchain, plus Visual Studio C++ Build Tools and the Windows SDK.

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

Useful shortcuts:

| Shortcut | Action |
| --- | --- |
| **Ctrl+Enter** | Send the current request |
| **Ctrl+Shift+Enter** | Run tests against the retained response |
| **Ctrl+S** | Save the whole collection |
| **Ctrl+Shift+O** | Import an OpenAPI specification |
| **Ctrl+T** | Paste the clipboard into the bearer token field |
| **Ctrl+F** | Find in the focused editor or response body |

Sending does not save automatically. Duckie retains dirty drafts, and **Ctrl+S** saves the entire collection together.

Use **Request → Preview prepared request** to resolve the current draft without sending it. The preview shows Duckie-prepared URL, headers, body representation, environment, and template-value sources. Secrets are masked. File bodies show bounded content and metadata.

Use **File → Paste cURL from clipboard** to replace the current draft with a reviewable import. Duckie supports URLs, methods, repeated headers, textual `--data` forms, URL-encoded bodies, multipart fields, and file references. Unsupported cURL options stop the import. **Request → Copy as cURL** offers POSIX and PowerShell syntax; exports redact common credential headers unless you deliberately choose a “with credentials” action.

New requests allow 10 minutes by default, including connection setup and response transfer. You can change the timeout and the encoded and decoded response-size limits per request under **Settings**.

## Collections and secrets

A collection is a normal folder containing a `duckie.json` manifest and separate files for requests, bodies, tests, and environments. That makes collections suitable for ordinary source control and collaboration.

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

Node.js is needed for the local development server and the two end-to-end tests that use it. See [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md) for current decisions and known defects, and [PERFORMANCE.md](PERFORMANCE.md) for reproducible benchmark procedures and measurements. [ARCHITECTURE.md](ARCHITECTURE.md) and [UI_DESIGN.md](UI_DESIGN.md) are historical design proposals rather than an introduction to the current product.

The UI uses [eframe/egui](https://docs.rs/eframe/0.36.2/eframe/), and response assertions use [rquickjs](https://docs.rs/rquickjs/0.13.0/rquickjs/). Dependencies are fixed by `Cargo.lock`.

## License

Duckie is available under the [MIT License](LICENSE). The packaging script generates a third-party license notice for the dependencies included in a build.
