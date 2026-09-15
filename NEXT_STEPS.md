# Duckie: proposed next features

Repository review: 14 September 2026.

Duckie already covers the core request-edit-send-inspect-test loop unusually well for a small native application. The next investment should make those requests more reusable, make failures easier to understand, and reduce the work required to bring an existing API workflow into Duckie.

This document proposes **18 features**, ordered within three suggested delivery waves. These are product recommendations based on the repository, not accepted owner decisions, implementation commitments, or findings from user research. The numbered v1 backlog is closed according to [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md). That document remains the authority for accepted scope and future backlog decisions; this file is the requested feature exploration. The `.ms` filename is intentional; its contents are Markdown.

## What the recommendations build on

The [README](README.md) describes the current product: local collections and environments, bearer/API-key authentication, several request-body types, Swagger/OpenAPI JSON import, response inspection, and isolated synchronous JavaScript assertions. The implementation already includes request duplication, folders, search, manual redirect navigation, cancellation, whole-body raw search, retained-response test reruns, and reviewed whole-operation spec updates. Those capabilities should not be repackaged as new features.

The source supports several useful extension points:

- [duckie-model](crates/duckie-model/src/lib.rs) separates request definitions, environment snapshots, secret bindings, and run bindings from the desktop. Run bindings already override request variables, but are not a complete scenario or dataset workflow.
- [duckie-app](crates/duckie-app/src/lib.rs) orchestrates one active request/test execution and publishes response and test events separately. This is a useful base for sequential automation.
- [duckie-http](crates/duckie-http/src/lib.rs) owns transport and bounded response processing. Its current elapsed duration is useful, but does not provide a detailed phase breakdown.
- [duckie-openapi](crates/duckie-openapi/src/lib.rs) already handles substantial import complexity and source provenance. Adding another importer or contract validation should reuse that work where appropriate.
- The [desktop UI](crates/duckie-desktop/src/ui.rs), [desktop state](crates/duckie-desktop/src/state.rs), and [dialogs](crates/duckie-desktop/src/dialogs.rs) show where new workflows can enter the existing interface.
- The [test harness](crates/duckie-test-worker/src/harness.js) supplies a small assertion API with bounded reports and logs. Extensions should preserve fresh workers and their resource limits.

All proposals preserve the local-only product contract: no account, telemetry, cloud dependency, automatic spec polling, or unsolicited API traffic. User-started runs may contact the targets the user selects. The [performance document](PERFORMANCE.md) owns historical measurements and their limitations; this review did not rerun them. Any resource limits suggested below are design constraints to validate, not measured results.

## Wave 1: make everyday requests easier to reuse and debug

### 1. Import and export cURL requests

**Implemented 14 September 2026.** Supported cURL commands import as reviewable drafts; strict unsupported-option diagnostics, repeated headers/query values, form/file references, POSIX and PowerShell export, and redacted-by-default credential handling are covered.

**What to add.** A Paste cURL action that turns a copied command into a reviewable request draft, plus Copy as cURL for the current request. Start with an explicit supported subset: URL, method, repeated headers, textual data, URL-encoded fields, and file references. Let the user select the shell syntax when exporting, especially PowerShell versus POSIX shells.

**Why build it.** Developers commonly receive a reproduction as a command copied from documentation, a terminal, or browser developer tools. Reconstructing the method, headers, and body by hand creates friction before Duckie can deliver any value. Export provides the reverse path: a Duckie investigation becomes a reproduction someone can run without installing the application.

**Useful first boundary.** Parse the command as data; never execute it. Report unsupported flags rather than silently dropping their meaning. Show credential placement during import, redact exports by default, and offer deliberate credential inclusion. Preserve duplicate headers and query encoding. File references should remain visibly unresolved until reviewed. The first release succeeds when supported commands round-trip semantically and unsupported behavior is clearly identified.

**Implementation fit.** Add a focused conversion module around `RequestDefinition`, with thin desktop entry points. It should not become shell execution inside the transport layer.

### 2. Resolved request preview and variable inspector

**Implemented 14 September 2026.** The Request menu opens a no-send preview produced by the Send preparation path, with masked URL/headers/body, bounded file metadata/preview, environment, variable provenance, and actionable preparation errors.

**What to add.** A Preview request panel showing the prepared URL, effective headers, body representation, selected environment, and where each template value came from. Missing variables should link back to the relevant editor. Secret values stay masked, including occurrences in URLs and bodies.

**Why build it.** A templated request has two forms: what the developer edits and what Duckie prepares to send. When the wrong environment or an encoded path value produces an unexpected result, seeing only the template makes diagnosis harder. Existing redacted request summaries are a useful foundation; a dedicated pre-send inspector makes preparation explainable before another network call.

