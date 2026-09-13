# Duckie UI design proposal

Status: proposed design, 13 September 2026. These wireframes describe behavior and layout; they are not screenshots of an implemented application. Companion document: [Architecture proposal](ARCHITECTURE.md).

## 1. Design intent

Make the common task immediate: open Duckie, select or enter an endpoint, paste credentials if needed, send, and inspect the result. Request composition and the last response should be visible together. The application is a focused Windows developer tool with a native executable, familiar desktop interactions, and no account or onboarding funnel.

The recommended egui shell uses custom-drawn controls with Windows conventions. The interface should feel restrained and readable, not imitate a browser dashboard. Performance requirements in the architecture take precedence over decorative or convenience features.

### Primary tasks

| Task | Path through the interface |
| --- | --- |
| Try an endpoint immediately | Startup scratch request -> URL -> Auth if needed -> Send |
| Call a saved internal API | Select request -> confirm environment -> paste/replace bearer -> Send |
| Call an API-key service | Auth -> API key header name/value -> Send |
| Start from a spec | Import OpenAPI -> File or URL -> select server/operations/examples -> Import |
| Add assertions | Request Tests tab -> write JavaScript -> Send, or Run tests on retained response |
| Share work | Save collection folder; share requests, bodies, environments, and tests through existing tools |
| Share credentials deliberately | Environments -> Export secrets -> separate local JSON file |

## 2. Main window

Default size: approximately 1280 x 820 device-independent pixels, bounded by the available desktop. Minimum practical layout: 900 x 600. Restore window bounds and split positions locally. Use the standard Windows title bar, resizing, minimize/maximize, and native file dialogs.

```text
+----------------------------------------------------------------------------------------+
| Duckie - Company APIs                                                    _  []  X      |
| File   Request   View   Help                           Environment [ dev v ] [Edit...]  |
+----------------------+-----------------------------------------------------------------+
| Company APIs     [+] | Get customer                                      [Save] [ ... ] |
| [Find requests...]   | GET  [{{env.baseUrl}}/customers/{{request.customerId}}]   [Send]    |
|                      |                                                                 |
| v Customers          | Params  Headers  Auth  Body  Tests  Settings                    |
|   GET Get customer * |-----------------------------------------------------------------|
|   POST Create        | Path / request variables                                        |
| v Orders             | Name                 Value                                     |
|   POST Create order  | customerId           42                                        |
|                      |                                                                 |
|                      | Query parameters                                   [+ Add row]  |
|                      | Use  Name                  Value                                |
|                      | [x]  includeOrders         true                                 |
|                      |                                                                 |
|                      |=================================================================|
|                      | Response  200 OK   142 ms   2.4 KiB       Tests: 2 passed         |
|                      | Body  Headers  Test results  Request details                    |
|                      |-----------------------------------------------------------------|
|                      | [Pretty v] [Find...]                        [Copy] [Save body]   |
|                      |  1  {                                                           |
|                      |  2    "id": 42,                                                 |
|                      |  3    "name": "Example Customer"                                |
|                      |  4  }                                                           |
+----------------------+-----------------------------------------------------------------+
| Unsaved changes       Last run 14:32:10 - dev                                            |
+----------------------------------------------------------------------------------------+
```

The left pane defaults to 240 pixels and is collapsible. The right pane contains a request editor above a response viewer, separated by a draggable horizontal divider. The method, URL, and Send button stay visible when request tabs change. The selected environment stays visible during execution.

V1 uses one selected request editor, without a second layer of document tabs. Switching requests retains unsaved drafts in memory and marks them with an asterisk. The last response belongs to its request and never appears under a different request name. Retain at most three completed responses during a session, each bounded by its request's capture limit; evict the least recently viewed inactive result and show a clear empty state if revisited. Keep the selected response until replaced or closed.

The sidebar lists requests by user-defined folders; imported tags seed that grouping. Search matches request name, method, and path. Display method text as well as color. Only visible rows are laid out; do not open body or test files just to populate the sidebar. An unavailable last-used collection leaves a working scratch request and an inline message with Open folder.

### First launch

Open directly into an empty scratch request with GET selected and the URL field focused. The sidebar contains **Open collection**, **New collection**, and **Import OpenAPI**. The response area says **Send a request to see its response**. A scratch request needs no collection folder until the user chooses Save. Temporary auth works without creating a secrets file.

There is no splash screen, mandatory tour, sample cloud workspace, or startup network check. Offline use includes editing collections, local import, and viewing the current session's retained response; sending HTTP still depends on the target service being reachable.

