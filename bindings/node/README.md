<div align="center">

# 🪵 witslog (Node.js SDK)

[![npm](https://img.shields.io/npm/v/%40all-wits%2Fwitslog?label=npm&logo=npm)](https://www.npmjs.com/package/@all-wits/witslog)
[![License: Apache 2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/all-wits/witslog/blob/main/LICENSE)
[![node](https://img.shields.io/node/v/%40all-wits%2Fwitslog?logo=nodedotjs)](https://www.npmjs.com/package/@all-wits/witslog)
[![CI](https://img.shields.io/github/actions/workflow/status/all-wits/witslog/release-node-sdk.yml?logo=githubactions)](https://github.com/all-wits/witslog/actions/workflows/release-node-sdk.yml)

**Framework-agnostic Node.js SDK for [witslog](https://github.com/all-wits/witslog/blob/main/README.md) structured error logging.**

</div>

---

Thin wrapper over the native `witslog-ffi` C ABI via [`koffi`](https://koffi.dev) — one
dependency, **prebuilt, no native build step**. As of 0.4.0 it also bundles the real `witslog`
CLI binary per platform, so `witslog query`/`stats`/`export`/`serve-mcp`/`doctor` etc. (the
read/ops surface that has no FFI equivalent — see CONTRACT.md) work straight after install, no
separate CLI install required. See [CONTRACT.md](https://github.com/all-wits/witslog/blob/main/bindings/CONTRACT.md)
for the full SDK↔native ABI, and [CHANGELOG.md](https://github.com/all-wits/witslog/blob/main/bindings/node/CHANGELOG.md)
for this package's release history.

## 📦 Install

```bash
npm install @all-wits/witslog
```

```bash
pnpm add @all-wits/witslog
```

```bash
bun add @all-wits/witslog
```

Native libraries **and** the `witslog` CLI binary for Windows x64, Linux x64/arm64, and macOS
(Apple Silicon) are bundled — `npm install` / `pnpm add` / `bun add` alone is enough on those
platforms, for both the SDK and `npx witslog <command>` / a global-install `witslog` on your
PATH. See [Platform support](#-platform-support) below for the current gap.

## 🚀 Quick Start

```js
const witslog = require('@all-wits/witslog');

witslog.init({ createProject: true }); // scaffolds .witslog/ if missing, then mounts
witslog.error('myapp', 'db timeout', { context: { request_id: 'r1' }, tags: ['db'] });

try {
  risky();
} catch (e) {
  witslog.exception('myapp', e);      // captures err.stack (and, if e.cause is set —
                                       // e.g. Node's own fetch() failures — the full
                                       // cause chain, folded into stacktrace + context.root_cause)
}
```

`init()` needs a `.witslog/` project directory to write into — pass `createProject: true`
(scaffolds one at `process.cwd()`) or `createProject: '/path/to/project'` the first time you
mount in a fresh project; it's a no-op on later runs once `.witslog/` already exists.

**As of 0.4.0, `npm install @all-wits/witslog` also gives you the real `witslog` CLI** — on the
4 bundled platforms (see [Platform support](#-platform-support)), a plain `npm install` wires up
`npx witslog <command>` (and a global install puts `witslog` on your PATH) with the same binary
[`docs/install.md`](https://github.com/all-wits/witslog/blob/main/docs/install.md) or Homebrew/Scoop/`cargo install` would give you —
`witslog init`, `witslog query`, `witslog stats`, `witslog serve-mcp`, all of it:

```bash
npx witslog init .
npx witslog query "db timeout*"
```

For programmatic use `createProject: true` remains the way to scaffold `.witslog/` from code
without shelling out:

```js
const witslog = require('@all-wits/witslog');
witslog.init({ createProject: true }); // scaffolds .witslog/, cross-platform
```

If you *also* separately install the [`witslog` CLI](https://github.com/all-wits/witslog/blob/main/docs/install.md) (`cargo install`,
Homebrew, Scoop, or a release binary), or the bundled binary isn't available for your platform,
point `WITSLOG_CLI=/path/to/witslog` at it — the npm-bundled and separately-installed CLIs are
interchangeable, pick whichever is already in your toolchain.

> **⚠️ For MCP (AI-assistant integration), install the CLI globally instead of relying on the
> npm-bundled binary** — see the [root README's MCP section](https://github.com/all-wits/witslog/blob/main/README.md#-integration-with-ai-mcp)
> for why: macOS Intel has no npm-bundled CLI at all (only [curl/irm](https://github.com/all-wits/witslog/blob/main/docs/install.md)/
> Homebrew/Scoop/`cargo install` cover it), and an MCP config generated from a path inside this
> project's `node_modules/` breaks if `node_modules` is ever removed or reinstalled elsewhere. The
> npm-bundled `npx witslog <command>` is for ad-hoc/manual use from inside a project — a globally
> installed CLI is what `serve-mcp --print-mcp-config` should point an MCP client at.

> **🔒 Security:** `argv` enrichment defaults on and captures the full command line. If your
> app may receive secrets as bare CLI args, call `witslog.init({ enrich: { argv: false } })` —
> see [CONTRACT.md](https://github.com/all-wits/witslog/blob/main/bindings/CONTRACT.md#security-note-argv-enrichment-vs-secrets).

## 🧩 Express

```js
const { witslogErrorHandler } = require('@all-wits/witslog/frameworks/express');

witslog.init();
app.use(witslogErrorHandler('myapp'));   // last, after routes
```

### 🌐 Browser-side error capture

Pairs with [`bindings/browser/witslog-browser.js`](https://github.com/all-wits/witslog/tree/main/bindings/browser) — a zero-dep client reporter
that batches `window.onerror` / unhandled-rejection events and ships them via
`navigator.sendBeacon` to this ingest endpoint.

```js
const { witslogBrowserIngest } = require('@all-wits/witslog/frameworks/express');

app.use(witslogBrowserIngest({
  allowedOrigins: ['https://your-app.example'], // required — fail-closed, default []
}));
```

> **🔒 Security:** the request body is untrusted input that lands in `events.message`, which
> MCP serves verbatim to an AI assistant. This handler is armed fail-closed: empty origin
> allowlist by default, refuses to run under `NODE_ENV=production` unless `{ force: true }`,
> rate-limited per client, and severity clamped to `error`/`warn` (never `fatal`/`critical`).
> `tags: ['browser']` is advisory only, not a trust boundary. See
> [CONTRACT.md](https://github.com/all-wits/witslog/blob/main/bindings/CONTRACT.md) for the
> Python/PHP ingest recipe and the full guardrail rationale.

## 🧵 Zero-boilerplate auto-instrumentation

Mount instrumentation once instead of hand-writing `try/catch` +
`witslog.exception`/`witslog.error` at every route handler and outbound `fetch` call.

### Instrumented fetch — `witslogFetch`

Explicit wrapper around `fetch` (no global monkeypatch — safe alongside Next.js's own fetch
caching/instrumentation). Swap it in at your outbound-request choke points:

```js
const { witslogFetch } = require('@all-wits/witslog/fetch');

const res = await witslogFetch(upstreamUrl, init, {
  application: 'my-proxy',
  tags: ['proxy'],
  context: { path: '/cards/123' },
});
```

Auto-captures a correlation id (`x-request-id` by default, propagated to the outbound
request), `context.timing.latency_ms`, and on failure:

- **Thrown error** (network unreachable/timeout) — logs via `exception()` (full `.cause`
  chain, `error_code: 'UPSTREAM_UNREACHABLE'`), then rethrows the original error unchanged.
- **Non-2xx response** — peeks the body via `.clone()` (your code still gets the untouched
  `Response`), extracts `error_code`/`message`/`details` from a `{error:{code,message,details}}`
  body when present, and logs at **`warn` for 4xx** (expected client-caused conflicts) /
  **`error` for 5xx**.

### Next.js adapter

```ts
// instrumentation.ts — Next.js's own server-boot hook. Captures every uncaught error in
// route handlers, Server Components, Server Actions, and middleware — zero per-route code.
import { register as registerWitslog, onRequestError as witslogOnRequestError } from '@all-wits/witslog/frameworks/next';

export function register() {
  registerWitslog('my-app', { createProject: true });
}
export const onRequestError = witslogOnRequestError;
```

`withWitslog(handler, opts?)` wraps a single route handler explicitly, for Next < 15 or when
you want per-route timing/correlation without global instrumentation.

`witslogNextIngest(options)` is the Next.js Route Handler-shaped equivalent of
`witslogBrowserIngest` above (Express's raw `req`/`res` and Next's Web `Request`/`Response`
aren't interchangeable, so this is a separate export, not a re-export — same guardrails):

```ts
// app/api/__witslog/route.ts
import { witslogNextIngest } from '@all-wits/witslog/frameworks/next';
export const POST = witslogNextIngest({ allowedOrigins: ['https://your-app.example'] });
```

### React Query client capture

Subscribes to a TanStack `QueryClient`'s `MutationCache`/`QueryCache` — the same event
stream TanStack Query Devtools itself observes — so every failed query/mutation (key,
variables, error) is captured with zero per-hook code. Browser-safe, no hard
`@tanstack/react-query` dependency.

```js
import { attachWitslog } from '@all-wits/witslog/frameworks/react-query';

// `report` is any {enqueue(event)} sink — typically WitslogBrowser.init(...)
// from bindings/browser/witslog-browser.js, which ships events to witslogNextIngest/witslogBrowserIngest
attachWitslog(queryClient, { report: myBrowserReporter, tags: ['my-app'] });
```

See [CONTRACT.md](https://github.com/all-wits/witslog/blob/main/bindings/CONTRACT.md#node-sdk-auto-instrumentation-fetch-nextjs-react-query-adapters)
for the full design (including the `context.root_cause` convention `exception()`/`witslogFetch`
use, and how `clampContext` bounds what an ingest endpoint accepts).

## 🧱 Works with your Node.js stack

`@all-wits/witslog` is a plain npm/pnpm/bun package with no bundler-specific glue — it works
in **any Node.js process**, which covers the server side of most modern frameworks:

- **Next.js** — call it from Route Handlers / API Routes, Server Actions, or Server
  Components (`witslog.init()` once at module scope, `witslog.error(...)` in a `catch`). **Next.js
  bundles server route code by default (webpack/turbopack), and that breaks resolution of
  `koffi`'s native `.node` addon** — you'll hit:
  ```
  Error: Cannot find the native Koffi module; did you bundle it correctly?
  ```
  Fix: tell Next to `require()` both packages natively instead of bundling them, in
  `next.config.ts`:
  ```ts
  const nextConfig: NextConfig = {
    serverExternalPackages: ["@all-wits/witslog", "koffi"],
  };
  ```
  (Next.js ≥15; pre-15 use `experimental.serverComponentsExternalPackages` instead.) This is the
  same fix any native-addon npm package needs under Next.js (e.g. Prisma, Sharp) — no witslog code
  change can make a `.node` binary bundler-safe, so this config is required, not optional.
- **Nuxt.js** — same idea inside server routes / the Nitro server (`server/api/*.ts`,
  `server/plugins/*.ts`).
- **Vite** — use it in a Vite **SSR** entry (`vite-node`, `vite-plugin-ssr`, a custom
  `server.js`) or in `vite.config.js` build hooks — anywhere Vite code actually executes in
  Node, not in code shipped to the browser.

> ⚠️ **Not for browser bundles.** The native `witslog_ffi` library is loaded via `koffi`,
> which needs a real Node.js process — it cannot run inside client-rendered **Vue.js**
> components, React components, or any code that ends up in a browser bundle. If you're
> using Vue.js/React purely client-side, log from your Node backend (API route, SSR
> middleware, server action) instead of the browser-rendered component itself.

## 📖 API

| Function | Description |
|----------|--------------|
| `init(config?)` | Mount the SDK; pass `{ createProject: true }` to scaffold `.witslog/` first, plus optional enrich/redact/buffer config (see [CONTRACT.md](https://github.com/all-wits/witslog/blob/main/bindings/CONTRACT.md)). |
| `error/warn/info(app, message, opts?)` | Log at the given severity. `opts`: `context`, `tags`, `metadata`, `error_code`, `exception`, `stacktrace`, `correlation_id`, `parent_event_id`, `category`, `version`, `environment`. |
| `log(app, message, opts?)` | Same as `error`, explicit severity via `opts.severity`. |
| `exception(app, err, opts?)` | Log a caught `Error`, capturing `err.stack` and, when set, `err.cause`'s full chain (folded into `stacktrace` + `context.root_cause`). |
| `flush()` / `shutdown()` | Drain buffered events before exit. |

### Auto-instrumentation (see [above](#-zero-boilerplate-auto-instrumentation))

| Import | Function | Description |
|--------|----------|--------------|
| `@all-wits/witslog/fetch` | `witslogFetch(input, init, opts?)` | Instrumented `fetch` wrapper. |
| `@all-wits/witslog/frameworks/next` | `register(app, config?)` / `onRequestError(err, req, ctx)` / `withWitslog(handler, opts?)` | Next.js server-error capture. |
| `@all-wits/witslog/frameworks/next` | `witslogNextIngest(options)` | Browser-ingest endpoint, Next.js Route Handler shape. |
| `@all-wits/witslog/frameworks/react-query` | `attachWitslog(queryClient, opts)` | Global React Query mutation/query failure capture. |

## 🌍 Platform support

| Platform | Status |
|----------|--------|
| Windows x64 | ✅ |
| Linux x64 | ✅ |
| Linux arm64 | ✅ |
| macOS arm64 (Apple Silicon) | ✅ |
| macOS x64 (Intel) | ⬜ not yet built by CI — [see CHANGELOG](https://github.com/all-wits/witslog/blob/main/CHANGELOG.md#known-limitations) |

If your platform isn't bundled, point at a local build via `WITSLOG_LIB=/path/to/witslog_ffi.*`
(native lib) and `WITSLOG_CLI=/path/to/witslog[.exe]` (CLI binary).

## 🧪 Test

```bash
pnpm install && pnpm test
```

## 📄 License

Apache License 2.0 — see [LICENSE](https://github.com/all-wits/witslog/blob/main/LICENSE).
