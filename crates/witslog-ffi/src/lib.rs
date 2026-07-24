use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Deserialize;
use serde_json::Value as JsonValue;
use witslog_core::{
    AsyncBuffer, BufferConfig, EnrichConfig, EventBuilder, FieldCipher, Redactor, Severity,
};
use witslog_store::{DeleteFilter, Store, StoreSink};

/// Version of the JSON contract this native library speaks. SDK wrappers call
/// `witslog_abi_version()` at load time and compare against the version they were
/// built for, so a native/SDK mismatch is detected rather than silently mis-parsed.
/// Bump on any breaking change to the `witslog_log` / `witslog_configure` payloads.
const WITSLOG_ABI_VERSION: i32 = 1;

/// Return the JSON-contract version this library implements. Stable, side-effect-free.
#[no_mangle]
pub extern "C" fn witslog_abi_version() -> i32 {
    WITSLOG_ABI_VERSION
}

fn resolve_db_path() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = witslog_config::Config::default_project();
    config.resolve_db_path(&cwd)
}

/// Process-wide runtime config set via `witslog_configure`. Absent a call to
/// `witslog_configure`, `witslog_log` behaves exactly as before P1 (built-in
/// redaction still applies per FR-P1-003, which is ubiquitous; enrichment
/// defaults all-on; buffering off).
#[derive(Clone)]
struct RuntimeConfig {
    enrich: EnrichConfig,
    redactor: Arc<Redactor>,
    buffer: BufferConfig,
    /// FR-P9-004: env var name holding the metadata-encryption key, or `None`
    /// (default) — encryption off. Mirrors `witslog_config::CryptoSection`;
    /// resolved to a `FieldCipher` fresh at each `witslog_log` call (see
    /// `witslog_runtime::resolve_cipher` for the same pattern/rationale).
    crypto_key_env: Option<String>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        RuntimeConfig {
            enrich: EnrichConfig::default(),
            redactor: Arc::new(Redactor::built_in()),
            buffer: BufferConfig::default(),
            crypto_key_env: None,
        }
    }
}

static RUNTIME_CONFIG: OnceLock<Mutex<RuntimeConfig>> = OnceLock::new();
static BUFFER: OnceLock<Mutex<Option<AsyncBuffer<StoreSink>>>> = OnceLock::new();

fn runtime_config() -> RuntimeConfig {
    RUNTIME_CONFIG
        .get_or_init(|| Mutex::new(RuntimeConfig::default()))
        .lock()
        .unwrap()
        .clone()
}

#[derive(Deserialize, Default)]
struct EnrichConfigDto {
    hostname: Option<bool>,
    pid: Option<bool>,
    cwd: Option<bool>,
    argv: Option<bool>,
    git_commit: Option<bool>,
    env_allowlist: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
struct RedactConfigDto {
    custom_patterns: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
struct BufferConfigDto {
    enabled: Option<bool>,
    batch_size: Option<usize>,
    flush_interval_ms: Option<u64>,
    queue_capacity: Option<usize>,
}

#[derive(Deserialize, Default)]
struct CryptoConfigDto {
    key_env: Option<String>,
}

#[derive(Deserialize, Default)]
struct ConfigureRequest {
    enrich: Option<EnrichConfigDto>,
    redact: Option<RedactConfigDto>,
    buffer: Option<BufferConfigDto>,
    crypto: Option<CryptoConfigDto>,
}

/// Configure runtime enrichment/redaction/buffering for subsequent `witslog_log`
/// calls in this process. Returns 0 on success, -1 on malformed JSON, -2 on an
/// invalid `redact.custom_patterns` regex (config left unchanged on -2).
///
/// # Safety
/// `json_ptr` must be a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn witslog_configure(json_ptr: *const c_char) -> i32 {
    let json = match cstr_to_string(json_ptr) {
        Some(s) => s,
        None => return -1,
    };

    let req: ConfigureRequest = match serde_json::from_str(&json) {
        Ok(r) => r,
        Err(_) => return -1,
    };