## 3. Request composition

| Control | Behavior |
| --- | --- |
| Request name | Editable; renaming preserves its stable identity. |
| Method | Common HTTP methods in a dropdown; custom method entry available. |
| URL | Single-line editable template; paste a full URL; horizontally scroll long content. |
| Send | Executes the current draft once. With enabled tests, evaluates them after the response. |
| Save | Saves the definition, body, and tests. Does not persist a session credential unless Remember was selected. |
| More menu | Duplicate, Rename, Show in folder, Close response, Delete request. |

The URL editor presents the base/path and query as a single address. Pasting an address decomposes literal query items into the same model used by Params; committing a Params edit updates the displayed address and never appends a second copy. Preserve untouched percent-encoding and duplicate query keys. Imported parameter rows additionally retain OpenAPI serialization metadata. If unresolved URL variables prevent parsing, explain the limitation, leave the authored URL intact, and disable structured query editing until decomposition is possible.

The Params tab separates path/request variables from query rows. Tables have enable checkboxes and keyboard-accessible add/delete controls. Header rows preserve duplicates and show generated auth/content headers as read-only entries with a link to the owning tab. A conflicting manual Authorization or API key header produces an inline correction before Send.

Body modes are **None**, **JSON**, **Text**, **Form URL-encoded**, **Multipart**, and **File**. JSON/text share an undoable code editor. Form and multipart use tables; multipart rows select text or a file. File mode shows the selected path, content type, and size. Formatting is an explicit action and never changes a body merely because it was opened. Missing upload files are highlighted before execution.

Variables appear as editable `{{env.baseUrl}}`-style text. A lightweight popover on selection shows the variable's source and public value, or a masked secret reference. Unknown variables are underlined with an explanatory message. Do not run interpolation scripts. An **Insert JSON value** action escapes a selected variable value for JSON; ordinary raw template substitution is shown as literal substitution.

Request Settings holds timeout, response limits, and proxy mode. Defaults remain sufficient for an ordinary request. The main editor does not expose thread counts, transport libraries, renderer settings, or plugin protocol details.

## 4. Authentication and environments

Authentication supports None, Bearer token, API key header, or the combination of bearer and API key. The combined mode accommodates gateways that require both an access token and a subscription key.

```text
+---------------------------------------------------------------------------+
| Params  Headers  [Auth]  Body  Tests  Settings                              |
|---------------------------------------------------------------------------|
| Authentication   [ Bearer token + API key header v ]                       |
|                                                                           |
| Bearer token                                                              |
| Value          [ **************************************** ] [Show]        |
| Source         This session                                               |
| [ ] Remember in secrets file    Key [ internalBearer              ]        |
|                                                                           |
| API key                                                                   |
| Header name    [ Ocp-Apim-Subscription-Key                         ]        |
| Value          [ **************************************** ] [Show]        |
| Source         .duckie/secrets.json / dev / gatewayKey                      |
| [Replace value] [Clear binding]                                            |
|                                                                           |
| Tokens are supplied by you. Duckie does not sign in or refresh them.       |
+---------------------------------------------------------------------------+
```

Pasting is sufficient; no separate Apply step is needed before Send. Accept a token with or without one `Bearer ` prefix. Mask values initially and keep Reveal an explicit action. A saved secret binding loads for the selected environment. A replacement can remain in the session or update the separate secrets file, as indicated next to the input.

Changing environments changes secret lookup as well as public variables. Session overrides are keyed by environment and secret binding; they do not spill into another environment. Missing credentials show **No value for internalBearer in staging** beside Auth and block sending when auth is configured. The correction opens the affected field directly.

On HTTP 401 or 403, preserve the status, headers, and body. Offer **Replace token** when bearer auth is configured, but do not automatically refresh, retry, or label every rejection an expired token. The next send remains an explicit user action.

The environment editor is a small local-data dialog with **Variables** and **Secrets** tabs. Variables show name/value rows; Secrets show name/masked value and whether they are saved or session-only. Display the actual file location and provide **Open file location**, **Load secrets file...**, and **Export secrets...**. Export opens a Save dialog whose text states that the output contains plaintext credentials. Ordinary collection sharing excludes that file by default.

For a scratch request, choosing Remember first asks the user to choose/create a collection folder. Dismissing that file choice leaves the credential session-only. Secrets never enter normal request autosave/draft data, diagnostics, or request summary output.

## 5. Sending, cancellation, and response display

