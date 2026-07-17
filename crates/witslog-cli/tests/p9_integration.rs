//! P9 extensibility/security feature + regression tests, driven against the
//! real built `witslog` binary: the audit hash chain (FR-P9-006/007) and
//! file-permission hardening (FR-P9-005).

use std::process::Command;
use tempfile::TempDir;

fn witslog_bin() -> &'static str {
    env!("CARGO_BIN_EXE_witslog")
}

#[test]
fn verify_audit_reports_clean_chain_after_logging() {
    let tmp = TempDir::new().unwrap();

    let init = Command::new(witslog_bin())
        .args(["init", "."])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(init.status.success());

    for i in 0..3 {
        let log = Command::new(witslog_bin())
            .args(["log", "app", &format!("event {i}")])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        assert!(log.status.success(), "stderr: {}", String::from_utf8_lossy(&log.stderr));
    }

    let doctor = Command::new(witslog_bin())
        .args(["doctor", "--verify-audit"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(doctor.status.success(), "stderr: {}", String::from_utf8_lossy(&doctor.stderr));
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("audit chain verified"), "stdout: {stdout}");
    assert!(stdout.contains("3 rows"), "stdout: {stdout}");
}

/// Regression: a row tampered with directly via SQL (bypassing the write
/// path) must be caught by `doctor --verify-audit`, naming the broken row.
#[test]
fn verify_audit_detects_tampering_and_exits_nonzero() {
    let tmp = TempDir::new().unwrap();

    let init = Command::new(witslog_bin())
        .args(["init", "."])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(init.status.success());

    let log = Command::new(witslog_bin())
        .args(["log", "app", "original message"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(log.status.success());

    let db_path = tmp.path().join(".witslog/witslog.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute("UPDATE events SET message = 'tampered!' WHERE id = 1", [])
        .unwrap();
    drop(conn);

    let doctor = Command::new(witslog_bin())
        .args(["doctor", "--verify-audit"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(!doctor.status.success(), "tampered chain must exit non-zero");
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("audit chain broken"), "stdout: {stdout}");
    assert!(stdout.contains("row id=1"), "stdout: {stdout}");
}

#[cfg(unix)]
#[test]
fn init_restricts_db_and_dir_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let init = Command::new(witslog_bin())
        .args(["init", "."])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(init.status.success());

    let dir_mode = std::fs::metadata(tmp.path().join(".witslog"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir_mode, 0o700);

    let db_mode = std::fs::metadata(tmp.path().join(".witslog/witslog.db"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(db_mode, 0o600);
}
