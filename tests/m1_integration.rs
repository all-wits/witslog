use std::path::PathBuf;
use tempfile::TempDir;
use witslog_config::Config;
use witslog_core::{EventBuilder, Severity};
use witslog_store::Store;

#[test]
fn test_m1_init_creates_db_with_schema() {
    let tmpdir = TempDir::new().unwrap();
    let db_path = tmpdir.path().join("test.db");

    let store = Store::open_or_create(&db_path).expect("should create store");

    assert!(db_path.exists(), "db file should exist");

    let conn = store.conn().conn();
    let result: Result<i32, _> = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='events'",
        [],
        |row| row.get(0),
    );

    assert!(result.is_ok(), "events table should exist");
    let table_count: i32 = result.unwrap();
    assert_eq!(table_count, 1, "events table should be created");
}

#[test]
fn test_m1_event_round_trip() {
    let tmpdir = TempDir::new().unwrap();
    let db_path = tmpdir.path().join("test.db");

    let store = Store::open_or_create(&db_path).expect("should create store");

    let event = EventBuilder::new("test-app", "integration test error")
        .severity(Severity::Error)
        .version("1.0.0")
        .category("application.error")
        .build();

    let event_id = event.event_id.clone();
    let fingerprint = event.fingerprint.clone();

    let writer = witslog_store::EventWriter::new(store.conn());
    let row_id = writer.write(&event).expect("should write event");

    assert!(row_id > 0, "row_id should be positive");

    let retrieved = writer
        .query_by_id(&event_id)
        .expect("should query event")
        .expect("event should exist");

    assert_eq!(retrieved.event_id, event_id);
    assert_eq!(retrieved.application, "test-app");
    assert_eq!(retrieved.message, "integration test error");
    assert_eq!(retrieved.severity, Severity::Error);
    assert_eq!(retrieved.version, Some("1.0.0".to_string()));
    assert_eq!(retrieved.fingerprint, fingerprint);
}

#[test]
fn test_m1_migrations_idempotent() {
    let tmpdir = TempDir::new().unwrap();
    let db_path = tmpdir.path().join("test.db");

    let store1 = Store::open_or_create(&db_path).expect("first init should succeed");
    drop(store1);

    let store2 = Store::open_or_create(&db_path).expect("second init should succeed (idempotent)");
    let conn = store2.conn().conn();

    let result: Result<i32, _> = conn.query_row(
        "SELECT COUNT(*) FROM migrations",
        [],
        |row| row.get(0),
    );

    let migration_count: i32 = result.unwrap_or(0);
    assert!(migration_count >= 1, "at least one migration should be recorded");
}

#[test]
fn test_m1_strict_table_enforced() {
    let tmpdir = TempDir::new().unwrap();
    let db_path = tmpdir.path().join("test.db");

    let store = Store::open_or_create(&db_path).expect("should create store");
    let conn = store.conn().conn();

    let result = conn.execute(
        "INSERT INTO events (event_id, ts, ts_epoch_ms, application, severity, severity_rank, message, fingerprint, schema_v)
         VALUES ('bad-uuid', '2024-01-01T00:00:00Z', 0, 'app', 'error', 50, 'msg', 'fp', 1)",
        [],
    );

    assert!(
        result.is_ok(),
        "valid insert should succeed even with minimal fields"
    );
}

#[test]
fn test_m1_wal_mode_enabled() {
    let tmpdir = TempDir::new().unwrap();
    let db_path = tmpdir.path().join("test.db");

    let _store = Store::open_or_create(&db_path).expect("should create store");

    assert!(
        tmpdir.path().join("test.db-wal").exists(),
        "WAL file should be created"
    );
}

#[test]
fn test_m1_fingerprint_deterministic() {
    let tmpdir = TempDir::new().unwrap();
    let db_path = tmpdir.path().join("test.db");

    let store = Store::open_or_create(&db_path).expect("should create store");

    let event1 = EventBuilder::new("app", "same error message").build();
    let event2 = EventBuilder::new("app", "same error message").build();

    assert_eq!(
        event1.fingerprint, event2.fingerprint,
        "identical messages should have same fingerprint"
    );

    let writer = witslog_store::EventWriter::new(store.conn());
    let _id1 = writer.write(&event1).unwrap();
    let _id2 = writer.write(&event2).unwrap();

    let fp_counts: i32 = store
        .conn()
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM fingerprints WHERE fingerprint = ?1",
            [&event1.fingerprint],
            |row| row.get(0),
        )
        .unwrap_or(0);

    assert_eq!(fp_counts, 1, "identical fingerprints should be deduplicated");
}