| State | Visible behavior |
| --- | --- |
| Ready | Send enabled; last response, if any, remains visible with its run time. |
| Invalid input | Inline field messages; Send takes focus to the first problem. No HTTP call. |
| Sending | Send becomes Cancel; show elapsed time and Receiving bytes when available. |
| HTTP complete | Show actual status, body, duration, and size immediately. Tests may still be running. |
| Tests running | A separate Tests running label and Stop tests action; response stays usable. |
| Complete | Independent HTTP and test outcomes; Send available again. |
| Transport failed | Name the DNS, connection, TLS, timeout, or limit error. Keep any partial body labeled incomplete. |
| Cancelled | Show Cancelled; explain that the server may already have processed the request. |

Only one single-request run can be active at a time in v1, including its automatic test evaluation or a manual test rerun. Switching requests and editing remain available; a small run strip names the active request and offers Cancel or Stop tests as appropriate. Send is disabled until the run completes or tests are stopped. Labels remain attached to the original run ID. The response is available to inspect as soon as HTTP completes, even while tests are running.

The running request uses the input snapshot captured at Send. If the user edits it or changes environments meanwhile, show **Response from previous draft** or **Response from dev** as appropriate. Never relabel an old response with the newly selected environment. During a repeat send, dim the old result heading and label it **Previous response** until the new result arrives.

HTTP status and assertion outcome are independent. For example, **500 Internal Server Error / 1 test passed** is valid when the test expects a 500. A failed test does not recolor or replace the HTTP status. A network failure has no invented HTTP status.

### Response tabs

- **Body:** Raw or Pretty for JSON; plain text for textual responses; metadata and Save body for binary. Read-only, selectable, searchable text. Do not execute HTML, scripts, or embedded resources.
- **Headers:** Ordered name/value rows, preserving repeated headers, with local filtering and explicit copy actions.
- **Test results:** Pass/fail rows, suite errors, bounded console output, and an action to jump to the failing test line.
- **Request details:** The resolved method/URL, effective request headers with credentials masked, environment, draft revision, timeout, and total duration. Label this as the effective request summary, not a raw network capture.

Pretty view is automatically available for valid JSON within the initial 1 MiB preview budget. Parse it once in the background and cache its presentation. Larger bodies open as a paged preview with a byte count and Save body. Search can scan the retained full body as a cancelable background action; the label distinguishes **Find in preview** from **Search full body**.

```text
Response  200 OK   3.2 s   24.8 MiB decoded
Body  Headers  Test results  Request details
--------------------------------------------------------------------------
Showing first 1 MiB of 24.8 MiB.  [Next page] [Search full body] [Save body]
Body assertions exceed the 10 MiB test limit. Metadata assertions remain available.
```

When capture stops at a configured limit, show **Response incomplete: 50 MiB limit reached** and label Save as **Save partial body**. Automatic tests are skipped for incomplete transport results. The UI does not present truncated content as a successful complete response.

Redirects display their 3xx response. **Open Location as request** creates a draft for review without sending it; changing origins clears inherited credentials. There is no automatic retry button behavior hidden inside Send.

## 6. Writing and running tests

The request **Tests** tab is the editor; response **Test results** is the output. This distinction keeps source editing and inspection clear while both panes are visible.

```text
+---------------------------------------------------------------------------+
| Params  Headers  Auth  Body  [Tests]  Settings                              |
| [x] Run after Send                  [Insert example] [Run tests]           |
|---------------------------------------------------------------------------|
| 1  test("returns the requested customer", () => {                          |
| 2    expect(response.status).toBe(200);                                    |
| 3    expect(response.json().id).toBe(42);                                  |
| 4  });                                                                    |
|                                                                           |
| JavaScript                                           get-customer.test.js |
|===========================================================================|
| Response 200 OK  142 ms  2.4 KiB                Tests: 0 passed, 1 failed   |
| Body  Headers  [Test results]  Request details                             |
|---------------------------------------------------------------------------|
| FAIL  returns the requested customer                           line 3     |
|       Expected: 42                                                        |
|       Received: 43                                            [Go to code]|
|                                                                           |
| Tests ran against response from 14:32:10 (dev).                            |
+---------------------------------------------------------------------------+
```

An empty test file shows a short explanation and **Insert example**. Importing OpenAPI does not populate tests or example responses. Once tests exist, Run after Send defaults to enabled and is saved with the request. The documented starter API is `test`, `expect`, and `response`; the architecture document specifies its behavior.

**Run tests** evaluates the current code against the last complete retained response for this request. Its tooltip states **Does not send an HTTP request**. Disable it when there is no applicable response. If the draft/environment has changed since that response, keep the action available with the snapshot mismatch label; this is useful for fixing assertions without repeating side effects.

