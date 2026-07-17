# P10 — MTTR/Resolution, Notifiers, Browser-Side Error Capture

## Context

PLAN.md §10 lists three deferred enhancements: **MTTR/resolution tracking**, **notifiers**, and **browser-side error capture**. All three are now wanted. The driving goal is the third one's rationale: witslog's whole pitch is that an AI assistant can query a project's failure history. Today the SDKs are Node/Python/PHP processes calling native FFI, so a JS error thrown in a browser tab is invisible. That means the AI sees only half the system. Closing that gap — client-side and server-side errors in one queryable DB — is what makes cross-boundary debugging automatic rather than manual.

A `/feature-forge` requirements pass produced a draft; a `/cto-advisor` subagent argued against it using only `codegraph_*`. The argument surfaced a **pre-existing production bug** that reframes the whole phase (below). This plan is the post-argument result: three features survive, but scoped down, plus one bug fix promoted to a blocker.

---

## Ground truth established via codegraph

Verified, with locations. Read these before implementing.

| Fact | Where |
|---|---|
| `migrate()` runs migrations 1..6; `CURRENT_SCHEMA_VERSION == 6`. Next is 7. | `crates/witslog-store/src/migrate.rs:17` |
| `events.resolved_at TEXT` + `ix_events_unresolved` already exist. | `crates/witslog-store/src/migrate.rs:190` |
| `mark_resolved` ignores the affected-row count → silently "succeeds" on an unknown `event_id`, and overwrites an already-set `resolved_at`. | `crates/witslog-store/src/writer.rs:72` |
| `Filters` has **no** resolved/unresolved axis. | `crates/witslog-query/src/filters.rs:4` |
| `AggregateEngine` has statistics/timeline/top_failures/fingerprint_stats. No MTTR. | `crates/witslog-query/src/aggregates.rs:44` |
| `Notifier` trait + panic-isolated `PluginRegistry::dispatch_event` exist, but codegraph shows the **only callers are the crate's own tests** — the registry is not wired into the write path at all. | `crates/witslog-plugin/src/lib.rs:58` |
| `build_and_write` is the single write pipeline; `capture` → `write_via_snapshot`; `capture_sync` is the panic path that deliberately forces a sync write. | `crates/witslog-runtime/src/lib.rs:257`, `:178`, `:185` |
| SDK payload allow-list has no `subsystem`/`command`/`hostname`/`ingest_source`. | `bindings/python/witslog/__init__.py:51` (mirrored in `bindings/php/src/Payload.php:10`, `bindings/node/lib/payload.js`) |
| `WITSLOG_ABI_VERSION == 1`; every SDK checks it at load. | `crates/witslog-ffi/src/lib.rs:14` |
| `flush_batch` already catches panics and retries once — the precedent for notifier failure semantics. | `crates/witslog-core/src/buffer.rs:145` |

### Bugs found (not previously known)

1. **`delete` / `prune` / `archive` permanently break `doctor --verify-audit`.** `compute_hash` covers `prev|event_id|ts|message|fingerprint` (`crates/witslog-store/src/audit.rs:14`). `verify_chain` recomputes from `GENESIS` over **surviving** rows in `id` order (`audit.rs:124`). Three paths hard-`DELETE` rows — `delete_resolved` (`writer.rs:193`), `cmd_prune` (`crates/witslog-cli/src/main.rs:934`), `cmd_archive` (`main.rs:1014`) — so after any of them, every subsequent row's expected hash shifts and verification reports `Broken`, indistinguishable from tampering. No test covers `verify_chain` against deletion. **F1 makes resolution a first-class workflow, i.e. it funnels users into `delete_resolved`, whose default filter is literally `resolved_at IS NOT NULL` (`writer.rs:154`).** This is why the fix is a blocker, not a follow-up.
2. **`cmd_prune` and `cmd_archive` reach around the store layer** and run raw `DELETE FROM events` in the CLI, violating PLAN.md §1's modularity rule ("No component reaches around 2 to touch the DB file"). This is why the tombstone fix can't be local to `writer.rs`.
3. **`top_failures` ignores caller filters** — hardcodes `let filters = Filters::default();` (`crates/witslog-mcp/src/registry.rs:333`). Any new filter axis silently no-ops there.
4. **`resolved_at` is not covered by the audit chain.** Benign for F1's `UPDATE` (it won't break verification), but it means any "who resolved this" field would be unauthenticated and unbacked.

---

## Decisions locked

Accepted from the CTO argument:

