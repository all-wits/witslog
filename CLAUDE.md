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
| **witslog-mcp** | MCP tool registry + JSON-RPC transport | `registry.rs` (all 12 tools), `transport.rs` (stdio JSON-RPC), `tools.rs` (schema) — built, **not yet wired to CLI** |
| **witslog-cli** | CLI subcommands (init/log/get/query/stats/export/import/vacuum/prune/config/archive/backup/list-dbs/migrate/resolve/delete/doctor/category) | `main.rs` (clap Commands) |
| **witslog-ffi** | C ABI for embedding | `lib.rs` (witslog_log, witslog_resolve, witslog_delete) |

**Not yet built**: witslog-plugin (extensibility, P9).

## Specs & Docs

- **PLAN.md**: Authoritative design doc. Architecture, schema (§3), FTS design (§4), MCP spec (§5), crate map (§6), install/bootstrap (§7), roadmap (§8-10). Read first for "how should X work".
- **PHASES.md**: Detailed per-phase engineering. Each phase has EARS requirements, acceptance criteria, error-handling table, TODO checklist, verification recipe. Read before implementing a phase. **Current phase: P6 (SDK bindings) — specs at PHASES.md §P6**.
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

- **Unit**: in module `#[cfg(test)]` blocks (e.g., `crates/witslog-core/src/taxonomy.rs`).
- **Integration**: `crates/witslog-store/tests/pN_integration.rs` (e.g., `p1_integration.rs`, `p2_integration.rs`). Mirrors phase.
- **Run**: `cargo test -p witslog-core taxonomy` (unit by module name); `cargo test --test p2_integration` (integration by file).

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
- ⬜ **P6** (current): SDK bindings (Python, Node).
- ⬜ **P7**: Perf + hardening (benches, concurrency).
- ⬜ **P8**: Packaging + install (cross-compile, release).
- ⬜ **P9**: Extensibility + security (plugins, encryption, audit).

**Critical path**: P0 → P3 → P5 → P8. P1/P2 parallelizable.

## Next Steps

1. **P4**: add global `--json` output flag.
2. **P6**: SDK bindings (Python, Node) over the C ABI.

## Dev Workflow

- **Build**: `cargo build --workspace` (or `cargo build -p witslog-cli` for just CLI).
- **Test**: `cargo test` (all), `cargo test -p witslog-core` (one crate), `cargo test --test p2_integration` (one file).
- **Lint**: `cargo clippy --workspace`.
- **Example**: `cargo run --example p2_classify` (examples in `examples/`, built via `[[example]]` in workspace Cargo.toml).
- **Manual test**: `witslog init . && witslog log app "msg" && witslog query <id>`.

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

---

Last updated: 2026-07-12. Reflect code reality, not aspirational state.
