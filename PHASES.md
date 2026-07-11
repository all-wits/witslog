# witslog — Implementation Phases

Detailed, executable per-phase engineering plan derived from `PLAN.md`. Each phase is
self-contained: EARS functional requirements, non-functional requirements,
Given/When/Then acceptance criteria, an error-handling table, a file-level TODO
checklist, and an end-to-end verification recipe.

## How to read

- **EARS** requirement patterns: *ubiquitous* (`shall`), *event* (`When … shall`),
  *state* (`While … shall`), *conditional* (`While … when … shall`), *optional*
  (`Where … shall`).
- Requirement IDs: `FR-<PHASE>-NNN`. Reference them in commits + tests.
- Reuse existing code shapes — new store methods mirror `EventWriter.write` /
  `delete_resolved`; new migrations mirror `migrate_0002_resolved_at` (versioned,
  idempotent, `pragma_table_info` guard); CLI subcommands mirror the `clap` `Commands`
  enum + `fn xxx(...)` handlers in `main.rs`; FFI exports mirror `witslog_log`
  (`cstr → JSON → core → result`, `#[no_mangle] extern "C"`).

## Status legend

- ✅ done — shipped + covered
- 🟡 partial — some capability landed, remainder specified below
- ⬜ todo — not started

## Dependency DAG

```
P0 (baseline) ─┬─> P1 (logging lib)      ─┬─> P6 (SDK bindings)
               ├─> P2 (taxonomy)          │
               └─> P3 (FTS + query) ──────┼─> P4 (CLI utils) ──> P8 (packaging)
                                          └─> P5 (MCP server) ─┘
P1,P2,P3,P4,P5 ──> P7 (perf/hardening)
P1..P6 ─────────> P9 (extensibility/security)
```
Critical path: **P0 → P3 → P5 → P8**. P1/P2 parallelizable. P7/P9 follow the core.

---

## P0 — DONE baseline

**Objective.** Establish SQLite-backed structured logging with per-project isolation:
initialise a DB, log a structured event, retrieve it, resolve it, prune it, and embed the
write path in any language via a C ABI. This is the foundation every later phase builds on.

**Status.** ✅ done (M1 + schema v2 + resolve/prune + FFI-core). Audited against source.

**Dependencies.** None.
**Complexity.** — (shipped).

**Shipped capability (audit).**
| Capability | Where | Notes |
|---|---|---|
| Event model + fluent builder | `witslog-core/src/event.rs` (`Event`, `EventBuilder`, `Severity`) | UUIDv7 id, deterministic fingerprint, stack normalization |
| Schema v1 + WAL + 12 indexes | `witslog-store/src/migrate.rs` (`migrate_0001_init`) | STRICT tables, generated JSON columns, fingerprints/edges satellites |
| Schema v2 resolved_at | `migrate_0002_resolved_at` | `resolved_at` col + partial `ix_events_unresolved` |
| Connection + pragmas | `witslog-store/src/conn.rs` (`DbConnection`) | WAL, NORMAL sync, busy_timeout 5s, mmap 256MB |
| Write + fingerprint rollup | `witslog-store/src/writer.rs` (`EventWriter.write`, `update_fingerprint`) | upsert `fingerprints` count |
| Query by id | `EventWriter.query_by_id` | full row hydrate incl. `resolved_at` |
| Resolve + prune | `mark_resolved`, `delete_resolved`, `DeleteFilter` | filter by event_id/fingerprint/resolved_before/force |
| DB resolution | `witslog-config/src/lib.rs` (`resolve_project_db`, `resolve_global_db`) | walk-up like `.git`; global fallback |
| CLI | `witslog-cli/src/main.rs` | `init`, `log`, `query`, `resolve`, `delete`, `doctor` |
| C ABI FFI | `witslog-ffi/src/lib.rs` | `witslog_log`, `witslog_resolve`, `witslog_delete`, `witslog_free_string` |

**Acceptance criteria (already passing).**
- Given an empty dir, when `witslog init .` runs, then `.witslog/witslog.db` is created
  with the full schema and WAL enabled.
- Given an initialised project, when an event is logged and queried by id, then the stored
  fields round-trip exactly.
- Given two identical errors, when both are logged, then they share one fingerprint and the
  `fingerprints.count` increments to 2.
- Given a logged event, when resolved then deleted via `DeleteFilter`, then it and its
  edges are removed and the row count drops.

**Error handling (in place).**
| Condition | Detection | Response |
|---|---|---|
| `log` with no project + no `--db` | parent-dir existence check | error "Database not initialized. Run 'witslog init'…" |
| Migration re-run | `schema_version` compare | idempotent, no duplicate DDL |
| Missing event on `query` | `QueryReturnedNoRows` | prints "Event not found" |

**TODO checklist.** — none (baseline). Follow-ups tracked in later phases.

**Verification.** `witslog init . && witslog log app "msg" && witslog query <id> &&
witslog doctor` — all succeed; `cargo test` green (m1_integration + ffi roundtrip).

---

## P1 — Logging library completion

**Objective.** Make embedding low-friction and safe: auto-capture runtime context,
strip secrets/PII before write, buffer writes off the hot path, and expose ergonomic
severity helpers — so an app gets rich, safe error records with one call.