    let mut cfg = runtime_config();

    if let Some(e) = req.enrich {
        if let Some(v) = e.hostname {
            cfg.enrich.hostname = v;
        }
        if let Some(v) = e.pid {
            cfg.enrich.pid = v;
        }
        if let Some(v) = e.cwd {
            cfg.enrich.cwd = v;
        }
        if let Some(v) = e.argv {
            cfg.enrich.argv = v;
        }
        if let Some(v) = e.git_commit {
            cfg.enrich.git_commit = v;
        }
        if let Some(v) = e.env_allowlist {
            cfg.enrich.env_allowlist = v;
        }
    }

    if let Some(r) = req.redact {
        if let Some(patterns) = r.custom_patterns {
            match Redactor::new(&patterns) {
                Ok(redactor) => cfg.redactor = Arc::new(redactor),
                Err(_) => return -2,
            }
        }
    }

    if let Some(b) = req.buffer {
        if let Some(v) = b.enabled {
            cfg.buffer.enabled = v;
        }
        if let Some(v) = b.batch_size {
            cfg.buffer.batch_size = v;
        }
        if let Some(v) = b.flush_interval_ms {
            cfg.buffer.flush_interval_ms = v;
        }
        if let Some(v) = b.queue_capacity {
            cfg.buffer.queue_capacity = v;
        }
    }

    if let Some(c) = req.crypto {
        // `key_env: null`/absent leaves the current setting unchanged (same
        // "only touch what's present" convention as enrich/redact/buffer
        // above); pass `key_env: ""` explicitly to turn encryption back off.
        if let Some(v) = c.key_env {
            cfg.crypto_key_env = if v.is_empty() { None } else { Some(v) };
        }
    }

    let lock = RUNTIME_CONFIG.get_or_init(|| Mutex::new(RuntimeConfig::default()));
    *lock.lock().unwrap() = cfg;

    // Drop any existing buffer so the next buffered `witslog_log` call rebuilds
    // it (flushing whatever was queued) under the new buffer config.
    if let Some(buf_lock) = BUFFER.get() {
        *buf_lock.lock().unwrap() = None;
    }

    0
}

fn severity_from_str(s: Option<&str>) -> Severity {
    match s.unwrap_or("error") {
        "trace" => Severity::Trace,
        "debug" => Severity::Debug,
        "info" => Severity::Info,
        "warn" => Severity::Warn,
        "error" => Severity::Error,
        "critical" => Severity::Critical,
        "fatal" => Severity::Fatal,
        _ => Severity::Error,
    }
}

unsafe fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string())
}

fn string_to_cstring_ptr(s: String) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}

#[derive(Deserialize)]
struct LogRequest {
    application: String,
    message: String,
    severity: Option<String>,
    version: Option<String>,
    environment: Option<String>,
    category: Option<String>,
    error_code: Option<String>,
    exception: Option<String>,
    stacktrace: Option<String>,
    correlation_id: Option<String>,
    parent_event_id: Option<String>,
    context: Option<JsonValue>,
    tags: Option<Vec<String>>,
    metadata: Option<JsonValue>,
}

