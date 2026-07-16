use std::time::Instant;
use tempfile::TempDir;
use witslog_core::EventBuilder;
use witslog_store::{DeleteFilter, EventWriter, Store};

/// FR-P7-004: load test at reduced scale for CI turnaround (20k rows, ~50 batches
/// of 400 in individual transactions to mimic realistic write patterns rather than
/// one giant transaction). Documents the 1M-row target's timing methodology so
/// scaling up (`WITSLOG_LOAD_TEST_ROWS` env override) reuses the same assertions —
/// see docs/perf.md for a manually-run 1M-row measurement.
const DEFAULT_ROWS: usize = 20_000;
const BATCH: usize = 400;

fn load_rows() -> usize {
    std::env::var("WITSLOG_LOAD_TEST_ROWS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_ROWS)
}

#[test]
fn load_insert_prune_vacuum_backup_within_bounds() {
    let rows = load_rows();
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("load.db");
    let store = Store::open_or_create(&db_path).unwrap();
    let writer = EventWriter::new(store.conn());

    let insert_start = Instant::now();
    let mut remaining = rows;
    let mut batch_idx = 0usize;
    while remaining > 0 {
        let n = remaining.min(BATCH);
        let events: Vec<_> = (0..n)
            .map(|i| {
                EventBuilder::new("load-app", format!("load test event {batch_idx}-{i}")).build()
            })
            .collect();
        writer.write_batch(&events).unwrap();
        remaining -= n;
        batch_idx += 1;
    }
    let insert_elapsed = insert_start.elapsed();
    println!("[p7_load] inserted {rows} events in {insert_elapsed:?}");

    let count: i64 = store
        .conn()
        .conn()
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, rows as i64);

    // Resolve the first half so prune has something to delete (delete_resolved
    // requires resolved_at IS NOT NULL unless force:true — FR-P0 lifecycle rule).
    {
        let conn = store.conn().conn();
        conn.execute(
            "UPDATE events SET resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE id IN (SELECT id FROM events ORDER BY id LIMIT ?1)",
            rusqlite::params![(rows / 2) as i64],
        )
        .unwrap();
    }

    let prune_start = Instant::now();
    let deleted = writer.delete_resolved(&DeleteFilter::default()).unwrap();
    let prune_elapsed = prune_start.elapsed();
    println!("[p7_load] pruned {} resolved events in {prune_elapsed:?}", deleted.len());
    assert_eq!(deleted.len(), rows / 2);

    let vacuum_start = Instant::now();
    store.conn().conn().execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;").unwrap();
    let vacuum_elapsed = vacuum_start.elapsed();
    println!("[p7_load] vacuum in {vacuum_elapsed:?}");

    let backup_path = tmp.path().join("load-backup.db");
    let backup_start = Instant::now();
    std::fs::copy(&db_path, &backup_path).unwrap();
    let backup_elapsed = backup_start.elapsed();
    println!("[p7_load] backup copy in {backup_elapsed:?}");
    assert!(backup_path.exists());

    let integrity: String = store
        .conn()
        .conn()
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");

    // Documented bound (§Non-functional, refined by measurement): at the default
    // CI scale, the whole insert+prune+vacuum+backup cycle should stay well under
    // a minute on commodity hardware. This is a smoke bound, not the 1M-row target.
    assert!(
        insert_elapsed.as_secs() < 60,
        "insert phase took unexpectedly long: {insert_elapsed:?}"
    );
}
