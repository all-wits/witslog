use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use rusqlite::Connection;
use tempfile::TempDir;
use witslog_core::EventBuilder;
use witslog_store::{EventWriter, Store};

const ROWS: usize = 2_000;

/// FR-P3-009: cost of backfilling `events_fts` from existing `events` rows
/// (the `migrate_0005_fts5` path), isolated from ordinary insert cost.
fn index_build_cost(c: &mut Criterion) {
    c.bench_function("fts5_backfill_2000_rows", |b| {
        b.iter_batched(
            || {
                let tmp = TempDir::new().unwrap();
                let db_path = tmp.path().join("bench.db");
                let store = Store::open_or_create(&db_path).unwrap();
                let writer = EventWriter::new(store.conn());
                let events: Vec<_> = (0..ROWS)
                    .map(|i| EventBuilder::new("bench-app", format!("index cost message {i}")).build())
                    .collect();
                writer.write_batch(&events).unwrap();

                // Clear the FTS shadow index to force a real backfill. A plain
                // `DELETE FROM events_fts` errors on this external-content table
                // (tags_text isn't a real `events` column, so FTS5 can't resolve
                // it for row-level delete) - the special 'delete-all' command is
                // the supported way to empty an external-content FTS5 index.
                store
                    .conn()
                    .conn()
                    .execute("INSERT INTO events_fts(events_fts) VALUES('delete-all')", [])
                    .unwrap();
                drop(store);
                (tmp, db_path)
            },
            |(_tmp, db_path)| {
                let conn = Connection::open(&db_path).unwrap();
                // Mirrors migrate_0005_fts5's own backfill exactly (migrate.rs) -
                // a plain SELECT without the LEFT JOIN json_each(...)/GROUP BY
                // fails FTS5's external-content column resolution.
                conn.execute_batch(
                    r#"
                    INSERT INTO events_fts(rowid,message,exception,stack_norm,root_cause,tags_text,category)
                    SELECT events.id,events.message,events.exception,events.stack_norm,events.root_cause,
                           COALESCE(group_concat(value,' '),''),events.category
                    FROM events
                    LEFT JOIN json_each(events.tags)
                    GROUP BY events.id;
                    "#,
                )
                .unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, index_build_cost);
criterion_main!(benches);