**Status.** ✅ done. Core builder + FFI write shipped (P0). Enrichment (`enrich.rs`) and
redaction (`redact.rs`) wired into CLI `log_event`. Async buffer (`buffer.rs`:
`AsyncBuffer`/`StoreSink`) wired into CLI, gated by `buffer.enabled`; disabled path writes
synchronously via `EventWriter`, enabled path enqueues and flushes on drop (short-lived CLI
process joins the flush thread before exit). Severity ergonomics (`error`/`warn`/`info`/etc.)
shipped in P0.

**Dependencies.** P0. Feeds P6 (SDKs surface these features).
**Complexity.** M.

**Functional requirements (EARS).**
- **FR-P1-001** (event): When an event is built without an explicit hostname/pid/cwd, the
  system shall auto-populate `context` with `hostname`, `pid`, `cwd`, `argv`, and
  `git_commit` (when a repo is detected).
- **FR-P1-002** (optional): Where enrichment for a field is disabled in config, the system
  shall omit that field rather than capturing it.
- **FR-P1-003** (ubiquitous): The system shall redact values matching built-in secret
  patterns (API keys, bearer tokens, passwords, `AWS_*`, connection strings) in `message`,
  `context`, and `metadata` before persistence, replacing with `«redacted»`.
- **FR-P1-004** (optional): Where custom redaction rules (regex list) are configured, the
  system shall apply them in addition to the built-ins.
- **FR-P1-005** (state): While an async buffer is enabled, when an event is logged, the
  system shall enqueue it and return without blocking on I/O, flushing in batches.
- **FR-P1-006** (event): When the process exits or the buffer reaches its batch size, the
  system shall flush pending events in a single transaction.
- **FR-P1-007** (ubiquitous): The system shall never propagate a logging failure to the
  host application; on write error it shall drop the event and increment a dropped counter.
- **FR-P1-008** (ubiquitous): The system shall provide `error`, `warn`, `info`, and
  `exception` convenience constructors mapping to the correct `Severity`.
- **FR-P1-009** (event): When `exception(err)` is called with a language error carrying a
  stack trace, the system shall capture and normalize `stacktrace` into `stack_norm`.

**Non-functional.**
- Buffered `log` call shall add < 100 µs on the caller thread (bench in P7).
- Redaction shall be applied deterministically and be unit-testable against a fixture set.
- Enrichment shall degrade gracefully — a missing `git` or unreadable `cwd` never errors.
- Zero unsafe data at rest: no un-redacted secret in any persisted column after P1.

**Acceptance criteria.**
- Given a repo checkout, when an event is logged, then `context.git_commit` equals the
  current `HEAD` short SHA and `context.hostname/pid/cwd` are populated.
- Given a message containing `Authorization: Bearer abc.def.ghi`, when logged, then the
  stored message shows `Authorization: Bearer «redacted»`.
- Given async buffering on with batch size 50, when 50 events are logged, then exactly one
  insert transaction runs and the caller never blocked on disk.
- Given the DB path is read-only, when logging, then the host call returns normally and the
  dropped counter increments (no panic, no error surfaced).
- Given `witslog.exception(e)` in Python/Rust, when `e` has a traceback, then `stacktrace`
  is stored and `stack_norm` has digits masked.

**Error handling.**
| Condition | Detection | Response | Code |
|---|---|---|---|
| Enricher failure (git/cwd) | per-enricher `Result` | skip that field, continue | — |
| Redaction regex invalid (custom) | compile at config load | reject config with named error | exit 2 |
| Buffer full + flush failing | flush `Result` err | drop batch, bump `dropped`, `doctor` surfaces | — |
| Write error on flush | rusqlite err | retry once (busy) then drop | — |

**TODO checklist.**
- [ ] `witslog-core/src/enrich.rs`: `enrich(builder, &EnrichConfig)` capturing hostname
      (`hostname` crate/std), pid, cwd, argv, env allow-list, git HEAD (spawn `git` or read
      `.git/HEAD`). Called from `EventBuilder::build` or an explicit `.enriched()`.
- [ ] `witslog-core/src/redact.rs`: built-in pattern set + `Redactor{rules}`; apply to
      message/context/metadata strings pre-build. Fixture-driven tests.
- [ ] `witslog-core/src/buffer.rs`: `Sink` trait; `AsyncBuffer{batch_size, flush_interval}`
      with background flush thread + `Drop` flush; sync fallback.
- [ ] `witslog-core/src/event.rs`: add `error/warn/info/exception` free fns or
      `EventBuilder` presets; `exception()` extracts trace.
- [ ] Extend `witslog-config`: `[enrich]`, `[redact]`, `[buffer]` sections + defaults.
- [ ] FFI: add `witslog_configure(json)` to set enrich/redact/buffer at runtime.
- [ ] Tests: enrichment presence, redaction fixtures, buffer batching + exit flush, drop-on-error.

**Verification.** In a git repo: `witslog log app "token=Bearer xyz"` → `witslog query
<id>` shows redacted token + populated `git_commit`/`hostname`. Kill process mid-buffer →
pending events flushed on exit (assert count).

---

## P2 — Taxonomy engine

