# witslog — AI-Native Error Intelligence Framework — Engineering Spec

## Context

Need embeddable, CLI-first error logging framework. SQLite = single source of truth.
Apps emit **structured** error events (not text lines). Events queryable + full-text
searchable. An **MCP server** exposes the log to any MCP-compatible AI assistant so it
can search, classify, summarize, correlate, and reason about failures — with **zero**
cloud, zero external services, zero embeddings/vectors.

Why now: plain-text logs are un-queryable and opaque to AI. Grepping stack traces
across releases/hosts is manual. This framework makes failure history a *structured,
local, AI-readable database* that ships inside the app and needs no infra.

**Decisions locked (this session):**
- **DB scope = per-directory/per-project.** Each project gets its OWN
  `./.witslog/witslog.db`. 10 projects → 10 independent DB files. NEVER one shared global
  DB across projects. DB is discovered by walking up from cwd for a `.witslog/` marker
  (same model as `.git`). This keeps each app's error history isolated, portable with the
  repo, and independently prunable/backupable. Cross-project views are opt-in via
  federation (ATTACH), not the default. Optional global DB exists only as an explicit
  fallback (`--global` / no project marker found + config opt-in).
- **Embedding model:** in-process native library writes SQLite directly (WAL); CLI +
  MCP server are separate processes over the *same project* DB file. Multi-writer WAL
  handled via a single internal write-serializing connection + busy_timeout + retry.
- **Reference language:** Rust. Core = one crate compiled to (a) static/dynamic lib with
  C ABI for FFI into any language, (b) single self-contained binary `witslog` (CLI +
  MCP). Architecture stays language-agnostic; Rust is only the reference build.

Deliverable = authoritative design doc. **No implementation code yet.** Roadmap at end
breaks the build into milestones.

---

## 1. Architecture Overview

```
        ┌──────────────────── host CLI app (any language) ────────────────────┐
        │  app code  →  witslog client API (fluent builder / macros / FFI)     │
        │                              │ structured event                       │
        │                     ┌────────▼─────────┐                              │
        │                     │  Logging library │  enrich · redact · classify  │
        │                     │  (in-process)    │  fingerprint · buffer         │
        │                     └────────┬─────────┘                              │
        └──────────────────────────────┼──────────────────────────────────────┘
                                        │ single write-serialized conn (WAL)
                                ┌───────▼────────┐
                                │ SQLite storage │  events · fts5 · dims · migrations
                                │  (file on disk)│  WAL, single source of truth
                                └───────┬────────┘
             ┌──────────────────────────┼──────────────────────────┐
     read-only conns                    │                    read-only conns
   ┌─────────▼─────────┐        ┌────────▼────────┐        ┌────────▼──────────┐
   │   CLI utilities   │        │   MCP server    │        │ Analytics/reporting│
   │ init log query …  │        │ tools over JSON-│        │ stats trends MTTR  │
   └───────────────────┘        │ RPC / stdio     │        └────────────────────┘
                                 └────────┬────────┘
                                          │ MCP (stdio/http)
                                 ┌────────▼────────┐
                                 │ any MCP AI asst │  (provider-independent)
                                 └─────────────────┘

  Cross-cutting: Config manager · Taxonomy engine · Query engine · Plugin host
                 Install/bootstrap tooling
```

Component boundaries (responsibility · does NOT do):

| # | Component | Owns | Boundary |
|---|-----------|------|----------|
| 1 | **Logging library** | accept event, enrich, redact, fingerprint, classify, buffer, write | no querying, no MCP, no schema migration at runtime beyond ensure-version |
| 2 | **SQLite storage layer** | schema, connections, WAL, migrations, prune/archive/backup, prepared stmts | no business logic, no taxonomy rules |
| 3 | **Taxonomy engine** | category tree, aliases, tag rules, auto-classify rules | no storage, no I/O — pure fn(event)→labels |
| 4 | **Query engine** | compile query intent → parameterized SQL, pagination, ranking | no MCP framing, no formatting |
| 5 | **MCP server** | JSON-RPC, tool registration, schema validation, call query engine, shape output | no direct SQL of its own; delegates to query engine |
| 6 | **CLI utilities** | subcommands, arg parse, human/JSON output, ops (vacuum/prune/migrate/doctor) | thin — orchestrates other components |
| 7 | **Analytics/reporting** | aggregates, timelines, trends, top-failures, MTTR | read-only; no writes |
| 8 | **Config manager** | resolve layered config, env, defaults, validate | no domain logic |
| 9 | **Install/bootstrap** | install binary, create DB, init config, register MCP, upgrade/uninstall | runtime-independent |

Modularity rule: 1→2 write-only; 4/5/7 read-only; 3 pure; everything depends on 8. No
component reaches around 2 to touch the DB file.

---

## 2. Design Rationale

- **SQLite single-file source of truth** — zero-config, zero-service, cross-platform,
  embeddable, ACID, ubiquitous. One file = trivial backup/ship/inspect. Matches "no infra".
