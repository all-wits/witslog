# witslog SDK ↔ native ABI contract

All language SDKs (Python, Node, PHP) are thin wrappers over the same C ABI exported by the
`witslog-ffi` crate (`witslog_ffi.dll` / `libwitslog_ffi.so` / `libwitslog_ffi.dylib`). This
file is the single source of truth every SDK core marshals against.

## Contract version

**Current version: `1`.**

The native library exports `int32 witslog_abi_version(void)`. Every SDK core calls it once at
load time and compares against the version it was built for. On mismatch the SDK raises its
`WitslogContractError` (naming expected vs. actual) rather than sending a payload the native
side may mis-parse. Bump `WITSLOG_ABI_VERSION` in `crates/witslog-ffi/src/lib.rs` on any
breaking change to the payloads below.

## Exported functions

| Symbol | Signature (C) | Meaning |
|---|---|---|
| `witslog_abi_version` | `int32 (void)` | Contract version (see above). |
| `witslog_configure` | `int32 (const char* json)` | Set enrich/redact/buffer for this process. `0` ok, `-1` bad JSON, `-2` bad redact regex. |
| `witslog_init` | `int32 (const char* json_or_null)` | Mount the process runtime + Rust panic hook. Applies `witslog_configure` payload first (null = defaults). `0`/`-1`/`-2` as above. |
| `witslog_log` | `int64 (const char* json)` | Log one event (payload below). Returns the DB rowid on the sync path, `0` when buffering is enabled (rowid not yet known), `-1` on error. Never panics. |
| `witslog_resolve` | `int32 (const char* event_id)` | Mark an event resolved. `0`/`-1`. |
| `witslog_bootstrap_project` | `int32 (const char* path_or_null)` | Scaffold a `.witslog/` project dir (create dir, open/create DB, migrate) — mirrors CLI `witslog init`. `path_or_null`: project root, or null for cwd. Idempotent. `0`/`-1`. |
| `witslog_delete` | `char* (const char* filter_json)` | Delete stale/resolved events. Returns a heap JSON string `{"deleted_count":N,"deleted_ids":[...]}` (free via `witslog_free_string`) or null on error. |
| `witslog_flush` | `int32 (void)` | Drain the async buffer (joins the flush thread). Idempotent. Call before exit. |
| `witslog_shutdown` | `int32 (void)` | Un-mount: flush + tear down. Alias of `witslog_flush` today. |
| `witslog_free_string` | `void (char*)` | Free a string returned by `witslog_delete`. |

All string parameters are NUL-terminated UTF-8. The caller owns the input buffers; the library
owns (and frees, via `witslog_free_string`) any `char*` it returns.

**This export list is write-only by design** — `query`/`stats`/`export`/`serve-mcp`/`doctor`
have no C ABI surface; they're only reachable via the CLI or the MCP server
(`crates/witslog-query`/`witslog-mcp`). The MCP server's `get_event` tool (added alongside this
contract version, still `1` — MCP JSON is a separate surface from `witslog_log`/`configure`)
returns the full event payload (stacktrace/exception/context/tags/metadata) by id, mirroring
CLI `get --json`; it does not add or change anything here.

**MCP `initialize` returns worked-example `instructions`, and every tool `description` carries
a worked `Example: {...}` call plus disambiguation against overlapping tools** (also MCP
JSON, not an ABI/contract-version change). Added after real-world use showed a working
connection wasn't enough — a lightweight model (Claude Haiku 4.5) failed to retrieve an error
list because tool descriptions were one-liners with no guidance on which of several
overlapping tools (`search_errors` vs. `latest_errors`, `explain_error` vs. `similar_errors`
vs. `list_traces` vs. `get_event`) to call, or what a legal input looked like. See
`crates/witslog-mcp/src/server.rs::INITIALIZE_INSTRUCTIONS` and
`crates/witslog-mcp/src/tools.rs::builtin_tools`.

## `witslog_log` payload (JSON object)

| Field | Type | Req | Notes |
|---|---|---|---|
| `application` | string | ✅ | app name |
| `message` | string | ✅ | error message (redacted before persist) |
| `severity` | string | | `trace\|debug\|info\|warn\|error\|critical\|fatal` (default `error`) |
| `version` | string | | app version / build id |
| `environment` | string | | `prod\|staging\|dev\|ci` |
| `category` | string | | canonical taxonomy leaf; when set, auto-classify is skipped |
| `error_code` | string | | app-defined stable code |
| `exception` | string | | exception/class type name |
| `stacktrace` | string | | raw trace; normalized into `stack_norm` |
| `correlation_id` | string | | request/trace id |
| `parent_event_id` | string | | caused-by parent event id |
| `context` | object | | structured context; redacted; hot keys promoted to columns |
| `tags` | string[] | | free-form tags |
| `metadata` | object | | free-form metadata; redacted |

