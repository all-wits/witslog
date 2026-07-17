# Changelog

All notable changes to witslog are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), versioning follows
[Semantic Versioning](https://semver.org/). Each SDK/crate is versioned
independently at pre-1.0 — this file tracks the project as a whole.

## [Unreleased]

### Added

- **P9 — Extensibility + security**:
  - `witslog-plugin` crate (FR-P9-001/002): six extension-point traits
    (`TaxonomyRule`, `Exporter`, `Enricher`, `StorageBackend`, `Notifier`,
    `McpTool`) plus `PluginRegistry` for static registration. Every dispatch
    path (`classify`, `run_enrichers`, `dispatch_event`, `export_all`,
    `call_mcp_tool`) wraps the call in `catch_unwind` so a panicking plugin is
    reported as a `PluginError::Panicked` rather than crashing the core write
    path or corrupting the DB (non-functional isolation requirement).
    Dynamic (`.so`/`.dll`) loading intentionally out of scope — static
    registration keeps the ABI surface small.
  - Audit hash chain (FR-P9-006/007): `migrate_0006_audit_chain` adds
    `events.audit_hash` + an `audit_meta` table; `witslog-store::audit`
    chains `sha256(prev_hash|event_id|ts|message|fingerprint)` on every
    insert (wired into the shared `write_event` path, so it covers the CLI,
    FFI, and buffered/batch writers alike) and back-fills any pre-existing
    rows on migration. `witslog doctor --verify-audit` recomputes the chain
    and reports the first tampered row (id + expected/actual hash), exiting
    non-zero on a break.
  - File-permission hardening (FR-P9-005): `witslog init` now chmods the DB
    file `0600` in addition to the pre-existing `0700` on `.witslog/` (Unix
    only — Windows ACL hardening intentionally out of scope, same as the
    existing dir-perm call).
  - `witslog-core::crypto::FieldCipher` (FR-P9-004, scoped): AES-256-GCM
    field-level cipher for `metadata` via `EventBuilder::encrypt_metadata`,
    key sourced from a 32-byte hex string or `FieldCipher::from_env`. Full
    SQLCipher-style DB-at-rest encryption was evaluated and deliberately
    **not** built: it conflicts with this schema's FTS5 index and
    `GENERATED ALWAYS AS (json_extract(...))` columns (both need plaintext),
    and vendoring SQLCipher adds real cross-compile cost for P8's release
    matrix — the same cost-vs-value call already made for winget/.deb/.rpm.
    Off by default either way.
  - Config-driven custom redaction rules (FR-P9-003) were already wired in
    P1 (`RedactSection::custom_patterns`); this phase didn't need to add
    anything there.
  - Tests: `witslog-plugin` unit tests (one per trait + a panic-isolation
    regression); `witslog-store::audit` unit tests (clean chain, tampered-row
    detection, backfill-from-legacy-rows); `witslog-core::crypto` unit tests
    (round-trip, wrong-key failure, envelope wrap/unwrap); `witslog-cli`
    `tests/p9_integration.rs` drives the real binary end-to-end (`doctor
    --verify-audit` clean vs. tampered, plus a Unix-only 0600/0700
    permission regression).

## [0.1.1] — 2026-07-17

### Fixed

- CI: `.github/workflows/release.yml` `publish` job failed with "Resource not
  accessible by integration" (403) on the first `v0.1.1` tag push — the
  default `GITHUB_TOKEN` had no `contents: write` permission to create a
  GitHub Release. Added a top-level `permissions: contents: write` block.
  `build` and `smoke_test` had already passed on that run; only `publish`
  needed the retry, so the `v0.1.1` tag was moved to the fix commit rather
  than bumping the version.

### Added

- **P8 — Packaging + install (partial)**:
  - Version-compatibility guard (FR-P8-007): `witslog-store::CURRENT_SCHEMA_VERSION`
    const + `Migrator::migrate()` refuses with an upgrade message
    (`StoreError::SchemaVersionMismatch`) when a DB's `schema_version` is newer
    than the binary supports, instead of silently corrupting/truncating.
  - `witslog serve-mcp --print-mcp-config` (FR-P8-004): emits a generic
    `mcpServers` JSON snippet (command/args/cwd) without opening a DB.
  - `witslog uninstall [--purge]` (FR-P8-006): unlinks the running binary on
    Unix; prints manual `del` instructions on Windows (a running exe can't
    self-delete there). `--purge` also removes the project `.witslog/` dir and
    the OS-appropriate global config dir.
  - `witslog migrate` now restores the pre-migration `.bak` snapshot and aborts
    cleanly on migration failure instead of leaving a half-migrated DB
    (FR-P8-005 error path).
  - `witslog doctor` prints the binary version and max supported schema
    version, and surfaces (rather than swallows) a failed DB health check.
  - `witslog --version` now works (`#[command(version)]` on the clap `Cli`).
  - Install scripts `install/install.sh` / `install/install.ps1`: detect
    OS/arch, download + verify SHA-256 checksum, place `witslog` on PATH.
  - Cross-compile release workflow `.github/workflows/release.yml`: Linux
    x86_64/aarch64, macOS x86_64/aarch64, Windows x86_64, checksummed
    archives uploaded to GitHub Releases.
  - Template Homebrew formula (`install/homebrew/witslog.rb`) and Scoop
    manifest (`install/scoop/witslog.json`) — placeholder checksums until a
    real release is cut.
  - `docs/install.md`: install/upgrade/uninstall guide per OS.
  - Tests: `witslog-store/src/migrate.rs` unit tests (fresh migrate, idempotent
    re-run, refuse newer-than-binary schema); `witslog-cli/tests/p8_integration.rs`
    feature/regression tests driving the real built binary
    (`--print-mcp-config` shape + no-DB-required, schema-too-new refusal
    end-to-end, normal round-trip still works); `witslog-cli` `uninstall_tests`
    unit tests for the pure `purge_data_dirs` helper.
  - `smoke_test` CI job in `.github/workflows/release.yml`: builds and runs
    the real per-OS happy path (`--version`/`init`/`log`/`query`/`stats`/
    `doctor`/`serve-mcp --print-mcp-config`/`serve-mcp --stdio` `tools/list`/
    `uninstall --purge`) against the freshly built artifact on Linux, macOS,
    and Windows runners; gates `publish` so nothing ships without a live pass.
    Confirmed green end-to-end via `workflow_dispatch` — all 5 `build` matrix
    legs (linux x86_64/aarch64, macos x86_64/aarch64, windows x86_64) and all
    3 `smoke_test` legs passed on real GitHub-hosted runners.
  - Fixed: install scripts/docs/Homebrew-Scoop templates pointed at the wrong
    GitHub org (`witslog/witslog` instead of the actual `all-wits/witslog`
    remote) — would have 404'd for every real download. Corrected across
    `install/install.sh`, `install/install.ps1`, `docs/install.md`,
    `install/homebrew/witslog.rb`, `install/scoop/witslog.json`, `README.md`.
  - winget manifest and `.deb`/`.rpm` packaging deliberately not added:
    `cargo install witslog-cli` and the npm/pip/composer SDK packages already
    give cross-platform distribution pre-1.0, and there's no cut release yet
    to package — revisit once one exists.

### Changed

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
