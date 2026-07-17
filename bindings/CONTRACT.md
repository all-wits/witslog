# witslog SDK ↔ native ABI contract

All language SDKs (Python, Node, PHP) are thin wrappers over the same C ABI exported by the
`witslog-ffi` crate (`witslog_ffi.dll` / `libwitslog_ffi.so` / `libwitslog_ffi.dylib`). This
file is the single source of truth every SDK core marshals against.

## Contract version

**Current version: `1`.**

The native library exports `int32 witslog_abi_version(void)`. Every SDK core calls it once at
load time and compares against the version it was built for. On mismatch the SDK raises its
`WitslogContractError` (naming expected vs. actual) rather than sending a payload the native
side may mis-parse. Bump `WITSLOG_ABI_VERSION` in `crates/witslog-ffi/src/lib.rs` on any
breaking change to the payloads below.

## Exported functions

| Symbol | Signature (C) | Meaning |
|---|---|---|
| `witslog_abi_version` | `int32 (void)` | Contract version (see above). |
| `witslog_configure` | `int32 (const char* json)` | Set enrich/redact/buffer for this process. `0` ok, `-1` bad JSON, `-2` bad redact regex. |
| `witslog_init` | `int32 (const char* json_or_null)` | Mount the process runtime + Rust panic hook. Applies `witslog_configure` payload first (null = defaults). `0`/`-1`/`-2` as above. |
| `witslog_log` | `int64 (const char* json)` | Log one event (payload below). Returns the DB rowid on the sync path, `0` when buffering is enabled (rowid not yet known), `-1` on error. Never panics. |
| `witslog_resolve` | `int32 (const char* event_id)` | Mark an event resolved. `0`/`-1`. |
| `witslog_bootstrap_project` | `int32 (const char* path_or_null)` | Scaffold a `.witslog/` project dir (create dir, open/create DB, migrate) — mirrors CLI `witslog init`. `path_or_null`: project root, or null for cwd. Idempotent. `0`/`-1`. |
| `witslog_delete` | `char* (const char* filter_json)` | Delete stale/resolved events. Returns a heap JSON string `{"deleted_count":N,"deleted_ids":[...]}` (free via `witslog_free_string`) or null on error. |
| `witslog_flush` | `int32 (void)` | Drain the async buffer (joins the flush thread). Idempotent. Call before exit. |
| `witslog_shutdown` | `int32 (void)` | Un-mount: flush + tear down. Alias of `witslog_flush` today. |
| `witslog_free_string` | `void (char*)` | Free a string returned by `witslog_delete`. |

All string parameters are NUL-terminated UTF-8. The caller owns the input buffers; the library
owns (and frees, via `witslog_free_string`) any `char*` it returns.

## `witslog_log` payload (JSON object)

| Field | Type | Req | Notes |
|---|---|---|---|
| `application` | string | ✅ | app name |
| `message` | string | ✅ | error message (redacted before persist) |
| `severity` | string | | `trace\|debug\|info\|warn\|error\|critical\|fatal` (default `error`) |
| `version` | string | | app version / build id |
| `environment` | string | | `prod\|staging\|dev\|ci` |
| `category` | string | | canonical taxonomy leaf; when set, auto-classify is skipped |
| `error_code` | string | | app-defined stable code |
| `exception` | string | | exception/class type name |
| `stacktrace` | string | | raw trace; normalized into `stack_norm` |
| `correlation_id` | string | | request/trace id |
| `parent_event_id` | string | | caused-by parent event id |
| `context` | object | | structured context; redacted; hot keys promoted to columns |
| `tags` | string[] | | free-form tags |
| `metadata` | object | | free-form metadata; redacted |

`context`, `tags`, `metadata` are passed through unchanged (FR-P6-006).

## `witslog_init` / `witslog_configure` payload (JSON object)

```json
{
  "enrich": { "hostname": true, "pid": true, "cwd": true, "argv": true,
              "git_commit": true, "env_allowlist": ["PATH"] },
  "redact": { "custom_patterns": ["MY_TOKEN_[A-Z0-9]+"] },
  "buffer": { "enabled": false, "batch_size": 50, "flush_interval_ms": 1000,
              "queue_capacity": 10000 }
}
```

All keys optional; omitted keys keep their current value.

## `witslog_delete` filter payload (JSON object)

```json
{ "event_id": "...", "fingerprint": "...", "resolved_before": "RFC3339", "force": false }
```

Only deletes events with `resolved_at IS NOT NULL` unless `force:true`.

## `witslog_bootstrap_project` (no JSON — plain path string or null)

None of the write-path exports (`witslog_log`/`witslog_resolve`/`witslog_delete`) create the
parent `.witslog/` directory — `SQLITE_OPEN_CREATE` (used internally) creates the DB *file*
only, not missing parent directories. Historically only the separately-distributed CLI's
`witslog init` created that directory, which left SDKs installed via a package manager alone
(no CLI binary bundled) with no way to bootstrap a project. Call
`witslog_bootstrap_project(path_or_null)` once before the first `witslog_log`/`witslog_init`
call in a fresh project. Safe to call repeatedly (dir creation and the underlying
`Store::open_or_create` migrate step are both idempotent).