`context`, `tags`, `metadata` are passed through unchanged (FR-P6-006).

**`root_cause` is not a payload field.** `witslog-core::EventBuilder::root_cause` /
`witslog-core::exception()` populate `Event.root_cause` from Rust's `std::error::Error::source()`
chain, but that field was never wired into the `witslog_log` JSON contract above — no SDK can set
it as a top-level key. SDKs that unwrap a host-language cause chain (e.g. the Node SDK's
`exception()` walking JS's `Error.cause`, for `TypeError: fetch failed` from `fetch`/undici, whose
real reason — `ECONNREFUSED`/`ETIMEDOUT`/`ENOTFOUND`/etc — lives on `.cause`, not the top-level
error) fold the deepest cause's code/name into **`context.root_cause`** instead, and append
`Caused by: ...` lines to `stacktrace`. This needs no ABI change and every language SDK doing the
same unwrap should follow the same `context.root_cause` convention for consistency.

## `witslog_init` / `witslog_configure` payload (JSON object)

```json
{
  "enrich": { "hostname": true, "pid": true, "cwd": true, "argv": true,
              "git_commit": true, "env_allowlist": ["PATH"] },
  "redact": { "custom_patterns": ["MY_TOKEN_[A-Z0-9]+"] },
  "buffer": { "enabled": false, "batch_size": 50, "flush_interval_ms": 1000,
              "queue_capacity": 1024 }
}
```

All keys optional; omitted keys keep their current value.

## `witslog_delete` filter payload (JSON object)

```json
{ "event_id": "...", "fingerprint": "...", "resolved_before": "RFC3339", "force": false }
```

Only deletes events with `resolved_at IS NOT NULL` unless `force:true`.

## `witslog_bootstrap_project` (no JSON — plain path string or null)

None of the write-path exports (`witslog_log`/`witslog_resolve`/`witslog_delete`) create the
parent `.witslog/` directory — `SQLITE_OPEN_CREATE` (used internally) creates the DB *file*
only, not missing parent directories. Historically only the separately-distributed CLI's
`witslog init` created that directory, which left SDKs installed via a package manager alone
(no CLI binary bundled) with no way to bootstrap a project. Call
`witslog_bootstrap_project(path_or_null)` once before the first `witslog_log`/`witslog_init`
call in a fresh project. Safe to call repeatedly (dir creation and the underlying
`Store::open_or_create` migrate step are both idempotent).

