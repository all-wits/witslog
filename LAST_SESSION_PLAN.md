# P6 — Framework-agnostic SDK bindings (Python, Node, PHP/Laravel)

## Context

P6 (`PHASES.md §P6`) is the last unshipped piece of the witslog delivery layer. The C ABI
(`witslog_log/resolve/delete/init/flush/shutdown/configure/free_string`) and the ambient
`witslog-runtime` Provider already exist (P0 + the runtime landing). **No language wrappers
exist yet** — `bindings/` is absent. This pass builds them.

Goal: apps written in **Node.js, PHP/Laravel, and Python (FastAPI/Django/Flask)** log
structured witslog events via their own package manager (npm/pnpm, Composer, pip) without
touching FFI details, and the CLI reads back exactly what the SDK wrote. The SDK core is
**framework-agnostic** (works in any app of that language); web frameworks get thin optional
mount adapters on top.

### Key facts found via codegraph (drive the design)

- **C ABI is complete except two gaps** (`crates/witslog-ffi/src/lib.rs`):
  1. `LogRequest` (lib.rs:178) has **no `context`/`tags`/`metadata`** fields, yet `EventBuilder`
     exposes `.context()/.tags()/.metadata()` (`event.rs:194-207`) and **FR-P6-006** requires the
     SDK pass context/tags through the contract unchanged. → must extend `LogRequest` + wire the
     three builder calls in before `.enrich().redact().build()` (lib.rs:245-248).
  2. **No contract-version export.** P6 TODO + non-functional "versioned contract so SDK/native
     mismatches are detectable" → add `witslog_abi_version()`.
- FFI already builds `cdylib`+`staticlib`+`rlib` (`witslog-ffi/Cargo.toml:9`) → `witslog_ffi.dll`
  on Windows. DB path resolves by **cwd walk-up** (`resolve_db_path`, lib.rs:9) — SDKs just run
  from the project dir, same as the CLI.
- `witslog_log` returns the **rowid** (i64), not the event_id → SDK `log()` returns an int; e2e
  reads back by **CLI `query "<marker>"`**, not by id.
- `witslog_init(json)` mounts + installs the Rust panic hook; `witslog_shutdown()` flushes. SDKs
  add the **host-language** uncaught hook (Python `sys.excepthook`, Node `uncaughtException`, PHP
  `set_exception_handler`) → `witslog_log`, since `tracing` does not cross the ABI (CLAUDE.md).
- Per-language FFI mechanism: **Python `ctypes`** (stdlib, 0 deps), **PHP `ext-ffi`** (builtin),
  **Node `koffi`** (one prebuilt npm dep — no native build).

### Local toolchains (verified — e2e runs here)

cargo 1.95 ✅ · Python 3.14 via **`py`** launcher ✅ · Node v22 + npm 10 ✅ · PHP 8.5 + Composer ✅.
PHP `FFI` is off in default php.ini but **enables via `php -d extension=ffi -d ffi.enable=1`** ✅
(the SDK docs/tests use that flag; php_ffi.dll is present in `C:\php\ext`).

---

## Plan

### 1. Extend the C ABI (Rust, additive) — `crates/witslog-ffi/src/lib.rs`

- Add to `LogRequest`: `context: Option<serde_json::Value>`, `tags: Option<Vec<String>>`,
  `metadata: Option<serde_json::Value>`.
- In `witslog_log`, before `.enrich(...)`, apply `builder = builder.context(c)` / `.tags(t)` /
  `.metadata(m)` when present (mirrors the existing `if let Some(...)` chain, lib.rs:220-243).
  Redaction of context/metadata already happens in `.redact()` (`event.rs:226-231`) — no extra work.
- Add `const WITSLOG_ABI_VERSION: i32 = 1;` and
  `#[no_mangle] pub extern "C" fn witslog_abi_version() -> i32 { WITSLOG_ABI_VERSION }`.
