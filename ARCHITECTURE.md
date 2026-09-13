# Duckie architecture proposal

Status: proposed design, 13 September 2026. No application has been implemented or benchmarked yet. Companion document: [UI design](UI_DESIGN.md).

## 1. Product contract

Duckie is a Windows desktop application for composing an HTTP request, sending it, inspecting its response, and running code assertions against that response. Its primary users call internal Azure-exposed services with externally acquired JWT bearer tokens, plus external services with API key headers.

The core product works entirely without a Duckie account, hosted service, subscription, activation, telemetry, or automatic network activity. Network connections originate from explicit request execution or OpenAPI URL import. Collections remain usable as ordinary local files. Distribution and dependency choices must preserve the ability to build and use the application independently.

Performance has veto power over convenience features. Optional functionality must carry its cost when used, and disabled plugins must add no running processes or background work. These principles should become repository contribution criteria. They are product commitments; software architecture alone cannot prevent a future maintainer from changing direction.

### V1 scope

Windows-only delivery, externally supplied tokens, separate local secret files, OpenAPI JSON import, single-response tests, and preparation for future scenarios are confirmed requirements. The specific controls, body modes, platform baseline, libraries, and limits below are proposed implementation choices.

| Area | Scope |
| --- | --- |
| Platform | Windows only; propose Windows 11 x64 as the initial validation baseline. Windows ARM64 and other Windows versions require separate validation. |
| Requests | Standard HTTP methods plus custom method text; URL, path/query parameters, headers, body, timeout, cancellation. HTTP/1.1 and HTTP/2. |
| Authentication | Paste a bearer token; configurable API key header; allow both together when an API gateway requires them. No token acquisition or refresh. |
| Bodies | JSON, raw text, URL-encoded form, multipart fields/files, and a file as the request body. Files stream from disk. |
| Responses | Status, headers, elapsed time, size, raw/text/JSON display, save body to a local file. |
| Tests | Built-in JavaScript assertions over one completed response, editable beside the request. |
| Import | OpenAPI 3.x JSON from local files or HTTP(S) URLs; editable requests and example request data. |
| Persistence | Readable local files; secrets in a separate file; no database or sync service required. |
| Extensibility | Independent modules and explicit execution contracts; future plugins reuse the request engine. |

Scenario execution, schedules, load testing, OAuth sign-in, token refresh, shared cookie sessions, cloud collaboration, GraphQL tooling, gRPC, and other protocol clients are outside v1. GraphQL commonly uses HTTP, but Duckie will not provide its schema, query, or subscription tooling. An arbitrary HTTP body remains possible.

## 2. Recommended stack and decision gates

Use **Rust for the application and request engine, egui/eframe for the native desktop GUI, and embedded JavaScript through rquickjs for response tests**. JavaScript is a recommendation based on familiarity and its fit for JSON assertions; the user has not selected a test language.

| Component | Proposal | Reason and condition |
| --- | --- | --- |
| Desktop UI | egui + eframe, initially evaluate the wgpu renderer on Windows | Native executable with custom-drawn controls. Dense editors and tables fit its programming model. Commit only after startup, idle, text editing, and accessibility measurements. |
| HTTP | reqwest with Tokio | Reuse connection pools and perform I/O outside the UI thread. Enable only needed crate features. |
| TLS | Explicit native-tls backend on Windows | Aim to use Windows trust configuration for company certificates. Verify with the actual corporate trust chain and proxy setup. |
| Structured files | serde + serde_json; JSON Schema for file validation/tooling | Deterministic, familiar files without an application database. |
| Response tests | rquickjs in a separate, lazily launched native worker | Familiar syntax, a small host API, and bounded execution without embedding a browser or Node.js. |
| Integration | Native Windows file dialogs, clipboard, shell file opening | Standard desktop behavior without a web view. |

