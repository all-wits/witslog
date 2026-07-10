use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

use serde::Deserialize;
use witslog_core::{EventBuilder, Severity};
use witslog_store::{DeleteFilter, Store};

fn resolve_db_path() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = witslog_config::Config::default_project();
    config.resolve_db_path(&cwd)
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

/// Log an event from a JSON payload. Returns the inserted row id, or -1 on error.
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

    let store = match Store::open_or_create(resolve_db_path()) {
        Ok(s) => s,
        Err(_) => return -1,
    };

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

    let event = builder.build();
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

    fn with_tmp_cwd<F: FnOnce()>(f: F) {
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
}
