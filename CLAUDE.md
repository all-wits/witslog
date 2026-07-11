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
| **witslog-cli** | CLI subcommands (init/log/query/resolve/delete/doctor) | `main.rs` (clap Commands) |
| **witslog-ffi** | C ABI for embedding | `lib.rs` (witslog_log, witslog_resolve, witslog_delete) |

**Not yet built**: witslog-query (FTS + aggregates), witslog-mcp (server), witslog-plugin (extensibility).

## Specs & Docs

- **PLAN.md**: Authoritative design doc. Architecture, schema (§3), FTS design (§4), MCP spec (§5), crate map (§6), install/bootstrap (§7), roadmap (§8-10). Read first for "how should X work".
- **PHASES.md**: Detailed per-phase engineering. Each phase has EARS requirements, acceptance criteria, error-handling table, TODO checklist, verification recipe. Read before implementing a phase. **Current phase: P2 (taxonomy) — specs at PHASES.md §P2**.
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
- ✅ **P2** (current): Taxonomy engine — builtin tree seeded, classifier rules, auto-classify wired into builder, migration 4, store CRUD, config section, integration tests.
- ⬜ **P3**: FTS5 + query engine (search/stats/timeline).
- ⬜ **P4**: CLI ops (export/import/prune/migrate/backup).
- ⬜ **P5**: MCP server (tools over JSON-RPC).
- ⬜ **P6**: SDK bindings (Python, Node).
- ⬜ **P7**: Perf + hardening (benches, concurrency).
- ⬜ **P8**: Packaging + install (cross-compile, release).
- ⬜ **P9**: Extensibility + security (plugins, encryption, audit).

**Critical path**: P0 → P3 → P5 → P8. P1/P2 parallelizable.

## Next Steps

After P2 ships:
1. **CLI integration**: wire auto_classify_enabled from config → ClassifierBuilder in CLI log command.
2. **P3**: FTS5 migration (migrate_0005), query engine crate, bm25 ranking, keyset pagination.
3. **P4**: CLI query/stats/export/import subcommands.
4. **P5**: MCP server.

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
- **No FTS yet**: P3 will add; for now schema has `events_fts` placeholder, not yet triggered.

---

Last updated: 2026-07-11. Reflect code reality, not aspirational state.
