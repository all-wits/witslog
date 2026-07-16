# witslog — Project Guide for Claude

witslog = SQLite-backed error intelligence framework. Per-project DB (`./.witslog/witslog.db`), structured events, AI-queryable via future MCP. See **PLAN.md** for full spec.

## Architecture

- **Embedding**: in-process Rust lib + CLI. Single write-serialized connection over WAL; readers concurrent.
- **Storage**: SQLite per-project; no cloud, no infra. Events append-only; FTS5 full-text search; dimension tables (categories); migrations numbered, idempotent.
- **Taxonomy**: deterministic, pure-fn rules engine; auto-classify errors without embedding/model.
- **Delivery**: Rust reference binary; C ABI FFI for embedding in any language; CLI + future MCP server.

## Crate Map

Workspace in `crates/`. Built crates:

| Crate | Purpose | Key Files |
|-------|---------|-----------|
| **witslog-core** | Event model, builders, enrichment, redaction, fingerprinting, taxonomy, severity | `event.rs` (Event/EventBuilder/Severity), `taxonomy.rs` (Classifier, builtin categories), `enrich.rs`, `redact.rs`, `fingerprint.rs`, `buffer.rs` |
| **witslog-store** | SQLite schema, migrations, connections, write path, taxonomy store-layer | `migrate.rs` (M1-M4: init/resolved_at/dropped_counter/seed_taxonomy), `conn.rs` (pragma setup, WAL), `writer.rs` (event insert, fingerprint rollup), `taxonomy.rs` (store CRUD) |
| **witslog-config** | Layered config resolution, defaults, sections | `lib.rs` (Config struct + EnrichSection/RedactSection/BufferSection/TaxonomySection) |
| **witslog-query** | FTS5 search + structured filters + aggregates | `search.rs` (bm25 + keyset cursor), `filters.rs`, `aggregates.rs` (stats/timeline/top_failures), `correlate.rs` (edge walks) |
| **witslog-mcp** | MCP tool registry + JSON-RPC transport | `registry.rs` (all 12 tools), `transport.rs` (stdio JSON-RPC), `tools.rs` (schema) — wired into CLI as `witslog serve-mcp` |
| **witslog-cli** | CLI subcommands (init/log/get/query/stats/export/import/vacuum/prune/config/archive/backup/list-dbs/migrate/resolve/delete/doctor/category) | `main.rs` (clap Commands) |
| **witslog-ffi** | C ABI for embedding | `lib.rs` (witslog_log, witslog_resolve, witslog_delete, witslog_init/flush/shutdown) |
| **witslog-runtime** | Ambient "Provider" — mount-once init-guard, ambient capture, panic hook, `tracing` Layer, `Result::log_err`, shared enrich→redact→classify→write pipeline | `lib.rs` (init/Guard/arm/capture/build_and_write/LogErr/macros), `tracing_layer.rs` (feature `tracing`) |

**Not yet built**: witslog-plugin (extensibility, P9).

**SDK bindings** (P6, outside `crates/`): `bindings/python` (ctypes, 0 deps) + `bindings/node` (koffi) +
`bindings/php` (ext-ffi) — each a framework-agnostic core (`error/warn/info/exception/log`,
`init/flush/shutdown`, host-excepthook capture) plus thin adapters (`bindings/python/witslog/frameworks/{fastapi,django,flask}.py`,
`bindings/node/frameworks/express.js`, `bindings/php/src/Laravel/WitslogServiceProvider.php`).
Contract: **bindings/CONTRACT.md**. e2e/regression driver: `bindings/e2e/run.ps1` (workspace test
gate + per-language SDK↔CLI readback + argv-mitigation lock — see Gotchas).

## Specs & Docs