Currently only the [Node SDK](https://github.com/all-wits/witslog/blob/main/bindings/node) wires this into a convenience API
(`init({ createProject: true })` / `{ createProject: '/path' }`). Python/PHP can call the
native symbol directly through their FFI/ctypes/ext-ffi bridge but don't expose a wrapper
yet — see each SDK's README.

## Native library location (locator, identical in every SDK)

Resolved in order:

1. `WITSLOG_LIB` environment variable — explicit path to the shared library (used in dev/CI;
   point it at `target/release/witslog_ffi.dll`).
2. Package-bundled `_libs/<platform>/witslog_ffi.{dll,so,dylib}`, where `<platform>` is e.g.
   `win32-x64`, `linux-x64`, `darwin-arm64`.
3. The OS default loader search path.

On failure the SDK raises `WitslogLibraryError` listing the paths it tried.

**Bundler note (Node SDK only):** the locator above is witslog's own logic and works fine under a
bundler. But `koffi` (the Node SDK's FFI dependency) does its own native `.node` module resolution
internally, via static per-platform `require(...)` calls in its own loader — those are
bundler-incompatible by nature. A consumer using webpack/turbopack/esbuild/Vite SSR to bundle
server-side code (Next.js Route Handlers being the common case) must externalize both packages so
they're `require()`d natively instead of bundled, e.g. in Next.js:
`serverExternalPackages: ["@all-wits/witslog", "koffi"]`. Without it, `koffi`'s loader throws
`"Cannot find the native Koffi module; did you bundle it correctly?"` at load time — this is a
`koffi`-internal failure, not a witslog `_libs/` locator failure, and no witslog code change can
fix it. See [bindings/node/README.md](https://github.com/all-wits/witslog/blob/main/bindings/node/README.md#-works-with-your-nodejs-stack).

## CLI binary location (Node SDK only, `bindings/node/lib/cli-locator.js`)

Same convention as the native lib locator above, parallel `_bin/` tree instead of `_libs/`.
Resolved in order:

1. `WITSLOG_CLI` environment variable — explicit path to the `witslog` binary (used in dev/CI;
   point it at `target/release/witslog(.exe)`).
2. Package-bundled `_bin/<platform>/witslog{,.exe}`, same `<platform>` values as `_libs/`
   (`win32-x64`, `linux-x64`, `linux-arm64`, `darwin-arm64`).
3. The OS `PATH` (bare `witslog`/`witslog.exe`) — a separately-installed CLI still works.

`npx witslog <command>` / a global install both invoke `bin/witslog.js`, a thin `spawnSync`
shim that forwards argv/stdio/exit code to whichever binary the locator resolves. On a
spawn-time `ENOENT` (nothing resolved) it reports `WitslogCliNotFoundError` listing the paths
tried. Python/PHP don't have this — no bundled CLI binary for those SDKs; use a separately
installed `witslog` (see `docs/install.md`) for `query`/`stats`/etc. there.

## DB resolution

The native library resolves the target DB by walking up from the current working directory for a
`.witslog/` marker (same as the CLI). An SDK-hosted app therefore logs into its own project DB
automatically — just run from the project directory, or `witslog init` it first.

## Security note: argv enrichment vs. secrets

Enrichment defaults `argv: true` (see `EnrichConfig::default()` in
`crates/witslog-core/src/enrich.rs`), so the **full process command line** is captured into
`context.argv` on every event by default. Built-in + custom redaction (`redact_json`) recurses
into `argv` and redacts anything matching a known secret *pattern* (Bearer tokens, `api_key=`,
`password=`, `AWS_*`, connection strings) — but a secret passed as a **bare CLI argument** that
doesn't match one of those shapes (e.g. `myapp --token abc123secret`) is not pattern-matched and
will be persisted verbatim.

If your app may receive secrets via CLI arguments, close this exposure explicitly:

```json
{ "enrich": { "argv": false } }
```

passed to `witslog_init`/`witslog_configure` (or the SDK's `init(config)`). This is proven to
fully suppress `argv` capture end-to-end — see `witslog-ffi::configure_argv_false_suppresses_argv_capture`
and the equivalent regression test in each SDK's unit test suite (`test_init_forwards_argv_disable_config`
/ `init forwards argv-disable config unchanged` / `testInitForwardsArgvDisableConfig`). Other
enrichment (`pid`, `cwd`, `git_commit`, `hostname`) is unaffected and can be disabled independently
the same way.

## Browser-side error capture (P10) — ingest recipe for Python/PHP

`bindings/browser/witslog-browser.js` ships client-side JS errors to a server-side ingest
endpoint via `navigator.sendBeacon`/`fetch`. Also published (0.6.1+) as the npm subpath
`@all-wits/witslog/browser` (`bindings/node/browser.js` + `.d.ts`) — a byte-identical packaged
copy for bundler/import usage, regression-locked against the canonical file by
`bindings/node/test/browser_subpath.test.js`; the canonical `bindings/browser/witslog-browser.js`
stays the source of truth for standalone `<script src>` usage. A Node adapter ships
(`witslogBrowserIngest` in `bindings/node/frameworks/express.js`); Python/PHP adapters are
**not** shipped — three parallel handlers accepting untrusted input is three attack
surfaces to keep in sync for a feature whose whole risk lives in that handling. The
guardrails below are not optional extras; skipping any of them turns the endpoint into
an unauthenticated write into the AI's evidence base (this text ends up in
`events.message`, which `search_errors`/`explain_error` return verbatim to an
MCP-connected LLM). Port the Node handler's logic (see its source for the full rationale):

1. **Origin allowlist, fail-closed.** Reject unless the `Origin` header is in an explicit
   list you provide — default to none. This is the actual defense; the request
   genuinely originates from `127.0.0.1` (the attack is a malicious page open in the same
   browser as your dev server), so a loopback check alone does not stop it.
2. **Refuse to arm in production** unless explicitly forced by the caller.
3. **Rate-limit by client** — per-request size caps don't bound request *volume*.
4. **Clamp severity to `error`/`warn`** (never let untrusted input claim
   `fatal`/`critical`) and cap message/stacktrace/batch/body sizes.
5. Map each accepted event through your SDK's normal `log`/`exception` call with
   `tags: ["browser", ...]` — `"browser"` is always first and cannot be removed by the
   client; a bounded number of additional low-cardinality tags (and `error_code`, and a
   generically-clamped `context` object — see `clampContext` below) may pass through.
   None of this is a trust boundary; `classify()` merges suggested tags into whatever is
   already there, and clamping bounds *shape/size*, not the untrustworthiness of the text.

True provenance (an `ingest_source` field trusted by the query layer) isn't in the
payload contract above and would need a `WITSLOG_ABI_VERSION` bump — out of scope until
a real need shows up.

**`captureConsole` (0.6.1+, `init({..., captureConsole: true})`).** Off by default — the
reporter otherwise only captures uncaught throws (`window.onerror`) and unhandled promise
rejections, missing most DevTools "red" lines (`console.error`/`console.warn` calls that never
throw — React caught-error logs, prop/hydration warnings, third-party libs). When enabled:

- Wraps `console.error` (severity `error`) and `console.warn` (severity `warn`), tagging
  every captured event `['console']`. The **original** console method is always invoked first
  — capture never swallows real developer output, including when reporting itself fails.
- A module-level re-entrancy guard prevents a `console.error`/`warn` call triggered *by*
  reporting (e.g. via a `toJSON()` that itself logs, or a failed `fetch` that logs) from
  recursing back into capture — the original console method still runs for that nested call,
  only its own enqueue is skipped. Regression-locked by
  `bindings/browser/test/witslog-browser.test.js`'s re-entrancy test.
- Also adds capture-phase resource-load error capture (tag `['resource']`) for
  `<img>`/`<script>`/`<link>` failures — these fire a non-bubbling `error` event that only a
  capture-phase `window` listener observes; `onError`'s existing bubble-phase listener handles
  ordinary script errors and is unaffected (different handler function — no double-enqueue).
- Same trust posture as the rest of this section applies once captured text reaches the
  ingest endpoint: it is untrusted input landing in `events.message`.

**Context passthrough (`clampContext`, `bindings/node/lib/clamp.js`).** The Node ingest
handler originally forwarded only `context.url`, dropping everything else an event's
`context` might carry — which meant a richer client-side capture layer (see React Query
adapter below) had nowhere to put a mutation's variables/response. `clampContext` now
recursively bounds an arbitrary `context` object (max key count, max nesting depth, max
string length per leaf, max array length, max total serialized size — collapsing to
`{"_truncated":true}` if still too large after clamping) before it is persisted, so a
whole structured `context` survives ingest instead of just one field, without accepting
unbounded/DoS-shaped input. Reused by `fetch.js`'s error-response body snapshot and
`frameworks/react-query.js`'s captured mutation/query context — see below.

## Node SDK auto-instrumentation (fetch / Next.js / React Query adapters)

Three additions (Node-only for now — see PHASES.md / CLAUDE.md "Next Steps") remove the
per-call-site `try/catch` + `witslog.exception`/`witslog.error` boilerplate that a plain
route handler or fetch call previously required:

- **`bindings/node/fetch.js` — `witslogFetch(input, init, opts)`.** An explicit wrapper
  around `fetch` (no global monkeypatch — stays safe alongside Next.js's own fetch
  caching/instrumentation). On a thrown error it logs via `exception()` (cause chain
  included, `error_code: "UPSTREAM_UNREACHABLE"`); on a non-2xx response it peeks the body
  via `.clone()` (caller still gets the original, unconsumed response), extracts
  `error_code`/`message`/`details` from the `{error:{code,message,details}}` contract
  shape when present, and logs at **`warn` for 4xx / `error` for 5xx** (expected
  client-caused conflicts vs. real server failures — keeps fingerprinting/MTTR
  meaningful). Always attaches/propagates a correlation id (`x-request-id` by default)
  and `context.timing.latency_ms`.
- **`bindings/node/frameworks/next.js` — `register`/`onRequestError`/`withWitslog`.**
  Mirrors the `express.js`/`flask.py` adapter convention: hook the framework's own global
  error signal. `onRequestError` re-exported from `instrumentation.ts` is Next.js 15's
  official server-error hook — it fires for uncaught errors in route handlers, Server
  Components, Server Actions, and middleware alike, captured with the request
  method/path and Next's router context (`routePath`/`routeType`/`renderSource`/etc), zero
  per-route code. `withWitslog(handler)` is the fallback for Next < 15 or a single route
  wanting explicit timing without global instrumentation.
- **`bindings/node/frameworks/react-query.js` — `attachWitslog(queryClient, opts)`.**
  Subscribes to a TanStack `QueryClient`'s `MutationCache`/`QueryCache` — the same public
  event stream TanStack Query Devtools itself observes — so every failed
  query/mutation is captured: mutation/query key, variables, and the error, with zero
  per-hook code. Browser-safe (no Node built-ins, no FFI, no hard `@tanstack/react-query`
  dependency — duck-typed against `.getMutationCache()`/`.getQueryCache()`). Hands events
  to a `report` sink — typically the object returned by `WitslogBrowser.init(...)`
  (`bindings/browser/witslog-browser.js`), which ships them to a server-side ingest
  endpoint (so the same guardrails 1–5 apply).
- **`bindings/node/frameworks/next.js` — `witslogNextIngest(options)`.** A Next.js Route
  Handler-shaped ingest endpoint (`(request: Request) => Promise<Response>`), for the
  React Query adapter's traffic (or any `WitslogBrowser.init(...)`-shaped client). This is
  a **separate entry point from `witslogBrowserIngest`**, not a re-export — Express's raw
  `req`/`res` and Next's Web `Request`/`Response` are not interchangeable shapes. Both call
  the same framework-neutral guardrail/persist logic, factored out into
  `bindings/node/lib/ingest-core.js` (`checkIngestGuardrails`, `persistIngestBatch`) so the
  security-relevant checks (guardrails 1–5 above) can't drift between the two transports.

## Correlation + network-tab-equivalent capture (axios / WebSocket adapters)

Two more Node/browser-only additions close the "client failure can't be correlated with
the proxy log for the same request" gap and cover transports outside React Query's
mutation/query lifecycle:

- **`bindings/node/frameworks/axios.js` — `witslogAxiosInterceptor(axiosInstance, opts)`.**
  A request interceptor mints a correlation id (`crypto.randomUUID()`) if the outbound
  request doesn't already carry one, and sets it as a header (default `x-request-id`,
  configurable via `opts.correlationHeader` — same option name as `witslogFetch`). The
  response/error interceptor stamps `correlationId`/`latencyMs` onto the response or
  rejected error object; it does **not** call `witslog.log` for every rejection itself
  (that would double-log every request React Query's `attachWitslog` already captures) —
  direct capture is opt-in per request via `config.witslogDirectCapture = true`, for call
  sites that bypass React Query entirely. `frameworks/react-query.js`'s `buildEvent` reads
  `error.correlationId`/`error.latencyMs` (duck-typed, same pattern as its existing
  `error.status`/`error.code` reads) into `correlation_id`/`context.timing.latency_ms` when
  present — this is what lets a `witsnote-client` mutation-failure event and the
  `witsnote-proxy` event for the same HTTP request share one `correlation_id`. No
  `route.ts`/proxy-side change is needed: `witslogFetch` already reads an inbound
  `x-request-id` as its own correlation-id fallback.
- **`frameworks/react-query.js`'s `buildEvent` also computes client-side latency** from
  TanStack Query v5's `state.submittedAt`/`state.errorUpdatedAt` into
  `context.timing.latency_ms` when the axios-stamped `latencyMs` isn't present — always-on,
  no new opts required.
- **`bindings/browser/witslog-websocket.js` — `witslogWebSocketWatch(opts)`.** Browser-only,
  alongside `witslog-browser.js`. Returns `{onClose, onDisconnect}` handlers shaped to drop
  directly into a WebSocket-wrapping provider's constructor options (verified against
  `HocuspocusProvider`: `onClose({event: CloseEvent})` / `onDisconnect({event: CloseEvent})`,
  a standard `CloseEvent` with `.code`/`.reason`/`.wasClean`). Logs only "abnormal" closes
  (`event.code` not `1000`/`1001`) with `error_code: WS_CLOSE_<code>`,
  `context.ws: {code, reason, wasClean}`, `tags: ['network', 'websocket', ...opts.tags]`.
- **`bindings/node/frameworks/hocuspocus.js` — `attachWitslogHocuspocus(provider, opts)`.**
  Node-side counterpart to `witslog-websocket.js`, purpose-built for a `HocuspocusProvider`
  (or any `EventEmitter`-shaped target exposing `on(event, fn)`/`off(event, fn)`) rather than
  a raw `CloseEvent` hook. Two differences from the vendored watcher above:
  `isAbnormalClose(code, wasClean)` treats any `wasClean:true` close as normal — the vendored
  watcher checks `code` alone, so a clean disconnect that synthesizes code 1005 ("No Status
  Rcvd", which the browser does locally whenever the server closes without an explicit
  status — a routine `provider.destroy()`/tab-nav/HMR reload) is misclassified as abnormal;
  and it additionally captures `authenticationFailed` (`error_code: COLLAB_AUTH_FAILED`). Same
  urgency posture as `witslogWebSocketWatch`: flushes the reporter immediately after every
  `emit()`. Returns a `detach()` cleanup function that removes all three listeners
  (`close`/`disconnect`/`authenticationFailed`).
- **`buildBatch`/`makeErrorEvent` (`bindings/browser/witslog-browser.js`) and
  `persistIngestBatch` (`bindings/node/lib/ingest-core.js`) now forward `error_code` /
  `correlation_id` / `tags`** from a captured browser event through to the ingest payload
  and on into `witslog.log` — previously only `message`/`severity`/`exception`/`stacktrace`/
  `context.url` survived the browser→ingest hop, silently dropping the correlation id a
  richer capture layer (React Query adapter, WebSocket watch) now always sets.

## Node SDK framework-adapter contract (`bindings/node/frameworks/*.js`)

Not an ABI/wire contract — this is the shape every `frameworks/*.js` adapter (`axios.js`,
`express.js`, `react-query.js`, `next.js`, `hocuspocus.js`, …) follows, so a consuming app gets
"import + one function call" instead of hand-rolling capture logic per integration (the
`witslog-websocket.ts` boilerplate in an early WitsNote integration, later replaced by
`hocuspocus.js`, is the cautionary example this contract exists to prevent recurring). A new
adapter for another runtime/framework/infra (Redis pub/sub, BroadcastChannel, socket.io, a queue
consumer, …) MUST follow all five:

1. **Duck-typed against the target's public API — no hard dependency on the target framework
   package.** `react-query.js` only assumes `.getMutationCache()/.getQueryCache().subscribe()`;
   `hocuspocus.js` only assumes `.on(event, fn)/.off(event, fn)` (the public `EventEmitter`
   surface `HocuspocusProvider` happens to expose). Never `require()` the target package itself.
2. **`opts.report` resolved via a local `resolveEmit(report)` helper**: accepts a function
   `(event) => void` or a `{enqueue(event)}`-shaped reporter (e.g. the object
   `bindings/browser/witslog-browser.js`'s `WitslogBrowser.init(...)` returns), throws
   `TypeError` on anything else. If the adapter also needs to flush eagerly (see point 5), add a
   parallel `resolveFlush(report)` that no-ops when `report.flush` isn't present — never require
   a `flush` capability from a bare function-style `report`.
3. **The adapter owns event → witslog-event normalization** (`message`/`severity`/`error_code`/
   `tags`/`context`). Callers pass raw `tags`/`context` to merge in, not a mapping function —
   keeps call sites (e.g. `app/providers.tsx`) to a single `attachWitslog<X>(target, opts)` line.
4. **Returns a `detach()` cleanup function** that undoes everything the adapter attached
   (`interceptors.eject(...)`, cache `unsubscribe()`, `provider.off(...)`, …). Callers are
   expected to invoke it alongside the target's own teardown (e.g. `hp.destroy()`).
5. **Pick a flush strategy deliberately, don't default to "batched."** High-volume sources
   (react-query mutations/queries, axios direct-capture) are fine relying on the reporter's own
   batch window (pagehide/visibilitychange/periodic flush). Low-volume, urgent sources (a
   collab-server WebSocket dying, an auth failure) should flush immediately after every `emit()`
   — see `resolveFlush` in `hocuspocus.js` — so the event is durable within seconds regardless of
   tab visibility, not queued indefinitely.

Every adapter ships a matching `.d.ts` (duck-typed `*Like` interface, not the real framework's
types) and a `test/<name>.test.js` exercising it against a fake/duck-typed target — no real
framework dependency in tests either (see `test/axios.test.js`, `test/hocuspocus.test.js`).

## Mount / flush lifecycle

`tracing` (the Rust ambient capture) does **not** cross the ABI. Each SDK:

1. calls `witslog_init` once at startup (installs the Rust panic hook, applies config), and
2. registers the **host language's** uncaught-exception hook (Python `sys.excepthook`, Node
   `process.on('uncaughtException')`, PHP `set_exception_handler`) to route those to
   `witslog_log`, and
3. calls `witslog_shutdown` before process exit (atexit / shutdown handler), since the C ABI has
   no RAII drop to flush a buffer.