Distinguish **Assertion failed**, **Script error**, **Timed out**, **Memory limit exceeded**, **Body unavailable at this limit**, and **Worker stopped unexpectedly**. Do not summarize a suite crash as zero failed tests. An evaluation with no registered tests says **No tests defined**. Show the response revision and test-file revision in details when they differ from the last Send snapshot.

Provide line numbers, indentation, bracket pairing, undo/redo, find, modest syntax coloring, and clickable error locations. Keep syntax work limited to edited/visible content. No language server, Node.js installation, package browser, debugger, or general IDE is required for v1. Ordinary JavaScript source remains editable in the user's preferred editor.

## 7. OpenAPI import flow

Use a two-stage dialog launched from File or the empty sidebar. Closing before Import writes no collection changes. Long acquisition/parsing work shows a cancelable stage label without blocking the main window.

### Stage 1: source

```text
+--------------------------------------------------------------------------+
| Import OpenAPI                                                       X   |
| [ Local file ] [ URL ]                                                  |
| File [ C:\specs\company-api.json                         ] [Browse...]    |
|                                                                          |
| OpenAPI 3.x JSON                                                         |
|                                                       [Cancel] [Read]    |
+--------------------------------------------------------------------------+
```

The URL variant adds a URL input and a collapsed **Authentication** group for bearer/API key values. Fetch only on Read. Show HTTP/TLS/authentication errors in the dialog and let the user replace credentials. Import auth is distinct from generated request auth and never silently copied into request files.

### Stage 2: review

```text
+--------------------------------------------------------------------------------+
| Import OpenAPI - Company API 3.1.0                                           X  |
| Server [ https://api.example.internal v ]   Target [ New collection v ]          |
|                                                                                |
| [Find operations...]                  Selected request preview                 |
| [x] Customers                         POST /customers                          |
|   [x] GET  /customers/{id}             Content type [ application/json v ]      |
|   [x] POST /customers                 Example      [ Minimal customer v ]      |
| [ ] Orders                                                                     |
|   [ ] POST /orders                    { "name": "Example Customer" }           |
|                                                                                |
| 2 operations selected.  1 needs input.                                         |
| ! Get customer: customerId needs a value before sending.                       |
| Auth: Bearer token -> internalBearer (no saved value)                           |
|                                                                                |
| [Back] [Cancel]                                            [Import 2 requests] |
+--------------------------------------------------------------------------------+
```

The review includes editable collection name and destination folder, server-variable values, operation checkboxes, media/example selection, and diagnostics linked to affected operations. Unchecked operations are not imported. Selecting an alternative auth requirement is explicit. If a request requires both bearer and key, both appear.

Distinguish **Needs input** from **Unsupported**. Missing path values and credentials can be filled after import; unsupported serialization leaves the affected request disabled for Send until corrected. A malformed root document or unrecognized major version blocks import entirely. Unaffected operations can still be imported when others have diagnosed problems.

Generated values are labeled **From example** or **Generated placeholder**. They are editable immediately after import. Reference resolution issues list the referring location and referenced file/URL. Additional remote origins require explicit selection in this review. Import never follows API operation links or sends a generated example request.

On completion, select the first imported request and show a compact result message: **Imported 24 requests; 3 need input**. The ordinary sidebar/editor is now the workspace. No separate spec-bound editor or read-only imported state persists.

Reimport defaults to a new collection. An explicit Update existing choice shows per-request changes and options to keep local edits or replace selected generated fields. Tests are preserved unless deliberately replaced. No silent source synchronization or periodic URL polling.

## 8. Saving, conflicts, and errors

Use explicit Save in v1. Keep unsaved edits across request selection in memory; prompt Save/Discard/Cancel when closing a collection or the application with dirty drafts. Deleting a saved request asks for confirmation naming that request. A failed save leaves the draft intact and identifies the file and actionable cause.

When external edits are detected, show **Changed on disk** with **Reload**, **Compare**, and **Keep my draft**. Keeping the draft does not silently overwrite disk; Save requires choosing which version to retain. Do not continuously parse every file for synchronization. Check on selection/focus and before writes, with bounded metadata work.

Local state saves window position and selected IDs, but no request/response bodies or secret values. Session credentials disappear when their scope closes. Secret saves and exports display the destination so sharing behavior remains legible.