/// Log an event from a JSON payload. Returns the inserted row id on the
/// synchronous (unbuffered) path, or -1 on error. **When buffering is enabled**
/// (via `witslog_configure`), the event is queued for background flush and this
/// returns `0` — the row id isn't known synchronously, since the write hasn't
/// happened yet. Never panics: a write failure (including a read-only DB) is
/// swallowed and reported as -1 (or, when buffered, folded into the dropped
/// counter instead of surfacing an error at all).
///
/// # Safety
/// `json_ptr` must be a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn witslog_log(json_ptr: *const c_char) -> i64 {
    let json = match cstr_to_string(json_ptr) {
        Some(s) => s,
        None => return -1,
    };

    let req: LogRequest = match serde_json::from_str(&json) {
        Ok(r) => r,
        Err(_) => return -1,
    };

    let cfg = runtime_config();

    let mut builder = EventBuilder::new(req.application, req.message)
        .severity(severity_from_str(req.severity.as_deref()));

    if let Some(v) = req.version {
        builder = builder.version(v);
    }
    if let Some(e) = req.environment {
        builder = builder.environment(e);
    }
    if let Some(c) = req.category {
        builder = builder.category(c);
    }
    if let Some(c) = req.error_code {
        builder = builder.error_code(c);
    }
    if let Some(e) = req.exception {
        builder = builder.exception(e);
    }
    if let Some(s) = req.stacktrace {
        builder = builder.stacktrace(s);
    }
    if let Some(c) = req.correlation_id {
        builder = builder.correlation_id(c);
    }
    if let Some(p) = req.parent_event_id {
        builder = builder.parent_event_id(p);
    }
    if let Some(c) = req.context {
        builder = builder.context(c);
    }
    if let Some(t) = req.tags {
        builder = builder.tags(t);
    }
    if let Some(m) = req.metadata {
        builder = builder.metadata(m);
    }

    // Fail-closed (FR-P9-004): if `crypto.key_env` is configured but the key
    // can't be resolved (unset var, bad hex), refuse the write rather than
    // silently persisting `metadata` in plaintext. `-1` matches every other
    // write-failure code this function already returns.
    let cipher = match &cfg.crypto_key_env {
        None => None,
        Some(var) => match FieldCipher::from_env(var) {
            Ok(Some(c)) => Some(c),
            Ok(None) | Err(_) => return -1,
        },
    };

    let builder = builder.enrich(&cfg.enrich).redact(cfg.redactor.as_ref());
    let builder = match &cipher {
        Some(c) => builder.encrypt_metadata(c),
        None => builder,
    };
    let event = builder.build();

    if cfg.buffer.enabled {
        let buf_lock = BUFFER.get_or_init(|| Mutex::new(None));
        let mut guard = buf_lock.lock().unwrap();
        if guard.is_none() {
            let store = match Store::open_or_create(resolve_db_path()) {
                Ok(s) => s,
                Err(_) => return -1,
            };
            *guard = Some(AsyncBuffer::new(StoreSink::new(store), cfg.buffer.clone()));
        }
        if let Some(buffer) = guard.as_ref() {
            buffer.enqueue(event);
        }
        return 0;
    }

    let store = match Store::open_or_create(resolve_db_path()) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let writer = witslog_store::EventWriter::new(store.conn());

    match writer.write(&event) {
        Ok(row_id) => row_id,
        Err(_) => -1,
    }
}

/// Mark an event resolved. Returns 0 on success, -1 on error.
///
/// # Safety
/// `event_id_ptr` must be a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn witslog_resolve(event_id_ptr: *const c_char) -> i32 {
    let event_id = match cstr_to_string(event_id_ptr) {
        Some(s) => s,
        None => return -1,
    };

    let store = match Store::open_or_create(resolve_db_path()) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let writer = witslog_store::EventWriter::new(store.conn());

    // `force: false` — first resolution wins; this FFI entrypoint's ABI
    // (no new params) is unchanged, so an already-resolved event now
    // reports -1 rather than silently moving resolved_at, matching the
    // CLI's `mark_resolved` fix (FR-P10-002).
    match writer.mark_resolved(&event_id, false) {
        Ok(true) => 0,
        Ok(false) => -1,
        Err(_) => -1,
    }
}

#[derive(Deserialize, Default)]
struct DeleteRequest {
    event_id: Option<String>,
    fingerprint: Option<String>,
    resolved_before: Option<String>,
    #[serde(default)]
    force: bool,
}

