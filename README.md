# witslog — AI-Native Error Intelligence Framework

Structured error logging that AI assistants (via MCP) can query and reason about. **No embeddings, no cloud, no infra.**

Errors are stored in a local SQLite DB organized by category, searchable via full-text indexing, and queryable with deterministic taxonomy + structured filters. Built for embedding inside apps and for cross-language integration via C FFI.

## Quick Start

### Installation

**Via installer** (coming in P8):
```bash
curl -fsSL https://witslog.dev/install.sh | sh
```

**Via Rust** (dev):
```bash
cargo install --path crates/witslog-cli
```

### Initialize a project
```bash
witslog init .
```
Creates `./.witslog/witslog.db` in the current directory.

### Log an event
```bash
witslog log app "connection timeout" --error-code ETIMEDOUT --severity error
```

### Query by ID
```bash
witslog query <event-id>
```

### Full CLI help
```bash
witslog --help
```

## Status

**Pre-1.0, usable for**: init, log, query (by ID), resolve, delete, doctor.

**Shipped** (P0 + partial P1/P2):
- ✅ Structured event model (JSON-friendly).
- ✅ SQLite storage with WAL, per-project isolation.
- ✅ Event fingerprinting for dedup (normalized message + stack + category).
- ✅ Redaction of secrets/PII before store.
- ✅ Resolved/unresolved event lifecycle + deletion.
- ✅ **Taxonomy**: builtin error category tree (infrastructure/application/runtime/external), deterministic auto-classify via rules (error_code/exception/message keyword/regex).
- ✅ C FFI for embedding in any language.
- ✅ CLI: init, log, query, resolve, delete, doctor.

**In progress** (P1):
- 🟡 Async buffering, config sections.
- 🟡 Enrichment (hostname/pid/cwd/git_commit auto-capture).

**Not yet** (P3–P9):
- ⬜ Full-text search (FTS5) — placeholder in schema, migration pending.
- ⬜ Query engine (stats, timeline, aggregates).
- ⬜ CLI: query (FTS), export, import, prune, archive, backup.
- ⬜ MCP server (AI assistant integration).
- ⬜ SDK bindings (Python, Node).
- ⬜ Packaging + cross-platform install.
- ⬜ Extensibility (plugins) + security (encryption, audit).

## Architecture

```
App code
   ↓
EventBuilder (fluent) → enrich → redact → classify → build
   ↓
SQLite (WAL mode, per-project)
   ├─ events (append-only, denormalized)
   ├─ categories (taxonomy tree)
   ├─ fingerprints (dedup/rollup)
   ├─ error_edges (causality graph)
   └─ schema_meta + migrations
   ↓
Read-only access:
   ├─ CLI (init, query, stats, export, ...)
   ├─ MCP server (JSON-RPC tools for AI)
   └─ Analytics (trends, MTTR)
```

**Single source of truth**: local SQLite file. No syncing, no cloud, full control.

## Examples

### Rust
```rust
use witslog_core::{error, Classifier};

let classifier = Classifier::built_in();

let event = error("my-app", "DNS resolution failed")
    .error_code("ENOTFOUND")
    .classify(&classifier)  // Assigns category: infrastructure.network.dns
    .build();

eprintln!("Event ID: {}", event.event_id);
eprintln!("Category: {:?}", event.category);
```

### Python (via FFI, coming in P6)
```python
import witslog

witslog.error("my-app", "connection refused", error_code="ECONNREFUSED")
```

### Node (via FFI, coming in P6)
```javascript
const witslog = require('witslog');

witslog.error('my-app', 'out of memory', { context: { pid: process.pid } });
```

## CLI Subcommands

**Current**:
- `witslog init [DIR]` — initialize DB at `.witslog/witslog.db`.
- `witslog log <app> <msg>` — log an event. Flags: `--severity`, `--error-code`, `--exception`, `--category`, `--context JSON`, `--tags`, etc.
- `witslog query <event-id>` — fetch one event by ID (human or `--json`).
- `witslog resolve <event-id>` — mark resolved.
- `witslog delete [filters]` — delete resolved events. `--dry-run` for preview.
- `witslog doctor` — health check (schema version, migration status).

