# witslog (Node.js SDK)

Framework-agnostic Node.js SDK over the native witslog library. One dependency (`koffi`,
prebuilt — no native build). See [../CONTRACT.md](../CONTRACT.md) for the ABI.

```js
const witslog = require('witslog');

witslog.init();                       // mount once (flushes on process exit)
witslog.error('myapp', 'db timeout', { context: { request_id: 'r1' }, tags: ['db'] });

try {
  risky();
} catch (e) {
  witslog.exception('myapp', e);      // captures err.stack
}
```

Locate the native library via `WITSLOG_LIB` (dev/CI) or bundle it under `_libs/<platform>/`.
Run from a directory inside a `.witslog/` project (or `witslog init` one first).

> **Security:** `argv` enrichment defaults on and captures the full command line. If your app
> may receive secrets as bare CLI args, call `witslog.init({ enrich: { argv: false } })` — see
> [../CONTRACT.md](../CONTRACT.md#security-note-argv-enrichment-vs-secrets).

## Express

```js
const { witslogErrorHandler } = require('witslog/frameworks/express');
witslog.init();
app.use(witslogErrorHandler('myapp'));   // last, after routes
```

## Test

```
npm install && npm test
```
