# witslog — Zero-Boilerplate Auto-Instrumentation + Log-Quality Redesign

## Context

Two problems, one root cause.

1. **Logs too minimal.** Production witslog events like `[witsnote-proxy] Error :: upstream 409 for /cards/…` and `[witsnote-proxy] Error :: fetch failed` are not diagnosable from the log alone — no upstream response body, no exception cause chain, no correlation id, no latency, no error code.
2. **Boilerplate.** To get even those weak logs, a developer must hand-write `ensureWitslog()` + `try/catch` + `witslog.exception(...)` + per-status `witslog.error(...)` in **every** route handler (`route.ts`) and every fetch call site. There is no framework-level auto-capture, and nothing at all captures client-side (React Query) failures.

**Goal:** evolve witslog's architecture so a developer *mounts instrumentation once* and gets DevTools/Sentry/OTel-grade capture across server routes, outbound fetches, and client data-layer failures — without touching individual handlers. WitsNote (`C:\projects\WitsNote`) is the reference adopter used to prove the design.

This plan is the consolidated deliverable: the diagnostic investigation (Part A), the architectural gaps (Part B), the redesign (Part C–E), the preserved scope decision (Part F), verification, and a prioritized action plan.

---

## Part A — Investigation findings (evidence base)

Verified via codegraph over both repos.

**Emitter** = `client/app/api/proxy/[...path]/route.ts` (the catch-all API proxy). NOT `client/proxy.ts` — that is the auth-guard middleware, unrelated.

Propagation:
```
browser fetch /api/proxy/cards/{id}
 → route.ts handler → fetch(upstreamUrl)               [route.ts:59]
     ├─ throws (undici) → catch → witslog.exception("witsnote-proxy", e)      → "fetch failed"
     └─ resp.status>=400 → witslog.error("witsnote-proxy", `upstream ${s}…`)   → "upstream 409 for /cards/…"
 → upstreamData = resp.json()                           [route.ts:80, AFTER the log]
```

Weaknesses:

| # | Location | Defect |
|---|----------|--------|
| W1 | route.ts:69–78 | logs status only; **upstream response body (`upstreamData` = server `{error:{code,message,details}}`) never logged**, and it is read at :80 *after* the log — the 409's reason is unavailable at log time |
| W2 | `bindings/node/index.js:42` `exception()` | captures `err.name/stack/message`, **never `err.cause`**; undici `TypeError: fetch failed` carries the real reason (ECONNREFUSED/ETIMEDOUT/ENOTFOUND) on `.cause`, which is discarded |
| W3 | route.ts | no correlation id → cannot stitch browser→proxy→server |
| W4 | route.ts | no latency (no timing around fetch) |
| W5 | route.ts | no `error_code`; 409 (expected optimistic-concurrency conflict) logged at `error` severity → noise |
| W6 | `witslog-cli/src/main.rs:462` | `query` prints `id [app] Sev :: message` only — context/tags invisible |
| W7 | `witslog-cli/src/main.rs:400–420` | even `get <id>` detail drops context/tags/stacktrace/exception/error_code/correlation_id |

W1–W5 are **capture** gaps; W6–W7 are **display** gaps. Storage is not the problem — `witslog-core/event.rs:36` already persists `context, tags, exception, stacktrace, stack_norm, root_cause, error_code, correlation_id, parent_event_id, environment, version, hostname, metadata`, and the Node/PHP/Python `ALLOWED_FIELDS` already marshal all of them. The fields exist; nothing populates or renders them.

Latent correctness bug found while tracing: `client/lib/api/hooks.ts:160` reads `err.response?.status`, but the client `ApiError` (`client/lib/api/errors.ts`) exposes status at `.status`. The 409 conflict branch (invalidate + `card:conflict` event) likely never fires. Fix during adoption.

---

## Part B — Root architectural gaps