**Planned** (P3–P4):
- `witslog query <FTS-query> [--filters]` — full-text + structured search with `--json --cursor`.
- `witslog stats [--filters]` — aggregates (totals, by severity/category/host).
- `witslog export [--from] [--to]` — NDJSON stream to stdout/file.
- `witslog import <file>` — consume NDJSON.
- `witslog prune --older-than 30d` — delete aged rows in batches.
- `witslog archive --older-than 30d` — move to sibling archive DB.
- `witslog vacuum` — shrink DB.
- `witslog migrate` — apply pending migrations + backup.
- `witslog config [get|set|path]` — read/write config.

## Config

Optional: `.witslog/config.toml` (per-project) or global at `~/.config/witslog/config.toml`.

Example:
```toml
[enrich]
hostname = true
pid = true
cwd = true
git_commit = true

[redact]
custom_patterns = ["password=.*", "token=.*"]

[buffer]
enabled = false
batch_size = 50
flush_interval_ms = 1000

[taxonomy]
auto_classify_enabled = true
# custom_rules_file = "./taxonomy-rules.json"  # coming in P2
```

## Integration with AI

**Via MCP** (coming in P5):
```
witslog serve-mcp --stdio
```
Runs as a stdio server; clients (Claude, other LLMs) call tools:
- `search_errors` — FTS + structured filters.
- `latest_errors` — recent failures.
- `classify_error` — determine category for raw error.
- `explain_error` — full dossier + recurrence stats.
- `similar_errors` — near-dupes by fingerprint or lexical match.
- `list_categories` — taxonomy tree.
- `statistics` — headline metrics.
- `timeline` — bucketed trends.
- `top_failures` — ranked recurring issues.
- `list_traces` — causality chains.

MCP registration snippet:
```json
{
  "mcpServers": {
    "witslog": {
      "command": "witslog",
      "args": ["serve-mcp", "--stdio"]
    }
  }
}
```

**Via C FFI** (shipped in P0):
Any language can call:
```c
const char* event_json = "{ \"message\": \"boom\", ... }";
int result = witslog_log(db_path, event_json);
// result: 1 = ok, -1 = error (check via witslog_free_string)
```

## Taxonomy

**Builtin categories** (infrastructure/application/runtime/external):
- `infrastructure.network.{dns, timeout, connection}`
- `infrastructure.storage.{disk, database}`
- `infrastructure.compute.{memory, cpu}`
- `application.{error, validation, authentication, authorization}`
- `runtime.{panic, segfault, outofmemory}`
- `external.{api.rate_limit, service}`

**Auto-classify rules** (deterministic, order matters):
1. Error code map (e.g., `ETIMEDOUT` → `infrastructure.network.timeout`).
2. Exception type map (e.g., `IOException` → `infrastructure.storage`).
3. Message keyword/regex (case-insensitive substring or regex).

No match → `category: null`, tag: `unclassified`.

**Custom categories & aliases** (coming in P2):
```bash
witslog category add custom.app.feature "My Feature Error"
witslog alias "myalias" custom.app.feature
```

## Performance Targets

- **Buffered log call**: < 100 µs (hot path).
- **Single-writer throughput**: ≥ 10k events/s.
- **Search on 100k events**: < 50 ms.
- **Classification**: < 50 µs.
- **Idle CLI memory**: < 15 MB; MCP server < 30 MB.

(Measured in P7; in-flight targets.)

## Development

See **CLAUDE.md** for architecture, crate map, and conventions. **PLAN.md** has full spec; **PHASES.md** has phase-by-phase requirements.

```bash
# Build all
cargo build --workspace

# Test
cargo test -p witslog-core
cargo test --test p2_integration

# Lint
cargo clippy --workspace

# Examples
cargo run --example p2_classify
```

## License

Apache License 2.0. See LICENSE.

## Contributing

Not accepting external PRs yet (pre-1.0). Issues + feedback welcome.

---

**Learn more**: [PLAN.md](PLAN.md) (design doc), [PHASES.md](PHASES.md) (phase roadmap), [CLAUDE.md](CLAUDE.md) (dev guide).