- **PLAN.md**: Authoritative design doc. Architecture, schema (§3), FTS design (§4), MCP spec (§5), crate map (§6), install/bootstrap (§7), roadmap (§8-10). Read first for "how should X work".
- **PHASES.md**: Detailed per-phase engineering. Each phase has EARS requirements, acceptance criteria, error-handling table, TODO checklist, verification recipe. Read before implementing a phase. **P6 (SDK bindings) shipped — specs at PHASES.md §P6**.
- **bindings/CONTRACT.md**: SDK↔native ABI contract (single source every language SDK marshals against) — exported functions, `witslog_log` JSON payload fields, ABI-version handshake (`witslog_abi_version`), native-lib locator order, DB resolution, mount/flush lifecycle, and the argv/secret-exposure security note. Read before touching `witslog-ffi` or any `bindings/*` SDK.
- **LAST_SESSION_PLAN.md** (if present): Refined implementation plan from prior session. May be more detailed than PHASES.md for the current phase.

## Conventions

### Migrations

- **File**: `crate/witslog-store/src/migrate.rs`. Numbered sequence `migrate_000N_name()`.
- **Idempotent**: All migrations use `IF NOT EXISTS` or guard with `pragma_table_info` check (see `migrate_0002_resolved_at`).
- **Pattern**: new migration → check `if current_version < N`, call private `migrate_000N_name()`, then `record_migration(N, "name")`.
- **Schema seeding**: e.g., builtin categories use `INSERT OR IGNORE` (see `migrate_0004_seed_taxonomy`).

### Event I/O

- **Builder**: fluent chain in `EventBuilder` (enrich → redact → classify → build).
- **Methods mirror siblings**: `.enrich(cfg)`, `.redact(redactor)`, `.classify(classifier)` all return `Self`.
- **Write path**: `EventWriter::write(&event) -> Result<i64>` (rowid); fingerprint auto-rollup.
- **Read path**: `query_by_id(event_id) -> Result<Option<Event>>`.

### Testing

**Every phase's implementation work must ship with unit tests, regression tests, and feature
tests before it's called done** — not just "it compiles" or "it looks right by inspection":
- **Unit**: pure-logic coverage in module `#[cfg(test)]` blocks (e.g., `crates/witslog-core/src/taxonomy.rs`, `crates/witslog-store/src/migrate.rs`).
- **Regression**: a test that pins a specific bug/edge-case that was fixed or a guard that was added (e.g. `migrate::tests::schema_newer_than_binary_is_refused`), named so a future revert would fail it.
- **Feature/integration**: `crates/witslog-store/tests/pN_integration.rs` (e.g., `p1_integration.rs`, `p2_integration.rs`) or, for CLI-surfaced behavior, `crates/witslog-cli/tests/pN_integration.rs` (e.g. `p8_integration.rs`) driving the real built binary via `std::process::Command` + `env!("CARGO_BIN_EXE_witslog")`. Mirrors phase.
- **Run**: `cargo test -p witslog-core taxonomy` (unit by module name); `cargo test --test p2_integration` (integration by file); `cargo test --workspace` before calling any phase done.
- **SDK bindings**: per-language unit tests (`bindings/python/tests` via `py -m pytest`, `bindings/node/test` via `node --test`, `bindings/php/tests` via `phpunit`) cover marshalling + the FFI error table (missing lib, ABI mismatch, write error). Cross-language regression: `bindings/e2e/run.ps1` — gates workspace `cargo test`, SDK→CLI readback (message+tags cross the ABI), and the argv-mitigation lock in one pipeline; run before calling P6 changes done.
- **Don't test-spawn the real dev binary destructively**: a test that would delete/rename `target/debug/witslog(.exe)` (e.g. exercising `uninstall`'s binary-removal path) breaks every other test/build sharing that binary. Copy it to a temp path first, or refactor the destructive bit into a pure helper and unit-test that instead (see `purge_data_dirs` / `uninstall_tests` in `witslog-cli/src/main.rs`).

### Changelog discipline

- **Update `CHANGELOG.md` in the same change that lands the feature/fix** — not as an afterthought at release time. Add entries under `## [Unreleased]`, grouped by `### Added` / `### Changed` / `### Fixed` / `### Removed` (Keep a Changelog format, already in use here).
- Write entries for a future reader who wasn't in this session: name the phase/FR-ID, the files/commands touched, and *why* if it isn't obvious (mirrors the style already in `[0.1.0]`).
- On an actual version cut, move `[Unreleased]` content under a new dated `## [x.y.z] — YYYY-MM-DD` heading rather than deleting history.

### Error Handling