**Objective.** Give AI assistants and queries a stable classification axis: seed a builtin
error hierarchy, support aliases/custom categories/tags, and deterministically auto-classify
raw errors — without any model or embedding.

**Status.** ✅ done. Builtin taxonomy seeded at init via `migrate_0004`, classifier rules engine wired into EventBuilder, store CRUD for custom categories/aliases, config section for auto-classify, integration tests (8 tests, all green). CLI (`witslog log`) wired: `--error-code`/`--exception` flags, auto-classify gated by `taxonomy.auto_classify_enabled`, explicit `--category` still overrides — verified end-to-end through the built binary.

**Dependencies.** P0. Independent of P1/P3 (parallelizable). Feeds P5 (`classify_error`,
`list_categories`).
**Complexity.** S–M.

**Functional requirements (EARS).**
- **FR-P2-001** (event): When a DB is initialised or migrated, the system shall seed the
  builtin category hierarchy (infrastructure/application/runtime/external + documented
  leaves) into `categories` with `builtin=1`.
- **FR-P2-002** (ubiquitous): The system shall represent categories hierarchically via the
  `parent` self-reference, canonical form `a.b.c` (e.g. `infrastructure.network.dns`).
- **FR-P2-003** (optional): Where a user defines a custom category, the system shall insert
  it with `builtin=0` and reject a canonical that collides with a builtin.
- **FR-P2-004** (event): When an alias is registered, the system shall map it to an existing
  canonical in `category_aliases` and resolve aliases to canonicals on lookup.
- **FR-P2-005** (conditional): While auto-classify is enabled, when an event lacks an
  explicit `category`, the system shall apply ordered rules (error_code map → exception-name
  map → message keyword/regex) and assign the first match's canonical.
- **FR-P2-006** (event): When classification runs, the system shall be deterministic and
  return the matched rule id(s) and suggested tags for explainability.
- **FR-P2-007** (state): While no rule matches, the system shall leave `category` null and
  tag the event `unclassified`.

**Non-functional.**
- Classification shall be a pure function of (message, exception, error_code, rules) — no
  I/O, no randomness — so results are reproducible and testable.
- Rule evaluation shall be O(rules) and complete in < 50 µs for a typical event.
- The builtin taxonomy shall be data (embedded table/const), editable without code changes.

**Acceptance criteria.**
- Given a fresh init, when categories are listed, then the full builtin tree is present and
  `list_categories` can render it as a tree.
- Given an event with exception `ETIMEDOUT`, when auto-classified, then category resolves to
  `infrastructure.network.timeout` (or documented leaf) with the matching rule id returned.
- Given alias `dns_error → infrastructure.network.dns`, when an event tagged `dns_error` is
  classified, then it resolves to the canonical.
- Given a message with no matching rule, when classified, then category is null and tag
  `unclassified` is added — deterministically across runs.

**Error handling.**
| Condition | Detection | Response | Code |
|---|---|---|---|
| Custom category collides with builtin | unique canonical check | reject with name of conflict | exit 2 |
| Alias targets unknown canonical | FK / lookup | reject "unknown canonical" | exit 2 |
| Invalid rule regex | compile at load | reject config, name rule | exit 2 |
| Cyclic parent chain | walk-up depth guard | reject seed/insert | exit 2 |

**TODO checklist.**
- [x] `witslog-core/src/taxonomy.rs`: `Classifier{rules}` with `classify(&ClassifyInput) -> Classification{canonical, rule_ids, tags}`. Builtin const seed data (25 categories).
- [x] `migrate_0004_seed_taxonomy` in `migrate.rs` (idempotent `INSERT OR IGNORE` of builtins). Seeds on every fresh init or upgrade from v<4.
- [x] Store helpers: `insert_category`, `insert_alias`, `resolve_alias`, `list_categories` in `witslog-store/src/taxonomy.rs`. Unit tests pass.
- [x] Wire auto-classify into `EventBuilder::classify()` when `category.is_none()`. Respects explicit `.category()` set beforehand.
- [x] Config: `[taxonomy]` (auto_classify_enabled, custom_rules_file) in `witslog-config`.
- [x] Tests: 7 unit tests in core (builtin count, error_code/exception/keyword/regex matches, determinism, priority); 8 integration tests in store (seed count, category exists, classify chain, respects explicit, determinism). All green.

**Verification.** `witslog init` → seeded tree visible; log an `ETIMEDOUT` exception with no
`--category` → `witslog query <id>` shows auto-assigned canonical; register alias, re-log, confirm resolution.

---

## P3 — FTS5 + Query engine

**Objective.** Turn the store into a searchable index: activate FTS5 (placeholder at
`migrate.rs:115`), add a `witslog-query` crate compiling filter intent → parameterized SQL
with bm25 ranking + keyset pagination, plus analytics aggregates. Unlocks P4 CLI
`query`/`stats` and P5 MCP tools.

**Status.** ⬜ todo. FTS table not created; no query crate.
**Dependencies.** P0 (schema/writer). Blocks P4, P5.
**Complexity.** L.

