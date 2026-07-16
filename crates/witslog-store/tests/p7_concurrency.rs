use std::sync::Arc;
use std::thread;
use tempfile::TempDir;
use witslog_core::EventBuilder;
use witslog_store::{DbConnection, EventWriter, Store};

/// FR-P7-003: multiple writer connections against one project DB, serialized via
/// WAL + busy_timeout + SQLite's own bounded retry. Verifies zero corruption and
/// that every event persists (no silent loss under contention).
#[test]
fn concurrent_writers_no_corruption_no_loss() {
    const WRITERS: usize = 8;
    const EVENTS_PER_WRITER: usize = 200;

    let tmp = TempDir::new().unwrap();
    let db_path = Arc::new(tmp.path().join("concurrent.db"));

    // Create schema up front so every thread opens an already-migrated DB.
    Store::open_or_create(db_path.as_ref()).unwrap();

    let handles: Vec<_> = (0..WRITERS)
        .map(|w| {
            let db_path = Arc::clone(&db_path);
            thread::spawn(move || {
                let conn = DbConnection::open(db_path.as_ref()).expect("open own connection");
                let writer = EventWriter::new(&conn);
                for i in 0..EVENTS_PER_WRITER {
                    let event = EventBuilder::new("bench-app", format!("writer {w} event {i}"))
                        .build();
                    writer.write(&event).expect("write should succeed under contention");
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("writer thread should not panic");
    }

    let store = Store::open_or_create(db_path.as_ref()).unwrap();
    let conn = store.conn().conn();

    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        total,
        (WRITERS * EVENTS_PER_WRITER) as i64,
        "every event from every writer must persist, none lost"
    );

    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok", "DB must remain uncorrupted under concurrent writers");
}
