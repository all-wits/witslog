# Changelog

All notable changes to witslog are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), versioning follows
[Semantic Versioning](https://semver.org/). Each SDK/crate is versioned
independently at pre-1.0 — this file tracks the project as a whole.

## [Unreleased]

### Fixed

- **Install scripts only printed a PATH suggestion instead of acting on it**: after
  `install/install.ps1` copied `witslog.exe`, it just echoed the
  `[Environment]::SetEnvironmentVariable(...)` command for the user to run manually — same
  gap in `install/install.sh` (a `note: add ... to your PATH` echo, no actual PATH change).
  Confirmed live via a real `v0.1.3` install (`irm .../install.ps1 | iex`): binary landed at
  `%LOCALAPPDATA%\witslog\bin\witslog.exe`, version check at the end of the script only worked
  because that script invokes the binary by full path, not `witslog` on PATH — a fresh
  `witslog` in a new terminal would 'command not found' until the user manually ran the
  printed suggestion. Fixed both scripts to act, not suggest: `install.ps1` now calls
  `[Environment]::SetEnvironmentVariable('Path', ..., 'User')` (persists across terminals) plus
  updates `$env:Path` for the current session; `install.sh` detects the user's shell (`$SHELL`
  → `.zshrc`/`.bashrc`/`.profile`) and appends an idempotent `export PATH=...` line (skips if
  the install dir is already present, so re-running the installer doesn't duplicate it), plus
  exports it for the current (piped-into-`sh`) session. `docs/install.md` updated to describe
  the new automatic behavior.
  - Follow-up fixes found reviewing cross-platform coverage of the above: (1) `install.sh`'s
    shell detection only branched `zsh`/`bash`, silently writing a bash-style `export PATH=...`
    line into `~/.profile` for fish users — fish never reads `.profile` and doesn't understand
    `export` syntax anyway, so fish users got no PATH fix at all. Added a `*/fish` branch
    writing `set -gx PATH ... $PATH` to `~/.config/fish/config.fish` instead (creating the
    `~/.config/fish/` dir if missing). (2) `install.ps1`'s arch switch mapped `ARM64` to an
    asset (`witslog-windows-aarch64.zip`) that `release.yml`'s Windows matrix never builds
    (only `x86_64-pc-windows-msvc`) — Windows-on-ARM users hit a raw 404 instead of the same
    clean "no prebuilt binary, use cargo install" error every other unsupported arch gets.
    Windows ARM64 now falls through to that same unsupported-arch path.

## [0.1.3] — 2026-07-18

### Added

- **Node SDK (`@all-wits/witslog`) now bundles the real `witslog` CLI binary**, closing the
  remaining npm-install-only gap: `createProject: true` (previous session) fixed `init`, but
  `query`/`stats`/`export`/`serve-mcp`/`doctor` have no FFI surface at all (by design — see
  `bindings/CONTRACT.md`), so they were unreachable without a separate CLI install. `bin/
  witslog.js` (new) is a thin `spawnSync` shim resolving the binary via `bindings/node/lib/
  cli-locator.js` — `WITSLOG_CLI` env override → bundled `_bin/<platform>/witslog{,.exe}` →
  bare `witslog` on `PATH` (mirrors the existing `_libs/`/`WITSLOG_LIB` native-lib locator
  convention exactly). Wired into `package.json`'s `bin` field, so `npx witslog query ...` and
  a global install both work post-`npm install`, on the 4 already-bundled platforms (Windows
  x64, Linux x64/arm64, macOS Apple Silicon — `darwin-x64` stays unbundled, same known gap as
  the native lib). `.github/workflows/release-node-sdk.yml` extended to also
  `cargo build --release -p witslog-cli` per matrix leg and assemble into `_bin/`.
  `bindings/e2e/run.ps1` gained Gate 5 (npm CLI shim e2e, real binary via `WITSLOG_CLI`, real
  DB, real query readback through `bin/witslog.js` itself, not just `$cli` directly). Node SDK
  bumped to 0.4.0. Regression lock: `bindings/node/test/cli_locator.test.js` (`resolveCliPath()`
  falls through to the bare filename when nothing bundled exists, `package.json.bin.witslog`
  wiring itself), `bindings/node/test/bin_shim.test.js` (argv/exit-code forwarding,
  `WitslogCliNotFoundError` on spawn-time `ENOENT`).

### Fixed

- **Node SDK (`@all-wits/witslog`) had no way to bootstrap a `.witslog/` project**:
  `npm install` bundles the native `witslog_ffi` lib but ships no CLI binary, and the
  README's `witslog init` step referenced a command with no install path from npm alone.
  Every FFI write path (`witslog_log`/`witslog_resolve`/`witslog_delete`) opens the DB via
  `SQLITE_OPEN_CREATE`, which creates the DB *file* but not a missing parent `.witslog/`
  directory — so `log()`/`error()`/`info()` all failed (`rc=-1`) in a project that had never
  run the CLI's `witslog init`. Fixed by adding `witslog_bootstrap_project(path_or_null)` to
  `witslog-ffi` (mirrors the CLI's `init_db`: create dir, `Store::open_or_create`, migrate;
  idempotent) and wiring it into the Node SDK as `witslog.init({ createProject: true })` /
  `{ createProject: '/path' }`. Documented in `bindings/CONTRACT.md` and
  `bindings/node/README.md`. Regression lock:
  `witslog-ffi::tests::witslog_log_fails_when_witslog_dir_absent` (pins the original
  failure) + `bootstrap_project_creates_dir_and_enables_logging` /
  `bootstrap_project_is_idempotent` / `bootstrap_project_accepts_explicit_path`
  (`crates/witslog-ffi/src/lib.rs`), plus `bindings/node/test/bootstrap.test.js` for the JS
  wiring (config-stripping, error surfacing, no-op when `createProject` is absent).
- `witslog_query::SearchEngine::search` errored unconditionally when called
  with `"*"` or `""` — FTS5 rejects a bare `*`/empty string as `MATCH` syntax
  ("unknown special query"), but that literal was the codebase's own
  "match everything, just apply filters" convention: the MCP `latest_errors`
  tool, `similar_errors`'s fingerprint mode, and any user running
  `witslog query "*"` all failed every time, regardless of filters. Fixed by
  special-casing an empty/whitespace-only/`"*"` query to skip the FTS5 join
  entirely and query `events` directly (ordered by recency — there's no bm25
  rank without a real FTS match); a genuine FTS syntax error is still
  rejected. Predates P10 (confirmed via `git diff` against the P10 session);
  found in passing while proving P10's MCP `resolved`-filter surface with a
  real client. Regression lock:
  `witslog-query::search::tests::match_all_query_returns_filtered_results`
  (+ `..._honours_filters_and_orders_by_recency`,
  `non_match_all_bad_syntax_still_errors`).

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

- **P10 — MTTR/resolution tracking, notifiers, browser-side error capture**:
  - **Audit tombstones (blocker fix, FR-P10-001)**: `delete`/`prune`/`archive`
    previously broke `doctor --verify-audit` permanently for every row after
    the deleted one, because `verify_chain` recomputed the hash chain over
    surviving `id`s with no way to account for a gap — indistinguishable from
    tampering. `migrate_0007_audit_tombstones` adds an `audit_tombstones`
    table recording each deleted row's `audit_hash` before removal;
    `witslog-store::writer::delete_events_by_id` is now the single path all
    three delete sites (`delete_resolved`, `cmd_prune`, `cmd_archive`) route
    through (previously `prune`/`archive` ran raw `DELETE` in the CLI,
    reaching around the store layer); `audit::verify_chain` bridges a gap via
    its tombstone hash and reports it as informational
    (`tombstones_bridged`), while an undocumented gap still reports `Broken`.
    `CURRENT_SCHEMA_VERSION` bumped 6→7 for this migration alone.
  - MTTR is **fingerprint-level, not event-level** (`AggregateEngine::mttr`):
    `MIN(resolved_at) − MIN(ts)` per fingerprint among events matching the
    filter — "time from first sighting to first fix" — deliberately not
    per-event, since a fingerprint firing hundreds of times before one fix
    would otherwise measure error volume and report it as recovery time. Mean
    only in v1 (no percentiles — `ts`/`resolved_at` are TEXT with no
    epoch-ms mirror, so duration is computed from parsed RFC3339 in Rust, not
    SQL `julianday`).
  - `EventWriter::mark_resolved` now returns `Result<bool>` and guards
    `resolved_at IS NULL` unless `force:true` (previously ignored the
    affected-row count, so it silently "succeeded" on an unknown `event_id`
    and could move `resolved_at` on a re-resolve). `witslog_resolve` (FFI)
    and `witslog resolve <id> [--force]` (CLI) updated to match.
  - `witslog_query::Filters.resolved: Option<bool>` (`resolved_at IS
    NULL`/`IS NOT NULL`); surfaced as `witslog query --unresolved`,
    `witslog stats --mttr`, and `resolved` on the MCP common-filters object.
    Also fixed `top_failures` (MCP), which hardcoded `Filters::default()` and
    silently ignored every filter param a caller passed.
  - New read-only MCP tool `mttr`. **No MCP write tool for resolution** —
    PLAN.md §5 deliberately made `witslog_delete` the only write tool, and a
    resolve tool would let an agent silently qualify rows for
    `witslog_delete`'s `resolved_at IS NOT NULL` default filter.
  - Notifiers: new `[notify]` config section (`enabled`, `min_severity`,
    `path`, `once_per_fingerprint_secs`) wires `witslog_plugin::Notifier`
    (P9, previously defined but never dispatched from the write path) into
    `witslog-runtime`. Builtin `FileNotifier` (NDJSON append) only — no
    webhook/HTTP dependency: `witslog-runtime` links into `witslog-ffi`,
    which is `dlopen`'d into every Python/Node/PHP host process, so adding an
    HTTP client there was rejected; `Notifier` is already the extension
    point for anyone who wants a webhook. Dispatch is synchronous
    post-write in `build_and_write`/`write_via_snapshot`, but **never** from
    the panic hook's forced-sync path (`capture_sync`) — a panic may precede
    process abort, and notifier I/O in that path is the one place a stall is
    unacceptable.
  - Browser-side error capture (PLAN.md §10): `bindings/browser/witslog-browser.js`,
    a zero-dep reporter installing `window.onerror`/`unhandledrejection`,
    batching, and shipping via `navigator.sendBeacon` (fallback
    `fetch(...,{keepalive:true})`), flushing on `pagehide`/hidden. Server-side
    ingest via `witslogBrowserIngest` in `bindings/node/frameworks/express.js`
    — the request body is untrusted input whose text reaches
    `events.message`, which MCP serves verbatim to an LLM, so this is armed
    fail-closed: empty `allowedOrigins` by default (Origin check, not just a
    loopback check, since the real attack is a malicious page open in the
    *same* browser as the dev server doing a same-machine cross-origin POST),
    refuses to arm under `NODE_ENV=production` unless `force:true`, a
    token-bucket rate limit (per-request size caps alone don't bound request
    *volume*), and severity clamped to `error|warn` (never `fatal`/`critical`
    from untrusted input) plus message/stacktrace/batch/body size caps.
    Python/PHP ingest intentionally not shipped as adapters — documented as a
    recipe in `bindings/CONTRACT.md` instead. `tags:['browser']` is advisory,
    not a trust boundary (`classify()` merges suggested tags); true
    provenance (`ingest_source` in the payload contract) would need an
    ABI-version bump and is out of scope here.
  - Deliberately out of scope: `resolved_by`/`resolution_note` columns (the
    audit chain hashes `event_id|ts|message|fingerprint` only, so a "who
    resolved this" field would be unauthenticated and unverifiable on a
    single-user local tool with no identity system — resolution provenance,
    if ever needed, is a child event with `parent_event_id`); resolution
    SLAs/reopen-tracking; notifier retries/queues; dynamic plugin loading.
  - Tests: `witslog-store::audit` regression locks
    (`deleting_a_row_keeps_verify_chain_ok`,
    `deleted_row_without_tombstone_still_breaks_chain`); `witslog-query`
    unit tests for the `resolved` filter axis and fingerprint-level MTTR
    (`mttr_excludes_unresolved_fingerprints`); `witslog-runtime`
    `tests/p6_integration.rs` regression locks
    (`notifier_never_dispatches_from_panic_path`,
    `notifier_dispatches_on_normal_capture`,
    `notifier_failure_does_not_fail_write`); `witslog-runtime::notify` unit
    tests (file append, throttle); Node `bindings/browser/test` +
    `bindings/node/test/express_ingest.test.js` (origin/loopback/rate-limit/
    production-guard/severity-clamp regression locks).

## [node-sdk 0.3.0] — 2026-07-17

Version cut for `@all-wits/witslog` on npm specifically (package.json bump; does not move
the `[Unreleased]` section above, since the Rust crates/CLI/MCP side hasn't cut its own
release yet — same reasoning as `0.2.0`).

### Added

- `init({ createProject: true })` / `init({ createProject: '/path' })`: scaffolds a
  `.witslog/` project directory (dir + DB + migrate) via the new native
  `witslog_bootstrap_project` export before mounting. Closes the gap where `npm install`
  bundled the native lib but shipped no CLI, so a project that never separately installed
  and ran `witslog init` had no way to create `.witslog/` — every `log()`/`error()`/`info()`
  call failed with `rc=-1`. See `bindings/CONTRACT.md` and `bindings/node/README.md`. Only
  wired into the Node SDK so far; Python/PHP can call the native symbol directly but have
  no convenience wrapper yet.

## [node-sdk 0.2.1] — 2026-07-17

Docs-only follow-up to `0.2.0` — no code changes. `0.2.0` published
successfully once `release-node-sdk.yml` was fixed to use npm Trusted
Publishing (OIDC) instead of an automation token (see `### Fixed` below),
but that publish ran off a commit that predated the README updates
documenting `witslogBrowserIngest` and the P10 CLI surface (`resolve`,
`--unresolved`, `--mttr`, `--verify-audit`) — so the README shown on the npm
package page was stale. npm versions are immutable, so a docs-only change
still needs its own version bump to actually reach the published listing.

### Changed

- `README.md` and `bindings/node/README.md`: document P10 (MTTR/resolution
  tracking, notifiers, browser-side error capture) — feature list, status
  table, MCP tool count (12 → 13, `mttr` added), CLI examples, and a new
  "Browser-side error capture" section in both, including the
  `witslogBrowserIngest` fail-closed defaults.

### Fixed

- `.github/workflows/release-node-sdk.yml`: the `npm publish` step passed
  `NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}` (an automation token), which
  npm's registry now rejects for this package with a 2FA-required 403 —
  npm's own Trusted Publisher (OIDC) config on npmjs.com doesn't override a
  token if one is still sent. Fixed by adding `permissions: id-token:
  write` to the `assemble-and-publish` job, pinning npm to `latest` (OIDC
  Trusted Publishing needs npm ≥ 11.5.1), and removing `NODE_AUTH_TOKEN`
  from the publish step entirely.

## [node-sdk 0.2.0] — 2026-07-17

Version cut for `@all-wits/witslog` on npm specifically (package.json bump;
does not move the `[Unreleased]` section above, since the Rust
crates/CLI/MCP side of P10 hasn't cut its own release yet). Prepared for
publish via `release-node-sdk.yml` (`workflow_dispatch`, manual, `publish:
true`) — not auto-triggered by merging to `main`.

### Added

- `witslogBrowserIngest` in `bindings/node/frameworks/express.js` (P10):
  Express handler accepting batches from `bindings/browser/witslog-browser.js`.
  New export; existing `witslogErrorHandler` unchanged.

### Fixed

- The bundled native lib's `witslog_resolve` now guards `resolved_at IS
  NULL` (first resolution wins) and returns `-1` on an unknown or
  already-resolved event id, instead of silently reporting success and
  potentially moving `resolved_at` on a re-resolve. No JS-facing API change
  (still `witslog_resolve(event_id_ptr) -> i32`), but the bundled binary
  behaves differently — republishing is what actually ships this fix to
  Node SDK users, since it lives in `_libs/<platform>/`, not JS source.

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