Errors belong close to their cause: URL validation below the URL, missing key beside Auth, import errors inside import, transport failures in Response, and assertion errors beside test output. Use dialogs for file choices and potentially destructive choices; ordinary request failures must not interrupt the keyboard flow with modal alerts.

## 9. Visual system and accessibility

Use a compact 4-pixel spacing scale: 8 pixels inside cells, 12 between related controls, 16 around major groups. Default control height is 30-32 pixels, with adequate hit targets and a less dense option if needed for accessibility. UI type is Segoe UI at approximately 14 pixels; code uses Cascadia Mono when installed, otherwise Consolas, at 13-14 pixels. Do not download fonts.

Support System, Light, and Dark appearance through shared tokens. Proposed dark palette: canvas `#17191D`, panels `#202329`, fields `#292D35`, borders `#454C58`, main text `#F2F4F8`, secondary text `#B7BFCD`, focus/accent `#82B7FF`, success `#8BDCAB`, warning `#F1CA77`, error `#FFA1A1`. Light mode uses the same roles with separately checked colors. Validate contrast in the implemented controls; a palette alone does not establish accessibility.

Use visible focus outlines, text labels for outcomes, and icons only as reinforcement. Keep method names and PASS/FAIL text visible so color is never the sole cue. Prefer static separators and clear selection to shadows, translucency, or animated panels. No idle animation; refresh elapsed time only during active work, at most a few times per second.

At narrower widths, collapse the sidebar and keep the vertical request/response split. Tabs can scroll or use an overflow selector. Keep the URL and Send accessible. Validate layouts at 100%, 150%, and 200% scale, with large text, window snapping, and display changes. Expose control names, roles, values, and error descriptions to Windows accessibility; verify the main path using Narrator and keyboard only before finalizing the framework.

### Keyboard map

| Shortcut | Action |
| --- | --- |
| Ctrl+N | New scratch request |
| Ctrl+O | Open collection |
| Ctrl+Shift+O | Import OpenAPI |
| Ctrl+S | Save current request, body, and tests |
| Ctrl+Enter | Send current request once; disabled during an active run, including tests |
| Ctrl+Shift+Enter | Run tests against the retained response |
| Ctrl+L | Focus URL |
| Ctrl+K | Focus request search |
| Ctrl+F | Find in focused editor or response preview |
| Ctrl+B | Toggle sidebar |
| F6 / Shift+F6 | Move between navigation, request, and response regions |
| Escape | Close the active menu/dialog first; otherwise cancel active network execution or test evaluation |

Standard Tab navigation, selection, copy/paste, and editor undo/redo apply. Show shortcuts in menus and tooltips. Ctrl+Enter does not become a second-send or automatic retry trigger while busy.

## 10. Extension surface and acceptance walkthroughs

Keep v1 navigation focused on Requests. Do not show disabled Scenarios, marketplace, or upgrade destinations. When a scenario extension exists, it may add a **Scenarios** sidebar section and a step editor in the main area. It reuses the same environment picker, request references, response viewer, and assertion results. A disabled plugin contributes no visible placeholder or background process. This is a future layout allowance, not a v1 deliverable.

The initial UI is ready for implementation when these walkthroughs have clear outcomes:

1. Launch offline into a focused URL field; paste a URL/token, Send, and inspect status/body using only the keyboard. No collection or account is required first.
2. Save an internal request with a remembered bearer binding; inspect the files and find the token only in the separate secrets file. Change environment and see missing credentials rather than reusing the previous token.
3. Import a protected OpenAPI JSON URL, choose operations and example data, then edit and send one generated request. The import itself sends no API operation.
4. Receive a 401, inspect its body, replace the token, and deliberately Send again. Receive a 500 and assert it successfully without conflicting status/test messaging.
5. Write a failing assertion, jump to its line, correct it, and rerun tests without another HTTP call. An infinite loop is stopped while the response stays usable.
6. Edit a request during execution and see the returned response labeled with the original snapshot. Switch requests without losing a dirty draft or confusing response ownership.
7. Receive a large or compressed response, scroll its preview, cancel ongoing work, and save full or explicitly partial bytes without freezing the interface.
8. Share a collection normally without secrets; deliberately export a separate secrets file when wanted. Handle an external file edit without overwriting it silently.
9. Repeat the common path with Narrator, keyboard only, 200% scaling, and remote desktop. Confirm launch/idle/input targets against the architecture's benchmark protocol.

These walkthroughs validate the proposed behavior. Visual spacing, editor choice, accessibility coverage, and performance remain subjects for the first native prototype; neither this document nor its wireframes claims that those checks have already passed.
