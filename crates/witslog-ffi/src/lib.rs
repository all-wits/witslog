use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Deserialize;
use witslog_core::{AsyncBuffer, BufferConfig, EnrichConfig, EventBuilder, Redactor, Severity};
use witslog_store::{DeleteFilter, Store, StoreSink};

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
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        RuntimeConfig {
            enrich: EnrichConfig::default(),
            redactor: Arc::new(Redactor::built_in()),
            buffer: BufferConfig::default(),
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
struct ConfigureRequest {
    enrich: Option<EnrichConfigDto>,
    redact: Option<RedactConfigDto>,
    buffer: Option<BufferConfigDto>,
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

    let event = builder
        .enrich(&cfg.enrich)
        .redact(cfg.redactor.as_ref())
        .build();

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

    match writer.mark_resolved(&event_id) {
        Ok(()) => 0,
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
