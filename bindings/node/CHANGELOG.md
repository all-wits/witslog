# Changelog (`@all-wits/witslog`, Node SDK)

Node-SDK-specific history only — extracted from the project-wide
[`../../CHANGELOG.md`](../../CHANGELOG.md), which also covers the Rust crates/CLI/MCP server on
their own independent version numbers. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this package versions independently of
the Rust workspace (pre-1.0).

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