/// Delete stale/resolved event(s) matching a JSON filter.
/// Returns a JSON string `{"deleted_count":N,"deleted_ids":[...]}` (caller must free
/// via `witslog_free_string`), or a null pointer on error.
///
/// # Safety
/// `filter_json_ptr` must be a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn witslog_delete(filter_json_ptr: *const c_char) -> *mut c_char {
    let json = match cstr_to_string(filter_json_ptr) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let req: DeleteRequest = match serde_json::from_str(&json) {
        Ok(r) => r,
        Err(_) => return std::ptr::null_mut(),
    };

    let store = match Store::open_or_create(resolve_db_path()) {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let writer = witslog_store::EventWriter::new(store.conn());

    let filter = DeleteFilter {
        event_id: req.event_id,
        fingerprint: req.fingerprint,
        resolved_before: req.resolved_before,
        force: req.force,
    };

    match writer.delete_resolved(&filter) {
        Ok(ids) => {
            let body = serde_json::json!({
                "deleted_count": ids.len(),
                "deleted_ids": ids,
            });
            string_to_cstring_ptr(body.to_string())
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// Scaffold a `.witslog/` project directory (mirrors the CLI's `witslog init`):
/// creates `<path>/.witslog/`, opens/creates `witslog.db` inside it, and runs
/// migrations. `path_ptr` may be null to use the current working directory.
/// Idempotent — safe to call against an already-initialized project (dir
/// creation and `Store::open_or_create`'s migrate step are both no-ops on a
/// second call). This exists because none of the FFI write paths
/// (`witslog_log`/`witslog_resolve`/`witslog_delete`) create the parent
/// `.witslog/` directory themselves — `SQLITE_OPEN_CREATE` creates the DB
/// *file*, not missing parent directories — so a process that never ran the
/// separately-distributed CLI's `witslog init` had no way to bootstrap a
/// project from the native lib alone. Returns 0 on success, -1 on I/O or DB
/// error.
///
/// # Safety
/// `path_ptr` must be null or a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn witslog_bootstrap_project(path_ptr: *const c_char) -> i32 {
    let base = match cstr_to_string(path_ptr) {
        Some(s) => PathBuf::from(s),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };

    let witslog_dir = base.join(".witslog");
    if std::fs::create_dir_all(&witslog_dir).is_err() {
        return -1;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::set_permissions(&witslog_dir, std::fs::Permissions::from_mode(0o700)).is_err()
        {
            return -1;
        }
    }

    let db_path = witslog_dir.join("witslog.db");
    let _store = match Store::open_or_create(&db_path) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o600)).is_err() {
            return -1;
        }
    }

    0
}

/// Mount witslog for the current process ("Provider" entrypoint). Applies the
/// same configuration payload as `witslog_configure` (pass a null pointer to
/// mount with defaults), then arms the ambient runtime so **Rust-side panics in
/// this library are auto-captured** as `Fatal` events.
///
/// Capturing the *host language's* uncaught exceptions (Python `sys.excepthook`,
/// Node `process.on('uncaughtException')`, …) is the SDK wrapper's job — it
/// should route those to `witslog_log`. Call `witslog_flush`/`witslog_shutdown`
/// before the process exits (e.g. from an atexit handler), since the C ABI has
/// no RAII drop to flush a buffer.
///
/// Returns 0 on success, -1 on malformed JSON, -2 on an invalid redaction regex.
///
/// # Safety
/// `json_ptr` must be null or a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn witslog_init(json_ptr: *const c_char) -> i32 {
    if !json_ptr.is_null() {
        let rc = witslog_configure(json_ptr);
        if rc < 0 {
            return rc;
        }
    }

    // Arm the ambient runtime (installs the process-wide panic hook). It has no
    // RAII guard here — the process lives until the host tears it down, and
    // flushing is driven explicitly via `witslog_flush`.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    witslog_runtime::arm(witslog_config::Config::load_or_default(&cwd));
    0
}

/// Flush any buffered events, joining the background flush thread so queued
/// events are persisted before return. Idempotent. Call before process exit.
#[no_mangle]
pub extern "C" fn witslog_flush() -> i32 {
    if let Some(buf_lock) = BUFFER.get() {
        // Dropping the AsyncBuffer joins its flush thread.
        *buf_lock.lock().unwrap() = None;
    }
    witslog_runtime::flush();
    0
}