- **No `resolved_by` / `resolution_note`.** Not a cost argument (migration 7 happens anyway) — a truth argument. `compute_hash` can't back them, so they'd be provenance the audit chain provably can't verify, on a single-user local tool with no identity. Resolution provenance, if ever wanted, is a child event with `parent_event_id` — zero schema, and it *is* chained.
- **No `witslog_resolve` MCP write tool.** PLAN.md §5 made `witslog_delete` the only write tool deliberately. Resolve is quieter *and* composes into delete: an agent that can resolve qualifies rows for `delete_resolved`'s `resolved_at IS NOT NULL` default filter, handing out delete through the back door. Read-only `mttr` tool only.
- **No `ureq`, no webhook notifier.** `witslog-runtime` links into `witslog-ffi`, which is `dlopen`'d into every Django/Node/PHP host process. Injecting a TLS stack there to POST a webhook contradicts the "no cloud, no infra" pitch and wrecks the build matrix. The `Notifier` trait already *is* the webhook extension point — P9 built it for exactly this. File sink only.
- **No percentiles in v1.** Mean + counts. Median/p90 over a distribution that isn't meaningful is precision theater.
- **Fingerprint-level MTTR, not event-level.** Event-level is dishonest for recurring failures: a fingerprint fires 500× and you fix it once, so you'd be measuring error *volume* and calling it *recovery time*.

Rejected from the CTO argument:

- **Cutting browser capture to P11.** The advisor's objection (it adds a network trust boundary to a tool that has none, and its text reaches an LLM verbatim) is correct and is addressed with real guardrails below — but the feature *is* the goal. Deferring it defers the reason the phase exists. It ships in P10 with a threat model, not as a bullet point.

---

## P10a — Audit tombstones + delete consolidation (blocker; do first)

Fixes bug 1 and bug 2. This is the only thing justifying `CURRENT_SCHEMA_VERSION` 6→7. Note the bump is a one-way door: 0.1.1 binaries will refuse a v7 DB via the P8 guard (`migrate.rs:41`). That is the guard working as designed, and a real fix earns it.

- **`migrate_0007_audit_tombstones`** in `crates/witslog-store/src/migrate.rs`, following the existing numbered/idempotent pattern (`IF NOT EXISTS`, then `record_migration(7, "audit_tombstones")`):
  ```sql
  CREATE TABLE IF NOT EXISTS audit_tombstones (
    row_id     INTEGER PRIMARY KEY,   -- the deleted events.id
    event_id   TEXT NOT NULL,
    audit_hash TEXT NOT NULL,         -- the deleted row's hash, so the chain can bridge the gap
    deleted_at TEXT NOT NULL
  );
  ```
- **One store-layer delete helper** in `crates/witslog-store/src/writer.rs` (or a small `delete.rs`): `delete_events(conn, event_ids) -> Result<Vec<String>>` — records a tombstone per row (id + its `audit_hash`), then deletes the row and its `error_edges`. All three call sites route through it: `delete_resolved` (`writer.rs:151`), `cmd_prune` (`main.rs:934`), `cmd_archive` (`main.rs:1014`). This is the modularity fix from bug 2, not a new abstraction.
- **`verify_chain`** (`audit.rs:124`): when the `id` sequence gaps, look up `audit_tombstones.row_id`; if present, use its `audit_hash` as `prev` and continue. A gap with **no** tombstone stays `Broken` — tamper-evidence survives, and undocumented row removal is still caught.
- **`doctor --verify-audit`** (`main.rs:525`): report tombstoned gaps as informational (`N row(s) removed by delete/prune/archive`) and keep `Broken` meaning tampering. Today it exits 1 on both.

## P10b — MTTR / resolution tracking

