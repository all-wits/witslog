# Changelog

All notable changes to witslog are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), versioning follows
[Semantic Versioning](https://semver.org/). Each SDK/crate is versioned
independently at pre-1.0 — this file tracks the project as a whole.

## [Unreleased]

- CI: version-gate on the Node SDK release workflow — only publishes to npm
  when `package.json` version differs from what's already on the registry.
- CI: Node SDK release workflow now builds against the latest Node.js release.

## [0.1.0] — 2026-07-16

### Added

- **P0 — Storage + event model**: SQLite schema (WAL, STRICT tables), fluent
  `EventBuilder`, deterministic fingerprinting, per-project DB resolution
  (`.witslog/` walk-up), CLI (`init/log/query/resolve/delete/doctor`), C ABI
  FFI core (`witslog_log/resolve/delete`).
- **P1 — Logging library**: auto-enrichment (hostname/pid/cwd/argv/git_commit),
  built-in + custom secret redaction, async buffered writes, severity
  convenience constructors.
- **P2 — Taxonomy engine**: builtin category tree, deterministic rule-based
  auto-classification, custom categories/aliases.
- **P3 — FTS5 + query engine**: full-text search (bm25 ranking, prefix/phrase/
  boolean/NEAR), structured filters, keyset pagination, aggregates
  (stats/timeline/top failures), correlation/causality walks.
- **P4 — CLI utilities**: `query`, `stats`, `export`/`import` (NDJSON),
  `vacuum`, `prune`, `migrate`, `config`, `archive`, `backup`, `list-dbs`,
  `category`.
- **P5 — MCP server**: JSON-RPC/stdio server exposing all 12 tools
  (`search_errors`, `latest_errors`, `summarize_errors`, `classify_error`,
  `explain_error`, `similar_errors`, `list_categories`, `statistics`,
  `timeline`, `top_failures`, `list_traces`, `search_all`), schema validation,
  per-call statement timeout, write-gated `witslog_delete`.
- **P6 — SDK bindings**: framework-agnostic SDKs over the C ABI —
  [`@all-wits/witslog`](bindings/node) (Node, via `koffi`),
  [`witslog`](bindings/python) (Python, via stdlib `ctypes`),
  [`witslog/witslog`](bindings/php) (PHP, via `ext-ffi`) — plus thin adapters
  for Express, FastAPI/Django/Flask, and Laravel. Shared contract documented
  in [`bindings/CONTRACT.md`](bindings/CONTRACT.md), including an
  `argv`-enrichment security note and the `witslog_abi_version()` handshake.
- **witslog-runtime**: ambient "Provider" runtime — mount-once init, panic
  capture, `tracing` layer (Rust-only), shared enrich→redact→classify→write
  pipeline shared by the CLI and the ambient capture path.
- **Cross-platform native lib CI**: GitHub Actions workflow builds
  `witslog_ffi` natively for Windows x64, Linux x64/arm64, and macOS
  arm64 (Apple Silicon), then publishes the Node SDK to npm.

### Known limitations

- Intel Mac (`darwin-x64`) native lib is not built by CI yet — the
  `macos-13` hosted-runner queue proved impractically slow. Tracked for a
  future revisit.
- No cross-platform installer/packaging yet (P8).
- No perf benches/concurrency hardening yet (P7).
