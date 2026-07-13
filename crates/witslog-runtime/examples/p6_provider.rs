//! P6 provider demo: mount witslog once, then let a panic and an ambient
//! `error!` be captured with zero per-callsite ceremony.
//!
//! Run: `cargo run -p witslog-runtime --example p6_provider`

use witslog_config::Config;
use witslog_store::Store;

fn main() {
    // Self-contained DB so the demo runs from anywhere.
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("witslog.db");
    let mut cfg = Config::default_project();
    cfg.db_path = Some(db.clone());

    // Mount once at the entrypoint — the "Provider".
    witslog_runtime::arm(cfg);

    // Ambient one-liner (no builder chain, no store wiring).
    witslog_runtime::error!("demo", "handled error: {}", 42);

    // A panic anywhere in the program is auto-captured as Fatal.
    let _ = std::panic::catch_unwind(|| panic!("unexpected reactor meltdown"));

    // Show what landed.
    let store = Store::open_or_create(&db).unwrap();
    let db_conn = store.conn();
    let conn = db_conn.conn();
    let mut stmt = conn
        .prepare("SELECT severity, message FROM events ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .unwrap();

    println!("\ncaptured events:");
    for row in rows {
        let (severity, message) = row.unwrap();
        println!("  [{severity}] {message}");
    }
}
