# Changelog (`witslog/witslog`, PHP / Laravel SDK)

PHP-SDK-specific history only — extracted from the project-wide
[`../../CHANGELOG.md`](https://github.com/all-wits/witslog/blob/main/CHANGELOG.md), which also
covers the Rust crates/CLI/MCP server and the Node/Python SDKs on their own independent version
numbers. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- **Metadata-field encryption (`crypto.key_env`) passthrough** — passing
  `['crypto' => ['key_env' => 'WITSLOG_ENCRYPTION_KEY']]` to init now reaches the native
  `witslog_configure` payload and enables AES-256-GCM encryption of the `metadata` field on
  write. No SDK code change was needed (init already forwards the config array as-is to the C
  ABI); this is a Rust-side capability (`crates/witslog-core/src/crypto.rs`, wired via
  `crates/witslog-ffi/src/lib.rs`) documented here because it's now reachable from this SDK. See
  the root [`CHANGELOG.md`](https://github.com/all-wits/witslog/blob/main/CHANGELOG.md) and
  [`bindings/CONTRACT.md`](https://github.com/all-wits/witslog/blob/main/bindings/CONTRACT.md#metadata-encryption-fr-p9-004)
  for the full design (scope, fail-closed write, `"<encrypted>"` placeholder on read, key
  rotation). `metadata` is the only encrypted field — `message`/`context`/`stacktrace`/etc. stay
  plaintext so search keeps working.

This SDK has not cut an independent version since its initial release below — unlike the Node
SDK (which has its own `0.2.0`/`0.3.0`/`0.4.0` history in
[`bindings/node/CHANGELOG.md`](https://github.com/all-wits/witslog/blob/main/bindings/node/CHANGELOG.md)),
PHP has no `composer.json` version field of its own (Packagist infers it from git tags). Known
gaps tracked against a future bump (see [README.md](https://github.com/all-wits/witslog/blob/main/bindings/php/README.md) for detail):

- Doesn't wrap the native `witslog_bootstrap_project` export the way the Node SDK's
  `init({ createProject: true })` does — still needs the CLI installed separately to bootstrap
  a fresh `.witslog/` project.
- No bundled native `witslog_ffi` library / release CI matrix like the Node SDK's — point
  `WITSLOG_LIB` at a locally built one, or drop it under `_libs/<platform>/`.

## [0.1.0] — 2026-07-16

### Added

- Initial release: framework-agnostic core (`Witslog::log`/`error`/`warn`/`info`/`exception`,
  `init`/`flush`/`shutdown`) over the native `witslog-ffi` C ABI using PHP's built-in `ext-ffi`
  — no third-party runtime dependency. Laravel service provider (auto-discovered via
  `extra.laravel.providers`). `witslog_abi_version()` handshake, `WITSLOG_LIB` locator.