Currently only the [Node SDK](node) wires this into a convenience API
(`init({ createProject: true })` / `{ createProject: '/path' }`). Python/PHP can call the
native symbol directly through their FFI/ctypes/ext-ffi bridge but don't expose a wrapper
yet — see each SDK's README.

## Native library location (locator, identical in every SDK)

Resolved in order:

1. `WITSLOG_LIB` environment variable — explicit path to the shared library (used in dev/CI;
   point it at `target/release/witslog_ffi.dll`).
2. Package-bundled `_libs/<platform>/witslog_ffi.{dll,so,dylib}`, where `<platform>` is e.g.
   `win32-x64`, `linux-x64`, `darwin-arm64`.
3. The OS default loader search path.

On failure the SDK raises `WitslogLibraryError` listing the paths it tried.

## DB resolution

The native library resolves the target DB by walking up from the current working directory for a
`.witslog/` marker (same as the CLI). An SDK-hosted app therefore logs into its own project DB
automatically — just run from the project directory, or `witslog init` it first.

## Security note: argv enrichment vs. secrets

Enrichment defaults `argv: true` (see `EnrichConfig::default()` in
`crates/witslog-core/src/enrich.rs`), so the **full process command line** is captured into
`context.argv` on every event by default. Built-in + custom redaction (`redact_json`) recurses
into `argv` and redacts anything matching a known secret *pattern* (Bearer tokens, `api_key=`,
`password=`, `AWS_*`, connection strings) — but a secret passed as a **bare CLI argument** that
doesn't match one of those shapes (e.g. `myapp --token abc123secret`) is not pattern-matched and
will be persisted verbatim.

If your app may receive secrets via CLI arguments, close this exposure explicitly:

```json
{ "enrich": { "argv": false } }
```

passed to `witslog_init`/`witslog_configure` (or the SDK's `init(config)`). This is proven to
fully suppress `argv` capture end-to-end — see `witslog-ffi::configure_argv_false_suppresses_argv_capture`
and the equivalent regression test in each SDK's unit test suite (`test_init_forwards_argv_disable_config`
/ `init forwards argv-disable config unchanged` / `testInitForwardsArgvDisableConfig`). Other
enrichment (`pid`, `cwd`, `git_commit`, `hostname`) is unaffected and can be disabled independently
the same way.

## Browser-side error capture (P10) — ingest recipe for Python/PHP

`bindings/browser/witslog-browser.js` ships client-side JS errors to a server-side ingest
endpoint via `navigator.sendBeacon`/`fetch`. A Node adapter ships
(`witslogBrowserIngest` in `bindings/node/frameworks/express.js`); Python/PHP adapters are
**not** shipped — three parallel handlers accepting untrusted input is three attack
surfaces to keep in sync for a feature whose whole risk lives in that handling. The
guardrails below are not optional extras; skipping any of them turns the endpoint into
an unauthenticated write into the AI's evidence base (this text ends up in
`events.message`, which `search_errors`/`explain_error` return verbatim to an
MCP-connected LLM). Port the Node handler's logic (see its source for the full rationale):

1. **Origin allowlist, fail-closed.** Reject unless the `Origin` header is in an explicit
   list you provide — default to none. This is the actual defense; the request
   genuinely originates from `127.0.0.1` (the attack is a malicious page open in the same
   browser as your dev server), so a loopback check alone does not stop it.
2. **Refuse to arm in production** unless explicitly forced by the caller.
3. **Rate-limit by client** — per-request size caps don't bound request *volume*.
4. **Clamp severity to `error`/`warn`** (never let untrusted input claim
   `fatal`/`critical`) and cap message/stacktrace/batch/body sizes.
5. Map each accepted event through your SDK's normal `log`/`exception` call with
   `tags: ["browser"]` — advisory only, not a trust boundary; `classify()` merges
   suggested tags into whatever is already there.

True provenance (an `ingest_source` field trusted by the query layer) isn't in the
payload contract above and would need a `WITSLOG_ABI_VERSION` bump — out of scope until
a real need shows up.

## Mount / flush lifecycle

`tracing` (the Rust ambient capture) does **not** cross the ABI. Each SDK:

1. calls `witslog_init` once at startup (installs the Rust panic hook, applies config), and
2. registers the **host language's** uncaught-exception hook (Python `sys.excepthook`, Node
   `process.on('uncaughtException')`, PHP `set_exception_handler`) to route those to
   `witslog_log`, and
3. calls `witslog_shutdown` before process exit (atexit / shutdown handler), since the C ABI has
   no RAII drop to flush a buffer.