- **Store**: `crate::error::Result<T>` (rusqlite wrapper). Domain-specific errors in `error.rs`.
- **Never log from library**: buffer drops errors silently + count; log only at CLI/MCP boundary.

## Phase Status

- ✅ **P0** (M1): Storage + event model, init/log/query/resolve/delete, migrations 1-3, FFI-core.
- 🟡 **P1**: Logging lib (enrich/redact/buffer) — core shipped, config sections added, buffer/enrich partially wired.
- ✅ **P2**: Taxonomy engine — builtin tree seeded, classifier rules, auto-classify wired into builder, migration 4, store CRUD, config section, integration tests.
- ✅ **P3**: FTS5 + query engine — migrate_0005_fts5 shipped, witslog-query crate (search/aggregates/filters/correlate), p3_integration tests.
- 🟡 **P4**: CLI ops — query/stats/export/import/vacuum/prune/migrate/config/archive/backup/list-dbs/category all shipped; missing global `--json` flag.
- ✅ **P5**: MCP server — witslog-mcp crate (all 12 tools, JSON-RPC stdio transport, schema validation, statement timeout), wired into CLI as `witslog serve-mcp [--stdio] [--attach] [--allow-write]`, conformance test (`p5_integration.rs`) green.
- ✅ **P6**: SDK bindings. Provider/runtime landed (`witslog-runtime`); C ABI extended additively with `context`/`tags`/`metadata` on `witslog_log` + a `witslog_abi_version()` handshake. Three framework-agnostic SDKs under `bindings/` — Python (`ctypes`, 0 deps) + FastAPI/Django/Flask, Node (`koffi`) + Express, PHP (`ext-ffi`) + Laravel provider — over the shared JSON contract (`bindings/CONTRACT.md`). Per-language unit tests + cross-language e2e (`bindings/e2e/run.ps1`, SDK→CLI readback) green.
- ✅ **P7**: Perf + hardening. Criterion bench suite (`bench/`: write throughput, buffered-log latency, search latency, FTS index-build cost), concurrency harness (`witslog-store/tests/p7_concurrency.rs`, 8 independent connections on one DB, zero loss + `integrity_check=ok`), load harness (`p7_load.rs`, scales via `WITSLOG_LOAD_TEST_ROWS`), memory script (`scripts/measure_memory.ps1`), CI (`.github/workflows/ci.yml`) with a bench-regression gate (`scripts/check_bench_regression.ps1`). Docs: `docs/perf.md`.
- 🟡 **P8**: Packaging + install. Version-compat guard (`witslog-store::CURRENT_SCHEMA_VERSION`, refuses newer-than-binary schema), `serve-mcp --print-mcp-config`, `uninstall [--purge]`, migrate `.bak` restore-on-failure, install scripts (`install/install.sh`/`.ps1`), release workflow with a `smoke_test` job (`.github/workflows/release.yml`: builds per OS, runs the real init/log/query/serve-mcp/doctor/uninstall happy path, gates `publish`), Homebrew/Scoop manifest templates, `docs/install.md`. winget/`.deb`/`.rpm` intentionally out of scope — `cargo install`/npm/pip/composer already cover cross-platform distribution pre-1.0. Workflow confirmed green via `workflow_dispatch` on real Linux/macOS/Windows runners (build matrix + smoke_test). Missing: cutting a real `v*.*.*` tag to exercise `publish`.
- ⬜ **P9**: Extensibility + security (plugins, encryption, audit).

**Critical path**: P0 → P3 → P5 → P8. P1/P2 parallelizable.

## Next Steps

1. **P4**: add global `--json` output flag.
2. **P8**: packaging + install (cross-compile, install scripts, MCP registration) — see PHASES.md §P8.

## Dev Workflow

- **Build**: `cargo build --workspace` (or `cargo build -p witslog-cli` for just CLI).
- **Test**: `cargo test` (all), `cargo test -p witslog-core` (one crate), `cargo test --test p2_integration` (one file).
- **Lint**: `cargo clippy --workspace`.
- **Example**: `cargo run --example p2_classify` (examples in `examples/`, built via `[[example]]` in workspace Cargo.toml).
- **Manual test**: `witslog init . && witslog log app "msg" && witslog query <id>`.
- **Bench**: `cargo bench -p witslog-bench` (see `docs/perf.md`); memory: `pwsh scripts/measure_memory.ps1 -Release`.