**Functional requirements (EARS).**
- **FR-P3-001** (ubiquitous): The system shall maintain an FTS5 external-content table
  `events_fts` over `message, exception, stack_norm, root_cause, tags_text, category` using
  tokenizer `unicode61 remove_diacritics 2 tokenchars '._:/-'`.
- **FR-P3-002** (event): When an event row is inserted, the system shall index its text into
  `events_fts` via an `AFTER INSERT` trigger within the same transaction.
- **FR-P3-003** (event): When an event row is deleted, the system shall remove its
  `events_fts` entry via an `AFTER DELETE` trigger.
- **FR-P3-004** (conditional): While a search has an FTS query string, when the query engine
  runs, the system shall rank results by `bm25(events_fts)` with column weights (message 3,
  exception 2, root_cause 2, tags 2, stack_norm 1, category 1).
- **FR-P3-005** (conditional): While structured filters are supplied
  (application/version/environment/subsystem/category/severity_min/hostname/from/to/tags),
  when the query runs, the system shall AND them onto the SQL as bound parameters.
- **FR-P3-006** (event): When `limit` results are returned and more exist, the system shall
  return an opaque keyset `next_cursor` encoding `(ts_epoch_ms,id)`.
- **FR-P3-007** (ubiquitous): The system shall cap `limit` at 200 and default it to 20.
- **FR-P3-008** (event): When `stats`/`timeline`/`top_failures` are requested, the system
  shall return aggregates via `GROUP BY` over indexed columns / the `fingerprints` table.
- **FR-P3-009** (optional): Where a rebuild is requested (`migrate` backfill), the system
  shall populate `events_fts` from existing `events` rows idempotently.

**Non-functional.**
- Search on 100k events shall return the first page < 50 ms on commodity SSD (measure P7).
- Query builder shall use only bound params — no string interpolation of user input.
- FTS index write overhead shall stay < 60% of text byte size (choose indexed columns).
- All queries shall use the intended index (verify via `EXPLAIN QUERY PLAN` in tests).

**Acceptance criteria.**
- Given events containing `ECONNREFUSED`, when `search query="econnrefused*"` runs, then
  matching events return ranked by bm25, highest first.
- Given 25 matching events and `limit=20`, when searched, then 20 items + a `next_cursor`
  return; re-querying with the cursor yields the remaining 5 and no `next_cursor`.
- Given filters `severity_min=error, from=-24h`, when combined with an FTS query, then only
  error+ events in the window match, and `EXPLAIN QUERY PLAN` shows an index scan.
- Given a fresh DB migrated from v2, when the FTS migration runs, then `events_fts` exists
  and back-fills all prior rows (count matches `events`).

**Error handling.**
| Condition | Detection | Response | Code |
|---|---|---|---|
| Malformed FTS query (`"unbalanced`) | rusqlite FTS parse err | typed `QueryError::BadFtsSyntax`, names issue | 2 / -32602 |
| `from > to` | validate pre-SQL | reject `QueryError::BadRange` | 2 / -32602 |
| `limit` over cap | clamp | clamp to 200 silently | — |
| Corrupt/absent `events_fts` | table check on open | rebuild or `doctor` warning | — |
| Cursor tampered/undecodable | base64/parse guard | ignore cursor, first page + warn | — |

**TODO checklist.**
- [ ] `migrate_0005_fts5` in `migrate.rs` (idempotent like `migrate_0002_resolved_at`):
      create `events_fts`, `events_ai`/`events_ad` triggers, back-fill from `events`.
- [ ] Wire `if current_version < 5` + `record_migration(5, "fts5")`.
- [ ] New crate `crates/witslog-query/` (`search.rs`, `filters.rs`, `aggregates.rs`,
      `correlate.rs`, `error.rs`) + root `Cargo.toml` workspace member.
- [ ] `filters.rs`: `struct Filters{…}` → `(where_sql, params)`, bound params only.
- [ ] `search.rs`: FTS MATCH JOIN + `bm25(...)` weights + keyset `Cursor` encode/decode.
- [ ] `aggregates.rs`: `statistics`, `timeline(bucket)`, `top_failures(by)`.
- [ ] `correlate.rs`: recursive `error_edges` walk + `correlation_id` trace assembly.
- [ ] Tests: prefix/phrase/boolean/NEAR; pagination continuity; `EXPLAIN QUERY PLAN`
      assertions; back-fill count equality.

**Verification.** `witslog init` → log 30 varied events → `witslog-query` integration test
returns ranked page + cursor and correct aggregate counts. Full CLI surface arrives in P4.

---

## P4 — CLI utilities

**Objective.** Expose the query engine and operational tooling through the CLI so a human
can search, summarise, and maintain a project's log without writing code.

**Status.** 🟡 partial. `init/log/query(by-id)/resolve/delete/doctor` shipped (P0);
search-`query`, `stats`, `export/import`, `vacuum`, policy `prune`, `migrate`, `config`,
`archive`, `backup`, `list-dbs` remain.

**Dependencies.** P3 (query engine) for `query`/`stats`. P0 for ops commands.
**Complexity.** M.

**Functional requirements (EARS).**
- **FR-P4-001** (event): When `witslog query <FTS> [filters]` runs, the system shall call
  the query engine and print ranked results (human table or `--json`), with cursor paging.
