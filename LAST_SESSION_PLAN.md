# P1 — Logging Library Completion

## Context

`witslog` = SQLite-backed structured error logging framework, spec in `PLAN.md`/`PHASES.md`. P0 shipped (event model, schema v1+v2, WAL conn, write/resolve/prune, CLI, C-ABI FFI) — confirmed via codegraph audit matching PHASES.md exactly (11 files, no enrich/redact/buffer/taxonomy/query/mcp code exists yet). Branch `feat/P1-logging-library-completion` targets P1: make embedding low-friction and safe — auto-capture runtime context, strip secrets/PII before write, buffer writes off the hot path, add severity ergonomics. This is the P1 TODO checklist in full.

## Architecture decisions

**Buffer↔Store.** `witslog-core` stays store-agnostic: `buffer.rs` defines `Sink` trait (`write_batch(&[Event]) -> Result<(), SinkError>`); `witslog-store` implements it (`StoreSink`) using the **already-existing** `DbConnection::transaction()` (crates/witslog-store/src/conn.rs:46) — no new transaction plumbing needed. `AsyncBuffer<S: Sink>` uses `std::sync::mpsc` + a plain `std::thread` (matches crate's sync style; no tokio in this path). Callers (FFI/CLI) own the buffer + choose the Sink.

**Dropped counter.** In-process `AtomicU64` on `AsyncBuffer` (source of truth during process life) mirrored best-effort into a new DB table `runtime_stats(key,value)` (migration `migrate_0003_dropped_counter`) so `witslog doctor` in another process can see it. `EventWriter::bump_dropped`/`dropped_count` added.

**Enrich/redact wiring.** New `EventBuilder::enrich(self, &EnrichConfig) -> Self` and `.redact(self, &Redactor) -> Self` — opt-in chain calls inserted before `.build()` at call sites (FFI, CLI). `.build()` itself unchanged, so no existing caller breaks.

**FFI runtime config.** `witslog_configure(json)` sets a process-wide `OnceLock<Mutex<RuntimeConfig>>` (cached compiled `Arc<Redactor>`, not raw patterns). `witslog_log` reads it each call; unbuffered path unchanged if `configure` never called.

**Decided:** buffered `witslog_log` returns `0` (queued, not a row id) when buffering enabled — document as ABI note. Hostname enrichment uses `hostname = "0.4"` crate (add to `[workspace.dependencies]` and `witslog-core/Cargo.toml`).

## New files

- `crates/witslog-core/src/enrich.rs` — `EnrichConfig{hostname,pid,cwd,argv,git_commit,env_allowlist}` (all bool default true, allowlist empty), `enrich(builder, &cfg) -> EventBuilder` merging into `context` (existing keys win). Each field independently best-effort (`.ok()`/`and_then`, never short-circuits others). `git_commit`: read `.git/HEAD` by walking up like `resolve_project_db` (crates/witslog-config/src/lib.rs:63), no subprocess spawn.
- `crates/witslog-core/src/redact.rs` — `Redactor{rules}`, `Redactor::new(&[String]) -> Result<Self, RedactError>` (compiles built-ins + custom), `Redactor::built_in()`, `.redact(&str)->String`, `.redact_json(&mut Value)` (recursive). Built-ins: Bearer token, api_key/apikey, password, `AWS_*`/AKIA keys, `user:pass@host` connection strings — replace captured secret only, keep prefix (`Authorization: Bearer «redacted»`). Add `regex = "1"` workspace dep.
- `crates/witslog-core/src/buffer.rs` — `Sink` trait, `SinkError`, `BufferConfig{enabled,batch_size=50,flush_interval_ms=1000,queue_capacity=1024}`, `AsyncBuffer<S: Sink>` (bounded `mpsc::SyncSender`, background thread, `catch_unwind` per flush iteration, retry-once-then-drop+count on write error, `Drop` does final flush), `SyncSink<S>` fallback for buffer-disabled mode.
- `crates/witslog-store/src/sink.rs` — `StoreSink{store}` implementing `Sink` via `store.conn().transaction(|conn| { for e in events { write_event(conn, e)? } Ok(()) })`. Requires extracting `EventWriter::write`'s SQL body (crates/witslog-store/src/writer.rs) into `pub(crate) fn write_event(conn: &Connection, event: &Event) -> Result<i64>` reused by both single-write and batch paths.

## Edits

- `crates/witslog-core/src/event.rs` — free fns `error/warn/info` (severity presets) and `exception(app, &dyn Error)` (captures `.to_string()` + `source()` chain into stacktrace, reuses existing `normalize_stacktrace` digit-masking → `stack_norm`). `EventBuilder::enrich()`/`.redact()` wrappers.
- `crates/witslog-core/src/lib.rs` — `pub mod enrich; pub mod redact; pub mod buffer;` + re-exports.
- `crates/witslog-store/src/writer.rs` — extract `write_event` helper; add `bump_dropped(&self, n: u64)`, `dropped_count(&self) -> Result<u64>`.
- `crates/witslog-store/src/migrate.rs` — `migrate_0003_dropped_counter` (idempotent, mirrors `migrate_0002_resolved_at` pattern at line 157): `CREATE TABLE IF NOT EXISTS runtime_stats(key TEXT PRIMARY KEY, value INTEGER NOT NULL)` seeded `dropped_events=0`.
- `crates/witslog-store/src/lib.rs` — `pub mod sink; pub use sink::StoreSink;`.
- `crates/witslog-config/src/lib.rs` — add `EnrichSection`, `RedactSection{custom_patterns}`, `BufferSection` to `Config` (`#[serde(default)]`), `Config::load_from_file(path) -> Result<Config, ConfigError>` (toml parse; new — no file loading exists today). Config crate stays leaf (no witslog-core dep) — callers build `witslog_core::EnrichConfig`/`Redactor` from these sections.
- `crates/witslog-ffi/src/lib.rs` — `witslog_configure(json_ptr) -> i32` (0 ok, -1 parse err, -2 invalid redact regex, validated via `Redactor::new` before committing). `witslog_log` reads `RUNTIME_CONFIG`, inserts `.enrich().redact()` before `.build()`; if buffer enabled, lazily-init `OnceLock<AsyncBuffer<StoreSink>>`, enqueue, return `0`; else unchanged synchronous path.
- `crates/witslog-cli/src/main.rs` — `log_event()`: load config sections, insert `.enrich().redact()` before `.build()`. `doctor()`: print `writer.dropped_count()`.
- Root `Cargo.toml`: add `regex = "1"`, `hostname = "0.4"` to `[workspace.dependencies]`.

## Sequencing

1. witslog-core: redact.rs → enrich.rs → event.rs edits → buffer.rs
2. witslog-store: writer.rs refactor (write_event, bump_dropped/dropped_count) → migrate.rs (0003) → sink.rs → lib.rs re-exports
3. witslog-config: sections + load_from_file
4. witslog-ffi: witslog_configure, wire into witslog_log
5. witslog-cli: wire log_event + doctor
6. Tests (below)

## Tests

- `redact.rs`: fixture table per built-in pattern + custom pattern + invalid-regex-rejects.
- `enrich.rs`: git repo present → `context.git_commit` set; no repo → graceful omission; field disabled → omission.
- `buffer.rs`: mock `Sink`, assert 50 enqueues → exactly 1 `write_batch(len=50)`; `Drop` flushes partial batch; `Sink` erroring twice → `dropped_count == batch.len()`, no panic.
- `sink.rs` / new `tests/p1_buffer_integration.rs`: `StoreSink`+`AsyncBuffer` against real tempdir `Store`, one-transaction batch write.
- `witslog-ffi` tests (extend existing `with_tmp_cwd` pattern): `witslog_configure` custom pattern → `witslog_log` → stored message redacted; read-only DB (`#[cfg(unix)]` chmod, mirrors `init_db`'s existing perms pattern) → `witslog_log` returns -1, no panic, dropped counter increments if buffered.
- New `tests/p1_integration.rs` (mirrors `tests/m1_integration.rs` style) covering acceptance criteria end-to-end.

## Verification

In a git repo: `witslog log app "token=Bearer xyz"` → `witslog query <id>` shows `«redacted»` token + populated `git_commit`/`hostname`/`pid`/`cwd`. `cargo test` green across all crates including new fixture/integration tests. Buffer test: enqueue 50 events via FFI test harness → assert exactly one transaction commits all 50, `witslog_log` returns 0 for each queued call. Read-only DB path: `witslog_log` doesn't panic, `doctor` shows nonzero dropped count.