# P2 — Taxonomy Engine, + CLAUDE.md/README.md

## Context

witslog = SQLite-backed structured error log, per-project DB, AI queryable via
future MCP. P0 (storage+events) done, P1 (enrich/redact/buffer) partial. P2 —
taxonomy engine — is next: **todo**, independent of P1/P3, feeds P5
(`classify_error`, `list_categories`). Repo has no CLAUDE.md/README.md yet;
user wants both created, matching current shipped state (not aspirational).

Verified via `codegraph_explore` (source, not guesses):
- `categories`/`category_aliases` tables exist since `migrate_0001_init`
  (crates/witslog-store/src/migrate.rs) but are never seeded — no INSERT
  anywhere in the codebase.
- No `taxonomy.rs` in `witslog-core` or `witslog-store`. No classifier.
- `EventBuilder` (crates/witslog-core/src/event.rs:62) has `.enrich(cfg)` and
  `.redact(redactor)` as explicit chained methods called by CLI/FFI — `build()`
  itself stays pure (only fills fingerprint). Taxonomy should follow the same
  shape: an explicit `.classify(&Classifier)` method, not automatic magic in
  `build()`.
- `witslog-config::Config` has `EnrichSection`/`RedactSection`/`BufferSection`
  as sibling `#[serde(default)]` structs on `Config` (crates/witslog-config/src/lib.rs) —
  taxonomy config mirrors this.
- Migrations are sequential guarded blocks in `Migrator::migrate()`
  (`if current_version < N`), each a private `migrate_000N_name` fn +
  `record_migration`. `migrate_0002_resolved_at` shows the idempotent-column
  pattern (`pragma_table_info` check before `ALTER TABLE`); `migrate_0003` shows
  the simple `CREATE TABLE IF NOT EXISTS` + seed-row pattern — the taxonomy
  seed migration mirrors `migrate_0003`.
- Store write helpers live on `EventWriter` (crates/witslog-store/src/writer.rs)
  taking `&DbConnection`/`&Connection` and returning `crate::error::Result<T>`;
  new taxonomy store fns mirror this shape in a new sibling file.

## Approach

**1. `crates/witslog-core/src/taxonomy.rs` (new)** — pure, no I/O, per PLAN.md §Design Rationale ("taxonomy engine: no storage, no I/O — pure fn(event)→labels"):
- `pub struct Category { canonical: String, parent: Option<String>, label: String }`
  and a `pub const BUILTIN_CATEGORIES: &[Category]` (or a `builtin_categories()` fn
  returning a `Vec`) — hierarchy: `infrastructure.*` (network.dns, network.timeout,
  network.connection_refused, disk, memory), `application.*` (validation, auth,
  business_logic), `runtime.*` (panic, oom, deadlock), `external.*` (api, database,
  third_party). Keep leaves modest (~15-20) — spec says "documented leaves", not exhaustive.
- `pub struct ClassifyRule { id: String, canonical: String, match_kind: MatchKind, pattern: String, tags: Vec<String> }`
  where `MatchKind` = `ErrorCode | ExceptionName | MessageKeyword | MessageRegex`.
  Ordered evaluation: error_code map → exception-name map → message keyword/regex,
  per FR-P2-005.
- `pub struct Classifier { rules: Vec<ClassifyRule> }` with `Classifier::built_in()`
  (default rule set covering the builtin leaves, e.g. `ETIMEDOUT`→
  `infrastructure.network.timeout`) and `Classifier::with_rules(extra)` to append
  custom rules (config-driven, FR-P2-004-adjacent).
- `pub struct ClassifyInput<'a> { message: &'a str, exception: Option<&'a str>, error_code: Option<&'a str> }`
  and `pub struct Classification { canonical: Option<String>, rule_ids: Vec<String>, tags: Vec<String> }`.
- `Classifier::classify(&self, input: &ClassifyInput) -> Classification` — deterministic,
  first-match-wins per category of rule, O(rules). No match → `canonical: None`,
  `tags: ["unclassified"]` (FR-P2-007).
- Alias resolution is pure too: `pub fn resolve_alias(aliases: &HashMap<String,String>, name: &str) -> String`
  (trivial lookup-or-identity) — the alias *table* lives in the store; this fn just
  encapsulates the resolution rule so it's unit-testable without a DB.

**2. Wire into `EventBuilder`** (crates/witslog-core/src/event.rs) — mirror
`.enrich()`/`.redact()`:
```rust
pub fn classify(mut self, classifier: &crate::taxonomy::Classifier) -> Self {
    if self.category.is_none() {
        let result = classifier.classify(&ClassifyInput { .. });
        self.category = result.canonical;
        self.tags = Some(merge tags with result.tags);
    }
    self
}
```
Only fills category if caller didn't set one explicitly (FR-P2-005: "lacks an
explicit category"). CLI/FFI call `.classify(&classifier)` in their build chain,
same place `.enrich()`/`.redact()` are called today (see crates/witslog-ffi/src/lib.rs
`witslog_log`, crates/witslog-cli/src/main.rs `log_event`).