- **FR-P4-002** (event): When `witslog stats [filters]` runs, the system shall print
  headline metrics (totals, by severity/category, unique fingerprints, error rate).
- **FR-P4-003** (event): When `witslog export [--from/--to/--format ndjson]` runs, the
  system shall stream matching events as portable NDJSON to stdout/file.
- **FR-P4-004** (event): When `witslog import <file>` runs, the system shall ingest NDJSON
  events idempotently (dedupe on `event_id`).
- **FR-P4-005** (event): When `witslog vacuum` runs, the system shall checkpoint WAL and
  run `VACUUM`, reporting reclaimed bytes.
- **FR-P4-006** (event): When `witslog prune [--older-than/--keep-last/--max-rows/--max-bytes]`
  runs, the system shall delete in batches within the policy and report counts.
- **FR-P4-007** (event): When `witslog migrate` runs, the system shall apply pending
  migrations after taking a `.bak` snapshot, printing before/after schema version.
- **FR-P4-008** (event): When `witslog config [get|set|path]` runs, the system shall read or
  write the resolved config layer and echo the effective value.
- **FR-P4-009** (event): When `witslog archive --older-than <N>` runs, the system shall move
  aged rows into a sibling archive DB via ATTACH and delete them from live.
- **FR-P4-010** (event): When `witslog backup <out.db>` runs, the system shall produce a
  consistent snapshot via the SQLite online backup API without blocking writers.
- **FR-P4-011** (ubiquitous): The system shall print the resolved DB path on every command
  and support `--json` for machine-readable output.
- **FR-P4-012** (event): When `witslog list-dbs` runs, the system shall enumerate known
  project DBs discovered from cwd upward and any configured roots.

**Non-functional.**
- Every command shall exit non-zero on failure with a single-line diagnostic to stderr.
- `export`/`import` shall stream (bounded memory) — no full-table load.
- Destructive commands (`prune`, `archive`, `delete`) shall report a dry-run count with
  `--dry-run` before mutating.

**Acceptance criteria.**
- Given 30 logged events, when `witslog query "timeout*" --severity error --json`, then a
  ranked JSON page + `next_cursor` prints; passing the cursor yields the next page.
- Given a populated DB, when `witslog stats`, then severity/category counts sum to the total.
- Given `witslog export > out.ndjson` then `witslog import out.ndjson` on a fresh DB, then
  event counts match and a re-import adds zero rows (idempotent).
- Given aged data, when `witslog prune --older-than 30d --dry-run`, then the would-delete
  count prints and nothing is removed; without `--dry-run`, rows are deleted in batches.

**Error handling.**
| Condition | Detection | Response | Code |
|---|---|---|---|
| Query engine error (bad FTS/range) | propagate `QueryError` | one-line stderr, name cause | 2 |
| `import` malformed line | per-line parse | skip + count skipped, continue | 0 (warn) |
| `migrate` on newer-than-binary schema | version guard | refuse, tell user to upgrade | 3 |
| `backup` target exists | pre-check | refuse unless `--force` | 2 |
| Ops command with no project | DB resolution | error + suggest `init`/`--global` | 2 |

**TODO checklist.**
- [ ] Extend `Commands` enum in `main.rs`: `Query`(FTS+filters), `Stats`, `Export`, `Import`,
      `Vacuum`, `Prune`, `Migrate`, `Config`, `Archive`, `Backup`, `ListDbs`.
- [ ] Handlers `fn query/stats/export/import/vacuum/prune/migrate/config/archive/backup/list_dbs`.
- [ ] Reuse `witslog-query` for `query`/`stats`; reuse `delete_resolved`/`DeleteFilter` for `prune`.
- [ ] Add `--json` global flag + a small output formatter; print resolved DB path helper.
- [ ] Online backup + ATTACH archive helpers in `witslog-store`.
- [ ] Integration tests per command (temp-dir driven, like `m1_integration.rs`).

**Verification.** End-to-end in a temp project: init → log 30 → `query`/`stats`/`export` →
`import` into fresh DB (counts match) → `prune --dry-run` then real → `backup` → `doctor`.

---

## P5 — MCP server

**Objective.** Expose the log to any MCP-compatible AI assistant as standards-compliant
tools over JSON-RPC, provider-independent, so assistants can search/summarise/classify/
correlate failures in a project.

**Status.** ⬜ todo. No `witslog-mcp` crate.
**Dependencies.** P3 (query engine), P2 (`classify_error`/`list_categories`). Blocks P8 docs.
**Complexity.** L.

**Functional requirements (EARS).**
- **FR-P5-001** (ubiquitous): The system shall implement MCP `tools/list` and `tools/call`
  over JSON-RPC 2.0 on stdio, launched by `witslog serve-mcp`.
- **FR-P5-002** (ubiquitous): The system shall expose the tools `search_errors`,
  `latest_errors`, `summarize_errors`, `classify_error`, `explain_error`, `similar_errors`,
  `list_categories`, `statistics`, `timeline`, `top_failures`, `list_traces`, and (opt-in)
  `search_all`.
- **FR-P5-003** (event): When a tool is called, the system shall validate params against the
  tool's JSON Schema and reject invalid input with JSON-RPC error `-32602`.
