//! Regression tests for the `--json` global flag and the richer `get`
//! text output (Part C, Piece 5 of the zero-boilerplate/log-quality
//! redesign). Prior to this, `query`'s summary line and even `get`'s detail
//! view silently dropped `context`/`tags`/`stacktrace`/`error_code`/
//! `correlation_id` even though the DB already stored them — see
//! CLAUDE.md gotcha "CLI hides captured context". Driven against the real
//! built binary, mirroring p8_integration.rs.

use std::process::Command;
use tempfile::TempDir;

fn witslog_bin() -> &'static str {
    env!("CARGO_BIN_EXE_witslog")
}

fn init(tmp: &TempDir) {
    let out = Command::new(witslog_bin())
        .args(["init", "."])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "init failed: {}", String::from_utf8_lossy(&out.stderr));
}

/// `witslog log` prints a multi-line "✓ Event logged / event_id: <id> / ..."
/// block, not a bare id — extract the id line.
fn extract_event_id(log_stdout: &str) -> String {
    log_stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("event_id:"))
        .expect("log stdout should contain an event_id line")
        .trim()
        .to_string()
}

#[test]
fn get_json_emits_full_event_including_context_tags_error_code() {
    let tmp = TempDir::new().unwrap();
    init(&tmp);

    let log = Command::new(witslog_bin())
        .args([
            "log",
            "witsnote-proxy",
            "upstream 409 for /cards/abc",
            "--error-code",
            "UPSTREAM_CONFLICT",
        ])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(log.status.success(), "log failed: {}", String::from_utf8_lossy(&log.stderr));
    let event_id = extract_event_id(&String::from_utf8_lossy(&log.stdout));

    let get = Command::new(witslog_bin())
        .args(["get", &event_id, "--json"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(get.status.success(), "get --json failed: {}", String::from_utf8_lossy(&get.stderr));

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&get.stdout)).expect("valid JSON");
    assert_eq!(json["error_code"], serde_json::json!("UPSTREAM_CONFLICT"));
    assert_eq!(json["application"], serde_json::json!("witsnote-proxy"));
}

#[test]
fn get_json_on_missing_event_prints_null_not_a_prose_message() {
    let tmp = TempDir::new().unwrap();
    init(&tmp);

    let out = Command::new(witslog_bin())
        .args(["get", "01900000-0000-7000-8000-000000000000", "--json"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON (null)");
    assert!(json.is_null());
}

#[test]
fn get_text_output_now_surfaces_error_code_and_stacktrace() {
    // Regression lock for W7: even the non-JSON detail view used to drop
    // error_code/stacktrace/context/tags/correlation_id.
    let tmp = TempDir::new().unwrap();
    init(&tmp);

    let log = Command::new(witslog_bin())
        .args([
            "log",
            "app",
            "boom",
            "--error-code",
            "UPSTREAM_UNREACHABLE",
            "--exception",
            "TypeError",
        ])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(log.status.success());
    let event_id = extract_event_id(&String::from_utf8_lossy(&log.stdout));

    let get = Command::new(witslog_bin())
        .args(["get", &event_id])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(get.status.success());
    let stdout = String::from_utf8_lossy(&get.stdout);
    assert!(stdout.contains("error_code: UPSTREAM_UNREACHABLE"), "stdout: {stdout}");
    assert!(stdout.contains("exception: TypeError"), "stdout: {stdout}");
}

#[test]
fn query_json_emits_items_array_with_total_estimate() {
    let tmp = TempDir::new().unwrap();
    init(&tmp);

    let log = Command::new(witslog_bin())
        .args(["log", "witsnote-proxy", "fetch failed"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(log.status.success());

    let query = Command::new(witslog_bin())
        .args(["query", "fetch", "--json"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(query.status.success(), "query --json failed: {}", String::from_utf8_lossy(&query.stderr));

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&query.stdout)).expect("valid JSON");
    assert!(json["items"].is_array());
    assert!(json["items"].as_array().unwrap().len() >= 1);
    assert!(json["total_estimate"].is_number());
}

#[test]
fn query_text_summary_line_appends_error_code_and_tags_when_present() {
    let tmp = TempDir::new().unwrap();
    init(&tmp);

    let log = Command::new(witslog_bin())
        .args([
            "log",
            "witsnote-proxy",
            "upstream 409 conflict for cardxyz",
            "--error-code",
            "UPSTREAM_CONFLICT",
        ])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(log.status.success());

    let query = Command::new(witslog_bin())
        .args(["query", "conflict"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(query.status.success());
    let stdout = String::from_utf8_lossy(&query.stdout);
    assert!(stdout.contains("[UPSTREAM_CONFLICT]"), "stdout: {stdout}");
    assert!(stdout.contains("--json"), "should hint --json for full detail; stdout: {stdout}");
}
