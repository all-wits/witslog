<div align="center">

# 🪵 witslog (Node.js SDK)

[![npm](https://img.shields.io/npm/v/%40all-wits%2Fwitslog?label=npm&logo=npm)](https://www.npmjs.com/package/@all-wits/witslog)
[![License: Apache 2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](../../LICENSE)
[![node](https://img.shields.io/node/v/%40all-wits%2Fwitslog?logo=nodedotjs)](https://www.npmjs.com/package/@all-wits/witslog)
[![CI](https://img.shields.io/github/actions/workflow/status/all-wits/witslog/release-node-sdk.yml?logo=githubactions)](https://github.com/all-wits/witslog/actions/workflows/release-node-sdk.yml)

**Framework-agnostic Node.js SDK for [witslog](../../README.md) structured error logging.**

</div>

---

Thin wrapper over the native `witslog-ffi` C ABI via [`koffi`](https://koffi.dev) — one
dependency, **prebuilt, no native build step**. As of 0.4.0 it also bundles the real `witslog`
CLI binary per platform, so `witslog query`/`stats`/`export`/`serve-mcp`/`doctor` etc. (the
read/ops surface that has no FFI equivalent — see CONTRACT.md) work straight after install, no
separate CLI install required. See [../CONTRACT.md](../CONTRACT.md) for the full SDK↔native ABI.

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
  witslog.exception('myapp', e);      // captures err.stack
}
```

`init()` needs a `.witslog/` project directory to write into — pass `createProject: true`
(scaffolds one at `process.cwd()`) or `createProject: '/path/to/project'` the first time you
mount in a fresh project; it's a no-op on later runs once `.witslog/` already exists.

**As of 0.4.0, `npm install @all-wits/witslog` also gives you the real `witslog` CLI** — on the
4 bundled platforms (see [Platform support](#-platform-support)), a plain `npm install` wires up
`npx witslog <command>` (and a global install puts `witslog` on your PATH) with the same binary
[`docs/install.md`](../../docs/install.md) or Homebrew/Scoop/`cargo install` would give you —
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

If you *also* separately install the [`witslog` CLI](../../docs/install.md) (`cargo install`,
Homebrew, Scoop, or a release binary), or the bundled binary isn't available for your platform,
point `WITSLOG_CLI=/path/to/witslog` at it — the npm-bundled and separately-installed CLIs are
interchangeable, pick whichever is already in your toolchain.

> **⚠️ For MCP (AI-assistant integration), install the CLI globally instead of relying on the
> npm-bundled binary** — see the [root README's MCP section](../../README.md#-integration-with-ai-mcp)
> for why: macOS Intel has no npm-bundled CLI at all (only [curl/irm](../../docs/install.md)/
> Homebrew/Scoop/`cargo install` cover it), and an MCP config generated from a path inside this
> project's `node_modules/` breaks if `node_modules` is ever removed or reinstalled elsewhere. The
> npm-bundled `npx witslog <command>` is for ad-hoc/manual use from inside a project — a globally
> installed CLI is what `serve-mcp --print-mcp-config` should point an MCP client at.

> **🔒 Security:** `argv` enrichment defaults on and captures the full command line. If your
> app may receive secrets as bare CLI args, call `witslog.init({ enrich: { argv: false } })` —
> see [../CONTRACT.md](../CONTRACT.md#security-note-argv-enrichment-vs-secrets).

## 🧩 Express

```js
const { witslogErrorHandler } = require('@all-wits/witslog/frameworks/express');

witslog.init();
app.use(witslogErrorHandler('myapp'));   // last, after routes
```

### 🌐 Browser-side error capture

Pairs with [`bindings/browser/witslog-browser.js`](../browser) — a zero-dep client reporter
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
> [`../CONTRACT.md`](../CONTRACT.md) for the Python/PHP ingest recipe and the full guardrail
> rationale.

## 🧱 Works with your Node.js stack

`@all-wits/witslog` is a plain npm/pnpm/bun package with no bundler-specific glue — it works
in **any Node.js process**, which covers the server side of most modern frameworks:

- **Next.js** — call it from Route Handlers / API Routes, Server Actions, or Server
  Components (`witslog.init()` once at module scope, `witslog.error(...)` in a `catch`).
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
| `init(config?)` | Mount the SDK; pass `{ createProject: true }` to scaffold `.witslog/` first, plus optional enrich/redact/buffer config (see [CONTRACT.md](../CONTRACT.md)). |
| `error/warn/info(app, message, opts?)` | Log at the given severity. `opts`: `context`, `tags`, `metadata`, `error_code`, `exception`, `stacktrace`, `correlation_id`, `parent_event_id`, `category`, `version`, `environment`. |
| `log(app, message, opts?)` | Same as `error`, explicit severity via `opts.severity`. |
| `exception(app, err, opts?)` | Log a caught `Error`, capturing `err.stack`. |
| `flush()` / `shutdown()` | Drain buffered events before exit. |

## 🌍 Platform support

| Platform | Status |
|----------|--------|
| Windows x64 | ✅ |
| Linux x64 | ✅ |
| Linux arm64 | ✅ |
| macOS arm64 (Apple Silicon) | ✅ |
| macOS x64 (Intel) | ⬜ not yet built by CI — [see CHANGELOG](../../CHANGELOG.md#known-limitations) |

If your platform isn't bundled, point at a local build via `WITSLOG_LIB=/path/to/witslog_ffi.*`
(native lib) and `WITSLOG_CLI=/path/to/witslog[.exe]` (CLI binary).

## 🧪 Test

```bash
npm install && npm test
```

## 📄 License

Apache License 2.0 — see [../../LICENSE](../../LICENSE).