- **FR-P5-004** (event): When a result set exceeds `limit`, the system shall return a keyset
  `next_cursor` (reusing P3 pagination).
- **FR-P5-005** (state): While serving, the system shall bind to exactly one resolved project
  DB (walk-up or `--db`) and open it read-only.
- **FR-P5-006** (optional): Where `--attach <paths>` is supplied, the system shall enable
  `search_all` across the attached project DBs read-only via ATTACH + UNION.
- **FR-P5-007** (ubiquitous): The system shall enforce a per-call statement timeout (e.g. 2s)
  via `sqlite3_progress_handler`/interrupt and never leak raw SQL in errors.
- **FR-P5-008** (ubiquitous): The system shall be AI-provider-independent — raw protocol,
  no vendor SDK.

**Non-functional.**
- The server shall answer `tools/list` in < 10 ms and typical tool calls in < 100 ms on 100k
  events.
- Memory footprint idle shall stay < 30 MB.
- The server shall be read-only by default; writes only behind `--allow-write`.

**Acceptance criteria.**
- Given a running `serve-mcp`, when an MCP client calls `tools/list`, then all specified
  tools return with valid JSON Schemas.
- Given logged errors, when `search_errors` is called with an FTS query + filters, then a
  ranked, paginated result returns matching the CLI `query`.
- Given an `event_id`, when `explain_error` is called, then the dossier includes recurrence
  stats, category path, and the caused-by chain.
- Given invalid params (bad `from`), when any tool is called, then `-32602` returns with a
  descriptive message and no SQL leakage.
- Given a pathological query, when it exceeds the statement timeout, then the call is
  interrupted and a `-32000` retriable error returns.

**Error handling.**
| Condition | Detection | Response |
|---|---|---|
| Invalid params | JSON Schema validate | `-32602` + `data.detail` |
| Domain/DB error | query engine `Result` | `-32000`, generic msg, `data.retriable` |
| Statement timeout | progress handler | interrupt, `-32000` retriable |
| Unknown tool | registry lookup | `-32601` method not found |
| DB newer than binary | version guard on open | fail fast with upgrade message |

**TODO checklist.**
- [ ] New crate `crates/witslog-mcp/` (`transport.rs`, `registry.rs`, `tools/` one file per tool).
- [ ] `transport.rs`: stdio JSON-RPC 2.0 framing (+ optional HTTP/SSE behind a feature).
- [ ] `registry.rs`: tool registration, `tools/list`, dispatch, JSON Schema validation.
- [ ] `tools/*`: each delegates to `witslog-query`/`witslog-core` (taxonomy) — no direct SQL.
- [ ] Statement-timeout wrapper on read connections.
- [ ] `serve-mcp` subcommand in `main.rs` (`--stdio`/`--http`, `--db`, `--attach`, `--allow-write`).
- [ ] Conformance test: spawn server, script `tools/list` + one call per tool over a pipe.

**Verification.** `witslog serve-mcp --stdio` driven by a scripted JSON-RPC client (or a
real MCP client): `tools/list` lists all tools; `search_errors`/`explain_error`/`statistics`
return valid shapes; invalid params → `-32602`.

---

## P6 — SDK bindings

**Objective.** Provide idiomatic, thin language wrappers (Python, Node) over the shipped C
ABI so apps in those languages log structured events without touching FFI details.

**Status.** 🟡 partial. C ABI (`witslog_log/resolve/delete/free_string`) shipped (P0);
no language wrappers yet.
**Dependencies.** P0 (FFI-core), P1 (enrich/redact/configure surface).
**Complexity.** M.

**Functional requirements (EARS).**
- **FR-P6-001** (ubiquitous): The system shall provide a Python package exposing
  `error/warn/info/exception` + a builder that serialises to the FFI JSON contract.
- **FR-P6-002** (ubiquitous): The system shall provide a Node package with the same surface.
- **FR-P6-003** (event): When an SDK logs an event, the system shall call `witslog_log` and
  free any returned strings via `witslog_free_string`.
- **FR-P6-004** (ubiquitous): Each SDK shall bundle or locate the correct native library for
  the host OS/arch.
- **FR-P6-005** (event): When an SDK call fails at the FFI boundary, the system shall raise a
  language-native error without crashing the host process.
- **FR-P6-006** (optional): Where the app provides context/tags, the SDK shall pass them
  through the JSON contract unchanged.

**Non-functional.**
- SDK overhead over raw FFI shall be negligible (< 10 µs marshalling).
- SDKs shall have no third-party runtime deps beyond the native lib + stdlib where feasible.
- The JSON contract shall be versioned so SDK/native mismatches are detectable.

**Acceptance criteria.**
- Given the Python SDK installed, when `witslog.error("boom", context={...})` is called in a
  project dir, then an event is written to that project's DB and retrievable by the CLI.
- Given the Node SDK, when `witslog.exception(err)` is called with a stack, then `stacktrace`
  is stored and normalized.
- Given a missing native lib, when an SDK is imported, then a clear language-native error is
  raised naming the expected lib path.

