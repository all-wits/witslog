use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use tempfile::TempDir;
use witslog_core::EventBuilder;
use witslog_store::{EventWriter, Store};

/// FR-P7-001: single-writer insert throughput. Target: >= 10k events/s on SSD.
fn write_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_throughput");

    group.bench_function("single_insert", |b| {
        b.iter_batched(
            || {
                let tmp = TempDir::new().unwrap();
                let store = Store::open_or_create(tmp.path().join("bench.db")).unwrap();
                (tmp, store)
            },
            |(_tmp, store)| {
                let writer = EventWriter::new(store.conn());
                let event = EventBuilder::new("bench-app", "bench message")
                    .category("application.error")
                    .build();
                writer.write(&event).unwrap();
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("batched_1000_in_one_transaction", |b| {
        b.iter_batched(
            || {
                let tmp = TempDir::new().unwrap();
                let store = Store::open_or_create(tmp.path().join("bench.db")).unwrap();
                let events: Vec<_> = (0..1000)
                    .map(|i| EventBuilder::new("bench-app", format!("bench message {i}")).build())
                    .collect();
                (tmp, store, events)
            },
            |(_tmp, store, events)| {
                let writer = EventWriter::new(store.conn());
                writer.write_batch(&events).unwrap();
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, write_throughput);
criterion_main!(benches);
