# witslog — Performance & Hardening (P7)

Companion to PHASES.md §P7. Documents the bench suite, concurrency/load harness,
memory measurement, and how the CI regression gate works.

## Bench suite (`bench/`)

Criterion benches, run via `cargo bench -p witslog-bench`:

| Bench | File | Measures | Target |
|---|---|---|---|
| `write_throughput` | `benches/write_throughput.rs` | single-insert cost + 1000-row single-transaction batch cost | ≥ 10k events/s single-writer |
| `buffered_log_latency` | `benches/buffered_log_latency.rs` | caller-thread cost of `AsyncBuffer::enqueue()` | < 100 µs |
| `search_latency` | `benches/search_latency.rs` | FTS5 `search()` first-page latency on a 5k-row seeded corpus | < 50 ms at 100k rows (see below for the full-scale run) |
| `index_build_cost` | `benches/index_build_cost.rs` | `events_fts` backfill cost (the `migrate_0005_fts5` path) for 2000 rows | informational — no hard target, tracked for regression |

Run: `cargo bench -p witslog-bench`. HTML reports land in `target/criterion/`.

**100k-row search measurement (manual, not run every CI cycle):** bump
`SEED_COUNT` in `search_latency.rs` to `100_000` and re-run locally; on
commodity SSD this should still return the first page in well under 50 ms
because the query is index/FTS-covered (see `EXPLAIN QUERY PLAN` coverage in
`p3_integration.rs`). Left at 5k by default so CI stays fast.

## Regression gate (FR-P7-002)

`scripts/check_bench_regression.ps1` compares each bench's Criterion `mean_ns`
against a committed baseline in `bench/baseline/*.json` and fails if any
mean regressed more than 20%.

- CI job `bench-regression` runs `cargo bench -p witslog-bench -- --save-baseline ci`
  then the check script.
- No baseline is committed yet for a fresh bench — the script prints
  "No baseline yet" and passes (non-blocking) until one exists.
- To create/refresh baselines after an intentional perf change:
  ```
  cargo bench -p witslog-bench -- --save-baseline ci
  pwsh scripts/check_bench_regression.ps1 -UpdateBaseline
  git add bench/baseline
  ```

## Concurrency (FR-P7-003)

`crates/witslog-store/tests/p7_concurrency.rs`: 8 threads, each opening its
**own** `DbConnection` (own SQLite connection/handle) against the same project
DB file — the same topology as multiple OS processes sharing one project DB,
since contention happens at the SQLite connection/file level, not the Rust
process level. Each writer inserts 200 events independently.

Serialization relies on the pragmas already set on every connection
(`conn.rs::setup_pragmas`): `journal_mode=WAL` (concurrent readers, single
writer at a time per SQLite's own locking) + `busy_timeout=5000` (SQLite's own
bounded wait-and-retry when a writer collides with another writer holding the
write lock — this *is* the "bounded retry" required by FR-P7-003; no
additional retry loop was needed in application code because `busy_timeout`
already implements it at the driver level).

Assertions: total row count == writers × events-per-writer (no silent loss),
and `PRAGMA integrity_check` returns `ok` (no corruption).

## Load test (FR-P7-004)

`crates/witslog-store/tests/p7_load.rs`: inserts `WITSLOG_LOAD_TEST_ROWS`
(default 20,000, batched 400/transaction) events, marks half resolved, then
times `delete_resolved` (prune), `PRAGMA wal_checkpoint(TRUNCATE); VACUUM;`,
and a post-checkpoint file copy (backup — the same strategy PLAN.md §3
documents as the alternative to the SQLite online backup API). Asserts
`integrity_check = ok` throughout.

To run at the documented 1M-row scale (not part of default CI — takes
several minutes):
```
$env:WITSLOG_LOAD_TEST_ROWS = "1000000"
cargo test -p witslog-store --test p7_load -- --nocapture
```
Record the printed insert/prune/vacuum/backup timings here after a run on
target hardware; no numbers are hardcoded into the test because absolute
wall-clock time is machine-dependent — the test's own bound (insert phase
< 60s at the default 20k scale) is the CI-enforced smoke check.

## Memory footprint (FR-P7-005)

`scripts/measure_memory.ps1` builds `witslog-cli`, inits a scratch project,
and reports:
- peak working set of one `witslog log` invocation (target < 15 MB)
- working set of `witslog serve-mcp --stdio` two seconds after launch, idle
  (target < 30 MB)

Run: `pwsh scripts/measure_memory.ps1 -Release` (release binary gives the
representative number; omit `-Release` for a quick debug-build check).
Not wired into CI (process/measurement tooling differs too much across
GitHub-hosted OS images to give a stable pass/fail); run manually per-OS
before a release and record results here:

| OS | CLI one-shot | Idle serve-mcp |
|---|---|---|
| (fill in after a manual run) | | |

## Acceptance criteria mapping

| PHASES.md acceptance criterion | Verified by |
|---|---|
| bench suite reports write/read/index numbers, fails CI on >20% regression | `bench/`, `scripts/check_bench_regression.ps1`, CI job `bench-regression` |
| N concurrent writers, all events persist, `integrity_check = ok` | `p7_concurrency.rs` |
| 1M events, prune/archive/backup complete within documented bounds, DB stays consistent | `p7_load.rs` (scale via `WITSLOG_LOAD_TEST_ROWS`) + this doc's timing table once run at scale |