egui supports native integration and optional AccessKit accessibility. It does not provide standard Win32 widgets: native execution and native control appearance are different properties. Validate Narrator, keyboard navigation, IME, DPI changes, remote desktop, and large text fields before committing. [egui project documentation](https://github.com/emilk/egui)

eframe can wake for explicit repaint requests, including from background work. The application should use that mechanism and avoid an unconditional frame loop. This makes low idle usage plausible, but does not establish a measured resource budget. [egui repaint API](https://docs.rs/egui/latest/egui/struct.Context.html#method.request_repaint)

reqwest offers selectable TLS, streaming, multipart, compression, and system-proxy features. Pin a tested version and explicitly select the TLS backend rather than relying on changing defaults. Proxy auto-configuration and integrated proxy authentication need a company-network spike; do not claim complete enterprise proxy support from a feature flag. [reqwest documentation](https://docs.rs/reqwest/latest/reqwest/)

rquickjs exposes Rust bindings and lists Windows MSVC targets. Its runtime is an embedded JavaScript environment, not the Node.js ecosystem. Verify the selected engine revision, Windows build toolchain, interrupt behavior, and license notices when pinning dependencies. [rquickjs documentation](https://docs.rs/rquickjs/latest/rquickjs/)

If the GUI candidate cannot meet the budgets or fundamental editing/accessibility requirements, evaluate a Win32-based shell before broad feature implementation. Keep that change confined to the UI adapter. Do not adopt a web wrapper as an implicit fallback. Likewise, keep the JavaScript host contract independent of the selected interpreter.

## 3. Module boundaries

Use a Cargo workspace. Modules are separate crates when they need an independently enforced dependency boundary; avoid turning every helper into a crate.

```text
duckie-desktop   GUI, commands, presentation state, Windows integration
       |
duckie-app       single-request orchestration, snapshots, run lifecycle
       |
       +-- duckie-model       versioned request/result/diagnostic types
       +-- duckie-storage     local files, migrations, secret resolution
       +-- duckie-http        HTTP transport and bounded response capture
       +-- duckie-openapi     import -> proposed requests + diagnostics
       +-- duckie-tests       assertion contract and worker client
                                   |
                            duckie-test-worker.exe (on demand)

Future scenario plugin -> host execution API -> duckie-app
```

The domain model does not depend on egui, reqwest, or the JavaScript engine. The transport knows nothing about collection files or widgets. Import produces a reviewable draft; it does not write directly into storage. Test code receives an execution result; it does not control transport in v1.

### Execution contract

The following are conceptual interfaces, not finalized Rust signatures:

```text
prepare(RequestDefinition, EnvironmentSnapshot, SecretResolver, RunBindings)
    -> PreparedRequest | ValidationDiagnostics

execute(PreparedRequest, ExecutionPolicy, CancellationToken)
    -> ExecutionResult

evaluate(TestSource, TestContext, TestLimits, CancellationToken)
    -> TestReport

import(SourceDocument, ImportOptions)
    -> ImportDraft { requests, provenance, diagnostics }
```

`RequestDefinition` has a stable ID, method, URL template, ordered parameter/header rows, auth bindings, body description, test-file reference, and settings. Header and query collections preserve duplicate names. `PreparedRequest` owns an immutable snapshot of resolved input, including any credentials required for that run. Sensitive types must not expose values through default debug formatting.

`ExecutionResult` contains a run ID, request revision, redacted request summary, transport outcome, optional status, ordered response headers, body handle, byte counts, and timings. Model HTTP 4xx/5xx as completed HTTP responses; DNS, TLS, timeout, cancellation, and size-limit outcomes are distinct. Tests can intentionally assert a 401 or 500 response.

`RunBindings` is empty for ordinary v1 execution. It is an explicit, ephemeral override map reserved for callers such as a future scenario runner. The core does not infer relationships between requests or mutate environment files after a run.

### Thread and process model

The main thread handles input and rendering only. One small Tokio runtime performs HTTP I/O; bounded background jobs perform import, file reads, and body formatting. Send state changes through bounded channels and wake the UI on meaningful events. Avoid a worker thread per request and repeated copies of response bodies.

V1 allows one active single-request run across the application, including its automatic test evaluation. Opening or editing another request remains possible while it runs. Send becomes available after evaluation finishes or is stopped; a manual test rerun also occupies this run slot. The engine contract remains independent of that GUI policy so a later runner can manage concurrency explicitly.

The test worker starts only when tests actually execute. Reuse it briefly for repeated runs, then stop it after 30 seconds idle. Create a fresh JavaScript runtime per evaluation so state and memory do not leak between runs. Launch without a console window; terminate the worker on application exit. All performance reporting includes the child process.

## 4. Request lifecycle and HTTP policy

1. Capture the current unsaved request, selected environment, auth bindings, body, and test source as one immutable revision. Sending does not implicitly save edits.
2. Resolve variables once, validate the URL and headers, and report missing values at their source. No network call occurs when required inputs are unresolved.
3. Apply bearer/API key headers immediately before transport. Reject ambiguous auth collisions instead of silently replacing a manually supplied header.
4. Execute once, stream the body into bounded storage, and emit coarse progress events. Reuse a client pool keyed by transport configuration, with a small eviction bound.
5. Publish the response independently of test completion. Run enabled tests automatically against the completed body.
6. Keep response and test outcomes separate. Publish exactly one terminal transport outcome even if cancellation races with completion.

Proposed defaults: 10-second connect timeout, 30-second total request timeout, TLS verification enabled, automatic retries disabled, and redirect following disabled. The last two choices preserve the single-call model and avoid implicit repeat writes or credential forwarding. Show a 3xx response normally; following its Location is a separate user action that creates an editable request and clears auth on an origin change. Cancellation stops local work but cannot undo a request already processed by the server.

Support automatic system proxy selection, explicit HTTP(S) proxy, bypass hosts, and direct connection. Proxy credentials belong in the secrets file. Surface the selected transport mode in request settings. Do not ship a global TLS-verification-off switch; trust corporate certificates through Windows configuration. PAC/WPAD and integrated authentication compatibility are validation items, not assumed guarantees.

No cookie jar persists between requests. Users can explicitly edit a Cookie header when needed. Request bodies retain authored bytes unless the user chooses formatting; structured form modes perform their defined serialization. Preserve URL encoding intentionally and expose the final redacted URL in request details.

### Bounded response handling

Keep up to 2 MiB of decoded response bytes in memory; spill larger bodies to a private local temporary file. Apply a default 50 MiB decoded-body cap and a separate 50 MiB encoded-body cap, both configurable per request. Stop and label an incomplete response when either cap is exceeded. The transport adapter must retain encoded byte accounting: disable opaque automatic decompression if necessary and use a bounded streaming decoder. Check both limits during streaming; do not first buffer an oversized compressed response.

Show at most 1 MiB in the initial text preview. Parsing, pretty printing, search, and additional pages run on demand off the UI thread. Binary responses show metadata and a Save action. A spooled body is read in pages rather than copied into a giant editor string. State whether size refers to received encoded body bytes or decoded body bytes; do not imply that either includes all wire overhead.

Tests can access complete bodies up to 10 MiB by default. Above that limit, allow metadata assertions but have body access report a specific limit error. Never pass a truncated preview to an assertion as though it were the full body. Transport-truncated bodies skip automatic tests with a visible explanation.

Retain at most three completed response results in the session, each bounded by its request's capture limit. Evict the least recently viewed inactive result when a new result needs space; keep the selected result until replaced or closed. Body handles have explicit ownership so display and test access do not require duplicated files. V1 does not retain multiple historical results per request.

Temporary bodies are local sensitive data. Store them in an application-owned temporary directory with access restricted to the current user where Windows allows, delete when the response is released, and clean only owned stale files on the next startup. Do not persist response history in v1. OS paging, backups, and external tools are outside the application's deletion guarantees.

## 5. Local file format and secrets

Use UTF-8 JSON with two-space indentation and LF newlines, plus JavaScript and separate body files. Stable IDs survive renames; ordering is explicit in the collection manifest. Avoid timestamps in shared files unless they are meaningful user data.

```text
My APIs/
  duckie.json                  schema version, collection name, ordered IDs
  requests/
    get-customer.request.json
  bodies/
    create-customer.json
  tests/
    get-customer.test.js
  environments/
    dev.json                   non-secret values
  secrets.example.json         empty keys, safe to share
  .duckie/
    secrets.json               real values; separate, ignored by default
    state.json                 selection, layout; no resolved secrets
  .gitignore
```

Example request definition:

```json
{
  "schemaVersion": 1,
  "id": "req_get_customer",
  "name": "Get customer",
  "method": "GET",
  "url": "{{env.baseUrl}}/customers/{{request.customerId}}",
  "variables": { "customerId": "42" },
  "query": [],
  "headers": [
    { "enabled": true, "name": "Accept", "value": "application/json" }
  ],
  "auth": {
    "bearer": { "secret": "internalBearer" },
    "apiKey": null
  },
  "body": { "kind": "none" },
  "tests": { "file": "tests/get-customer.test.js", "enabled": true },
  "timeoutMs": 30000
}
```

File references are relative to the collection root. Collection-managed files must remain within that root; file-upload attachments outside it require explicit selection and are marked non-portable.

For query parameters, the saved URL contains the scheme/authority/path template and the `query` array stores ordered editable query items. The URL editor presents these together as one address. Its parser preserves raw encoding for untouched literal items and stores imported structured serialization metadata with the corresponding row. Do not keep two independently editable copies of the query string. If an unresolved template prevents decomposition, retain the opaque URL and disable structured query editing until it can be parsed safely.

An environment file is `{ "schemaVersion": 1, "name": "dev", "values": { "baseUrl": "https://api.example.internal" } }`. The separate secrets file is `{ "schemaVersion": 1, "environments": { "dev": { "internalBearer": "PASTE_TOKEN_HERE" } } }`. The latter value is an illustrative placeholder, never a generated real credential. API key bindings additionally store the header name in the request and only the value in this file.

Use explicit namespaces: `env`, `request`, and `secret`. Interpolation is one-pass string substitution, never code evaluation. Typed JSON value insertion is a separate editor operation that JSON-encodes the selected value; plain raw-body interpolation must not claim to escape arbitrary JSON safely. Future run bindings override named request variables for that run; they cannot silently replace secrets. Surface unresolved variables before Send.

The app creates or preserves `.gitignore` entries for `/.duckie/`, updates `secrets.example.json` with empty keys, and never includes real secrets in ordinary collection export. An explicit **Export secrets...** action writes a separate JSON file that the user can share. A file chooser can load a separately shared secrets file into the active environment. Plaintext separation prevents common accidental sharing, not access by other software or inclusion when somebody manually zips the whole directory. Git ignore rules do not remove files already tracked.

Authentication fields default to a masked session value. **Remember in secrets file** is an explicit choice; it stores the value separately and saves only a binding in the request. Pasting `Bearer <token>` normalizes one optional prefix so it is not sent twice. Do not decode or refresh tokens automatically. On 401, show the actual response with a Replace token action; do not assume expiry caused the rejection.

Write files through same-directory temporary files and Windows-compatible atomic replacement where available; report failures without discarding the in-memory draft. For multi-file saves, write dependencies first and update the manifest last. Recover interrupted import/save transactions from a small local journal. Detect external modifications using saved content hashes before overwriting, and reload or compare explicitly. Read newer unsupported schema versions without rewriting them, or refuse editing with an actionable version message. Preserve supported extension fields through round trips.

## 6. Response tests

Ship a small synchronous JavaScript API. No package manager, transpilation, network client, filesystem API, native module loader, or shell access is exposed to tests. A response assertion suite should be understandable without installing anything.

```javascript
test("returns the requested customer", () => {
  expect(response.status).toBe(200);
  const customer = response.json();
  expect(customer.id).toBe(42);
  expect(customer.name).toBeType("string");
});

test("returns JSON promptly", () => {
  expect(response.header("content-type")).toContain("application/json");
  expect(response.durationMs).toBeLessThan(1000);
});
```

`test(name, fn)` records pass/fail and continues after a failed assertion. A top-level syntax or runtime error is a suite error with file, line, and column; a promise-returning callback is an explicit unsupported-async error in v1. `expect` initially provides `toBe` (strict scalar equality), `toEqual` (deep JSON equality), `toContain`, `toBeType`, and numeric comparisons. Failure output includes bounded expected/actual values and location.

`response` exposes status, case-insensitive `header(name)`, `headers(name)` for all duplicate values, `text()`, `json()`, decoded body size, and duration to completion of the body, excluding test time. JSON decoding is lazy and cached within the evaluation. Invalid JSON throws an ordinary test error. Standard JavaScript numeric precision applies; APIs requiring exact large integers should return them as strings or use text assertions.

The test context includes read-only public environment values and the redacted request summary, never the secrets map. Response content itself may be sensitive and is still available for assertions. Console output is bounded to 64 KiB and 1,000 entries. UI output is escaped text; it does not render remote HTML or load resources.

Proposed limits are 2 seconds of evaluation wall time, 64 MiB of JavaScript heap, a bounded stack, and 128 MiB of worker private memory. The host allows 3 seconds for worker startup separately. Engine interrupt and allocation limits are backed by a parent watchdog and a Windows Job Object for process lifetime/memory control. QuickJS documents memory, stack, and interrupt controls; the selected Rust binding and native callbacks still need dedicated verification. [QuickJS embedding documentation](https://bellard.org/quickjs/quickjs.html#Memory-handling)

Process separation supports crash recovery and cancellation; it is not an OS security sandbox. Restrict the exposed host API and keep engine dependencies patched. Do not execute scripts when opening a collection. User-triggered Send with tests enabled or Run tests is the execution boundary. Re-running tests operates on the retained response without sending HTTP again, and records both response revision and current test revision.

## 7. OpenAPI import

Target the request-relevant portions of OpenAPI **3.0.x, 3.1.x, and 3.2.x** JSON. Do not claim full specification validation. Unknown future minor versions get an explicit compatibility message. JSON Schema dialect differences and newer request serialization features require version-aware adapters and fixtures. The current specification family includes 3.2.0, so a generic “3.x” label must not silently mean only 3.0. [OpenAPI 3.2.0](https://spec.openapis.org/oas/v3.2.0.html)

The import flow is acquire -> bounded parse -> resolve -> normalize -> generate -> review -> commit. URL acquisition supports session bearer/API key headers because the specification may itself be protected. Import credentials are scoped to the selected origin and are not saved in source provenance. Import never calls the generated API operations.

Map operation/path/root servers with the applicable precedence; let the user select a server and edit its variables. Combine path and operation parameters correctly, preserve serialization metadata, group primarily by tags, and use operationId or method/path as the generated name. Map bearer and header API key schemes into empty secret bindings. For OAuth2/OpenID-described services, propose manual bearer binding with an explanatory diagnostic. Preserve security alternatives and combined requirements in the review instead of flattening them incorrectly. [OpenAPI 3.0.4 request and security objects](https://spec.openapis.org/oas/v3.0.4.html)

Example generation prefers a selected named/media/parameter example, then schema examples, defaults, enum choices, and finally deterministic type-based values. Resolve references, omit read-only properties from request bodies, bound recursion and array size, and identify choices for composed schemas. Examples are starting data, not proof that a complex schema is satisfied. Handle 3.1 schema semantics in its adapter. [OpenAPI 3.1.2 schema and example objects](https://spec.openapis.org/oas/v3.1.2.html)

Product policy for difficult inputs:

- Internal JSON pointers and relative local JSON references are supported. Resolve local references only within the selected specification directory tree unless the user selects another file explicitly.
- Remote references are fetched only as part of the explicit import operation, from the same origin by default. Other origins appear in review and require selection; do not forward credentials across origins. No background ref refresh.
- Apply a 20 MiB root-document limit, 50 MiB total fetched input limit, 100-document reference limit, recursion-depth limit of 32, and 30-second acquisition deadline. Detect cycles and show their paths.
- Respect supported parameter encoding, including ordinary `style`/`explode` combinations. Unsupported or ambiguous serialization blocks that affected request until edited; never create a runnable request with guessed encoding.
- Unsupported auth schemes, callbacks, webhooks, external example assets, and schema features get operation-specific diagnostics. Import unaffected requests. Unresolved required values remain visible placeholders and block Send.
- Multiple media types and examples are selectable before import. Generate no response fixtures and no test scripts. A Tests tab starts empty.
- Store source identity and operation identity without auth values. Reimport defaults to a new collection. Updating an existing collection requires an explicit per-request diff and preserves edited bodies/tests unless replacement is chosen.

## 8. Extension architecture and future scenarios

V1 ships built-in importer and test modules behind the contracts above. Their code is packaged with the application, but parsing and test runtime initialization happen only on demand. A small in-process registry describes their IDs and supported capabilities. Prove replacement with a fake importer/test provider in contract tests; avoid a marketplace or generic plugin UI in v1.

For later independently installed plugins, propose **local out-of-process executables with a versioned framed protocol over inherited pipes**. Use a manifest declaring plugin ID, executable, protocol range, capabilities, and optional commands. Validate manifests without launching binaries; launch an enabled plugin only when its capability is invoked. Do not use a public Rust dynamic-library ABI as the compatibility boundary.

Define capability-specific messages: `import`, `evaluate`, and, later, `runScenario`. Shared host operations include `executeRequest`, `readResponseChunk`, `releaseResponse`, and `reportResult`. Exchange body handles and bounded chunks rather than serializing entire large responses repeatedly. Negotiate protocol major/minor versions, reject incompatible majors, bound frames and outstanding calls, and propagate cancellation and deadlines. A crashing plugin fails its operation without terminating the GUI.

This is a planned extension contract, not a v1 promise of binary plugin compatibility. Native third-party executables run with the user's OS privileges unless separately sandboxed; a capabilities manifest describes host access, not complete confinement. Installation/enablement must be explicit and local. No plugin discovery network calls or launch-on-folder-open behavior.

The later scenario module owns step ordering, extraction, run-scoped variables, conditions, and final comparisons. It invokes the existing execution service for each step:

```text
Step 1: GET customer -> save response JSON field id as customerId
Step 2: POST quote using customerId -> save quote.id as quoteId
Step 3: POST order using customerId and quoteId
        -> compare order.customerId and order.quoteId with those values
```

The scenario context holds extracted values in memory; it does not rewrite the original requests or environment files. Stable request IDs, immutable results, body-handle lifetimes, cancellation, and structured assertions already support this path. V1 does not implement a scenario language, scheduler, graph editor, or hidden request chaining through test scripts.

## 9. Performance acceptance targets

These are proposed release gates, not measurements. Establish the reference laptop during the first spike: Windows 11 x64, four or more CPU cores, 16 GiB RAM, SSD, integrated graphics, normal Windows security software enabled. Measure release binaries outside a debugger; record build, hardware, power mode, renderer, display scale, and OS version.

| Measurement | Initial target |
| --- | --- |
| Warm launch to usable URL editor | p95 <= 300 ms over 30 launches |
| Cold launch to usable URL editor | p95 <= 1 second over 20 controlled cold trials |
| Restored 1,000-request collection searchable | p95 <= 500 ms after first usable frame; load bodies/tests lazily |
| Idle private bytes, normal collection, no retained response | <= 80 MiB; also record working set and GPU allocations |
| Idle CPU, after 30 seconds without input | <= 0.1% of total machine CPU averaged over 60 seconds; no continuous repaint |
| Typing, selecting requests, and visible scrolling | p95 input-to-paint <= 16 ms on the reference 60 Hz display |
| Warm local request dispatch | p95 <= 5 ms from valid Send command to transport dispatch, excluding network latency |
| Cold test worker overhead | p95 <= 150 ms before evaluation for a 1 KiB response and trivial test |
| 50 MiB response streaming | Responsive cancellation and preview; bounded buffers, no full-body copies; record peak private bytes including workers |

Measure both an empty start and a restored collection so deferred loading cannot hide unusable startup. Use a fixed 1,000-request collection and a 10 MiB OpenAPI document for normal fixtures, plus a 10,000-request stress collection. Benchmark small JSON, large single-line JSON, binary, compressed, truncated, and slow-streaming responses. A 50 MiB stream with tests disabled should add no more than 32 MiB private bytes over the settled baseline; metadata-only tests on that body must not load it wholesale.

A repeatable regression beyond measurement noise blocks a convenience feature even if the absolute ceiling still passes. Record the cause and reduce or defer the feature; do not quietly relax budgets. Some first-spike targets may prove unrealistic, in which case present the measured tradeoff before committing the framework. No runtime monitoring dashboard or telemetry is required to enforce development benchmarks.

## 10. Implementation sequence and validation

1. **Resolve stack risk.** Build only an empty native shell, representative editor/table, one HTTPS request, and one JavaScript assertion in a worker. Measure launch/idle; check Windows corporate TLS/proxy, remote desktop, DPI, keyboard, Narrator, and worker termination. Pin dependencies after this gate.
2. **Complete the manual request flow.** URL, parameters, headers, body, pasted auth, Send/Cancel, response capture/display, and local save/load. Demonstrate offline startup and no unsolicited network calls.
3. **Complete built-in tests.** Implement the documented assertion API, diagnostics, auto-run after Send, and explicit re-run without HTTP. Verify pass/fail, malformed JSON, script exceptions, infinite loop, excessive allocation, worker crash, and body limits.
4. **Complete OpenAPI import.** Local and protected URL acquisition, examples, references, review, and editable output. Use small independent fixtures for each supported version/serialization rule and a sanitized real company specification.
5. **Harden and package.** Exercise interrupted saves, external edits, schema migrations, secret exclusion, duplicate headers, auth collisions, redirects, compression limits, timeouts, and cancellation races. Deliver a portable Windows ZIP first; an installer and signing follow distribution needs without a runtime account requirement.

Keep protocol/domain tests independent of the GUI. Use a deterministic local HTTP test server for transport behavior and ensure network-failure tests never depend on a public service. Test future scenario readiness with a development-only caller executing three dependent requests through the same contracts; this does not ship as a v1 feature.

Before implementation, the remaining choices are limited: validate the proposed Windows baseline against the team's machines, obtain representative sanitized OpenAPI fixtures and corporate network behavior, and select an application license appropriate to the project's distribution goals. These do not block this proposal. Source and dependency license decisions deserve an explicit repository decision before public distribution; no license is inferred from the product philosophy alone.