No schema of its own (P10a's migration is the only one).

- **`Filters.resolved: Option<bool>`** in `crates/witslog-query/src/filters.rs:4`; `to_sql()` (`:25`) emits `resolved_at IS NULL` / `IS NOT NULL`. Follows the existing `Option` + `clauses.push` pattern exactly.
- **`EventWriter::mark_resolved(event_id, force) -> Result<bool>`** (`writer.rs:72`): use the `usize` that `conn.execute` already returns; add `AND resolved_at IS NULL` unless `force`, so first resolution wins and MTTR can't be moved by a re-resolve. Returns `false` when no row matched.
- **`AggregateEngine::mttr(&filters) -> MttrStats { fingerprints_resolved, fingerprints_unresolved, mean_seconds }`** in `crates/witslog-query/src/aggregates.rs`. Fingerprint-level: per `fingerprint`, `MIN(resolved_at) − MIN(ts)` over fingerprints having any resolved event — "time from first sighting to first fix", one row per distinct failure. Both columns are **TEXT** and there is no `resolved_at_epoch_ms` to mirror `ts_epoch_ms`, so use `(julianday(MIN(resolved_at)) - julianday(MIN(ts))) * 86400.0` for seconds; do **not** use `strftime('%s',…)`, which truncates the millis `mark_resolved` writes (`writer.rs:74`).
- **CLI** (`crates/witslog-cli/src/main.rs`): `witslog resolve <id> [--force]` reports "event not found" and exits non-zero when `mark_resolved` returns false; `witslog stats --mttr`; `witslog query --unresolved`.
- **MCP** (`crates/witslog-mcp/src/registry.rs`): new read-only `mttr` tool + `resolved` on the common filters. **`schema::validate` runs against the tool's JSON Schema before dispatch (`registry.rs:62`), so `resolved` must also land in `Tool::builtin_tools()`** or every call carrying it is rejected. Fix `top_failures`'s hardcoded `Filters::default()` (`registry.rs:333`) in the same change — otherwise the new axis silently no-ops there.

## P10c — Notifiers (wire the trait that already exists)

- **Config `[notify]`** in `crates/witslog-config/src/lib.rs`, mirroring the existing `EnrichSection`/`BufferSection` shape: `enabled` (default `false`), `min_severity` (default `"error"`), `path` (NDJSON target), `once_per_fingerprint_secs` (optional throttle — the difference between a notify feature and a self-inflicted log flood).
- **`FileNotifier`** in a new `crates/witslog-runtime/src/notify.rs` implementing `witslog_plugin::Notifier`: append one NDJSON line. No new deps.
- **Wiring**: build a `PluginRegistry` from config in the runtime; call `registry.dispatch_event(&event_json)` after a successful write in `build_and_write` (`runtime/src/lib.rs:257`) and `write_via_snapshot` (`:205`). Dispatch is already panic-isolated (`plugin/src/lib.rs`), failures are swallowed and counted, never fail the write.
- **Hard rule: never dispatch from `capture_sync`** (`runtime/src/lib.rs:185`). That path exists *because* the process may abort mid-panic; doing I/O there is the one place a stall is unacceptable. An NDJSON append is microseconds — same order as the SQLite insert already happening — so synchronous dispatch elsewhere is fine and needs no queue subsystem.

## P10d — Browser-side error capture

Per PLAN.md §10: no native/FFI code ever runs in the browser. Reporter ships to a backend endpoint; the Node SDK persists server-side.

- **`bindings/browser/witslog-browser.js`** — zero-dep, ~120 LOC. `init({endpoint, app, sampleRate})` installs `window.onerror` + `unhandledrejection`; batches; ships via `navigator.sendBeacon` (survives unload) with a `fetch(..., {keepalive:true})` fallback; flushes on `visibilitychange`→hidden and `pagehide`. Keep the batch-builder a pure exported fn so it unit-tests without a DOM.
- **`witslogBrowserIngest(...)` in `bindings/node/frameworks/express.js`** (today a 25-line `witslogErrorHandler`). The body is **untrusted input**, and this is the sharp edge: text posted here lands in `events.message`, which `search_errors`/`explain_error` return verbatim to an LLM (`registry.rs:122`, `:239`). That is a prompt-injection path into the AI's evidence base, and clamping severity does nothing about it — the severity isn't the payload, the text is. Guardrails, all of them:
  - bind **127.0.0.1** only;
  - `allowedOrigins` allow-list, **defaults to `[]` (fail closed)** — you list your dev origins;
  - token-bucket rate limit (per-request caps are meaningless against request *volume*: 20 × 64KB is nothing at 10k req/s, and unbounded local disk + FTS5 growth is the real DoS);
  - `enabled` defaults `false`, and **refuses to arm when `NODE_ENV === 'production'`** unless explicitly forced;
  - severity clamped to `error|warn` (never `fatal`/`critical`); message/stacktrace length caps; batch + body byte caps;
  - map to `witslog.exception(...)`/`witslog.log(...)` with `tags:['browser']` and `context:{url, ua}`.
  - **`tags:['browser']` is not a trust boundary** — `classify()` merges suggested tags into existing tags (`crates/witslog-core/src/event.rs:254`), so tags are advisory, not provenance. Real provenance needs `ingest_source` in the payload allow-list, which is an ABI-version conversation (`WITSLOG_ABI_VERSION == 1`). Out of scope here; documented as the known gap.
- **Python/PHP ingest**: documented recipe in `bindings/CONTRACT.md`, not shipped adapters. Honest tradeoff — a recipe is a rot site, but three parallel untrusted-input handlers is three attack surfaces to keep in sync. Revisit if the Node one proves out.

---

## Tests (per phase — unit + feature + smoke, then regression locks)

Per CLAUDE.md: no phase is done on "it compiles". Run `cargo test --workspace` before calling any of this done.

**P10a** — unit: tombstone recorded on delete; `verify_chain` bridges a tombstoned gap; gap with no tombstone still `Broken`; migration 7 idempotent on re-run and backfills a pre-P10 DB. Feature: `crates/witslog-store/tests/p10_integration.rs`. Smoke: real binary `init → log ×3 → resolve → delete → doctor --verify-audit` exits 0.

**P10b** — unit: `Filters` resolved axis emits the right SQL (mirrors `test_filters_to_sql_severity`, `filters.rs:158`); MTTR math on in-memory SQLite incl. the julianday/millis path; `mark_resolved` returns false on unknown id. Feature: `crates/witslog-cli/tests/p10_integration.rs` driving the real binary via `env!("CARGO_BIN_EXE_witslog")` — `init → log → resolve → stats --mttr → query --unresolved`. Plus MCP conformance for `mttr` + the `resolved` filter, extending `p5_integration.rs`.

**P10c** — unit: `[notify]` config parse; dispatch against a fake sink; `min_severity` gate; throttle. Feature: file notifier writes NDJSON through a real `build_and_write`.

**P10d** — unit: browser batch-builder pure fn (`node --test`, matching `bindings/node/test/payload.test.js`); ingest clamp/origin/rate-limit pure fns. Smoke: extend `bindings/e2e/run.ps1` with a browser→ingest→CLI readback (real node http server, POST the reporter's batch payload, read the event back through the CLI).

**Regression locks** — named so a revert fails them:
- `deleting_a_row_keeps_verify_chain_ok` and `deleted_row_without_tombstone_still_breaks_chain` (P10a — the two halves of the bug)
- `prune_and_archive_record_tombstones` (P10a — bug 2's real symptom)
- `resolve_unknown_event_id_is_reported` (P10b)
- `re_resolving_does_not_move_resolved_at` (P10b)
- `mttr_excludes_unresolved_fingerprints` (P10b)
- `top_failures_honours_caller_filters` (P10b — pins bug 3)
- `notifier_panic_does_not_break_write` and `notifier_failure_does_not_fail_write` (P10c)
- `notifier_never_dispatches_from_panic_path` (P10c — pins the `capture_sync` rule)
- `browser_ingest_rejects_disallowed_origin` and `browser_ingest_clamps_severity_and_size` (P10d)

## Verification

Beyond tests — drive it end to end (`/verify`):

1. `cargo build --workspace && cargo test --workspace && cargo clippy --workspace`.
2. Scratch project: `witslog init .` → `witslog log app "boom"` ×3 → `witslog query --unresolved` shows 3 → `witslog resolve <id>` → `witslog stats --mttr` shows a non-zero mean and 1 resolved fingerprint → `witslog resolve <bogus-id>` errors non-zero.
3. **The bug, proven:** `witslog delete --event-id <id>` then `witslog doctor --verify-audit` → exits 0 with a tombstone note. On `main` today this same sequence reports `Broken` — confirm that first, so the fix is demonstrated rather than asserted.
4. Enable `[notify]`, log an event, confirm the NDJSON line; point the path at an unwritable location and confirm the write still succeeds.
5. Browser: run the express ingest sample, POST a batch from a page, `witslog query browser` reads it back; POST from a disallowed Origin → rejected; set `NODE_ENV=production` → refuses to arm.
6. `witslog serve-mcp --stdio`, call `mttr` and `latest_errors` with `resolved:false` from a real MCP client.

Update `CHANGELOG.md` under `## [Unreleased]` in the same change that lands each piece (CLAUDE.md changelog discipline), and update `PHASES.md` §P10 + the CLAUDE.md phase table + gotchas (schema version 6→7, the tombstone rule, the `capture_sync` no-dispatch rule).

## Out of scope (deliberate)

Fingerprint reopen tracking; resolution SLAs; `resolved_by`/`resolution_note`; `witslog_resolve` as an MCP write tool; webhook/desktop notifiers and any HTTP dep in core; notifier retries/queues; dynamic plugin loading; source-map resolution; session replay; breadcrumbs; a witslog-hosted collector; `ingest_source` in the payload contract (needs an ABI bump).

## Biggest risk being carried

F1 and F3 compose in a way neither reviews alone: F3 lets attacker-reachable text into `events.message`, which MCP serves verbatim to an LLM; F1 gives that LLM a resolution workflow whose rows qualify for `delete_resolved`; and the audit chain that should catch the result hashes only `event_id|ts|message|fingerprint` and is already broken by the delete path. P10a + the fail-closed origin allow-list are the mitigations, but the composition is the thing to keep watching — not the three features individually.