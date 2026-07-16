use criterion::{criterion_group, criterion_main, Criterion};
use tempfile::TempDir;
use witslog_core::{EventBuilder, Severity};
use witslog_query::{Filters, SearchEngine};
use witslog_store::{EventWriter, Store};

const SEED_COUNT: usize = 5_000;

fn seed_store() -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let store = Store::open_or_create(tmp.path().join("bench.db")).unwrap();
    let writer = EventWriter::new(store.conn());

    let messages = [
        "connection refused ECONNREFUSED",
        "dns lookup failed ETIMEDOUT",
        "null pointer dereference",
        "out of memory killed process",
        "permission denied writing file",
    ];

    let events: Vec<_> = (0..SEED_COUNT)
        .map(|i| {
            EventBuilder::new("bench-app", messages[i % messages.len()])
                .severity(Severity::Error)
                .category("application.error")
                .build()
        })
        .collect();
    writer.write_batch(&events).unwrap();

    (tmp, store)
}

/// FR-P3: search on a seeded corpus. Target (§Non-functional): first page < 50ms at 100k rows;
/// this bench runs at SEED_COUNT for CI speed — see docs/perf.md for the 100k-row measurement.
fn search_latency(c: &mut Criterion) {
    let (_tmp, store) = seed_store();
    let conn = store.conn().conn();
    let engine = SearchEngine::new(&conn);
    let filters = Filters::default();

    c.bench_function("fts_search_first_page", |b| {
        b.iter(|| {
            engine
                .search("econnrefused*", &filters, 20, None, true)
                .unwrap()
        });
    });
}

criterion_group!(benches, search_latency);
criterion_main!(benches);