**Useful first boundary.** Invoke the same preparation logic used by Send, with no HTTP execution. Label application-prepared headers separately from anything the HTTP library may add later; do not promise a byte-exact wire capture. For streamed files, show metadata and a bounded preview instead of loading the file. Freeze generated values for the preview/send pair once feature 12 exists.

**Implementation fit.** Keep provenance and redaction rules near model preparation, with the desktop rendering the result. This also improves future scenario and dataset diagnostics.

### 3. Bounded request history with replayable drafts

**What to add.** A chronological session history with request name, environment, timestamp, result, duration, and test outcome. Selecting a run reveals its captured input and available response. Open as draft recreates an editable request without sending it.

**Why build it.** Debugging often means changing one input repeatedly. The latest response alone cannot explain which earlier combination worked. Duckie currently retains at most three responses and associates desktop responses with request IDs; a deliberate history model would preserve the investigation sequence rather than treating retention as history.

**Useful first boundary.** Start with session-only metadata and a bounded response-body budget. Clearly mark evicted bodies. Reuse retained bodies without redundant copies, and require current secret resolution when replaying a draft. Persistent history should be an explicit later option with retention and deletion controls because responses can contain sensitive data.

**Implementation fit.** Introduce run-indexed history in desktop state. Respect spool ownership and the existing multi-instance cleanup cautions rather than shortening spool lifetimes to implement eviction.

### 4. Response comparison and pinned baselines

**Implemented 14 September 2026.** The Compare response tab handles retained results and explicit 2 MiB baseline files, showing status, normalized-header, bounded text, and structural JSON changes with visible RFC 6901 ignore rules.

**What to add.** Compare two retained responses or a current response against an explicitly saved baseline. Show status changes, header changes, text differences, and structural JSON differences. Allow a small list of ignored JSON paths for volatile values such as timestamps.

**Why build it.** A successful HTTP status does not show whether a fix changed the intended field or accidentally removed another one. Comparison turns Duckie into a useful before/after investigation tool. It also helps compare development and staging without forcing developers to copy bodies into another application.

**Useful first boundary.** Let users choose both inputs; comparing environments must not automatically send either request. Treat JSON object key order as insignificant and array order as significant by default. Show every active ignore rule. Cap structural parsing and fall back to a bounded text comparison for large inputs. Saving a baseline must be explicit because it writes response content to disk.

**Implementation fit.** Build comparison as a pure module, then integrate with retained responses and feature 3. It can ship before history by comparing an available response with a selected baseline file.

### 5. JSON tree navigation and value extraction

**Implemented 14 September 2026.** Complete valid JSON bodies up to 2 MiB and 128 levels have a collapsible tree, RFC 6901 breadcrumb/pointer lookup, and value/pointer copy actions while the raw viewer remains available.

**What to add.** A collapsible JSON tree with a path breadcrumb, Copy value, Copy JSON Pointer, and Go to path. Include a simple path-based value lookup before considering a larger query language.

**Why build it.** Raw and pretty text are useful, but large nested objects are difficult to navigate when the task is to find one identifier or understand an array element. A tree makes structure visible, and copying a path creates a precise bridge from exploration to an assertion or scenario extraction rule.

**Useful first boundary.** Parse only within a documented size/depth budget, virtualize displayed rows, and preserve the existing raw viewer for larger or invalid JSON. Copying a value should not automatically create a variable or grant text template semantics. Show escaped property names correctly so copied pointers remain unambiguous.

**Implementation fit.** Extend response presentation without changing retained bytes. Reuse the path syntax in features 10 and 13 to avoid three incompatible ways of referring to a JSON value.

### 6. Assertion authoring helpers and actionable mismatch output

**Implemented 14 September 2026.** Response status, headers, and JSON tree values can generate editable status/header/value/type/array-length assertions; a duration template requires an explicit contract limit, and JSON-pointer assertions report bounded expected/actual values with the failing path.

**What to add.** Insert editable tests for status, header presence, JSON field value/type, array length, and response duration. Add targeted assertion helpers and show the failing path plus bounded expected/actual values for structural mismatches.

**Why build it.** Duckie already executes JavaScript tests and navigates to failing source lines. The next friction point is writing the first useful assertion and understanding why a nested comparison failed. Helpers can convert an inspected response into a maintainable test without requiring the user to memorize the harness API.

**Useful first boundary.** Generate ordinary visible JavaScript that users can edit. Distinguish an observed value from a contract: an observed status or duration should not silently become the expected behavior. Escape generated literals correctly and avoid embedding obvious credentials. Keep output caps, synchronous execution, and fresh worker isolation intact.