1. **SDK `exception()` drops the cause chain** (W2) — every consumer loses root cause on wrapped errors (undici fetch, DB drivers, etc.).
2. **No Next.js adapter.** `bindings/node/frameworks/` has `express.js` only; Python has fastapi/django/flask; PHP has Laravel. Next.js App Router (`route.ts`) has no `next(err)` middleware, so today there is no drop-in hook — forcing per-handler try/catch.
3. **No instrumented-fetch helper.** Outbound HTTP is the exact axis that fails (proxy → upstream), yet capturing it requires hand-written logging at each call site.
4. **No client-side auto-capture.** React Query per-mutation `onError` in `hooks.ts` only does cache rollback; there is no global `MutationCache`/`QueryCache` handler, and the witslog **browser reporter** (`bindings/browser/witslog-browser.js` → `witslogBrowserIngest`) is **not wired** into WitsNote. Client query/mutation failures are captured nowhere.
5. **CLI hides captured context** (W6/W7) — even a richly-captured event looks bare, so the redesign is invisible without a render fix.

---

## Part C — Redesign: reusable auto-instrumentation in the witslog Node/browser SDK

Five pieces. All map onto **existing** contract fields — no ABI bump required.

### Piece 1 — Instrumented fetch (the universal boilerplate killer) — NEW `bindings/node/fetch.js`
Export `witslogFetch(input, init, opts?)` — an **explicit wrapper** around `fetch` (decision: explicit only; no global monkeypatch, to stay safe with Next.js's own fetch patching/caching). Swap `fetch(` → `witslogFetch(` at the two choke points (api-client request helper + the proxy) — 2 sites, not N. Per call it automatically:
- generates/propagates a correlation id (reuse incoming `x-request-id`, else mint one; set it on the outbound request headers);
- times the call (`latency_ms`);
- on a thrown error: unwraps the `err.cause` chain, logs `witslog.exception` with `error_code` (`UPSTREAM_UNREACHABLE`/`UPSTREAM_TIMEOUT` from `cause.code`), `root_cause`, and `context.http` + `context.timing`;
- on a non-2xx response: reads a clamped body snapshot, logs at **`warn` for expected 4xx (409/422/404 etc.)** and **`error` for 5xx / unreachable** (decision), with `error_code` from the server body's `{error:{code}}` when present, plus `context.http`/`context.upstream`/`context.timing`.

Reuse the existing `clampString`/`clampSeverity`/redaction helpers from `express.js` for body snapshots so secrets/size are bounded.

### Piece 2 — Next.js adapter — NEW `bindings/node/frameworks/next.js`
Mirrors the express/flask adapter convention. Exports:
- `register(application, config?)` — mounts witslog once (calls `init`, optional `createProject`); used from Next.js `instrumentation.ts`.
- `onRequestError(err, request, context)` — Next.js 15's official server-error hook; auto-captures **any** uncaught route/RSC/SSR error with `{ method, path, routePath, renderSource }` context and full cause chain. Re-exported from `instrumentation.ts`.
- `withWitslog(handler, opts?)` — optional higher-order wrapper for a single `route.ts` handler when a team is on Next < 15 or wants explicit per-route timing/correlation without global instrumentation.

Developer cost after this: a 3-line `client/instrumentation.ts`. No per-handler code.

### Piece 3 — Client React Query auto-capture — NEW `bindings/node/frameworks/react-query.js` (browser-safe) + wire `bindings/browser/witslog-browser.js`
Export `attachWitslog(queryClient, opts)` (or a `witslogQueryClientConfig()` factory) that installs global `MutationCache({ onError })` and `QueryCache({ onError })` handlers. Each handler auto-captures the query key / mutation key, the **variables (request payload)**, and the error (and response via `meta`), then ships through the browser reporter to `witslogBrowserIngest`. This is the "TanStack Devtools-like, but persisted to witslog" layer the user asked for — installed once in `providers.tsx`, capturing every query/mutation failure with zero per-hook code. Honor the existing ingest trust boundary (Origin allowlist, `tags:['browser']` advisory, clamped fields) from the P10 gotchas.

### Piece 4 — `exception()` cause chain — EDIT `bindings/node/index.js`
Walk `err.cause` recursively; append `Caused by: <name>: <message>` lines to `stacktrace`; set `root_cause` to the deepest `.code`/name. Benefits every wrapper above and every existing caller for free. Add a regression test (this function has no covering tests today).

### Piece 5 — CLI render + `--json` — EDIT `crates/witslog-cli/src/main.rs`
- `get <id>`: print `context, tags, exception, stacktrace, error_code, correlation_id, environment, version` when present.
- `query`: add the `--json` global flag (already the known open P4 gap) emitting full event JSON; and/or a `--verbose` line that appends `error_code`/tags/first context keys.

Config nicety (optional): allow a default `application` in `witslog.init(config)` so wrappers don't repeat it.

---

## Part D — Files to change / add

witslog repo (`C:\projects\witslog`):
- **New** `bindings/node/fetch.js` — `witslogFetch` (explicit wrapper only).
- **New** `bindings/node/frameworks/next.js` — `register`, `onRequestError`, `withWitslog`.
- **New** `bindings/node/frameworks/react-query.js` — `attachWitslog` / cache config.
- **Edit** `bindings/node/index.js` — `exception()` cause chain; export new subpaths; optional default `application`.
- **Edit** `bindings/browser/witslog-browser.js` — accept structured events from the RQ adapter (payload/response fields).
- **Edit** `bindings/node/package.json` — `files`/`exports` for the new subpaths.
- **Edit** `crates/witslog-cli/src/main.rs` — `get`/`query` render + `--json`.
- **Docs/tests** — `bindings/CONTRACT.md` (new adapters + fetch/RQ event shapes), `CHANGELOG.md` + `bindings/node/CHANGELOG.md`, `node --test` suites for each new module, and a gate in `bindings/e2e/run.ps1`.

WitsNote repo (`C:\projects\WitsNote`) — reference adoption (see Part E).

Follow the established adapter conventions verbatim: framework adapter = hook the global error signal + attach request context, exactly like `frameworks/express.js` (`witslogErrorHandler`) and `frameworks/flask.py` (`got_request_exception`).

---

## Part E — WitsNote adoption (proves boilerplate removal)

Before → after:
- **Add** `client/instrumentation.ts` (3 lines): `export { register, onRequestError } from '@all-wits/witslog/next'` + a `register('witsnote-proxy')` call. Captures every uncaught server error.
- **Swap** raw `fetch` → `witslogFetch` at the api-client request helper (`client/lib/api/*`) and in `client/app/api/proxy/[...path]/route.ts`. Then **delete** the hand-written `ensureWitslog()` / `try-catch` / `witslog.exception` / per-status `witslog.error` from `route.ts` — the wrapper now captures upstream failures, cause chains, latency, correlation id, and 409 bodies automatically.
- **Wire client capture** in `client/app/providers.tsx`: `attachWitslog(queryClient, { application: 'witsnote-client' })` + mount the browser reporter; add a `client/app/api/__witslog/route.ts` that re-exports `witslogBrowserIngest({ allowedOrigins: […] })`.
- **Fix** the latent `hooks.ts:160` `err.response?.status` → `err.status` bug (surfaced in Part A).

Net: route handlers and hooks carry **no** logging code; capture is richer than the current hand-rolled version.

Improved output for the two reported errors (now diagnosable from the log alone):
```jsonc
// was: [witsnote-proxy] Error :: upstream 409 for /cards/…
{"severity":"warn","message":"PUT /cards/019f8786-… → 409 conflict","error_code":"UPSTREAM_CONFLICT",
 "correlation_id":"a1b2…","tags":["proxy","upstream-4xx","retryable"],
 "context":{"http":{"method":"PUT","path":"/cards/019f8786-…","status":409},"timing":{"latency_ms":38},
   "upstream":{"error_code":"CARD_VERSION_CONFLICT","error_message":"card modified by another client",
     "details":{"expected_version":7,"actual_version":8}}}}

// was: [witsnote-proxy] Error :: fetch failed
{"severity":"error","exception":"TypeError","message":"POST /cards → upstream unreachable (ECONNREFUSED)",
 "error_code":"UPSTREAM_UNREACHABLE","root_cause":"ECONNREFUSED","correlation_id":"c3d4…",
 "tags":["proxy","upstream-unreachable","retryable"],
 "context":{"http":{"method":"POST","path":"/cards","upstream_url":"http://localhost:8000/api/cards"},
   "timing":{"latency_ms":5},"cause":{"code":"ECONNREFUSED","address":"127.0.0.1","port":8000}}}
```

---

## Part F — Scope decision (preserved from the earlier AskUserQuestion)

The earlier question asked whether to target (1) all three layers, (2) WitsNote app only, or (3) report only; it was dismissed. The user's follow-up redirected the objective to **improving witslog's architecture to remove logging boilerplate**. Decisions confirmed at approval:

- **Deliverable:** witslog framework changes (Part C/D) **and** WitsNote adoption (Part E), together. Framework lands first within the same effort, then WitsNote migrates onto it (requires republishing `@all-wits/witslog` before adoption).
- **Instrumented fetch:** **explicit `witslogFetch` wrapper** only — no global monkeypatch.
- **Language coverage:** **Node + browser only** for now (the proxy + client are where the errors occur). Python/PHP already have their own framework adapters; extend the fetch/RQ patterns to them later if a need appears.
- **Severity:** expected 4xx → `warn`; 5xx / unreachable → `error`.

---

## Verification

- Unit: `node --test` for `fetch.js` (cause unwrap, non-2xx body clamp, correlation id), `frameworks/next.js` (`onRequestError` mapping), `frameworks/react-query.js` (cache handler payload capture); Rust `cargo test -p witslog-cli` for the render + `--json`.
- Regression: `exception()` cause-chain test in `bindings/node`.
- e2e: extend `bindings/e2e/run.ps1` — SDK→CLI readback proving `context`/`error_code`/`correlation_id` survive the ABI and render via `witslog get`/`query --json`.
- End-to-end on WitsNote: stop the Laravel upstream, POST a card → expect one `UPSTREAM_UNREACHABLE` event with `root_cause:"ECONNREFUSED"`; trigger a stale-version update → expect one `warn`/`UPSTREAM_CONFLICT` event with the server conflict body; confirm both readable via `witslog query "*" --json` with full context, and that `route.ts` contains no logging code.
- Use the project `verify` skill recipe for the CLI/FFI/Node-SDK smoke path.

---

## Prioritized action plan (highest impact first)

1. **Piece 4 — `exception()` cause chain** (`bindings/node/index.js`). One file, unblocks every wrapper, fixes all "fetch failed".
2. **Piece 1 — instrumented fetch** (`bindings/node/fetch.js`). Kills the proxy's per-call logging; makes both reported errors diagnosable.
3. **Piece 5 — CLI render + `--json`**. Makes captured context visible; cheap.
4. **Piece 2 — Next.js adapter** (`frameworks/next.js` + `instrumentation.ts`). Removes per-route try/catch for uncaught errors.
5. **Piece 3 — React Query client capture** + browser reporter wiring. Closes the client-side blind spot.
6. **WitsNote adoption (Part E)** + fix the `hooks.ts:160` status bug. Delete the now-dead manual logging; validate end-to-end.

---

**STATUS: Parts A–G below are DONE and committed** (witslog `c3a206a`/`eee9ca2` on `main`; WitsNote `26d173f`/`583cc2f` on `feat/canvas-notes`). Two bugs found during live verification and fixed: `witslogNextIngest`'s ingest route can't be named `__witslog` (Next.js excludes `_`-prefixed folders from routing — renamed to `witslog-ingest`), and `SearchEngine::search` (`witslog-query`) hydrated rows via hardcoded column indices that silently swapped `context`↔`tags`↔`metadata` once `resolved_at` (added by a later `ALTER TABLE ADD COLUMN`, always appended at the physical end) shifted everything — fixed by sharing `witslog_store::hydrate_event_row` (explicit column list) between `get` and `query`, with a regression test.

---

## Part G — Client-observability gap #2: correlation + network-tab-equivalent capture

### Context

Live verification of Parts A–F surfaced two further gaps, confirmed via code (not assumed):

1. **`witsnote-client` events are structurally poorer than `witsnote-proxy` events.** `witslogFetch` (proxy, server-side) always sets `correlation_id`, `context.timing.latency_ms`, `context.http{...}`, `context.upstream{...}`. `attachWitslog`/`buildEvent` (`bindings/node/frameworks/react-query.js`) never sets `correlation_id`, never computes latency, and only forwards `{mutationKey|queryKey, variables, http_status}`. There is currently **no way to correlate** a client mutation failure with the `witsnote-proxy` log line for the exact same HTTP request — no shared id. Separately, because `witslogNextIngest` runs `witslog.log()` in the same Next.js server process as the proxy, witslog-core's automatic enrichment (`crates/witslog-core/src/enrich.rs`) stamps `pid`/`cwd`/`argv` — describing the **server** — onto every browser-originated event too. (`hostname`/`git_commit` are still legitimately useful — "which build/machine ingested this".)

2. **No network-tab-equivalent capture** (the user's original ask). Real transport-layer failures outside React Query's mutation/query lifecycle are invisible today:
   - `client/lib/collab/useBoardDoc.ts:51-63` — the Hocuspocus (Y.js) WebSocket provider's `onStatus`/`onAuthenticationFailed` only call `setStatus(...)`, log nothing. **No `onClose`/`onDisconnect` handler exists at all** — confirmed the real hooks exist (`@hocuspocus/provider`'s `index.d.ts`: `onClose({event: CloseEvent})`, `onDisconnect({event: CloseEvent})`, standard WebSocket `CloseEvent` with `.code`/`.reason`/`.wasClean`) but are simply unused. This is the exact gap behind the `ws://localhost:1234` connection failure the user saw with nothing logged.
   - `client/lib/api/boards.ts:86-123` `uploadAttachment()` — raw XHR (needed for upload progress) with `onerror`/`ontimeout`/`onabort`/a non-2xx branch, all of which only reject a Promise; `client/lib/api/uploadCardAttachment.ts:19-51` catches and does `console.error` only.
   - The shared `apiClient` axios instance (`client/lib/api/client.ts`) normalizes every error into `ApiError` but never logs at the interceptor level — a stray non-React-Query axios call (`client/lib/collab/ticket.ts`'s `fetchCollabTicket`) is invisible to witslog even though it already gets normalized.

**Decisions confirmed by the user:**
- **No new `witsnote-network`/`witsnote-server` application.** Fold network-layer failures into the existing `witsnote-client` application via tags (`tags:['network','websocket'|'xhr'|'upload']`), matching how `witsnote-proxy` already differentiates `upstream-4xx`/`upstream-5xx`/`upstream-unreachable` via tags, not separate application names. Keeps `witslog stats --application witsnote-client` / MCP `top_failures`/`statistics` complete without composing multiple applications.
- **Full scope** — all pieces below land in one pass, not a reduced subset.

### Design

**1. Correlation ID propagation, browser → proxy → upstream.**
New SDK export **`witslogAxiosInterceptor(axiosInstance, opts)`** (`bindings/node/frameworks/axios.js`, mirrors `witslogFetch`'s shape and sits alongside `frameworks/react-query.js` — both are browser-safe "framework adapters", not Node-only code, matching the existing placement convention). Request interceptor mints a correlation id (`crypto.randomUUID()` — available in both Node ≥19 and browsers) if none is set, attaches it as a header (default `x-request-id`, configurable like `witslogFetch`'s `correlationHeader`), and stashes `{correlationId, startedAt}` on `config` for the response leg to read back. Response/error interceptor attaches `correlationId`/`latencyMs` onto the **rejected error object** — it does **not** call `witslog.log` itself for every error (would double-log every React-Query-managed failure, which `attachWitslog` already captures); it only directly captures calls that bypass React Query entirely (opt-in per-request flag, used for `ticket.ts`).

Verified: **no `route.ts` change needed** — `witslogFetch` already reads an inbound `x-request-id` off `init.headers` as a correlation-id fallback, and `route.ts`'s `forwardHeaders = new Headers(request.headers)` already carries through whatever header the browser sent. Once the axios interceptor sets that header client-side, propagation to the proxy's `witslogFetch` call works with zero proxy-side edit.

`ApiError` (`client/lib/api/errors.ts`) gains two optional fields: `correlationId?: string`, `latencyMs?: number`, populated by `client/lib/api/client.ts`'s existing response interceptor from the raw axios error. `react-query.js`'s `buildEvent` reads `error.correlationId`/`error.latencyMs` (duck-typed, same pattern as its existing `error.status`/`error.code` reads) into `correlation_id`/`context.timing.latency_ms`.

**2. Client-side latency capture.** TanStack Query v5 mutation/query `state` exposes `submittedAt`/`errorUpdatedAt`. In `attachWitslog`'s two cache-subscribe callbacks (`frameworks/react-query.js`), compute `latency_ms = state.errorUpdatedAt - state.submittedAt` into `context.timing = {latency_ms}` (mirrors `witslogFetch`'s shape). Self-contained SDK edit — always-on for React-Query-managed failures, no new opts. (Axios-layer `latencyMs` from Piece 1 covers the same field for non-React-Query calls.)

**3. Stop attaching server-process enrichment noise to browser events.** Config-only fix, not a Rust/core change (no per-call enrich override exists today — adding one is CONTRACT.md-level surgery, out of scope for this pass). `client/instrumentation.ts`'s `register('witsnote-proxy', {...})` call gains `enrich: {pid: false, cwd: false, argv: false}` — keeps `hostname`/`git_commit` (both useful), drops the three fields that are either constant-per-process-and-thus-zero-signal (pid/cwd/argv for a long-lived Next.js server) or actively wrong for a browser-originated event.

**4. Network-tab-equivalent capture — three transports:**
- **(a) axios** — same file/piece as #1. `providers.tsx` wires `witslogAxiosInterceptor(apiClient, {report: reporter, tags: ['witsnote']})`, reusing the reporter already created there. `client/lib/collab/ticket.ts`'s `fetchCollabTicket` call is marked for direct capture (its own call site, not React-Query-managed) tagged `['witsnote', 'network']`.
- **(b) XHR upload** — direct WitsNote wiring in `client/lib/api/boards.ts`'s `uploadAttachment()`, not a new SDK helper (single call site today — YAGNI; promote to an SDK export if a second XHR site appears). Each of `onerror`/`ontimeout`/`onabort`/the non-2xx branch also calls `reporter.enqueue(...)` directly: `error_code` per kind (`XHR_NETWORK_ERROR`/`XHR_TIMEOUT`/`XHR_ABORTED`/`HTTP_${xhr.status}`), `tags: ['witsnote', 'network', 'xhr', 'upload']`.
- **(c) WebSocket** — new SDK export **`witslogWebSocketWatch(opts)`** (`bindings/browser/witslog-websocket.js`, browser-only, alongside `witslog-browser.js`). Returns `{onClose, onDisconnect}` handler functions shaped to drop directly into `HocuspocusProvider`'s constructor options (verified real shape: `onClose({event: CloseEvent})`/`onDisconnect({event: CloseEvent})`) — logs "abnormal" closes (`event.code !== 1000 && event.code !== 1001`) with `context.ws = {code, reason, wasClean}`, `error_code: WS_CLOSE_${event.code}`, `tags: ['network', 'websocket', ...opts.tags]`. `useBoardDoc.ts` adds the currently-missing `onClose` handler and wires both it and `onAuthenticationFailed` (today silently just `setStatus("error")`) through the watch helper's log call, `context.board = {boardId}`.

### Files to change

witslog repo:
- **New** `bindings/node/frameworks/axios.js` — `witslogAxiosInterceptor`.
- **New** `bindings/browser/witslog-websocket.js` — `witslogWebSocketWatch`.
- **Edit** `bindings/node/frameworks/react-query.js` — latency in `buildEvent`; read `error.correlationId`/`error.latencyMs`.
- **Edit** `bindings/CONTRACT.md` — document the new `error.correlationId`/`error.latencyMs` convention and the two new adapters.
- Tests: `bindings/node/test/axios.test.js`, unit tests for the WebSocket watch helper (fake `CloseEvent`), extended `react_query_adapter.test.js` for latency.
- No `crates/witslog-core` changes.

WitsNote:
- `client/instrumentation.ts` — `enrich: {pid:false, cwd:false, argv:false}`.
- `client/lib/api/errors.ts` — `ApiError` gains `correlationId?`/`latencyMs?`.
- `client/lib/api/client.ts` — copy those fields from the raw axios error onto `ApiError`.
- `client/app/providers.tsx` — wire `witslogAxiosInterceptor` onto `apiClient`.
- `client/lib/collab/ticket.ts` — mark for direct capture.
- `client/lib/api/boards.ts` — XHR failure logging in `uploadAttachment`.
- `client/lib/collab/useBoardDoc.ts` — add `onClose`, wire `witslogWebSocketWatch`.
- Out of scope: `client/app/sw.ts` (no current logging, not one of the two confirmed gaps).

### Verification

- Node unit tests for the new axios interceptor (request id minted/reused, latency computed, no double-log for React-Query-managed calls) and WebSocket watch helper (abnormal vs normal close, `event.code`/`reason`/`wasClean` mapped correctly).
- `react_query_adapter.test.js` extended for latency + `correlationId`/`latencyMs` passthrough.
- End-to-end on WitsNote: force a card conflict → `witsnote-client` and `witsnote-proxy` events for the same request now share one `correlation_id`, both readable via `witslog get <id> --json`. Kill the collab server → `useBoardDoc`'s new `onClose` produces a `witsnote-client`/`tags:['network','websocket']` event with the real close code, instead of nothing. `tsc --noEmit` clean on touched files; `pnpm test` (vitest) green.

### Prioritized rollout

1. **§3 enrich-noise fix** — one-line config change, zero new code paths, cleans every future event immediately.
2. **§2 client-side latency** — self-contained SDK edit.
3. **§1 correlation id** (axios interceptor + `ApiError`/`client.ts`/`providers.tsx` wiring) — highest end-to-end diagnostic value, the literal "can't correlate client failure with proxy log" gap.
4. **§4c WebSocket watch** — directly answers the screenshot that prompted this; needs the now-verified Hocuspocus hook shapes.
5. **§4a axios direct-capture for `ticket.ts`** — same file as #3, do alongside it.
6. **§4b XHR upload logging** — single call site, lowest urgency (failures already surface to users via card UI state; gap is operator-visibility only).

---

**STATUS: Part G is DONE, uncommitted.** witslog: `frameworks/axios.js` (`witslogAxiosInterceptor`),
`bindings/browser/witslog-websocket.js` (`witslogWebSocketWatch`), `frameworks/react-query.js`
latency + `correlation_id` passthrough, `error_code`/`correlation_id`/`tags` forwarded end-to-end
through `witslog-browser.js`/`ingest-core.js` (previously silently dropped), CONTRACT.md +
CHANGELOGs updated, node-sdk bumped to 0.6.0. Tests: `test/axios.test.js`,
`bindings/browser/test/witslog-websocket.test.js`, extended `react_query_adapter.test.js` +
`witslog-browser.test.js` — 95+16+15 node tests green, full `bindings/e2e/run.ps1` (workspace
cargo test + 9 gates) green.

WitsNote: `instrumentation.ts` (enrich pid/cwd/argv off), `errors.ts`/`client.ts` (ApiError gains
`correlationId`/`latencyMs`, axios interceptor wired directly in `client.ts` — NOT `providers.tsx`
as originally sketched, since axios runs response interceptors in registration order and the
interceptor must see the raw error before the existing ApiError-normalizing interceptor replaces
it), `ticket.ts` (`witslogDirectCapture: true`), `boards.ts` (XHR upload failure logging, wired
directly per YAGNI), `useBoardDoc.ts` (`onClose`/`onDisconnect` via `witslogWebSocketWatch`, new
`lib/witslog-websocket.ts` TS port), `lib/witslog-browser.ts` (added `correlation_id` field, was
missing even though the upstream `.js` already had it).

**Known blocker, expected per Part F**: WitsNote's `@all-wits/witslog` npm dependency is pinned to
the published `0.5.0`, which predates all of Part G — `lib/api/client.ts`'s
`@all-wits/witslog/frameworks/axios` import fails `tsc` until the SDK is republished (0.6.0) and
WitsNote's dependency is bumped + reinstalled. Every other touched WitsNote file type-checks clean
(verified: `npx tsc --noEmit` shows only this one new error, no others, against a baseline of
pre-existing unrelated errors in `features/editor`/`features/notebook`/etc.).