- **WAL mode** — concurrent readers (CLI, MCP, analytics) while one writer appends.
  Reader never blocks writer. Fits our 1-writer / N-reader topology exactly.
- **In-process lib write path** — lowest latency, no IPC. SQLite handles durability.
  Multi-process writers (app + CLI + another app instance) reconciled by WAL +
  `busy_timeout` + bounded retry, since writes are short appends.
- **FTS5 lexical, not vectors** — error text is *lexical*: identifiers, error codes,
  file paths, exception class names, stack frames. Exact/prefix/phrase token matching on
  these beats semantic similarity, is deterministic, needs no model, no GPU, no rebuild
  on model change, and stays fully local. Trade-off: no synonym/paraphrase matching
  ("cannot reach host" ≠ "connection refused"). Mitigated by taxonomy + tags + fingerprint
  clustering, which give the AI structured axes to pivot on without embeddings.
- **AI does semantics, DB does structure** — we deliberately push semantic reasoning to
  the consuming AI (that's its strength) and keep the DB doing what it's best at: exact
  structured retrieval. So no embeddings needed in-house.
- **Fingerprints over embeddings for dedup/clustering** — deterministic hash of
  normalized (message + top stack frames + category) groups recurrences exactly and
  cheaply. Reproducible, explainable, joinable. Embeddings would be fuzzy + non-deterministic.
- **JSON columns for open-ended fields** (context/metadata/tags) + **generated columns +
  indexes** for the hot query axes — hybrid normalized/denormalized: fixed queryable
  dimensions get real columns/indexes; arbitrary structure lives in JSON, still queryable
  via `json_extract` and, when hot, promoted to a generated column later. Schema evolves
  without migration pain.
- **Rust reference** — single static binary, no runtime, cross-compiles to all 3 OSes,
  C ABI FFI to embed in any language, memory-safe for a many-year-maintained core.
- **MCP provider-independence** — implement raw MCP spec (tools + JSON Schema over
  JSON-RPC 2.0). No SDK tied to one vendor. Any compliant client works unchanged.

---

## 3. SQLite Schema

Design: one wide `events` table (append-optimized) + generated columns for hot axes +
FTS5 shadow table + small dimension/label tables + correlation edges + meta/migrations.
Denormalized-leaning for read speed; JSON for open fields; lookup tables only where
cardinality is low and joins pay off (categories).

```sql
-- ---- meta / migrations -------------------------------------------------
CREATE TABLE schema_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);  -- rows: schema_version, created_at, witslog_version

CREATE TABLE migrations (
  version     INTEGER PRIMARY KEY,
  name        TEXT NOT NULL,
  applied_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  checksum    TEXT NOT NULL
);

-- ---- core events -------------------------------------------------------
CREATE TABLE events (
  id            INTEGER PRIMARY KEY,          -- rowid, monotonic
  event_id      TEXT NOT NULL UNIQUE,         -- deterministic ULID/UUIDv7 (sortable)
  ts            TEXT NOT NULL,                -- RFC3339 UTC 'YYYY-MM-DDTHH:MM:SS.sssZ'
  ts_epoch_ms   INTEGER NOT NULL,            -- for fast range/bucketing, indexed

  application   TEXT NOT NULL,
  version       TEXT,                         -- app semver / build id
  environment   TEXT,                         -- prod/staging/dev/ci
  command       TEXT,                         -- subcommand / entrypoint
  subsystem     TEXT,                         -- module/component
  hostname      TEXT,

  severity      TEXT NOT NULL,                -- trace|debug|info|warn|error|critical|fatal
  severity_rank INTEGER NOT NULL,             -- 10..70, for range filters/order
  category      TEXT,                         -- canonical leaf, e.g. 'infrastructure.network.dns'
  error_code    TEXT,                         -- app-defined stable code

  message       TEXT NOT NULL,
  exception     TEXT,                         -- exception/class type name
  stacktrace    TEXT,                         -- raw
  stack_norm    TEXT,                         -- normalized (addresses/line#/temp paths stripped)
  root_cause    TEXT,                         -- deepest cause message (denormalized from chain)

  fingerprint   TEXT NOT NULL,                -- canonical dedup hash (hex)
  correlation_id TEXT,                        -- request/trace id (user-supplied grouping)
  parent_event_id TEXT,                       -- caused-by parent (FK-ish, soft)
  resolved_at   TEXT,                         -- RFC3339 UTC; NULL = unresolved

  context       TEXT,                         -- JSON object (structured context)
  tags          TEXT,                         -- JSON array of strings
  metadata      TEXT,                         -- JSON object (free-form)

  -- generated (virtual) columns promote hot JSON keys to indexable columns
  ctx_request_id TEXT GENERATED ALWAYS AS (json_extract(context,'$.request_id')) VIRTUAL,
  ctx_git_commit TEXT GENERATED ALWAYS AS (json_extract(context,'$.git_commit')) VIRTUAL,
  ctx_pid        INTEGER GENERATED ALWAYS AS (json_extract(context,'$.pid')) VIRTUAL,
  ctx_duration_ms INTEGER GENERATED ALWAYS AS (json_extract(context,'$.duration_ms')) VIRTUAL,

  ingest_source TEXT DEFAULT 'lib',           -- lib|cli|import
  schema_v      INTEGER NOT NULL,

  CHECK (json_valid(context) OR context IS NULL),
  CHECK (json_valid(tags)    OR tags    IS NULL),
  CHECK (json_valid(metadata)OR metadata IS NULL)
) STRICT;
```

Indexes — one per hot query axis (see §Query Model for mapping):

```sql
CREATE INDEX ix_events_ts            ON events(ts_epoch_ms DESC);            -- latest, timeline
CREATE INDEX ix_events_fp_ts         ON events(fingerprint, ts_epoch_ms DESC); -- recurring/dedup
CREATE INDEX ix_events_cat_ts        ON events(category, ts_epoch_ms DESC);   -- by category
CREATE INDEX ix_events_sub_ts        ON events(subsystem, ts_epoch_ms DESC);  -- by subsystem
CREATE INDEX ix_events_sev_ts        ON events(severity_rank, ts_epoch_ms DESC); -- by severity
CREATE INDEX ix_events_app_ver_ts    ON events(application, version, ts_epoch_ms DESC); -- regressions
CREATE INDEX ix_events_cmd_ts        ON events(command, ts_epoch_ms DESC);    -- by command
CREATE INDEX ix_events_host_ts       ON events(hostname, ts_epoch_ms DESC);   -- by host
CREATE INDEX ix_events_corr          ON events(correlation_id) WHERE correlation_id IS NOT NULL;
CREATE INDEX ix_events_parent        ON events(parent_event_id) WHERE parent_event_id IS NOT NULL;
CREATE INDEX ix_events_code_ts       ON events(error_code, ts_epoch_ms DESC) WHERE error_code IS NOT NULL;
CREATE INDEX ix_events_reqid         ON events(ctx_request_id) WHERE ctx_request_id IS NOT NULL;
CREATE INDEX ix_events_unresolved    ON events(ts_epoch_ms DESC) WHERE resolved_at IS NULL; -- unresolved backlog
```

`resolved_at IS NULL` is a first-class query axis (unresolved backlog); setting it is a
plain `UPDATE`, not part of `EventBuilder` construction — events are always logged
unresolved and resolved later by id.

FTS5 shadow + sync triggers (see §4):

```sql
CREATE VIRTUAL TABLE events_fts USING fts5(
  message, exception, stack_norm, root_cause, tags_text, category,
  content='events', content_rowid='id',
  tokenize = "unicode61 remove_diacritics 2 tokenchars '._:/-'"
);
-- tags_text = space-joined tags; supplied by triggers.
CREATE TRIGGER events_ai AFTER INSERT ON events BEGIN
  INSERT INTO events_fts(rowid,message,exception,stack_norm,root_cause,tags_text,category)
  VALUES (new.id,new.message,new.exception,new.stack_norm,new.root_cause,
          (SELECT group_concat(value,' ') FROM json_each(new.tags)), new.category);
END;
CREATE TRIGGER events_ad AFTER DELETE ON events BEGIN
  INSERT INTO events_fts(events_fts,rowid,message,exception,stack_norm,root_cause,tags_text,category)
  VALUES ('delete',old.id,old.message,old.exception,old.stack_norm,old.root_cause,'',old.category);
END;
-- events are append-only; UPDATE trigger optional (add if enrichment mutates rows).
```

Categories (taxonomy, low cardinality → normalized lookup):

```sql
CREATE TABLE categories (
  canonical  TEXT PRIMARY KEY,          -- 'infrastructure.network.dns'
  parent     TEXT REFERENCES categories(canonical),
  label      TEXT,                      -- human label
  builtin    INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE category_aliases (
  alias      TEXT PRIMARY KEY,          -- 'dns_error' -> canonical
  canonical  TEXT NOT NULL REFERENCES categories(canonical)
);
```

Fingerprint rollup (materialized recurrence stats; refreshed on write or on demand):

```sql
CREATE TABLE fingerprints (
  fingerprint  TEXT PRIMARY KEY,
  first_seen   TEXT NOT NULL,
  last_seen    TEXT NOT NULL,
  count        INTEGER NOT NULL DEFAULT 1,
  sample_event_id TEXT NOT NULL,        -- representative
  category     TEXT,
  title        TEXT                     -- normalized message
);
CREATE INDEX ix_fp_last ON fingerprints(last_seen DESC);
CREATE INDEX ix_fp_count ON fingerprints(count DESC);
```

Correlation edges (explicit graph beyond parent_event_id, for many-to-many correlation):

```sql
CREATE TABLE error_edges (
  src_event_id TEXT NOT NULL,
  dst_event_id TEXT NOT NULL,
  rel          TEXT NOT NULL,           -- 'caused_by'|'correlated'|'retry_of'|'child_of'
  PRIMARY KEY (src_event_id, dst_event_id, rel)
);
CREATE INDEX ix_edges_dst ON error_edges(dst_event_id, rel);
```

PRAGMAs (set on every connection open):

```
journal_mode = WAL;   synchronous = NORMAL;   foreign_keys = ON;
busy_timeout = 5000;  temp_store = MEMORY;     cache_size = -8000; -- ~8MB
wal_autocheckpoint = 1000;   mmap_size = 268435456;  -- 256MB, read conns
```

**Normalized vs denormalized:** events wide + denormalized (`root_cause`, `stack_norm`,
generated cols) for single-table read speed — errors are write-once/read-many. Only
`categories` normalized (small, hierarchical, aliased). `fingerprints`/`error_edges` are
derived/relational satellites, not duplication.

**Archival:** `witslog archive` moves rows older than N days into a sibling
`witslog-YYYYqN.db` (attached, `INSERT..SELECT`, delete from live), keeping live DB hot.
Archives are queryable via `ATTACH`.

**Pruning:** `witslog prune --older-than / --keep-last / --max-rows / --max-bytes`;
delete in batches, then incremental `VACUUM`/`wal_checkpoint(TRUNCATE)`.

**Backup:** SQLite online backup API (`witslog backup out.db`) — consistent snapshot
without stopping writers; or copy the file after `wal_checkpoint(TRUNCATE)`.

**Performance:** append-only INSERTs; prepared statements; batch writes in one
transaction when buffered; WAL checkpointed off the hot path; FTS5 `contentless`-style
external-content table avoids storing text twice.

---

## 4. FTS5 Search Design

**Virtual table:** external-content FTS5 (`content='events'`) so text isn't duplicated;
FTS index references `events.id`. Columns indexed: `message`, `exception`, `stack_norm`,
`root_cause`, `tags_text`, `category`.

**Tokenizer:** `unicode61 remove_diacritics 2` with `tokenchars '._:/-'`. Rationale:
error text is full of `module.sub.fn`, `ETIMEDOUT`, `/etc/hosts`, `api/v2/users`,
`snake_case`/`kebab-case`. Default tokenizer splits on `.` `/` `_` `-` and destroys these
identifiers. Adding them as token chars keeps `connection.refused`, `E_NOENT`, file paths
searchable as units. (Offer `trigram` tokenizer as an opt-in secondary index for
substring/contains queries on codes; note trigram costs more space.)

**Search over `stack_norm`, not raw `stacktrace`** — normalization strips memory
addresses, line numbers, tmp paths, PIDs so the *shape* of the trace is searchable and
duplicate traces collide.

**Query capabilities exposed by the query engine → FTS5 MATCH:**
- **prefix:** `timeout*` → all `timeout`, `timed`, `timeouts`. Configurable
  `prefix='2 3'` index for fast short prefixes.
- **phrase:** `"connection refused"` → adjacency.
- **NEAR:** `NEAR(dns timeout, 5)` → co-occurrence within window.
- **boolean:** `dns AND (timeout OR unreachable) NOT cache`.
- **column filter:** `exception:IOError`, `category:network`.
- **ranking:** `bm25(events_fts)` with column weights, e.g. weight message×3,
  exception×2, root_cause×2, stack_norm×1, tags×2, category×1. Return `ORDER BY rank`.

Query shape:

```sql
SELECT e.*, bm25(events_fts, 3.0,2.0,1.0,2.0,2.0,1.0) AS rank
FROM events_fts JOIN events e ON e.id = events_fts.rowid
WHERE events_fts MATCH :q
  AND e.ts_epoch_ms BETWEEN :from AND :to   -- structured filters combine freely
ORDER BY rank
LIMIT :limit OFFSET :offset;   -- (keyset preferred, see MCP pagination)
```

**Why FTS5 is sufficient:** target queries are lexical retrieval ("find errors mentioning
`ECONNREFUSED` in `network` since v1.4"), not paraphrase search. FTS5 gives ranked,
prefix/phrase/boolean, column-weighted, sub-ms lexical search with **zero** external
deps, deterministic results, and it's built into SQLite. Trade-offs: (a) no semantic/
synonym matching — mitigated by taxonomy + tags + fingerprints giving structured recall,
and the consuming AI supplies synonyms into the query; (b) tokenizer choice is a
compromise (identifier-aware unicode61 vs trigram substring) — solved by primary
unicode61 + optional trigram index; (c) FTS index adds write cost + space (~+30-60% of
text size) — acceptable, controllable via which columns are indexed.

---

## 5. MCP Specification

Transport: JSON-RPC 2.0 over **stdio** (default; how `witslog serve-mcp` runs) and
optional **HTTP/SSE**. Implements MCP `tools/list`, `tools/call`. Provider-independent —
raw protocol, JSON Schema per tool. All tools read-only except `witslog_delete`, the sole
write-capable tool (logging itself is still not an MCP tool by default). `witslog_delete`
is gated behind `--allow-write` and only deletes events with `resolved_at IS NOT NULL`
(or explicit `force:true`), so the AI assistant can clean up stale/resolved errors
without risk of deleting live ones.

Global conventions:
- **Pagination:** keyset via opaque `cursor` (base64 of `{ts_epoch_ms,id}`), plus `limit`
  (default 20, max 200). Response returns `next_cursor` when more.
- **Time:** all tools accept `from`/`to` (RFC3339 or relative `-24h`,`-7d`).
- **Common filters object** reused by most tools: `application, version, environment,
  command, subsystem, hostname, severity(min), category, error_code, correlation_id,
  fingerprint, tags[]`.
- **Errors:** JSON-RPC error object; `code` (-32602 invalid params, -32000 domain),
  `message`, `data.detail`. Never leak raw SQL. Enforce `limit` caps, statement timeout.
- **Output:** structured JSON content block; large text truncated with `truncated:true`.

Tools:

| Tool | Purpose | Key inputs | Output | SQL strategy | Perf notes |
|------|---------|-----------|--------|--------------|-----------|
| **search_errors** | lexical + structured search | `query`(FTS), common filters, `from/to`, `limit`, `cursor`, `order`(rank\|time) | `{items[], next_cursor, total_estimate}` | FTS5 MATCH JOIN events + filters (§4) | bm25 rank; keyset page; cap limit |
| **latest_errors** | most recent failures | filters, `severity_min`, `limit`, `cursor` | `{items[], next_cursor}` | `ix_events_ts` desc + filters | pure index scan, no FTS |
| **summarize_errors** | aggregate roll-up for a window/filter | filters, `from/to`, `group_by[]`(category\|subsystem\|version\|fingerprint\|host) | `{total, by_group[{key,count,last_seen}], top_fingerprints[]}` | `GROUP BY` over indexed cols; join `fingerprints` | covered by indexes; returns counts not rows |
| **classify_error** | assign taxonomy to a raw error text/event | `message, exception?, stacktrace?, context?` | `{category, subcategory, confidence, matched_rules[], suggested_tags[]}` | taxonomy engine rules (regex/keyword/code map) + `categories` table; no write | pure fn; deterministic |
| **explain_error** | full dossier on one error | `event_id` \| `fingerprint` | `{event, root_cause, chain[], recurrence{count,first,last}, similar_count, category_path, context}` | point lookup + `error_edges` walk + `fingerprints` | O(chain depth) |
| **similar_errors** | find recurrences / near-dupes | `event_id`\|`fingerprint`, `mode`(fingerprint\|lexical), `limit` | `{items[], grouping}` | fingerprint equality (exact) OR FTS MATCH on normalized message of source | fingerprint mode = index eq; lexical = FTS |
| **list_categories** | taxonomy tree + counts | `include_counts?`, `window?` | `{tree[], aliases[]}` | recursive CTE over `categories` + optional counts | small table; cache |
| **statistics** | headline metrics | filters, `from/to` | `{total, by_severity{}, by_category{}, error_rate_per_day, unique_fingerprints, top_hosts[]}` | multiple `GROUP BY`; single conn | index-covered aggregates |
| **timeline** | bucketed counts over time | filters, `from/to`, `bucket`(hour\|day\|week), `series_by?` | `{buckets:[{t,count,...series}]}` | `strftime` bucket on `ts_epoch_ms` + GROUP BY | range index scan |
| **top_failures** | ranked recurring failures | filters, `from/to`, `by`(count\|recency\|severity), `limit` | `{items:[{fingerprint,title,count,last_seen,category,sample_event_id}]}` | `fingerprints` ordered by chosen metric | materialized table = O(limit) |
| **list_traces** *(corr)* | events for a correlation/request id or caused-by chain | `correlation_id`\|`root_event_id` | `{ordered_events[], edges[]}` | `ix_events_corr` / recursive edge walk | bounded by trace size |
| **search_all** *(federation, opt-in)* | search across multiple attached project DBs | `query`, filters, `projects[]?` | `{items[] (+source_db), by_project[]}` | ATTACH each project DB read-only + `UNION ALL` over per-DB FTS queries | only enabled with `--attach`; caps # DBs |
| **witslog_delete** *(write)* | delete stale/resolved error(s) | `event_id`\|`fingerprint`\|`filter{resolved_before,...}`, `dry_run?`, `force?` | `{deleted_count, deleted_ids[]}` | `DELETE FROM events WHERE resolved_at IS NOT NULL AND ...` (+ cascade `error_edges` by src/dst) | gated behind `--allow-write`; requires `resolved_at IS NOT NULL` unless `force:true`; `dry_run` defaults true (preview only) |

Per-tool error handling: validate against JSON Schema first (reject -32602); enforce
window sanity (`from<=to`); clamp `limit`; wrap DB errors as -32000 with generic message +
`data.retriable`. Statement timeout (e.g. 2s) via `sqlite3_progress_handler`/interrupt to
protect the AI session from pathological queries.

Example tool schema (`search_errors`, abbreviated JSON Schema):

```json
{
  "name": "search_errors",
  "description": "Lexical + structured search over logged error events.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {"type":"string","description":"FTS5 expression (prefix*, \"phrase\", AND/OR/NOT, col:term)"},
      "from": {"type":"string"}, "to": {"type":"string"},
      "application":{"type":"string"}, "version":{"type":"string"},
      "environment":{"type":"string"}, "subsystem":{"type":"string"},
      "category":{"type":"string"}, "severity_min":{"type":"string"},
      "hostname":{"type":"string"}, "tags":{"type":"array","items":{"type":"string"}},
      "order":{"enum":["rank","time"],"default":"rank"},
      "limit":{"type":"integer","default":20,"maximum":200},
      "cursor":{"type":"string"}
    }
  }
}
```

---

## 6. Module Breakdown (reference: Rust workspace)

```
witslog/
├─ Cargo.toml                      # workspace
├─ crates/
│  ├─ witslog-core/                # #1 logging + #3 taxonomy + fingerprint + redact
│  │  ├─ event.rs                  #   event model, builder, severity
│  │  ├─ enrich.rs                 #   hostname/pid/git/cwd/env enrichers
│  │  ├─ redact.rs                 #   PII/secret filters
│  │  ├─ fingerprint.rs            #   stack normalize + canonical hash + event_id
│  │  ├─ taxonomy.rs               #   category tree, aliases, classify rules
│  │  └─ buffer.rs                 #   batching, backpressure, sync/async sink
│  ├─ witslog-store/               # #2 storage layer
│  │  ├─ schema/                   #   embedded .sql migrations 0001_init.sql ...
│  │  ├─ migrate.rs                #   version detect + apply + checksum
│  │  ├─ conn.rs                   #   pragma setup, write-serializer, read pool
│  │  └─ writer.rs                 #   insert path, fts sync, fingerprint rollup
│  ├─ witslog-query/               # #4 query engine + #7 analytics
│  │  ├─ search.rs                 #   FTS5 builder, bm25 weights
│  │  ├─ filters.rs                #   structured filter → SQL params
│  │  ├─ aggregates.rs             #   stats/timeline/top_failures/MTTR
│  │  └─ correlate.rs             #   edge walks, trace assembly
│  ├─ witslog-mcp/                 # #5 MCP server
│  │  ├─ transport.rs              #   stdio + http/sse json-rpc
│  │  ├─ tools/                    #   one file per tool, schema + handler
│  │  └─ registry.rs               #   tools/list, dispatch, validation
│  ├─ witslog-config/              # #8 config manager (layered resolve)
│  ├─ witslog-plugin/              # extensibility traits + dynamic load
│  ├─ witslog-cli/                 # #6 CLI (clap) — builds `witslog` binary
│  └─ witslog-ffi/                 # C ABI: witslog_log/witslog_resolve/witslog_delete for embedding
├─ bindings/                       # thin wrappers: python/ node/ go/ ruby/ (call ffi)
├─ install/                        # #9 scripts, packaging manifests
├─ tests/                          # integration, migration, load
└─ bench/                          # criterion benches
```

Layer deps: `cli`+`mcp`+`ffi` → `query`+`core` → `store` → `config`. No cycles.
`plugin` traits consumed by `core`(enrichers/taxonomy), `query`(exporters), `mcp`(extra tools).

---

## 7. Installation & Bootstrap Guide

**Install (minimal manual config, OS-agnostic):**
- **Standalone binary** (primary): one static `witslog` per OS/arch from GitHub Releases.
  `install.sh` (Linux/macOS) + `install.ps1` (Windows) detect OS/arch, download, verify
  checksum/signature, place on PATH (`~/.local/bin` / `%LOCALAPPDATA%\witslog\bin`).
- **Package managers:** Homebrew tap (mac/Linux), Scoop + winget (Windows), `cargo
  install witslog-cli`, AUR, `.deb`/`.rpm` via `cargo-dist`/`nfpm`. Language SDKs via
  pip/npm/gem wrapping the FFI lib.
- **Container (optional):** distroless image running `witslog serve-mcp` for a
  shared-host MCP endpoint over a mounted DB.

**DB resolution (per-project, precedence high→low):**
1. Explicit `--db PATH` flag or `WITSLOG_DB` env → use it.
2. Walk up from cwd; first ancestor containing `.witslog/` → `<that>/.witslog/witslog.db`
   (project-scoped; the normal case).
3. `witslog init` run in a dir with no marker → create `./.witslog/` HERE (new project root).
4. `--global` flag, or config `default_scope=global` with no marker found → OS global DB
   (per-OS dir below). Off by default; a `witslog log` with no resolvable project errors
   loudly rather than silently writing to a shared DB.

The FFI/lib uses the same walk-up so an embedded app logs into its own project DB
automatically. Each `.witslog/` holds its own `witslog.db`, WAL files, and optional
project `config.toml`.

**Bootstrap workflow (`witslog init`):**
1. Resolve DB path (rule above — default: create `./.witslog/witslog.db` at cwd).
2. Create `.witslog/` dir (0700) + DB file (0600); set WAL + pragmas.
3. Run migrations 0→latest; write `schema_meta` (schema_version, witslog_version, created_at).
4. Seed builtin `categories` + aliases.
5. Write default config file if absent.
6. Optionally emit MCP client registration snippet (`--print-mcp-config`).

**DB & config locations:**
- **Primary (per-project):** `<project>/.witslog/witslog.db` + optional
  `<project>/.witslog/config.toml`. Travels with the repo; add `.witslog/` to
  `.gitignore` (or commit config only, ignore `*.db*`).
- **Global config (defaults only, not error data)** — XDG/OS conventions:
  Linux `$XDG_CONFIG_HOME/witslog/config.toml`; macOS `~/Library/Application Support/witslog/config.toml`;
  Windows `%APPDATA%\witslog\config.toml`. Sets org-wide defaults (redaction rules, retention).
- **Optional global DB (opt-in fallback only):** Linux `$XDG_DATA_HOME/witslog/global.db`;
  macOS `~/Library/Application Support/witslog/global.db`; Windows `%LOCALAPPDATA%\witslog\global.db`.

**MCP server setup:** `witslog serve-mcp [--db PATH] [--stdio|--http PORT]`. Server binds
to ONE resolved project DB (walk-up from where it launches, or `--db`). Each project =
its own MCP endpoint → the AI inspects that project's errors in isolation. To reason
across projects, either run one endpoint per project, or use the federation tool
(`--attach PATH...` / `search_all`) that ATTACHes multiple project DBs read-only into one
query. Ship generic `mcpServers` snippet: command=`witslog`,
args=`["serve-mcp","--stdio"]`, launched with `cwd` = the project.

**Schema migration:** forward-only numbered SQL files embedded in binary; each migration
recorded with checksum. On any command, store layer checks `schema_version`; if binary
newer → auto-apply (with backup first); if DB newer than binary → refuse + tell user to
upgrade CLI (version compatibility guard).

**Upgrade:** re-run installer / `brew upgrade` / `scoop update`. Binary self-check:
`witslog doctor` reports version skew. Migrations applied lazily + idempotently. Always
snapshot DB before applying (`.bak`).

**Uninstall:** `witslog uninstall` (removes binary + optionally data with `--purge`), or
package-manager remove. Data files documented so manual cleanup is trivial (it's just files).

**Version compatibility:** semver on binary; `schema_version` integer. Matrix rule:
binary supports schema ≤ its max and ≥ a min; refuse out-of-range with clear message.
`witslog export` (portable NDJSON) is the cross-version escape hatch.

---

## 8. Development Roadmap

**M1 — Storage + event model foundation**
- Objectives: event schema, migrations engine, WAL/pragma conn mgmt, insert path,
  `witslog init`. Deps: none. Complexity: **M**.
- Accept: `init` creates DB+schema; can insert+read an event round-trip; migrations apply
  idempotently with checksums; WAL verified; STRICT tables enforced.

**M2 — Logging library + client API + FFI**
- Objectives: fluent builder, `error/warn/info/exception`, enrichers, redaction,
  fingerprint + deterministic `event_id`, buffering, C ABI, `resolved_at` lifecycle
  (`mark_resolved`) and deletion API (`delete_resolved`/`witslog_delete`) for
  stale-error cleanup by CLI/FFI/MCP. Deps: M1. Complexity: **M**.
- Accept: host program (Rust + one FFI language) logs structured event; PII redacted;
  identical errors share fingerprint; overhead <100µs buffered (bench in M7); an event
  can be marked resolved and then deleted via `witslog_delete`/CLI, unresolved events
  are protected without `force:true`.

**M3 — Taxonomy engine**
- Objectives: builtin hierarchy (infra/app/runtime/external), aliases, tag rules,
  rule-based auto-classify, custom categories. Deps: M1. Complexity: **S–M**.
- Accept: known samples map to correct canonical category; custom category + alias work;
  `classify` deterministic.

**M4 — Query engine + FTS5**
- Objectives: FTS5 table+triggers, tokenizer, bm25 ranking, structured filters, keyset
  pagination, correlation walks, aggregates (stats/timeline/top_failures). Deps: M1–M3.
  Complexity: **L**.
- Accept: every §Query-Model query returns correct rows using the intended index (verify
  with `EXPLAIN QUERY PLAN`); FTS prefix/phrase/boolean/NEAR all work.

**M5 — CLI utilities**
- Objectives: `log query stats export import vacuum prune migrate doctor config archive
  backup`. Human + `--json` output. Deps: M1–M4. Complexity: **M**.
- Accept: each command works cross-OS; `doctor` reports version/schema/health; export→import
  round-trips.

**M6 — MCP server**
- Objectives: JSON-RPC/stdio, tools/list + tools/call, all §5 tools, schema validation,
  pagination, statement timeout, HTTP/SSE optional. Deps: M4. Complexity: **L**.
- Accept: passes MCP protocol conformance; a real MCP client lists + calls every tool;
  invalid params rejected; large results paginate.

**M7 — Perf + hardening**
- Objectives: benches (write/read/index), load tests, concurrency (multi-writer WAL
  retry), memory footprint, prune/archive/backup at scale. Deps: M1–M6. Complexity: **M**.
- Accept: meets §Performance targets; no corruption under concurrent writers; bench suite
  in CI.

**M8 — Packaging + install + docs**
- Objectives: cross-compiled binaries, install.sh/ps1, brew/scoop/winget/cargo, SDK
  wrappers, install/upgrade/uninstall, MCP setup docs. Deps: M2,M5,M6. Complexity: **M**.
- Accept: clean install→init→log→serve-mcp on Linux/macOS/Windows from scratch in one
  command each.

**M9 — Extensibility + security**
- Objectives: plugin traits (taxonomy/exporter/enricher/storage/notifier/mcp-tool),
  redaction ruleset config, optional encryption-at-rest, audit trail + tamper hash chain.
  Deps: M2–M6. Complexity: **M–L**.
- Accept: a sample plugin of each type loads; redaction configurable; audit chain detects
  tampering.

Critical path: M1→M2/M4→M6→M8. M3 parallel to M2. M7/M9 after core.

---

## 9. Risks

| Risk | Impact | Mitigation |
|------|--------|-----------|
| Multi-process SQLite writer contention (app + CLI + 2nd app instance, same project DB) | write stalls / `SQLITE_BUSY` | per-project isolation already shrinks writer set; WAL + `busy_timeout` + bounded retry + short append txns; sidecar option if volume high |
| Wrong-dir / DB sprawl — command run outside project resolves wrong or empty DB; many `.witslog/` dirs to manage | confusing empty results, orphan DBs | every command prints the resolved DB path; `doctor` shows discovery walk + scope; error loudly when no project + no `--global`; `witslog list-dbs` to enumerate known project DBs |
| Logging in hot path adds latency / blocks app on I/O | app slowdown | async buffered sink + batch commit; sync mode only opt-in; never fail the app on log error (drop+count) |
| FTS5 no semantic match → AI misses paraphrased errors | recall gaps | taxonomy+tags+fingerprints as structured recall; AI expands query terms; optional trigram index |
| Fingerprint over/under-grouping (too coarse/fine) | bad dedup/clusters | tunable normalization rules + version in fingerprint algo; store `fp_algo_version`; recompute tool |
| Schema evolution / migration on user data | corruption/lock | forward-only numbered migrations, checksum, auto-backup before apply, version-compat guard, tests |
| Unbounded DB growth | disk blowout | prune/archive policy + `max_bytes`; `doctor` warns; WAL checkpoint/vacuum |
| Sensitive data (PII/secrets/stack locals) logged | privacy/leak | default redactors on, allow/deny lists, `--no-context` mode, encryption-at-rest option, restrictive file perms (0600) |
| MCP tool runs pathological query → stalls AI session | DoS-ish | statement timeout/interrupt, limit caps, index-only query plans verified |
| Cross-OS path/perm/line-ending divergence | install/runtime bugs | OS-abstraction in config layer; CI matrix Linux/macOS/Windows; compatibility tests |
| FFI ABI stability across many years | breakage for embedders | narrow, versioned C ABI (JSON-in/int-out); semver; keep surface tiny |
| MCP spec evolves | server drift | isolate protocol in `transport.rs`; conformance tests; version-negotiate |

---

## 10. Future Enhancements (deferred, intentionally)

- **Optional semantic layer** — pluggable, off by default; only if lexical proves
  insufficient. Never a core dep.
- **Sidecar daemon + wire protocol** — promote to first-class if in-proc contention hurts
  high-volume users (event schema already the contract).
- **MTTR / resolution tracking** — event `resolved_at`, `resolution` edges; MTTR analytics
  (schema leaves room; compute deferred).
- **Regression auto-detection** — alert when a fingerprint reappears after a version where
  it was absent.
- **Live tail / streaming MCP resource** — subscribe to new matching errors.
- **Cross-DB federation (near-term, opt-in)** — `search_all` / `--attach` already in scope
  to query several per-project DBs + archives in one read-only view via ATTACH; deeper
  cross-project analytics (org-wide trends, dedup across repos) deferred.
- **Notifiers** — plugin exporters to webhook/desktop/file (kept out of core, no cloud).
- **Web/TUI dashboard** — read-only local viewer over the same query engine.
- **Sampling / rate-limiting** — for very high-volume producers.

---

## Verification

No product code yet — this deliverable is the spec. Validate the spec itself:
1. **Schema sanity:** load §3 DDL into a scratch SQLite (`sqlite3 :memory:`), confirm it
   parses, FTS5 available, triggers fire on insert, `EXPLAIN QUERY PLAN` uses intended
   indexes for each §Query-Model pattern.
2. **FTS behavior:** insert sample errors, confirm prefix/phrase/boolean/NEAR + bm25 order.
3. **MCP shape:** dry-run each tool's JSON Schema against sample inputs (schema-validate).
4. Once M1 lands, verify end-to-end: `witslog init` → log event via lib → `witslog query`
   → `serve-mcp` → real MCP client calls `search_errors`/`latest_errors`.

Acceptance of *this plan* = user agrees scope, embedding model (in-proc lib + CLI),
Rust reference, and milestone order before any implementation begins.