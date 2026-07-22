# Changelog (`@all-wits/witslog`, Node SDK)

Node-SDK-specific history only — extracted from the project-wide
[`../../CHANGELOG.md`](../../CHANGELOG.md), which also covers the Rust crates/CLI/MCP server on
their own independent version numbers. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this package versions independently of
the Rust workspace (pre-1.0).

## [Unreleased]

### Added

- **`@all-wits/witslog/browser` subpath — `browser.js`/`browser.d.ts`.**
  Packaged copy of `bindings/browser/witslog-browser.js` (previously only a
  vendored/`<script src>` file, not importable from npm — package.json's
  `files` array can't reference paths outside `bindings/node/`). Import via
  `import WitslogBrowser from '@all-wits/witslog/browser'`. Kept
  byte-identical to the canonical file (regression-locked by
  `test/browser_subpath.test.js`, which fails the build if the two drift).
  New `captureConsole: true` config option (default `false`) additionally
  captures `console.error`/`console.warn` calls and capture-phase
  resource-load failures — see the canonical file's changelog entry in the
  root `CHANGELOG.md` for full behavior. Tests:
  `bindings/browser/test/witslog-browser.test.js`,
  `test/browser_subpath.test.js`.
- **Bundled CLI/native binary gains MCP `get_event` (full event payload)
  and colorized `get`/`query` output.** Rust-side changes — a new read-only
  MCP tool that returns the complete event (stacktrace/exception/context/
  tags/metadata, previously only reachable via CLI `get --json`) and a new
  CLI `--color <auto|always|never>` flag — ship to npm consumers via the
  version bump + the binary `_bin/<platform>/witslog` this package bundles;
  no SDK JS/TS API changed. See the root `CHANGELOG.md` for details.
- **Instrumented fetch — `fetch.js`, `witslogFetch(input, init, opts)`.** Explicit wrapper
  around `fetch` (no global monkeypatch — stays safe alongside Next.js's own fetch
  caching/instrumentation). Automatically mints/propagates a correlation id
  (`x-request-id` by default), times the call, and on failure captures via the same
  `exception()`/cause-chain path below; on a non-2xx response it peeks the body via
  `.clone()` (caller still gets the untouched response), pulls `error_code`/`message`/
  `details` out of the `{error:{code,message,details}}` contract shape when present, and
  logs at `warn` for 4xx / `error` for 5xx. Replaces the hand-written try/catch +
  `witslog.exception`/`witslog.error` boilerplate a Next.js API route (or any outbound-fetch
  call site) previously needed per call. Tests: `test/fetch.test.js`.
- **Next.js adapter — `frameworks/next.js`.** `register(application, config)` mounts once;
  `onRequestError(err, request, context)` is Next.js 15's official server-error hook —
  re-export it from `instrumentation.ts` and every uncaught route/Server-Component/
  Server-Action/middleware error is captured (method/path + Next's router context) with zero
  per-route code; `withWitslog(handler)` wraps a single handler explicitly for Next < 15.
  Mirrors the existing `frameworks/express.js`/`flask.py` adapter convention. Tests:
  `test/next_adapter.test.js`.
- **React Query client capture — `frameworks/react-query.js`, `attachWitslog(queryClient, opts)`.**
  Subscribes to a TanStack `QueryClient`'s `MutationCache`/`QueryCache` (the same public event
  stream TanStack Query Devtools itself observes), so every failed query/mutation — key,
  variables, error — is captured automatically. Browser-safe: no Node built-ins, no FFI, no
  hard `@tanstack/react-query` dependency (duck-typed against `.getMutationCache()`/
  `.getQueryCache()`). Hands events to a `report` sink, typically the object returned by
  `WitslogBrowser.init(...)` (`bindings/browser/witslog-browser.js`). Closes the gap where
  client-side query/mutation failures were captured nowhere. Tests:
  `test/react_query_adapter.test.js`.
- **`witslogNextIngest(options)` — `frameworks/next.js`.** A Next.js Route Handler-shaped
  ingest endpoint for the browser reporter / React Query adapter's traffic — a genuine second
  entry point alongside `witslogBrowserIngest` (Express), not a re-export, since Express's raw
  req/res and Next's Web Request/Response are different shapes. Both now call the same
  guardrail/persist logic factored into new `lib/ingest-core.js`
  (`checkIngestGuardrails`/`persistIngestBatch`), so the security guardrails can't drift
  between transports. Tests: `test/next_ingest.test.js`.
- **`witslogBrowserIngest` forwards a full clamped `context` object** (plus `error_code` and a
  bounded set of extra `tags`), not just `context.url` as before — via new shared
  `lib/clamp.js::clampContext` (bounded key count/nesting depth/string length/array length/
  total size, collapsing to `{"_truncated":true}` if still too large). This is what lets the
  React Query adapter's captured mutation variables/response actually reach storage instead of
  being silently dropped at the ingest boundary. `"browser"` remains the first, unremovable tag.
  Tests: `test/clamp.test.js`, new cases in `test/express_ingest.test.js`.

- **Axios interceptor — `frameworks/axios.js`, `witslogAxiosInterceptor(axiosInstance, opts)`.**
  Mints/reuses a correlation id per request (propagated as a header, default
  `x-request-id`, same convention as `fetch.js`) and stamps `correlationId`/`latencyMs`
  onto the response/rejected-error object; does not log every rejection itself (avoids
  double-logging what `attachWitslog` already captures) — direct capture is opt-in via
  `config.witslogDirectCapture = true`. `frameworks/react-query.js`'s `buildEvent` now
  reads `error.correlationId`/`error.latencyMs` into `correlation_id`/
  `context.timing.latency_ms`, and independently computes latency from TanStack Query
  v5's `state.submittedAt`/`state.errorUpdatedAt` when those aren't present. This is what
  lets a `witsnote-client` event and the `witsnote-proxy` event for the same request share
  one `correlation_id`. Tests: `test/axios.test.js`, new cases in
  `test/react_query_adapter.test.js`.
- **WebSocket close/disconnect capture — `bindings/browser/witslog-websocket.js`,
  `witslogWebSocketWatch(opts)`.** Browser-only. Returns `{onClose, onDisconnect}`
  handlers shaped for `HocuspocusProvider`'s constructor options (or any
  `{event: CloseEvent}`-shaped hook); logs abnormal closes (`code` not 1000/1001) with
  `error_code: WS_CLOSE_<code>` and `context.ws: {code, reason, wasClean}`. Closes the gap
  where a collab WebSocket disconnecting produced no log at all. Tests:
  `bindings/browser/test/witslog-websocket.test.js`.
