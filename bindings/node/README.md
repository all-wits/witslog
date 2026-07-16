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
dependency, **prebuilt, no native build step**. See [../CONTRACT.md](../CONTRACT.md) for the
full SDK↔native ABI.

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

Native libraries for Windows x64, Linux x64/arm64, and macOS (Apple Silicon) are bundled —
`npm install` / `pnpm add` / `bun add` alone is enough on those platforms. See
[Platform support](#-platform-support) below for the current gap.

## 🚀 Quick Start

```js
const witslog = require('@all-wits/witslog');

witslog.init();                       // mount once (flushes on process exit)
witslog.error('myapp', 'db timeout', { context: { request_id: 'r1' }, tags: ['db'] });

try {
  risky();
} catch (e) {
  witslog.exception('myapp', e);      // captures err.stack
}
```

Run from a directory inside a `.witslog/` project (or `witslog init` one first) so events
land in that project's DB.

> **🔒 Security:** `argv` enrichment defaults on and captures the full command line. If your
> app may receive secrets as bare CLI args, call `witslog.init({ enrich: { argv: false } })` —
> see [../CONTRACT.md](../CONTRACT.md#security-note-argv-enrichment-vs-secrets).

## 🧩 Express

```js
const { witslogErrorHandler } = require('@all-wits/witslog/frameworks/express');

witslog.init();
app.use(witslogErrorHandler('myapp'));   // last, after routes
```

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
| `init(config?)` | Mount the SDK; optionally pass enrich/redact/buffer config (see [CONTRACT.md](../CONTRACT.md)). |
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

If your platform isn't bundled, point at a local build via `WITSLOG_LIB=/path/to/witslog_ffi.*`.

## 🧪 Test

```bash
npm install && npm test
```

## 📄 License

Apache License 2.0 — see [../../LICENSE](../../LICENSE).