## Key Types & Paths

| Type | Path | Notes |
|------|------|-------|
| `Event` | witslog-core/event.rs:36 | full event row |
| `EventBuilder` | witslog-core/event.rs:62 | fluent builder; call `.build()` to create Event |
| `Classifier` | witslog-core/taxonomy.rs:89 | rules engine; use `.built_in()` for defaults |
| `CategoryNode` | witslog-core/taxonomy.rs:5 | tree node (canonical/parent/label) |
| `Severity` | witslog-core/event.rs:5 | enum: Trace..Fatal; rank 10-70 |
| `Store` | witslog-store/lib.rs | connection pool + schema version guard |
| `EventWriter` | witslog-store/writer.rs | main write interface |
| `Config` | witslog-config/lib.rs | layered config struct (fields: enrich/redact/buffer/taxonomy) |

## Gotchas

- **Fingerprint is deterministic**: same message+exception+stack_norm+category always produces same hash. Used for dedup.
- **Migrations are idempotent**: re-running `migrate()` is safe — all `INSERT OR IGNORE`, guards on table existence.
- **Builder methods mutate `self`**: fluent chain always legal; `build()` consumes builder.
- **Tags are merged in classify()**: suggested tags from rule appended to existing tags, not replaced.
- **Auto-classify respects explicit category**: if `EventBuilder.category(...)` already set, `.classify()` skips classification.
- **Redaction is applied pre-build**: message/exception/stacktrace/context/metadata all redacted in `.redact()`.
- **FTS5 live**: `events_fts` created + triggered by `migrate_0005_fts5`; query via `witslog-query::SearchEngine`.
- **P5 wiring merged 2026-07-12**: the agent worktree that had `serve-mcp` CLI wiring was committed and fast-forward merged into `feat/P5-MCP-tooling`, then removed — no longer a separate worktree.
- **Provider is additive**: `witslog-runtime` layers on top of `EventBuilder`/`witslog_log` — none of the existing APIs changed. `witslog_runtime::build_and_write(config, db_path, builder)` is the single home for the enrich→redact→classify→write pipeline (CLI `log_event` and the ambient `capture` both route through it; don't re-inline it).
- **Panic capture writes synchronously**: the runtime panic hook uses a sync write (never the async buffer) because a panic may precede process abort. It also chains to the previously-installed hook, and is installed at most once per process.
- **FFI has no `Drop`**: buffered events need an explicit `witslog_flush`/`witslog_shutdown` before exit (SDK atexit). The Rust `Guard` from `init()`/`init_default()` flushes automatically on drop.
- **`tracing` Layer is Rust-only + feature-gated**: `witslog-runtime`'s `WitslogLayer` lives behind the `tracing` feature and does not cross the C ABI. Cross-language SDKs capture host-language exceptions themselves.
- **argv enrichment defaults on and can leak CLI-arg secrets**: `EnrichConfig::default().argv == true` captures the full process command line into `context.argv`; redaction (`redact_json`) recurses into it but only catches pattern-matched secrets (Bearer/api_key/password/AWS_*/conn-strings), not an arbitrary secret passed as a bare CLI arg. Apps that may receive secrets that way must pass `{"enrich":{"argv":false}}` to `witslog_init`/`witslog_configure` (or the SDK's `init(config)`). Proven to fully suppress argv end-to-end (native + all 3 SDKs) by `configure_argv_false_suppresses_argv_capture` in `witslog-ffi/src/lib.rs` and the matching per-SDK regression test; documented in **bindings/CONTRACT.md** ("Security note").
- **ABI is versioned**: `witslog_abi_version()` returns `WITSLOG_ABI_VERSION` (currently `1`, in `witslog-ffi/src/lib.rs`). Every SDK core checks it at load time and raises `WitslogContractError` on mismatch — bump the constant on any breaking change to the `witslog_log`/`witslog_configure` JSON payloads and update **bindings/CONTRACT.md** in the same change.

---

Last updated: 2026-07-17. Reflect code reality, not aspirational state.
