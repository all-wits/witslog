//! P8 packaging/install feature + regression tests: `--print-mcp-config`
//! (FR-P8-004) and the version-compatibility guard (FR-P8-007), driven
//! against the real built `witslog` binary.

use std::process::Command;
use tempfile::TempDir;

fn witslog_bin() -> &'static str {
    env!("CARGO_BIN_EXE_witslog")
}

#[test]
fn print_mcp_config_emits_valid_mcp_servers_snippet() {
    let tmp = TempDir::new().unwrap();

    let output = Command::new(witslog_bin())
        .args(["serve-mcp", "--print-mcp-config"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run witslog");

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("output should be valid JSON");

    let server = &json["mcpServers"]["witslog"];
    assert_eq!(server["args"], serde_json::json!(["serve-mcp", "--stdio"]));
    assert!(server["command"].as_str().unwrap().contains("witslog"));
    // Windows canonicalizes to a `\\?\`-prefixed path; just check the tail matches.
    let cwd = server["cwd"].as_str().unwrap();
    let tmp_name = tmp.path().file_name().unwrap().to_string_lossy();
    assert!(cwd.ends_with(tmp_name.as_ref()), "cwd {cwd} should end with {tmp_name}");
}

#[test]
fn print_mcp_config_does_not_require_an_initialized_db() {
    // A fresh, un-initialized dir: --print-mcp-config must still succeed
    // since it never opens a DB.
    let tmp = TempDir::new().unwrap();
    assert!(!tmp.path().join(".witslog").exists());

    let output = Command::new(witslog_bin())
        .args(["serve-mcp", "--print-mcp-config"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
}

#[test]
fn schema_newer_than_binary_is_refused_by_doctor() {
    let tmp = TempDir::new().unwrap();

    // Init a real project DB with this binary.
    let init = Command::new(witslog_bin())
        .args(["init", "."])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(init.status.success());

    let db_path = tmp.path().join(".witslog/witslog.db");

    // Simulate a DB stamped by a future binary (FR-P8-007).
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "UPDATE schema_meta SET value = '9999' WHERE key = 'schema_version'",
        [],
    )
    .unwrap();
    drop(conn);

    let log = Command::new(witslog_bin())
        .args(["log", "app", "boom"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(!log.status.success(), "log against a too-new schema must fail");
}

#[test]
fn schema_at_current_version_round_trips_normally() {
    let tmp = TempDir::new().unwrap();

    let init = Command::new(witslog_bin())
        .args(["init", "."])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(init.status.success());

    let log = Command::new(witslog_bin())
        .args(["log", "app", "all good"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(log.status.success(), "stderr: {}", String::from_utf8_lossy(&log.stderr));
}