**Error handling.**
| Condition | Detection | Response |
|---|---|---|
| Native lib not found | load-time check | raise `WitslogLibraryError` with search paths |
| FFI returns -1 | return-code check | raise `WitslogWriteError` |
| JSON contract mismatch | version field | raise with expected/actual versions |
| Non-UTF8 input | encode guard | raise `ValueError`/`TypeError` |

**TODO checklist.**
- [ ] `bindings/python/`: ctypes/cffi wrapper, `witslog/__init__.py` surface, wheel packaging.
- [ ] `bindings/node/`: N-API or ffi-napi wrapper, `index.js` + types, npm packaging.
- [ ] Native-lib locator (bundled per-platform or env override `WITSLOG_LIB`).
- [ ] Contract version constant shared with `witslog-ffi`.
- [ ] Example apps + smoke tests per SDK that assert CLI can read what the SDK wrote.

**Verification.** In a temp project: Python `witslog.error(...)` then `witslog query <id>`
finds it; repeat for Node. Import with lib removed → clear error.

---

## P7 — Perf & hardening

**Objective.** Prove the system meets its overhead/throughput targets and survives
concurrency, large volumes, and maintenance operations without corruption.

**Status.** ⬜ todo.
**Dependencies.** P1–P6 (measures the assembled system).
**Complexity.** M.

**Functional requirements (EARS).**
- **FR-P7-001** (ubiquitous): The system shall provide a Criterion bench suite for write
  throughput, buffered `log` latency, search latency, and index build cost.
- **FR-P7-002** (event): When benches run in CI, the system shall record results and flag
  regressions beyond a threshold.
- **FR-P7-003** (state): While multiple processes write the same project DB, the system shall
  serialise via WAL + `busy_timeout` + bounded retry with zero corruption.
- **FR-P7-004** (event): When a load test inserts 1M events, the system shall complete prune/
  archive/backup within documented time/space bounds.
- **FR-P7-005** (ubiquitous): The system shall document measured overhead, throughput, and
  memory footprint per OS.

**Non-functional.**
- Buffered `log` < 100 µs; single-writer insert throughput ≥ 10k events/s on SSD (targets,
  refined by measurement).
- No `SQLITE_CORRUPT`/`SQLITE_BUSY` escapes under the concurrency test.
- Idle MCP server < 30 MB; CLI one-shot < 15 MB.

**Acceptance criteria.**
- Given the bench suite, when run, then it reports write/read/index numbers and fails CI on
  a >20% regression.
- Given N concurrent writer processes on one DB, when each logs 10k events, then all events
  persist and `PRAGMA integrity_check` returns `ok`.
- Given 1M events, when `prune`/`archive`/`backup` run, then each completes within the
  documented bound and the DB stays consistent.

**Error handling.**
| Condition | Detection | Response |
|---|---|---|
| `SQLITE_BUSY` under contention | rusqlite err | bounded retry w/ backoff, then drop+count |
| Bench regression | threshold compare | CI fails with diff |
| integrity_check != ok | post-test assert | test fails, capture DB |

**TODO checklist.**
- [ ] `bench/` Criterion benches: write, buffered log, search, index build.
- [ ] Concurrency test harness: spawn N processes, hammer one DB, `integrity_check`.
- [ ] Load-test generator (1M synthetic events) + prune/archive/backup timings.
- [ ] Memory measurement script per OS.
- [ ] CI wiring + regression thresholds; document results in `docs/perf.md`.

**Verification.** `cargo bench` produces numbers; concurrency + load harness pass with
`integrity_check = ok`; perf doc updated.

---

## P8 — Packaging & install

**Objective.** Make install a one-command experience on Linux/macOS/Windows with minimal
manual config, plus upgrade/uninstall and MCP registration.

**Status.** ⬜ todo.
**Dependencies.** P4 (CLI), P5 (serve-mcp), P6 (SDKs).
**Complexity.** M.

**Functional requirements (EARS).**
- **FR-P8-001** (ubiquitous): The system shall publish a single self-contained `witslog`
  binary per OS/arch from CI releases with checksums/signatures.
- **FR-P8-002** (event): When `install.sh`/`install.ps1` runs, the system shall detect
  OS/arch, download+verify, and place the binary on PATH.
- **FR-P8-003** (ubiquitous): The system shall be installable via Homebrew, Scoop, winget,
  and `cargo install witslog-cli`.
- **FR-P8-004** (event): When `witslog serve-mcp --print-mcp-config` runs, the system shall
  emit a generic `mcpServers` snippet for MCP clients.
- **FR-P8-005** (event): When an upgrade installs a newer binary, the system shall lazily
  apply pending migrations after a `.bak` snapshot.
- **FR-P8-006** (event): When `witslog uninstall [--purge]` runs, the system shall remove the
  binary and (with `--purge`) documented data files.
- **FR-P8-007** (ubiquitous): The system shall enforce version compatibility (binary
  supports schema range; refuses out-of-range with guidance).

**Non-functional.**
- Cold install → `init` → `log` → `serve-mcp` shall work in one command each per OS.
- Binaries shall be statically linked (bundled SQLite) with no runtime deps.
- Release pipeline shall be reproducible and cross-compiled from CI.

**Acceptance criteria.**
- Given a clean machine per OS, when the install script runs, then `witslog --version` works
  from a new shell.
