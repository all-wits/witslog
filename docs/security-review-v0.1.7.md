# Security review — GitHub tag `v0.1.7` / npm `@all-wits/witslog@0.6.4`

Scope: the tree as it will ship for both releases, reviewed via `codegraph_*` source
inspection plus direct file reads (CI workflow, crypto module). Three lenses applied:
code-level vulnerability review, secure-coding-pattern check, and CISO release-risk framing.
No dynamic scanning tools (semgrep/gitleaks/npm audit) were run in this pass — findings below
are from manual source review.

## Findings (ranked by severity)

### 🟠 Medium — Field-level encryption covers only `metadata`, easy to over-trust

**File:** `crates/witslog-core/src/crypto.rs` (`FieldCipher::encrypt_metadata`, event.rs's
`Event` struct)

`message`, `context`, `stacktrace`, `stack_norm`, `exception`, and `tags` are always stored
plaintext — only `metadata` can be wrapped in an AES-256-GCM envelope, and only if the caller
explicitly opts in via `.encrypt_metadata(&cipher)`. This is a deliberate, documented tradeoff
(FTS5 and the `GENERATED ALWAYS AS (json_extract(context, ...))` columns need plaintext), but
it's an easy trap for an adopter who reads "field-level encryption" in the docs and assumes
sensitive data anywhere in an event is covered. A stack trace or `context` blob containing a
credential/PII will be persisted in the clear regardless of `FieldCipher` use.

**Remediation:** Already correctly scoped/documented in CLAUDE.md's Gotchas; no code change
needed for this release. Recommend the README/CONTRACT feature bullet for encryption
explicitly say "metadata field only" rather than "field-level encryption" unqualified, so
adopters don't over-trust it for `message`/`context`/`stacktrace`.

### 🟠 Medium — Browser/Next.js ingest is an intentionally unauthenticated write surface into the AI's evidence base

**Files:** `bindings/node/lib/ingest-core.js`, `frameworks/express.js`, `frameworks/next.js`

Confirmed the documented guardrails are real and correctly implemented: Origin allowlist
defaults to `[]` (fail-closed), `NODE_ENV=production` refuses to arm without `force:true`,
severity is clamped to `error`/`warn` (`clampSeverity`), a token-bucket rate limit is enforced
per `remoteAddress || origin`, and `clampContext` bounds depth/keys/string length/total size.
`tags: ['browser', ...]` always keeps `'browser'` first and unremovable. This is solid defense
in depth for what is fundamentally an unauthenticated endpoint by design.

Two residual gaps, both already known/accepted tradeoffs rather than bugs:

1. **Rate-limit buckets are in-process (`Map`), not distributed** — a horizontally-scaled
   deployment (multiple Next.js/Express instances behind a load balancer) gets independent
   rate limits per instance, so the effective limit is `max × instance_count`. Low severity
   for a "local-dev endpoint" use case; worth a one-line doc caveat if anyone deploys this
   ingest handler behind a real load balancer.
2. **The Next.js variant cannot check `remoteAddress` at all** (`remoteAddress: undefined`
   passed explicitly, `next.js:179`) — the code comment correctly identifies this and leans on
   the Origin allowlist as the actual defense, which is sound reasoning (the loopback check
   was never sufficient anyway, since the attack model is a malicious page open in the same
   browser hitting `localhost`). No action needed; already correctly reasoned in the source.

**Remediation:** None required for release — both gaps are already load-bearing on the Origin
allowlist, which is correctly fail-closed by default. Recommend flagging gap #1 in
`bindings/CONTRACT.md`'s ingest section if not already covered, for anyone scaling the ingest
handler horizontally.

### 🟡 Low — `argv` enrichment on by default can leak bare CLI-argument secrets

**File:** `crates/witslog-core/src/enrich.rs` (`EnrichConfig::default().argv == true`)

Already documented extensively in `bindings/CONTRACT.md` and `CLAUDE.md`. Redaction
(`redact_json`) catches pattern-shaped secrets (Bearer/api_key/password/AWS_*/conn-strings) in
`context.argv` but not an arbitrary secret passed as a bare positional CLI arg
(`myapp --token abc123secret` — the value `abc123secret` has no recognizable shape). This is a
real, live gap, but it's opt-out (`{"enrich":{"argv":false}}`), tested end-to-end
(`configure_argv_false_suppresses_argv_capture`), and clearly documented as a security note.