**Implementation fit.** Pair desktop snippets with small harness improvements. This can deliver value independently of schema validation, which addresses a different, broader contract.

## Wave 2: turn saved requests into repeatable test workflows

### 7. Sequential collection and folder runner

**Implemented 14 September 2026.** A desktop runner reviews and sequentially executes the selected request, current folder, or collection using frozen hydrated revisions and the shared coordinator, with stop/continue, cancellation, distinct result states, and bounded summary retention.

**What to add.** Run selected requests, a folder, or a collection in a visible order, with a chosen environment and a final results table. Offer stop-on-failure, continue-on-failure, and cancellation.

**Why build it.** Individual executable requests become substantially more valuable when they form a repeatable smoke suite. After changing a service, the developer should be able to check a known set of endpoints without selecting and sending every request manually.

**Useful first boundary.** Execute sequentially through the existing service, with no automatic retries or concurrency. Show the selected operations before starting, including methods that may modify server state. Freeze the run inputs and identify their revisions; hydrate deferred bodies/tests before snapshotting them. Bound retained response bodies while preserving summary results for every request.

**Implementation fit.** Add a coordinator in `duckie-app`, not a loop embedded in UI drawing code. Keep assertion failures, transport failures, skipped requests, and cancellation distinct. This is proposed post-v1 automation, not unfinished accepted v1 work.

### 8. Headless command-line execution

**Implemented 14 September 2026.** `duckie-cli` runs an explicitly selected collection environment and either one saved request or the manifest-ordered suite, with text/JSON output, environment-based secret overrides, cancellation, and stable 0/1/2/3 exit codes.

**What to add.** A CLI capable of running a saved request or selected suite with an explicit collection path and environment. Return stable exit codes for success, assertion failure, execution/configuration failure, and cancellation; support concise text and machine-readable JSON output.

**Why build it.** Tests kept beside source code should be usable during development scripts and CI as well as in the desktop. Sharing one execution path prevents the GUI request and its automated equivalent from slowly becoming different tests.

**Useful first boundary.** Support Windows first, matching current product scope. Reuse storage, preparation, transport, and workers. Define secret inputs that do not require literal credentials in command-line arguments, and redact diagnostics. Do not save or rewrite collections during ordinary execution. Resolve the worker reliably relative to the installed binaries.

**Implementation fit.** Create a small CLI crate using the coordinator from feature 7. A separate executable can avoid initializing the renderer and keeps automation independent of desktop startup.

### 9. Data-driven request runs

**What to add.** Attach a local JSON dataset to a request or suite and execute once per row. Bind columns to request variables, display the row name/index in results, and allow selected-row runs. CSV can follow after the mapping and type semantics are clear.

**Why build it.** Boundary cases frequently differ only in one identifier, search term, body value, or expected outcome. Duplicating requests for each case produces maintenance work: a header or endpoint change must then be applied everywhere. A dataset expresses those cases directly.

**Useful first boundary.** Use the existing `RunBindings` precedence deliberately and document it. Specify how booleans, numbers, nulls, and JSON body values are encoded; string substitution alone is not safe typed JSON construction. Cap dataset bytes and row count, avoid mutating saved environments, and let assertions read non-secret case expectations. Reports must identify the exact failed row.

**Implementation fit.** Extend the runner and model with dataset definitions. Hydrate one request and process bounded case inputs rather than manufacturing thousands of permanent request copies.

### 10. Explicit request chaining and response capture

**What to add.** A small scenario definition referencing existing request IDs, with ordered steps and declarative captures from response JSON or headers. For example: create a resource, capture its ID, fetch it, then delete it.

**Why build it.** Many meaningful API checks depend on server-generated state. Manual copying interrupts the test loop and makes a successful investigation difficult to repeat. Chaining tests the behavior of a workflow while retaining the individual requests as independently useful assets.

**Useful first boundary.** Begin with sequential steps, explicit capture names, and stop-on-failure. Missing or invalid captures must fail the step rather than silently reuse an older value. Keep captures scoped to one run, support sensitive-value marking, and treat captured text as literal data without recursive template evaluation. Any cleanup steps need visible, separately reported execution rules.

**Implementation fit.** This explicitly extends the scenarios deferred from v1. Implement orchestration above transport and the worker; do not give test JavaScript unrestricted network access. Stable request IDs and run bindings provide useful foundations, but scenario persistence and result ownership still need design.

### 11. Portable test reports and CI artifacts

**Implemented 14 September 2026.** The shared run-result model exports deterministic summary JSON, JUnit XML, and standalone HTML from desktop and CLI; CLI response snippets are explicit, bounded, and redacted for known secrets.

