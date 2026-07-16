use criterion::{criterion_group, criterion_main, Criterion};
use tempfile::TempDir;
use witslog_core::{AsyncBuffer, BufferConfig, EventBuilder};
use witslog_store::{Store, StoreSink};

/// FR-P1-005 / FR-P7-001: caller-thread cost of `enqueue()` on a buffered sink.
/// Target: < 100 µs. This measures only the enqueue call, not the background flush.
fn buffered_log_latency(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    let store = Store::open_or_create(tmp.path().join("bench.db")).unwrap();
    let sink = StoreSink::new(store);
    let cfg = BufferConfig {
        enabled: true,
        batch_size: 200,
        flush_interval_ms: 60_000,
        queue_capacity: 1_000_000,
    };
    let buffer = AsyncBuffer::new(sink, cfg);

    c.bench_function("buffered_enqueue", |b| {
        b.iter(|| {
            let event = EventBuilder::new("bench-app", "buffered bench message").build();
            buffer.enqueue(event);
        });
    });

    drop(buffer);
}

criterion_group!(benches, buffered_log_latency);
criterion_main!(benches);