**Remediation:** No code change required — this is a documented, opt-out default, not a silent
vulnerability. Consider (future, not blocking): flipping the default to `argv:false` for a
future major/breaking version, since "secure by default" is generally preferable to "secure if
you read the docs," but that's a breaking behavior change out of scope for this patch release.

### 🟢 Info — `matching_delete_ids` SQL construction is safe (verified, not a finding)

**File:** `crates/witslog-store/src/writer.rs:191`

The `format!`-built clauses only ever interpolate **placeholder positions** (`?{}` with an
incrementing integer index) — every actual value (`event_id`, `fingerprint`,
`resolved_before`) is bound via `rusqlite::ToSql` in the `params` vec and passed through
`stmt.query_map(param_refs.as_slice(), ...)`, never string-concatenated into the SQL text
itself. No injection vector. Confirmed by direct read of the function; no remediation needed.

### 🟢 Info — MCP `witslog_delete` write gate is correctly structured

**File:** `crates/witslog-mcp/src/registry.rs` (`ToolRegistry::list_tools`)

`witslog_delete_tool()` is only appended to the tool list when the server is started with
`--allow-write`; the default MCP surface is strictly read-only. Combined with the deliberate
absence of an MCP `resolve` tool (documented rationale: would let an agent silently qualify
rows for `witslog_delete`'s default `resolved_at IS NOT NULL` filter), this is a well-reasoned,
narrow write surface. No finding.

### 🟢 Info — npm supply chain (`koffi`, bundled binaries, OIDC publishing)

**File:** `.github/workflows/release-node-sdk.yml`

- Publishing uses npm Trusted Publishing (OIDC) — no long-lived `NPM_TOKEN` secret in the
  workflow or repo. `id-token: write` is scoped to the one job that needs it.
- Version-gate check (`npm view ... version`) prevents accidental republish of an unchanged
  version.
- Native binaries (`witslog_ffi.*`, `witslog(.exe)`) are built from source in-workflow on
  GitHub-hosted runners per target platform — not fetched from a third party or committed as
  opaque binaries checked into git (aside from the local dev-verify `.tgz`, which is untracked
  scratch output per `.gitignore` conventions and must not be published as-is — already flagged
  in the release plan).
- `koffi` is a single external native-postinstall dependency; its supply-chain risk is inherent
  to using an FFI bridge at all and isn't something this release changes. No action beyond the
  existing `pnpm approve-builds koffi` friction already documented.

No blocking finding. Residual risk: a compromised `koffi` release could execute arbitrary code
at `postinstall`/load time in any host process embedding this SDK — this is accepted,
inherent-to-the-approach risk for any FFI-based Node SDK, not something introduced by this
release.

## CISO release verdict

**Both releases are suitable to ship.** Neither introduces a new vulnerability class; the two
delete-path bugfixes going into `v0.1.7` are pure correctness fixes with no security
implication (they make `--force` behave as documented, and make `--dry-run` actually preview —
if anything, the dry-run fix *reduces* risk of an operator mis-deleting data based on a wrong
preview). The new `hocuspocus.js` adapter in `0.6.4` reuses the existing, already-reviewed
framework-adapter contract (duck-typed, no new transport, no new persistence path) and doesn't
touch the ingest/write surfaces that carry this project's actual risk.

**Residual risk profile for early adopters**, unchanged by this release and already documented:
prompt-injection-into-agent risk via the browser-ingest → `events.message` → MCP → LLM path
(mitigated by fail-closed defaults, not eliminated — an adopter who sets `allowedOrigins` too
broadly, or who exposes an MCP server with `--allow-write` to a shared/multi-tenant agent,
inherits real risk that is theirs to manage); argv-secret leakage for apps that pass secrets as
bare CLI args and don't disable `argv` enrichment; and the inherent FFI/native-dependency
supply-chain surface of embedding `witslog_ffi` (via `koffi`) in a host process.

**Nothing blocks release.** No critical or high-severity finding. The two medium items above
(encryption scope, ingest rate-limit/remoteAddress gaps) are already correctly reasoned
tradeoffs in the existing code/docs, not defects — recommended as documentation clarifications,
not code changes, and not release blockers.