**3. `migrate_0004_seed_taxonomy`** in crates/witslog-store/src/migrate.rs —
add `if current_version < 4 { self.migrate_0004_seed_taxonomy()?; self.record_migration(4, "seed_taxonomy")?; }`
in `migrate()`. Body: `INSERT OR IGNORE INTO categories (canonical, parent, label, builtin) VALUES (...)`
for every entry in `witslog_core::taxonomy::BUILTIN_CATEGORIES` (insert parents
before children, or rely on `INSERT OR IGNORE` + no FK enforcement issue since
`foreign_keys=ON` requires parent-before-child — order the const list top-down),
plus builtin aliases into `category_aliases`. This makes `migrate.rs` depend on
`witslog-core` — check `witslog-store/Cargo.toml` already has `witslog-core` as a
path dep (it does, via `Event` usage in writer.rs) so this is free.

**4. `crates/witslog-store/src/taxonomy.rs` (new)** — store-layer CRUD mirroring
`EventWriter`'s shape:
- `insert_category(conn, canonical, parent, label) -> Result<()>` — reject if
  canonical collides with an existing `builtin=1` row (FR-P2-003) with a typed
  error naming the conflict.
- `insert_alias(conn, alias, canonical) -> Result<()>` — reject if `canonical`
  doesn't exist in `categories` (FR-P2-004) — typed error, not a raw FK failure.
- `resolve_alias_db(conn, name) -> Result<String>` — looks up `category_aliases`,
  falls through to identity if not an alias.
- `list_tree(conn) -> Result<Vec<CategoryNode>>` — recursive read of `categories`
  building the tree (recursive CTE or in-Rust fold over a flat SELECT — flat fold
  is simpler and fine at this cardinality).
- Add matching variant(s) to `crate::error::Error` (crates/witslog-store/src/error.rs)
  for the two reject cases above, e.g. `CategoryCollision(String)`, `UnknownCanonical(String)`.

**5. Config** — crates/witslog-config/src/lib.rs: add
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TaxonomySection {
    pub auto_classify: bool,
    pub rule_file: Option<PathBuf>,
}
```
with `Default` (`auto_classify: true, rule_file: None`), add `pub taxonomy: TaxonomySection`
field to `Config` + `Config::default_project()`. Rule-file parsing (custom rules
JSON/TOML → `Vec<ClassifyRule>`) is a small loader in `witslog-core::taxonomy`
(reuse the `Config::load_from_file`-style error pattern) — invalid rule regex
rejected at load per FR-P2's "Invalid rule regex … reject config, name rule".

**6. Tests** — crates/witslog-core (unit): builtin rule precedence order, alias
resolution, unclassified path + tag, determinism (same input twice → same
output). crates/witslog-store (integration, mirror `p1_integration.rs` style):
fresh DB → categories seeded with full builtin tree + `builtin=1`; custom
category insert; collision rejected; alias to unknown canonical rejected;
migration idempotent on re-run.

**7. CLAUDE.md** (repo root) — project overview for future Claude sessions:
architecture summary (per-project SQLite, WAL, workspace crates), crate map
(mirror PLAN.md §6 but only crates that exist:
core/store/config/cli/ffi — mcp/query/plugin not yet), where specs live
(PLAN.md = design doc, PHASES.md = phase-by-phase EARS/AC/TODO — "read PHASES.md
before implementing a phase"), migration/testing conventions observed above,
current phase status pointer (P0 done, P1/P6 partial, P2 next).

**8. README.md** (repo root) — user-facing: what witslog is, quickstart
(`witslog init`, `witslog log`, `witslog query <id>`), current status
(pre-1.0, CLI usable for init/log/query/resolve/delete/doctor; MCP/search not
yet built), link to PLAN.md for full spec.

**9. Update stale progress refs only** — after P2 lands, update PHASES.md P2
section status legend (⬜ todo → ✅ done / 🟡 partial) + its "Status" line and
Shipped-capability notes to match what actually shipped, same as P0's audited
block. Don't touch unrelated PLAN.md/PHASES.md content — no full sweep, no
rewrites beyond the P2 status marker itself. CLAUDE.md/README.md are written
fresh against current shipped state, so no drift to fix there.

## Verification

- `cargo test -p witslog-core taxonomy` — rule precedence, alias resolution,
  unclassified, determinism all green.
- `cargo test -p witslog-store` — new taxonomy integration test green alongside
  existing `m1_integration`/`p1_integration`.
- Manual: `witslog init .` → inspect `categories` table has full builtin tree;
  `witslog log app "boom" --exception ETIMEDOUT` (once CLI wired) → `witslog query <id>`
  shows auto-assigned canonical category.
- `cargo build --workspace` clean; `cargo clippy --workspace` clean.