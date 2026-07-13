//! P6 provider/runtime integration tests: ambient capture, panic hook, and the
//! `Result::log_err` boundary trait all land events in the DB.

use std::path::Path;
use std::sync::Mutex;

use witslog_config::Config;
use witslog_core::error;
use witslog_runtime::LogErr;
use witslog_store::Store;

// `arm()` and the panic hook mutate process-global state; serialize the tests
// that depend on which DB the global runtime points at.
static LOCK: Mutex<()> = Mutex::new(());

fn temp_config() -> (tempfile::TempDir, Config, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("witslog.db");
    let mut cfg = Config::default_project();
    cfg.db_path = Some(db.clone());
    (dir, cfg, db)
}

fn count_events(db: &Path) -> i64 {
    let store = Store::open_or_create(db).unwrap();
    let n = store
        .conn()
        .conn()
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap();
    n
}

fn one_column(db: &Path, col: &str) -> String {
    let store = Store::open_or_create(db).unwrap();
    let value = store
        .conn()
        .conn()
        .query_row(&format!("SELECT {col} FROM events LIMIT 1"), [], |r| r.get(0))
        .unwrap();
    value
}

#[test]
fn capture_writes_and_fingerprints() {
    let _g = LOCK.lock().unwrap();
    let (_dir, cfg, db) = temp_config();
    witslog_runtime::arm(cfg);

    let row_id = witslog_runtime::capture(error("app", "boom")).expect("captured");
    assert!(row_id > 0);
    assert_eq!(count_events(&db), 1);

    let fingerprint = one_column(&db, "fingerprint");
    assert!(!fingerprint.is_empty());
    let message = one_column(&db, "message");
    assert_eq!(message, "boom");
}

#[test]
fn log_err_captures_error_chain_and_passes_through() {
    let _g = LOCK.lock().unwrap();
    let (_dir, cfg, db) = temp_config();
    witslog_runtime::arm(cfg);

    let result: Result<(), std::io::Error> =
        Err(std::io::Error::new(std::io::ErrorKind::Other, "disk gone"));
    let passed = result.log_err("app");

    assert!(passed.is_err(), "log_err returns the Result unchanged");
    assert_eq!(count_events(&db), 1);
    let exception = one_column(&db, "exception");
    assert!(exception.contains("disk gone"));
}

#[test]
fn panic_is_captured_as_fatal() {
    let _g = LOCK.lock().unwrap();
    let (_dir, cfg, db) = temp_config();
    witslog_runtime::arm(cfg);

    // The installed hook fires even though catch_unwind swallows the unwind.
    let outcome = std::panic::catch_unwind(|| panic!("kaboom in the reactor"));
    assert!(outcome.is_err());

    assert_eq!(count_events(&db), 1);
    assert_eq!(one_column(&db, "severity"), "fatal");
    let message = one_column(&db, "message");
    assert!(message.contains("kaboom"));
    assert_eq!(one_column(&db, "error_code"), "panic");
}