/// Un-mount witslog: flush buffered events and tear down the buffer. Alias of
/// `witslog_flush` today; kept as a distinct symbol so SDK shutdown paths read
/// clearly and future teardown can hang off it.
#[no_mangle]
pub extern "C" fn witslog_shutdown() -> i32 {
    witslog_flush()
}

/// Free a string previously returned by this library (e.g. from `witslog_delete`).
///
/// # Safety
/// `ptr` must have been returned by a `witslog_*` function in this crate, and must not
/// be freed more than once.
#[no_mangle]
pub unsafe extern "C" fn witslog_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    // `std::env::set_current_dir` and the RUNTIME_CONFIG/BUFFER statics are
    // process-global, so tests using `with_tmp_cwd` must not run concurrently.
    static CWD_LOCK: Mutex<()> = Mutex::new(());

    fn with_tmp_cwd<F: FnOnce()>(f: F) {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let witslog_dir = dir.path().join(".witslog");
        std::fs::create_dir_all(&witslog_dir).unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        f();
        std::env::set_current_dir(orig).unwrap();
    }

    #[test]
    fn test_log_resolve_delete_roundtrip() {
        with_tmp_cwd(|| unsafe {
            let log_json =
                CString::new(r#"{"application":"app","message":"boom","severity":"error"}"#)
                    .unwrap();
            let row_id = witslog_log(log_json.as_ptr());
            assert!(row_id >= 0);

            let store = Store::open_or_create(resolve_db_path()).unwrap();
            let event_id: String = store
                .conn()
                .conn()
                .query_row("SELECT event_id FROM events LIMIT 1", [], |r| r.get(0))
                .unwrap();
            drop(store);

            let event_id_c = CString::new(event_id.clone()).unwrap();
            assert_eq!(witslog_resolve(event_id_c.as_ptr()), 0);

            let delete_json =
                CString::new(format!(r#"{{"event_id":"{}"}}"#, event_id)).unwrap();
            let result_ptr = witslog_delete(delete_json.as_ptr());
            assert!(!result_ptr.is_null());
            let result_str = CStr::from_ptr(result_ptr).to_str().unwrap().to_string();
            witslog_free_string(result_ptr);
            assert!(result_str.contains("\"deleted_count\":1"));

            let store = Store::open_or_create(resolve_db_path()).unwrap();
            let remaining: i64 = store
                .conn()
                .conn()
                .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
                .unwrap();
            assert_eq!(remaining, 0);
        });
    }

    #[test]
    fn init_log_flush_shutdown_roundtrip_drains_buffer() {
        with_tmp_cwd(|| unsafe {
            // Mount with buffering enabled and a flush interval long enough that
            // the count-before assertion below can't lose the race with the
            // background timer, but short enough that `witslog_flush`'s
            // shutdown-join (which only wakes on the *next* recv_timeout tick)
            // doesn't stall the test for a full minute.
            let init_json = CString::new(
                r#"{"buffer":{"enabled":true,"batch_size":50,"flush_interval_ms":2000}}"#,
            )
            .unwrap();
            assert_eq!(witslog_init(init_json.as_ptr()), 0);

            let log_json =
                CString::new(r#"{"application":"ffi-test","message":"buffered event"}"#)
                    .unwrap();
            let row_id = witslog_log(log_json.as_ptr());
            // Buffered path returns 0 immediately; the row id isn't known yet.
            assert_eq!(row_id, 0);

            let count_before: i64 = {
                let store = Store::open_or_create(resolve_db_path()).unwrap();
                let n = store
                    .conn()
                    .conn()
                    .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
                    .unwrap();
                n
            };
            assert_eq!(count_before, 0, "event must still be queued, not yet persisted");

            assert_eq!(witslog_flush(), 0);

            let count_after: i64 = {
                let store = Store::open_or_create(resolve_db_path()).unwrap();
                let n = store
                    .conn()
                    .conn()
                    .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
                    .unwrap();
                n
            };
            assert_eq!(count_after, 1, "flush must persist the buffered event");

            assert_eq!(witslog_shutdown(), 0);

            // Reset global config so later tests in this process see defaults.
            let reset_json = CString::new(r#"{"buffer":{"enabled":false}}"#).unwrap();
            witslog_configure(reset_json.as_ptr());
        });
    }

    #[test]
    fn witslog_init_with_null_config_mounts_with_defaults() {
        with_tmp_cwd(|| unsafe {
            assert_eq!(witslog_init(std::ptr::null()), 0);

            let log_json =
                CString::new(r#"{"application":"app","message":"unbuffered via init"}"#)
                    .unwrap();
            let row_id = witslog_log(log_json.as_ptr());
            // Buffering defaults off, so the synchronous path returns a real row id.
            assert!(row_id > 0);

            assert_eq!(witslog_flush(), 0);
        });
    }

    #[test]
    fn witslog_init_rejects_malformed_config_json() {
        let bad_json = CString::new("not json").unwrap();
        let rc = unsafe { witslog_init(bad_json.as_ptr()) };
        assert_eq!(rc, -1);
    }

    #[test]
    fn configure_custom_redact_pattern_applies_to_logged_message() {
        with_tmp_cwd(|| unsafe {
            let configure_json =
                CString::new(r#"{"redact":{"custom_patterns":["MY_TOKEN_[A-Z0-9]+"]}}"#)
                    .unwrap();
            assert_eq!(witslog_configure(configure_json.as_ptr()), 0);

            let log_json = CString::new(
                r#"{"application":"app","message":"leaked MY_TOKEN_ABC123 here","severity":"error"}"#,
            )
            .unwrap();
            let row_id = witslog_log(log_json.as_ptr());
            assert!(row_id >= 0);

            let store = Store::open_or_create(resolve_db_path()).unwrap();
            let message: String = store
                .conn()
                .conn()
                .query_row("SELECT message FROM events LIMIT 1", [], |r| r.get(0))
                .unwrap();
            assert!(!message.contains("MY_TOKEN_ABC123"));
            assert!(message.contains("«redacted»"));

            // Reset global config so later tests in this process see defaults.
            let reset_json = CString::new(r#"{"redact":{"custom_patterns":[]}}"#).unwrap();
            witslog_configure(reset_json.as_ptr());
        });
    }

    #[test]
    fn configure_rejects_invalid_regex() {
        let json = CString::new(r#"{"redact":{"custom_patterns":["(unclosed"]}}"#).unwrap();
        let result = unsafe { witslog_configure(json.as_ptr()) };
        assert_eq!(result, -2);
    }

    #[test]
    fn configure_argv_false_suppresses_argv_capture() {
        // Regression lock: SDKs must be able to fully close the argv/secret-exposure
        // gap (a secret typed as a bare CLI arg won't match redaction patterns) by
        // disabling argv enrichment via witslog_configure/witslog_init. This proves
        // the mitigation actually works end-to-end through the C ABI, not just that
        // the config field exists.
        with_tmp_cwd(|| unsafe {
            let configure_json = CString::new(r#"{"enrich":{"argv":false}}"#).unwrap();
            assert_eq!(witslog_configure(configure_json.as_ptr()), 0);

            let log_json =
                CString::new(r#"{"application":"app","message":"argv disabled test"}"#).unwrap();
            let row_id = witslog_log(log_json.as_ptr());
            assert!(row_id >= 0);

            let store = Store::open_or_create(resolve_db_path()).unwrap();
            let context: Option<String> = store
                .conn()
                .conn()
                .query_row("SELECT context FROM events LIMIT 1", [], |r| r.get(0))
                .unwrap();
            let context = context.unwrap_or_default();
            assert!(
                !context.contains("\"argv\""),
                "argv must be absent from context when enrich.argv=false, got: {context}"
            );
            // Other enrichment (pid/cwd) still applies — only argv was disabled.
            assert!(context.contains("\"pid\""));

            // Reset global config so later tests in this process see defaults.
            let reset_json = CString::new(r#"{"enrich":{"argv":true}}"#).unwrap();
            witslog_configure(reset_json.as_ptr());
        });
    }

    /// 64 hex chars ("07" * 32) = a valid 32-byte AES-256 key.
    fn test_key_hex() -> String {
        "07".repeat(32)
    }

    #[test]
    fn configure_crypto_key_env_encrypts_metadata_through_witslog_log() {
        // Regression lock (FR-P9-004, FFI pipeline): SDK writes (Python/Node/PHP)
        // go through this crate's own RuntimeConfig/pipeline, not witslog-runtime's
        // — this proves `metadata` encryption is wired here too, not just in the
        // Rust ambient/CLI path (see witslog-runtime's p9_crypto_integration.rs).
        with_tmp_cwd(|| unsafe {
            let var = "WITSLOG_TEST_FFI_CRYPTO_KEY";
            std::env::set_var(var, test_key_hex());

            let configure_json =
                CString::new(format!(r#"{{"crypto":{{"key_env":"{var}"}}}}"#)).unwrap();
            assert_eq!(witslog_configure(configure_json.as_ptr()), 0);

            let log_json = CString::new(
                r#"{"application":"app","message":"boom","metadata":{"user_email":"x@y.com"}}"#,
            )
            .unwrap();
            let row_id = witslog_log(log_json.as_ptr());
            assert!(row_id >= 0);

            let store = Store::open_or_create(resolve_db_path()).unwrap();
            let metadata: String = store
                .conn()
                .conn()
                .query_row("SELECT metadata FROM events LIMIT 1", [], |r| r.get(0))
                .unwrap();
            assert!(
                metadata.contains("__witslog_enc"),
                "metadata stored as envelope, got: {metadata}"
            );
            assert!(!metadata.contains("x@y.com"), "plaintext must not appear in storage");

            std::env::remove_var(var);
            // Reset global config so later tests in this process see defaults.
            let reset_json = CString::new(r#"{"crypto":{"key_env":""}}"#).unwrap();
            witslog_configure(reset_json.as_ptr());
        });
    }

    #[test]
    fn configure_crypto_key_env_fails_closed_when_var_unset() {
        with_tmp_cwd(|| unsafe {
            let var = "WITSLOG_TEST_FFI_CRYPTO_KEY_UNSET";
            std::env::remove_var(var); // ensure genuinely unset

            let configure_json =
                CString::new(format!(r#"{{"crypto":{{"key_env":"{var}"}}}}"#)).unwrap();
            assert_eq!(witslog_configure(configure_json.as_ptr()), 0);

            let log_json = CString::new(
                r#"{"application":"app","message":"boom","metadata":{"a":1}}"#,
            )
            .unwrap();
            let row_id = witslog_log(log_json.as_ptr());
            assert_eq!(row_id, -1, "write must be refused, not silently persisted in plaintext");

            let store = Store::open_or_create(resolve_db_path()).unwrap();
            let count: i64 = store
                .conn()
                .conn()
                .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);

            // Reset global config so later tests in this process see defaults.
            let reset_json = CString::new(r#"{"crypto":{"key_env":""}}"#).unwrap();
            witslog_configure(reset_json.as_ptr());
        });
    }

    #[test]
    fn witslog_log_fails_when_witslog_dir_absent() {
        // Regression lock for the npm-SDK gap: without a pre-existing `.witslog/`
        // dir (previously only creatable via the separately-distributed CLI's
        // `witslog init`), the FFI write path must fail cleanly (-1), not panic
        // or silently write outside the project.
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let log_json = CString::new(r#"{"application":"app","message":"no project yet"}"#).unwrap();
        let result = unsafe { witslog_log(log_json.as_ptr()) };

        std::env::set_current_dir(orig).unwrap();
        assert_eq!(result, -1);
    }

    #[test]
    fn bootstrap_project_creates_dir_and_enables_logging() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        assert!(!dir.path().join(".witslog").exists());
        assert_eq!(unsafe { witslog_bootstrap_project(std::ptr::null()) }, 0);
        assert!(dir.path().join(".witslog").join("witslog.db").exists());

        let log_json =
            CString::new(r#"{"application":"app","message":"project bootstrapped"}"#).unwrap();
        let row_id = unsafe { witslog_log(log_json.as_ptr()) };

        std::env::set_current_dir(orig).unwrap();
        assert!(row_id >= 0, "log must succeed once the project dir exists");
    }

    #[test]
    fn bootstrap_project_is_idempotent() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        assert_eq!(unsafe { witslog_bootstrap_project(std::ptr::null()) }, 0);
        assert_eq!(unsafe { witslog_bootstrap_project(std::ptr::null()) }, 0);

        std::env::set_current_dir(orig).unwrap();
    }

    #[test]
    fn bootstrap_project_accepts_explicit_path() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path_str = dir.path().to_str().unwrap();
        let path_c = CString::new(path_str).unwrap();

        assert_eq!(unsafe { witslog_bootstrap_project(path_c.as_ptr()) }, 0);
        assert!(dir.path().join(".witslog").join("witslog.db").exists());
    }

    #[test]
    fn abi_version_is_current() {
        assert_eq!(witslog_abi_version(), WITSLOG_ABI_VERSION);
        assert_eq!(witslog_abi_version(), 1);
    }

    #[test]
    fn log_persists_context_tags_and_metadata() {
        with_tmp_cwd(|| unsafe {
            let log_json = CString::new(
                r#"{"application":"app","message":"ctx event","severity":"error",
                    "context":{"request_id":"req-42","pid":7},
                    "tags":["alpha","beta"],
                    "metadata":{"k":"v"}}"#,
            )
            .unwrap();
            let row_id = witslog_log(log_json.as_ptr());
            assert!(row_id >= 0);

            let store = Store::open_or_create(resolve_db_path()).unwrap();
            let (context, tags, metadata): (String, String, String) = store
                .conn()
                .conn()
                .query_row(
                    "SELECT context, tags, metadata FROM events LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .unwrap();

            // context/metadata round-trip as JSON objects; tags as a JSON array.
            assert!(context.contains("\"request_id\""));
            assert!(context.contains("req-42"));
            assert!(tags.contains("alpha") && tags.contains("beta"));
            assert!(metadata.contains("\"k\"") && metadata.contains("\"v\""));
        });
    }

    #[cfg(unix)]
    #[test]
    fn read_only_db_does_not_panic_and_returns_error() {
        use std::os::unix::fs::PermissionsExt;

        with_tmp_cwd(|| unsafe {
            // Ensure the DB file exists first.
            let warm_json = CString::new(r#"{"application":"app","message":"warm"}"#).unwrap();
            assert!(witslog_log(warm_json.as_ptr()) >= 0);

            let db_path = resolve_db_path();
            let mut perms = std::fs::metadata(&db_path).unwrap().permissions();
            perms.set_mode(0o400);
            std::fs::set_permissions(&db_path, perms.clone()).unwrap();

            let dir_path = db_path.parent().unwrap();
            let mut dir_perms = std::fs::metadata(dir_path).unwrap().permissions();
            dir_perms.set_mode(0o500);
            std::fs::set_permissions(dir_path, dir_perms.clone()).unwrap();

            let log_json = CString::new(r#"{"application":"app","message":"blocked"}"#).unwrap();
            let result = witslog_log(log_json.as_ptr());

            // Restore perms so tempdir cleanup can proceed.
            dir_perms.set_mode(0o700);
            std::fs::set_permissions(dir_path, dir_perms).unwrap();
            perms.set_mode(0o600);
            std::fs::set_permissions(&db_path, perms).unwrap();

            assert_eq!(result, -1);
        });
    }
}