- Given an installed binary, when `--print-mcp-config` runs, then a valid client snippet
  prints that launches `serve-mcp` with the project cwd.
- Given a DB from an older schema, when a newer binary runs any command, then it snapshots
  and migrates; given a newer DB + older binary, then it refuses with an upgrade message.

**Error handling.**
| Condition | Detection | Response |
|---|---|---|
| Checksum/signature mismatch | verify step | abort install, report |
| Unsupported OS/arch | detect | clear "no prebuilt binary" + cargo fallback |
| Migration failure on upgrade | migrate `Result` | restore `.bak`, abort, report |
| Schema newer than binary | version guard | refuse + upgrade instructions |

**TODO checklist.**
- [ ] `cargo-dist` (or equivalent) release workflow: cross-compile Linux/macOS/Windows.
- [ ] `install/install.sh` + `install/install.ps1` (detect, download, verify, PATH).
- [ ] Homebrew tap, Scoop manifest, winget manifest, `.deb`/`.rpm` via `nfpm`.
- [ ] `--print-mcp-config` in `serve-mcp`; `uninstall` command; version-compat matrix + guard.
- [ ] `docs/install.md` per-OS; CI smoke test that installs + runs the happy path.

**Verification.** In a clean container/VM per OS: run installer → `witslog init/log/query/
serve-mcp` all succeed; upgrade path migrates; downgrade guard refuses.

---

## P9 — Extensibility & security

**Objective.** Open the framework to plugins and harden it for sensitive environments:
custom taxonomy/exporters/enrichers/notifiers/MCP-tools, configurable redaction,
optional encryption at rest, and a tamper-evident audit trail.

**Status.** ⬜ todo (redaction built-ins land in P1; this phase makes it configurable +
adds the rest).
**Dependencies.** P1–P6.
**Complexity.** M–L.

**Functional requirements (EARS).**
- **FR-P9-001** (ubiquitous): The system shall define plugin traits for taxonomy rules,
  exporters, enrichers, storage backends, notifiers, and additional MCP tools.
- **FR-P9-002** (optional): Where a plugin is registered, the system shall load it (static
  registration or dynamic library) and invoke it at the defined extension point.
- **FR-P9-003** (optional): Where redaction rules are configured, the system shall apply them
  in addition to built-ins (extends FR-P1-004).
- **FR-P9-004** (optional): Where encryption-at-rest is enabled, the system shall encrypt the
  DB (e.g. SQLCipher-compatible) using a key from config/keychain/env.
- **FR-P9-005** (ubiquitous): The system shall restrict DB/config file permissions (0600/0700).
- **FR-P9-006** (optional): Where the audit trail is enabled, the system shall chain a hash
  over appended events (each row references the prior hash) enabling tamper detection.
- **FR-P9-007** (event): When `witslog doctor --verify-audit` runs, the system shall verify
  the hash chain and report any break with the offending row.

**Non-functional.**
- Plugin failures shall be isolated — a misbehaving plugin never corrupts the DB or crashes
  the core write path.
- Encryption shall be optional and off by default; enabling it shall not change the query API.
- The audit chain shall add O(1) per-insert overhead.

**Acceptance criteria.**
- Given a sample plugin of each trait type, when registered, then each is invoked at its
  extension point and its effect is observable.
- Given custom redaction rules, when an event matching a rule is logged, then the value is
  redacted at rest.
- Given encryption enabled with a key, when the DB is opened without the key, then access
  fails; with the key, queries work unchanged.
- Given the audit trail enabled, when a stored row is tampered with, then
  `doctor --verify-audit` reports the break at that row.

**Error handling.**
| Condition | Detection | Response |
|---|---|---|
| Plugin load failure | load `Result` | skip plugin, warn, continue core |
| Plugin panics at hook | catch/isolate | isolate, log, don't corrupt DB |
| Wrong/absent encryption key | open error | fail fast, don't create plaintext DB |
| Audit chain break | verify walk | report row + expected/actual hash |

**TODO checklist.**
- [ ] `witslog-plugin` crate: trait defs (`TaxonomyRule`, `Exporter`, `Enricher`,
      `StorageBackend`, `Notifier`, `McpTool`) + registry.
- [ ] Static registration API + optional dynamic (`libloading`) behind a feature.
- [ ] Config-driven redaction rules (extends P1 `Redactor`).
- [ ] Optional encryption-at-rest (SQLCipher-compatible) + key sourcing.
- [ ] File-permission hardening (0600 DB / 0700 dir) verified cross-OS.
- [ ] Audit hash chain column + `doctor --verify-audit`.
- [ ] Sample plugins + tests per extension point; tamper-detection test.

**Verification.** Register one plugin of each type + assert effect; enable redaction/
encryption/audit in a temp project → confirm redacted-at-rest, key-gated access, and
`doctor --verify-audit` catching a manual tamper.

---

## Appendix — requirement traceability

Each `FR-<PHASE>-NNN` should be cited in the implementing commit and covered by at least one
test named for it. Acceptance criteria map 1:1 to integration tests; non-functional targets
are validated in P7. This file is the source of truth for scope per phase — update the
status legend as phases land, and keep the DAG acyclic.