**What to add.** Export a run as structured JSON and JUnit XML, with an optional standalone HTML report for human review. Include request/case/step identity, environment name, timings, assertions, and explicit failure or skip reasons.

**Why build it.** A test result becomes more useful when it can accompany a pull request, local bug report, or CI failure. Consistent exports let another developer understand the failed check without reproducing the original desktop session immediately.

**Useful first boundary.** Export summaries by default, with response snippets as an explicit inclusion choice. Redact known secrets and make clear that arbitrary response data can still be sensitive. Use stable identifiers and deterministic output ordering; preserve transport errors and suite errors instead of reporting them as ordinary assertion failures. Export remains a local file action.

**Implementation fit.** Define a common run-report model for desktop and CLI. Feature 8's basic JSON output should grow into this format rather than becoming a competing schema.

### 12. Built-in dynamic test values

**What to add.** Named generated inputs for UUIDs, timestamps, bounded random numbers, and seeded sample strings. Show a value's generator and allow it to be frozen for a reproduction.

**Why build it.** Creating resources repeatedly often fails on duplicate identifiers or stale timestamps. Developers currently have to edit these values manually or prepare them elsewhere. A small generator vocabulary removes that friction without introducing a general pre-request programming environment.

**Useful first boundary.** Generate each named value once per execution snapshot so repeated references agree. Make dataset/scenario scope explicit, record non-secret generated values for reproduction, and support a seed where meaningful. A retry or replay must state whether values are reused or regenerated. Preview must not show a different value from the subsequent send.

**Implementation fit.** Resolve generators into run bindings before request preparation. Avoid arbitrary filesystem/network hooks and an additional long-lived scripting runtime.

## Wave 3: broaden API coverage and improve collection maintenance

### 13. OpenAPI-backed response contract validation

**What to add.** Validate a response against a locally retained schema selected by operation, status, and content type. Present path-specific errors separately from authored assertions, and allow explicit validation of a request body before sending.

**Why build it.** Import currently helps create requests, but the specification can also explain what a correct response should contain. Contract checks catch missing fields and incorrect types across a larger response surface than developers usually cover with handwritten assertions.

**Useful first boundary.** Clearly declare supported schema dialects and keywords. Unsupported validation must be reported as unsupported, never as a pass. Handle exact status entries, wildcard/default responses, and media-type selection deliberately. Keep references local to the imported snapshot; validating a response must not trigger remote schema retrieval. Apply traversal and input budgets.

**Implementation fit.** Store a bounded contract snapshot or explicit local contract reference alongside provenance. The importer currently generates examples; that is not equivalent to retaining a complete validation contract. Reuse import acquisition safeguards and report validation results through the existing test presentation where practical.

### 14. YAML OpenAPI and Swagger import

**What to add.** Accept YAML specifications from local files and explicitly selected URLs, including supported referenced YAML files. Feed the normalized document into the existing review and import pipeline.

**Why build it.** Duckie's documented import surface is JSON. Teams whose source specification is YAML need an extra conversion step before they can use the existing importer. Supporting their source format makes initial import and later updates easier without adding another API protocol or workflow model.

**Useful first boundary.** Reuse origin restrictions, local containment, acquisition budgets, cancellation, and provenance sanitation. Bound aliases, nesting, and expanded document size; define duplicate-key and non-JSON value behavior rather than silently changing the document. Preserve useful file/location diagnostics when normalization fails.

**Implementation fit.** Add parsing at the acquisition boundary and keep downstream import behavior shared. This should not change the owner-selected whole-operation update semantics.

### 15. OAuth token acquisition and explicit refresh

**What to add.** Optional local OAuth profiles for client credentials and authorization code with PKCE. Show token expiry and offer a user-triggered refresh action. Bind acquired access tokens through the existing secret system.

**Why build it.** Pasting a bearer token is efficient when a token is already available. It becomes repetitive when short-lived credentials interrupt a test session. A focused token acquisition flow keeps that work close to the API request and reduces accidental use of a token from the wrong environment.

**Useful first boundary.** This explicitly revisits login/token refresh deferred from v1. It means authenticating to the user's API provider, not creating a Duckie account. Open the system browser only on user action, validate OAuth state and PKCE, scope any callback listener to the flow, and keep refresh tokens session-only unless deliberately remembered. Do not refresh on startup, in idle background work, or as an implicit retry after a 401.

**Implementation fit.** Introduce a focused authentication service and UI profile editor. Avoid mixing token endpoint behavior with ordinary request execution or automatically forwarding credentials across origins.

### 16. Opt-in cookie sessions