- **`buildBatch`/`makeErrorEvent` (`witslog-browser.js`) and `persistIngestBatch`
  (`lib/ingest-core.js`) now forward `error_code`/`correlation_id`/`tags`** from a
  captured browser event end-to-end — previously only `message`/`severity`/`exception`/
  `stacktrace`/`context` survived the hop, silently dropping the correlation id the React
  Query/WebSocket adapters now always set.

### Fixed

- **`exception()` dropped the JS `Error.cause` chain.** Node's own `fetch` (undici) throws
  `TypeError: fetch failed` whose real reason (`ECONNREFUSED`/`ETIMEDOUT`/`ENOTFOUND`/etc)
  lives on `.cause`, not the top-level error — every wrapped fetch failure logged as an
  undiagnosable bare "fetch failed". `exception()` now walks the `.cause` chain (depth-capped),
  appends `Caused by: ...` lines to `stacktrace`, and folds the deepest cause's code/name into
  `context.root_cause` (not a top-level field — `root_cause` is Rust-only, not part of the
  `witslog_log` payload contract; see `bindings/CONTRACT.md`). Regression lock:
  `test/exception_cause_chain.test.js`.

## [0.4.1] — 2026-07-21

### Fixed

- **Undocumented Next.js bundling break**: `witslog.init()` inside a Next.js Route
  Handler/Server Action threw `Cannot find the native Koffi module; did you bundle it
  correctly?` — Next bundles server route code by default, and `koffi`'s own native `.node`
  module resolution (internal to the `koffi` dependency, not this package's own `_libs/`
  locator) is bundler-incompatible. Fix is config, not code: add
  `serverExternalPackages: ["@all-wits/witslog", "koffi"]` to `next.config.ts` — see the new
  Next.js subsection in [README.md](README.md#-works-with-your-nodejs-stack) and
  [bindings/CONTRACT.md](https://github.com/all-wits/witslog/blob/main/bindings/CONTRACT.md).
  Regression lock: `test/bundler_koffi.test.js` (webpack bundle of `require('koffi')`, with
  and without externalizing `koffi`).

## [0.4.0] — 2026-07-18

### Added

- **Bundles the real `witslog` CLI binary per platform**, closing the remaining
  npm-install-only gap: `createProject: true` (0.3.0) fixed `init`, but `query`/`stats`/
  `export`/`serve-mcp`/`doctor` have no FFI surface at all (by design — see
  [`../CONTRACT.md`](../CONTRACT.md)), so they were unreachable without a separate CLI
  install. `bin/witslog.js` is a thin `spawnSync` shim resolving the binary via
  `lib/cli-locator.js` — `WITSLOG_CLI` env override → bundled `_bin/<platform>/witslog{,.exe}`
  → bare `witslog` on `PATH` (mirrors the existing `_libs/`/`WITSLOG_LIB` native-lib locator
  convention). Wired into `package.json`'s `bin` field, so `npx witslog query ...` and a
  global install both work post-`npm install`, on Windows x64, Linux x64/arm64, and macOS
  Apple Silicon (`darwin-x64` stays unbundled — see Platform support in the README).
  Regression lock: `test/cli_locator.test.js`, `test/bin_shim.test.js`.

> **⚠️ For MCP (AI-assistant) registration specifically**, install the CLI globally instead
> (curl/irm, Homebrew, Scoop, `cargo install`) rather than relying on this bundled binary — see
> the [README](README.md#-quick-start) and [root README's MCP section](../../README.md#-integration-with-ai-mcp)
> for why (macOS Intel has no bundled CLI at all; a config path inside this project's
> `node_modules/` isn't stable across reinstalls).

## [0.3.0] — 2026-07-17

### Added

- `init({ createProject: true })` / `init({ createProject: '/path' })`: scaffolds a
  `.witslog/` project directory (dir + DB + migrate) via the new native
  `witslog_bootstrap_project` export before mounting. Closes the gap where `npm install`
  bundled the native lib but shipped no CLI, so a project that never separately installed
  and ran `witslog init` had no way to create `.witslog/` — every `log()`/`error()`/`info()`
  call failed with `rc=-1`. See [`../CONTRACT.md`](../CONTRACT.md) and [README](README.md).

## [0.2.1] — 2026-07-17

Docs-only follow-up to 0.2.0 — no code changes. 0.2.0 published successfully once
`release-node-sdk.yml` was fixed to use npm Trusted Publishing (OIDC) instead of an
automation token, but that publish ran off a commit predating the README updates
documenting `witslogBrowserIngest` and the P10 CLI surface — so the README shown on the npm
package page was stale. npm versions are immutable, so a docs-only change still needs its
own version bump to actually reach the published listing.

### Changed

- README: document P10 (MTTR/resolution tracking, notifiers, browser-side error capture) —
  feature list, MCP tool count (12 → 13, `mttr` added), CLI examples, "Browser-side error
  capture" section including `witslogBrowserIngest` fail-closed defaults.

## [0.2.0] — 2026-07-17

### Added

- `witslogBrowserIngest` in `frameworks/express.js` (P10): Express handler accepting batches
  from [`bindings/browser/witslog-browser.js`](../browser). New export; existing
  `witslogErrorHandler` unchanged.

### Fixed

- The bundled native lib's `witslog_resolve` now guards `resolved_at IS NULL` (first
  resolution wins) and returns `-1` on an unknown or already-resolved event id, instead of
  silently reporting success. No JS-facing API change, but the bundled binary behaves
  differently — republishing is what actually ships this fix, since it lives in
  `_libs/<platform>/`, not JS source.

## [0.1.0] — 2026-07-16

### Added

- Initial release: framework-agnostic core (`log`/`error`/`warn`/`info`/`exception`,
  `init`/`flush`/`shutdown`, `installUncaughtHandler`) over the native `witslog-ffi` C ABI via
  [`koffi`](https://koffi.dev) — prebuilt, no native build step. Express adapter
  (`witslogErrorHandler`). `witslog_abi_version()` handshake, `WITSLOG_LIB` locator with
  bundled `_libs/<platform>/` native libs for Windows x64, Linux x64/arm64, macOS arm64
  (Apple Silicon).