- **Do not** change any existing signature — purely additive (keeps FR-P6 non-functional
  "additive" and CLAUDE.md's "existing signatures unchanged").

### 2. Shared contract doc — `bindings/CONTRACT.md`

Document contract **v1**: the JSON `witslog_log` accepts —
`application`(req), `message`(req), `severity?`, `version?`, `environment?`, `category?`,
`error_code?`, `exception?`, `stacktrace?`, `correlation_id?`, `parent_event_id?`,
`context?`(obj), `tags?`(str[]), `metadata?`(obj) — plus the lib-locator rule and ABI-version
handshake. Single source every core references.

### 3. Native-lib locator (identical rule in all 3 cores)

Search order: `WITSLOG_LIB` env (explicit path) → package-bundled
`_libs/<platform>/witslog_ffi.{dll,so,dylib}` → OS default loader. On failure raise the
language-native `WitslogLibraryError` **naming the searched paths** (FR-P6 error table). For
dev/e2e, `WITSLOG_LIB` points at `target/release/witslog_ffi.dll`.

### 4. Python core + adapters — `bindings/python/` (pip pkg `witslog`)

- `witslog/_ffi.py`: ctypes loader (locator above), declares the 8 exports + `witslog_abi_version`;
  asserts version == 1 else `WitslogContractError`.
- `witslog/errors.py`: `WitslogLibraryError`, `WitslogWriteError`, `WitslogContractError`.
- `witslog/__init__.py`: surface `error/warn/info/exception/log(**fields)`, a fluent `Builder`,
  `init(config=None)/flush/shutdown`, `install_excepthook()`. Marshals dict→`json.dumps`→
  `witslog_log`; return `-1` → `WitslogWriteError`; non-str/non-UTF8 → `ValueError`/`TypeError`.
  `exception(exc)` fills `exception`+`stacktrace` from `traceback.format_exc()`; passes
  `context`/`tags`/`metadata` through (FR-P6-006). Never raises into the host on a logging failure
  except the documented boundary errors (FR-P6-005).
- `witslog/frameworks/fastapi.py`: `add_witslog(app, **cfg)` — startup `init`, shutdown `shutdown`,
  exception handler logs via `exception()` then re-raises.
- `witslog/frameworks/django.py`: `WitslogMiddleware` (`process_exception`) + AppConfig `ready()`
  mounts.
- `witslog/frameworks/flask.py`: `Witslog(app)` extension — `got_request_exception` handler +
  teardown flush.
- `pyproject.toml` (pip/`build`), `tests/`.

### 5. Node core + adapter — `bindings/node/` (npm pkg `witslog`)

- `lib/ffi.js`: **koffi** loads the dll, declares the exports, ABI check → `WitslogContractError`;
  locator → `WitslogLibraryError`.
- `index.js` + `index.d.ts`: `error/warn/info/exception/log(fields)`, `init/flush/shutdown`,
  `installUncaughtHandler()` → `process.on('uncaughtException'|'unhandledRejection')` logs then
  rethrows; `process.on('exit')` flush. `exception(err)` pulls `err.stack`.
- `frameworks/express.js`: error middleware `(err, req, res, next)` → log → `next(err)`.
- `package.json` (dep: `koffi`), `test/` (`node:test`).

### 6. PHP core + Laravel adapter — `bindings/php/` (composer pkg `witslog/witslog`)

- `src/Ffi.php`: `\FFI::cdef("<C decls>", <lib path>)` via locator; `WitslogLibraryError`; ABI check
  → `WitslogContractError`.
- `src/Witslog.php`: static `error/warn/info/exception/log(array)`, `init/flush/shutdown`,
  `register_shutdown_function` flush, `set_exception_handler` capture. `exception(\Throwable $e)` →
  message + `getTraceAsString()`; passes context/tags/metadata.
- `src/Laravel/WitslogServiceProvider.php`: `boot()` mounts (`init`), registers app-terminating
  flush; documents the Laravel 11 `bootstrap/app.php` `->withExceptions(fn($e)=>Witslog::exception($e))`
  hook (and `Handler::report()` for L10).
- `composer.json` (`require: ext-ffi, php >=8.1`), `tests/` (PHPUnit).

### 7. Tests

**Unit** (fast, mostly no dll needed):
- Rust — extend `#[cfg(test)] mod tests` in `witslog-ffi/src/lib.rs`: (a) log with
  `context`+`tags`+`metadata` → persisted + readable (query the DB row like the existing
  roundtrip test at lib.rs:440); (b) `witslog_abi_version() == 1`.
- Python `pytest`, Node `node:test`, PHP `PHPUnit` — each covers the FR-P6 error table:
  lib-not-found → `WitslogLibraryError` (name paths); ABI mismatch (monkeypatch/stub) →
  `WitslogContractError`; write `-1` → `WitslogWriteError`; bad/non-UTF8 input → `ValueError`/type
  error; field marshalling maps every contract key (spy on the JSON handed to `witslog_log`).

**Regression / e2e** (the real "CLI reads what SDK wrote" contract — FR-P6 acceptance + PHASES P6
verification): per language, in a temp project with `.witslog/`, `WITSLOG_LIB` → built dll:
`init()` → `log()` an event with a **unique marker message + context + tags** → `flush()` →
run the built `witslog query "<marker>"` and assert the event (incl. context/tags) appears. Plus:
`exception()` stores a stacktrace; removing the lib raises `WitslogLibraryError`. The context/tags
assertion is the regression guard for the ABI extension (step 1).

**Driver** (Bash is broken in this env → PowerShell): `bindings/e2e/run.ps1` builds
`witslog-ffi` + `witslog-cli` release, then runs each language's smoke script via `py` / `node` /
`php -d extension=ffi -d ffi.enable=1`, asserting readback.

### Files

- **Modify**: `crates/witslog-ffi/src/lib.rs` (LogRequest fields + wiring, `witslog_abi_version`,
  unit tests). Optionally `Cargo.toml` workspace note; `CLAUDE.md`/`PHASES.md` P6 status → done.
- **Create**: `bindings/CONTRACT.md`; `bindings/python/**`, `bindings/node/**`, `bindings/php/**`
  (core + `frameworks/`/`Laravel/` + `tests/` + package manifest each); `bindings/e2e/run.ps1`
  (+ per-language smoke scripts).

---

## Verification (end-to-end, then `/verify`)

1. `cargo test -p witslog-ffi` — Rust unit incl. new context/tags/metadata + abi-version pass.
2. `cargo build --release -p witslog-ffi -p witslog-cli` — dll + `witslog.exe` produced.
3. Python: `py -m pytest bindings/python/tests` green; e2e smoke → CLI `query` finds the SDK-written
   event with its context/tags.
4. Node: `npm install && npm test` in `bindings/node` green; e2e smoke → CLI readback.
5. PHP: `composer install` then `php -d extension=ffi -d ffi.enable=1 vendor/bin/phpunit` green;
   e2e smoke → CLI readback.
6. `bindings/e2e/run.ps1` runs all three back-to-back and reports pass/fail.
7. Run the **`/verify`** skill to drive the assembled flow (SDK log → CLI read) and observe real
   behavior, not just test exit codes.

Success = each package manager (pip/npm/Composer) installs its core, an app in each named
framework mounts witslog in one call, and the CLI reads back the exact event — with `context`/`tags`
surviving the round-trip.