**What to add.** Named cookie jars scoped to a collection and environment, with inspect, edit, clear, and session enable/disable controls. Show which applicable cookies an enabled session will send.

**Why build it.** Cookie-authenticated APIs and session-oriented web backends are awkward to test when every `Set-Cookie` value must be copied manually. Explicit sessions make these workflows repeatable while giving users a way to isolate separate identities.

**Useful first boundary.** This extends shared cookie sessions explicitly deferred from v1. Start with memory-only jars and deliberate opt-in. Apply domain, path, expiry, and secure transport rules; document how a standalone client differs from browser SameSite behavior. Define conflicts with manually supplied Cookie headers and keep environment changes from silently carrying identity across targets.

**Implementation fit.** Make session identity part of transport/client selection and execution snapshots. Do not attach one global jar to every cached HTTP client. Clearing a jar should have predictable consequences for subsequent runs.

### 17. Request timing breakdown and connection diagnostics

**Implemented 15 September 2026.** Request details now show total duration, request-start-to-response-headers time, combined body-transfer/decoding time, and the negotiated HTTP version. The UI explicitly identifies DNS/TCP/TLS, proxy, upload, server wait, connection reuse, and separate transfer/decoding phases as overlapping or unavailable rather than presenting inferred measurements.

**What to add.** A response details panel separating observable setup/header wait, body transfer, and decoding work, plus negotiated HTTP version and connection reuse information where available. Expose DNS, TCP, and TLS phases only when the transport can measure them reliably.

**Why build it.** A single duration answers whether a request was slow but gives little guidance about where to investigate. Developers need to distinguish waiting for response headers from downloading a large body or decoding it locally. These observations help focus debugging and make timeout behavior easier to understand.

**Useful first boundary.** Begin with timestamps the existing transport can obtain accurately. Label unavailable phases and overlap; do not infer server processing time from time to first byte. Reused connections may have no new handshake, and proxies change what can be observed. Instrument only active execution and keep diagnostic data free of credentials.

**Implementation fit.** Extend execution result metadata in `duckie-http` and the model. Validate timing meaning with controlled local endpoints before presenting fine-grained figures as precise measurements.

### 18. Reviewable external-edit conflict resolution

**What to add.** An in-app comparison of the loaded version, current draft, and changed disk files, with explicit choices to retain disk content, retain draft content, or apply a reviewed merge. Explain conflicts at the request/body/test level.

**Why build it.** Plain-file collections are one of Duckie's strongest product choices, and existing conflict detection protects them from silent overwrite. The next improvement is helping a developer recover when a Git operation, editor, or second Duckie instance changes the same collection. Protection is much more useful when the resolution path preserves valuable edits without manual reconstruction.

**Useful first boundary.** Start with side-by-side comparisons and explicit whole-file choices; add three-way text merging only where a real base version is available. Resolve a coherent collection transaction and recheck hashes under the existing lock before saving. A file can change again while the comparison is open. Missing files, deferred content, and unknown fields must survive the review correctly.

**Implementation fit.** Reuse storage conflict hashes, locks, and journals. This concerns local file conflicts; it does not supersede the separate owner decision that OpenAPI updates replace whole operations.

## Suggested delivery order and decision points

Start with cURL exchange, resolved request preview, and assertion helpers (1, 2, 6). They improve the current single-request workflow without depending on a new execution model. Follow with bounded history, response comparison, and JSON navigation (3–5), keeping their body budgets explicit.

Then build the sequential runner and CLI (7–8) around one shared execution/report contract. Data-driven runs, chaining, and richer report export (9–11) can extend that foundation. Dynamic values (12) are small in surface area but should share snapshot semantics with preview and replay from the beginning.

Wave 3 contains more independent choices. YAML import (14) can move earlier if real collections require it. Contract validation (13) has substantial specification and resource-limit complexity. OAuth and cookies (15–16) should move earlier only when authenticated workflows are a recurring need; both intentionally extend previously deferred scope. Timing diagnostics and conflict resolution (17–18) are useful quality-of-work improvements that can ship incrementally.

These waves are relative recommendations, not calendar estimates. Before accepting an item, choose its first-release boundary and update the authoritative status document. For implementation, use the README's appropriate checks and focused behavioral coverage. For changes affecting launch, retention, parsing, or workers, use the relevant performance procedures and record fresh dated results in the performance document; historical v1 acceptance does not prove a new feature meets its resource targets.

Cloud sync, a plugin marketplace, automatic polling, a resident test service, and broad protocol expansion are not part of this proposed set. The 18 features above concentrate on making Duckie's existing HTTP workbench more useful while keeping its native, deliberate, local workflow recognizable.
