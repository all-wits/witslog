<div align="center">

# 🪵 witslog

**AI-native error intelligence — structured, local, queryable by any MCP assistant.**

[![License: Apache 2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![npm](https://img.shields.io/npm/v/%40all-wits%2Fwitslog?label=npm&logo=npm)](https://www.npmjs.com/package/@all-wits/witslog)
[![CI: Node SDK release](https://img.shields.io/github/actions/workflow/status/all-wits/witslog/release-node-sdk.yml?label=node%20sdk%20release&logo=githubactions)](https://github.com/all-wits/witslog/actions/workflows/release-node-sdk.yml)
[![Rust](https://img.shields.io/badge/rust-stable-orange?logo=rust)](https://www.rust-lang.org/)
[![SQLite](https://img.shields.io/badge/storage-SQLite-003B57?logo=sqlite&logoColor=white)](https://sqlite.org/)

**No embeddings. No cloud. No infra.**

</div>

---

Errors are captured as **structured events** (not text lines), stored in a local SQLite
database per project, indexed with full-text search, auto-classified by a deterministic
taxonomy engine, and exposed to AI assistants over **MCP** (Model Context Protocol) — so an
LLM can search, correlate, and reason about your failure history without you writing a single
query.

## ✨ Features

- 🗄️ **Per-project SQLite** — one `.witslog/witslog.db`, WAL mode, zero external services.
- 🔍 **Full-text search** — FTS5, bm25 ranking, prefix/phrase/boolean/NEAR queries.
- 🏷️ **Deterministic taxonomy** — rule-based auto-classification, no model/embedding needed.
- 🔒 **Redaction built in** — secrets/PII stripped before anything touches disk.
- 🔗 **Correlation & fingerprinting** — dedup recurring errors, walk causality chains.
- 🤖 **MCP server** — 12 read tools + 1 gated write tool for any MCP-compatible assistant.
- 🌍 **Cross-language SDKs** — Node, Python, PHP/Laravel over a shared C ABI.

## 📦 Install

### CLI (Rust)

```bash
# Linux/macOS installer (detects OS/arch, verifies checksum, places on PATH)
curl -fsSL https://raw.githubusercontent.com/all-wits/witslog/main/install/install.sh | sh

# Windows (PowerShell)
irm https://raw.githubusercontent.com/all-wits/witslog/main/install/install.ps1 | iex

# From source (dev)
cargo install --path crates/witslog-cli
```

See [docs/install.md](docs/install.md) for package-manager options, upgrade, and uninstall.

### SDKs

| Language | Package | Install |
|----------|---------|---------|
| **Node.js** | [`@all-wits/witslog`](bindings/node) | `npm install @all-wits/witslog` · `pnpm add @all-wits/witslog` · `bun add @all-wits/witslog` |
| **Python** | [`witslog`](bindings/python) | `pip install witslog` |
| **PHP / Laravel** | [`witslog/witslog`](bindings/php) | `composer require witslog/witslog` |

Each SDK is a **framework-agnostic core** over the native C ABI (`witslog-ffi`) — no cloud
calls, no telemetry. See each package's README for framework adapters (Express, FastAPI,
Django, Flask, Laravel) and the [SDK↔native contract](bindings/CONTRACT.md).

## 🚀 Quick Start

### Initialize a project

```bash
witslog init .
```

Creates `./.witslog/witslog.db` in the current directory.

### Log an event

```bash
witslog log app "connection timeout" --error-code ETIMEDOUT --severity error
```

### Search & inspect

```bash
witslog query "timeout*" --severity error
witslog stats
witslog serve-mcp --stdio   # expose the log to an MCP-compatible AI assistant
witslog doctor              # binary version, max supported schema, DB health
```

### From an app (Node example)

```js
const witslog = require('@all-wits/witslog');

witslog.init();
witslog.error('my-app', 'out of memory', { context: { pid: process.pid } });
```

See [bindings/node](bindings/node), [bindings/python](bindings/python), and
[bindings/php](bindings/php) for the Python/PHP equivalents.

## 🧭 Status

Pre-1.0. Core logging, storage, taxonomy, search, MCP server, SDKs, and perf hardening are
shipped and tested; packaging (P8) is in progress and extensibility/security (P9) is next.

| Phase | What | Status |
|-------|------|--------|
| P0 | Storage + event model, CLI core, C ABI | ✅ |
| P1 | Enrichment, redaction, async buffering | ✅ |
| P2 | Taxonomy engine (auto-classify) | ✅ |
| P3 | FTS5 + query engine (search/aggregates/correlation) | ✅ |
| P4 | CLI utilities (export/import/prune/archive/backup/...) | 🟡 missing global `--json` |
| P5 | MCP server (12 tools, JSON-RPC/stdio) | ✅ |
| P6 | SDK bindings (Node/Python/PHP + framework adapters) | ✅ |
| P7 | Perf benches + concurrency hardening | ✅ |
| P8 | Packaging + cross-platform install | 🟡 install scripts + release CI + smoke test shipped, verified green on GitHub Actions; no cut release yet |
| P9 | Extensibility (plugins) + security (encryption, audit) | ⬜ |

See [CHANGELOG.md](CHANGELOG.md) for release notes and [PHASES.md](PHASES.md) for the detailed
per-phase spec.

## 🏗️ Architecture

```
App code (any language)
   ↓  SDK / native EventBuilder
enrich → redact → classify → build
   ↓
SQLite (WAL mode, per-project)
   ├─ events (append-only, denormalized + FTS5)
   ├─ categories (taxonomy tree)
   ├─ fingerprints (dedup/rollup)
   ├─ error_edges (causality graph)
   └─ schema_meta + migrations
   ↓
Read-only access:
   ├─ CLI (init, query, stats, export, ...)
   ├─ MCP server (JSON-RPC tools for AI assistants)
   └─ Analytics (trends, MTTR)
```

**Single source of truth**: a local SQLite file. No syncing, no cloud, full control.

## 🤖 Integration with AI (MCP)

```bash
witslog serve-mcp --stdio
```

Runs as a stdio JSON-RPC server. Any MCP-compatible client (Claude, other LLMs) can call:

`search_errors` · `latest_errors` · `summarize_errors` · `classify_error` · `explain_error` ·
`similar_errors` · `list_categories` · `statistics` · `timeline` · `top_failures` ·
`list_traces` · `search_all` (opt-in federation) · `witslog_delete` (gated, write)

MCP client registration snippet — generate it directly (fills in the resolved
binary path and project `cwd`):

```bash
witslog serve-mcp --print-mcp-config
```

```json
{
  "mcpServers": {
    "witslog": {
      "command": "witslog",
      "args": ["serve-mcp", "--stdio"],
      "cwd": "/path/to/your/project"
    }
  }
}
```

## 🏷️ Taxonomy

**Builtin categories** (infrastructure/application/runtime/external):

```
infrastructure.network.{dns, timeout, connection}
infrastructure.storage.{disk, database}
infrastructure.compute.{memory, cpu}
application.{error, validation, authentication, authorization}
runtime.{panic, segfault, outofmemory}
external.{api.rate_limit, service}
```

**Auto-classify rules** (deterministic, in order): error-code map → exception-type map →
message keyword/regex. No match → `category: null`, tagged `unclassified`.

## ⚡ Performance Targets

| Metric | Target |
|--------|--------|
| Buffered log call | < 100 µs |
| Single-writer throughput | ≥ 10k events/s |
| Search on 100k events | < 50 ms |
| Classification | < 50 µs |
| Idle CLI memory | < 15 MB |
| Idle MCP server memory | < 30 MB |

(Measured in P7; in-flight targets.)

## 🛠️ Development

See **[CLAUDE.md](CLAUDE.md)** for architecture, crate map, and conventions.
**[PLAN.md](PLAN.md)** has the full spec; **[PHASES.md](PHASES.md)** has phase-by-phase
requirements.

```bash
# Build all
cargo build --workspace

# Test (Rust workspace)
cargo test --workspace

# Test an SDK
py -m pytest bindings/python/tests          # Python
npm test --prefix bindings/node             # Node
composer test --working-dir=bindings/php    # PHP

# Cross-language regression driver
pwsh bindings/e2e/run.ps1

# Lint
cargo clippy --workspace
```

## 📄 License

Apache License 2.0. See [LICENSE](LICENSE).

## 🤝 Contributing

Not accepting external PRs yet (pre-1.0). Issues + feedback welcome.

---

<div align="center">

**Learn more**: [PLAN.md](PLAN.md) (design doc) · [PHASES.md](PHASES.md) (phase roadmap) ·
[CLAUDE.md](CLAUDE.md) (dev guide) · [CHANGELOG.md](CHANGELOG.md) (release notes) ·
[bindings/CONTRACT.md](bindings/CONTRACT.md) (SDK↔native ABI)

</div>
