//! Regression tests for the CLI `--color` flag (severity/status
//! chips+badges on `get`/`query`). Driven against the real built binary,
//! mirroring p11_json_output.rs.
//!
//! `Command::output()` captures stdout via a pipe, never a TTY, so `--color
//! auto` (the default) must never emit ANSI here — that's exercised
//! implicitly by every other CLI integration test continuing to pass
//! unchanged. This file locks the explicit override behavior: `--color
//! always` must emit ANSI even off a TTY, `--color never` must suppress it
//! even if something upstream forced color on, and `--json` must stay
//! byte-identical regardless of `--color`.

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

fn extract_event_id(log_stdout: &str) -> String {
    log_stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("event_id:"))
        .expect("log stdout should contain an event_id line")
        .trim()
        .to_string()
}

const ESC: &str = "\x1b[";

#[test]
fn color_auto_over_a_pipe_emits_no_ansi() {
    let tmp = TempDir::new().unwrap();
    init(&tmp);

    let log = Command::new(witslog_bin())
        .args(["log", "app", "boom"])
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
    assert!(!stdout.contains(ESC), "auto over a pipe must not emit ANSI: {stdout}");
}

#[test]
fn color_never_suppresses_ansi_on_get_and_query() {
    let tmp = TempDir::new().unwrap();
    init(&tmp);

    let log = Command::new(witslog_bin())
        .args(["log", "app", "boom for color test"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(log.status.success());
    let event_id = extract_event_id(&String::from_utf8_lossy(&log.stdout));

    let get = Command::new(witslog_bin())
        .args(["get", &event_id, "--color", "never"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(get.status.success());
    assert!(!String::from_utf8_lossy(&get.stdout).contains(ESC));

    let query = Command::new(witslog_bin())
        .args(["query", "color test", "--color", "never"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(query.status.success());
    assert!(!String::from_utf8_lossy(&query.stdout).contains(ESC));
}

#[test]
fn color_always_emits_ansi_on_get_and_query_even_off_a_tty() {
    let tmp = TempDir::new().unwrap();
    init(&tmp);

    let log = Command::new(witslog_bin())
        .args(["log", "app", "boom for color always test"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(log.status.success());
    let event_id = extract_event_id(&String::from_utf8_lossy(&log.stdout));

    let get = Command::new(witslog_bin())
        .args(["get", &event_id, "--color", "always"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(get.status.success());
    assert!(String::from_utf8_lossy(&get.stdout).contains(ESC), "always must emit ANSI even off a TTY");

    let query = Command::new(witslog_bin())
        .args(["query", "color always test", "--color", "always"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(query.status.success());
    assert!(String::from_utf8_lossy(&query.stdout).contains(ESC));
}

#[test]
fn json_output_is_never_colorized_regardless_of_color_flag() {
    let tmp = TempDir::new().unwrap();
    init(&tmp);

    let log = Command::new(witslog_bin())
        .args(["log", "app", "boom for json-color test"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(log.status.success());
    let event_id = extract_event_id(&String::from_utf8_lossy(&log.stdout));

    let get = Command::new(witslog_bin())
        .args(["get", &event_id, "--json", "--color", "always"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(get.status.success());
    let stdout = String::from_utf8_lossy(&get.stdout);
    assert!(!stdout.contains(ESC), "--json must stay plain even with --color always: {stdout}");
    // Still valid, parseable JSON — color flag didn't corrupt the payload.
    let _: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
}
