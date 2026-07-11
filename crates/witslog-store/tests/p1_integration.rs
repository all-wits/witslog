use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use witslog_core::{AsyncBuffer, BufferConfig, EnrichConfig, Event, EventBuilder, Redactor, Sink, SinkError};
use witslog_store::{EventWriter, Store, StoreSink};

fn init_git_repo(dir: &std::path::Path, sha: &str) {
    let git_dir = dir.join(".git");
    std::fs::create_dir_all(git_dir.join("refs/heads")).unwrap();
    std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(git_dir.join("refs/heads/main"), format!("{sha}\n")).unwrap();
}

/// git repo -> logged event has context.git_commit = HEAD short SHA and
/// hostname/pid/cwd populated (FR-P1-001, acceptance criterion 1).
#[test]
fn enrichment_populates_context_from_git_repo() {
    let tmpdir = TempDir::new().unwrap();
    init_git_repo(tmpdir.path(), "1234567890abcdef1234567890abcdef12345678");

    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(tmpdir.path()).unwrap();

    let db_path = tmpdir.path().join("witslog.db");
    let store = Store::open_or_create(&db_path).unwrap();

    let event = EventBuilder::new("app", "boom")
        .enrich(&EnrichConfig::default())
        .build();

    std::env::set_current_dir(&orig).unwrap();

    let ctx = event.context.clone().expect("context populated");
    assert_eq!(ctx["git_commit"], "1234567");
    assert!(ctx.get("pid").is_some());
    assert!(ctx.get("cwd").is_some());

    let writer = EventWriter::new(store.conn());
    writer.write(&event).unwrap();
    let retrieved = writer.query_by_id(&event.event_id).unwrap().unwrap();
    assert_eq!(retrieved.context.unwrap()["git_commit"], "1234567");
}

/// message with `Authorization: Bearer abc.def.ghi` -> stored message shows
/// `Authorization: Bearer «redacted»` (FR-P1-003, acceptance criterion 2).
#[test]
fn redaction_masks_bearer_token_before_persist() {
    let tmpdir = TempDir::new().unwrap();
    let db_path = tmpdir.path().join("witslog.db");
    let store = Store::open_or_create(&db_path).unwrap();

    let redactor = Redactor::built_in();
    let event = EventBuilder::new("app", "Authorization: Bearer abc.def.ghi")
        .redact(&redactor)
        .build();

    assert_eq!(event.message, "Authorization: Bearer «redacted»");

    let writer = EventWriter::new(store.conn());
    writer.write(&event).unwrap();
    let retrieved = writer.query_by_id(&event.event_id).unwrap().unwrap();
    assert_eq!(retrieved.message, "Authorization: Bearer «redacted»");
}

struct CountingSink {
    inner: StoreSink,
    batch_calls: Arc<Mutex<Vec<usize>>>,
}

impl Sink for CountingSink {
    fn write_batch(&self, events: &[Event]) -> Result<(), SinkError> {
        self.batch_calls.lock().unwrap().push(events.len());
        self.inner.write_batch(events)
    }
}

/// async buffering, batch_size 50 -> exactly one insert transaction runs for 50
/// events, and the caller never blocks (acceptance criterion 3).
#[test]
fn buffered_batch_of_50_writes_in_one_transaction() {
    let tmpdir = TempDir::new().unwrap();
    let db_path = tmpdir.path().join("witslog.db");
    let store = Store::open_or_create(&db_path).unwrap();

    let batch_calls = Arc::new(Mutex::new(Vec::new()));
    let sink = CountingSink {
        inner: StoreSink::new(Store::open_or_create(&db_path).unwrap()),
        batch_calls: Arc::clone(&batch_calls),
    };

    let cfg = BufferConfig {
        enabled: true,
        batch_size: 50,
        flush_interval_ms: 1000,
        queue_capacity: 1024,
    };
    let buffer = AsyncBuffer::new(sink, cfg);

    for i in 0..50 {
        buffer.enqueue(EventBuilder::new("app", format!("event {i}")).build());
    }

    std::thread::sleep(std::time::Duration::from_millis(500));

    let calls = batch_calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "exactly one batch flush expected");
    assert_eq!(calls[0], 50);
    drop(calls);

    let writer = EventWriter::new(store.conn());
    let count: i64 = store
        .conn()
        .conn()
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 50);
    let _ = writer; // keep writer alive for clarity; not otherwise used here
}

struct AlwaysFailSink;
impl Sink for AlwaysFailSink {
    fn write_batch(&self, _events: &[Event]) -> Result<(), SinkError> {
        Err(SinkError::Write("simulated failure".to_string()))
    }
}

/// A failing sink never panics the flush thread and drops+counts instead
/// (acceptance criterion 4, adapted from "DB path read-only").
#[test]
fn failing_writes_drop_and_count_without_panicking() {
    let cfg = BufferConfig {
        enabled: true,
        batch_size: 5,
        flush_interval_ms: 50,
        queue_capacity: 1024,
    };
    let buffer = AsyncBuffer::new(AlwaysFailSink, cfg);

    for i in 0..5 {
        buffer.enqueue(EventBuilder::new("app", format!("event {i}")).build());
    }

    std::thread::sleep(std::time::Duration::from_millis(300));

    assert_eq!(buffer.dropped_count(), 5);
}

/// `witslog.exception(e)` with a traceback -> stacktrace stored and stack_norm
/// has digits masked (acceptance criterion 5).
#[test]
fn exception_constructor_stores_stacktrace_with_masked_digits() {
    let tmpdir = TempDir::new().unwrap();
    let db_path = tmpdir.path().join("witslog.db");
    let store = Store::open_or_create(&db_path).unwrap();

    #[derive(Debug)]
    struct Cause;
    impl std::fmt::Display for Cause {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at line 42 in module.rs")
        }
    }
    impl std::error::Error for Cause {}

    #[derive(Debug)]
    struct Top;
    impl std::fmt::Display for Top {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "operation failed")
        }
    }
    impl std::error::Error for Top {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&Cause)
        }
    }

    let err = Top;
    let event = witslog_core::exception("app", &err).build();

    assert_eq!(event.message, "operation failed");
    let stack_norm = event.stack_norm.clone().expect("stack_norm present");
    assert!(!stack_norm.contains("42"), "digits should be masked: {stack_norm}");
    assert!(stack_norm.contains("N"), "masked digits use 'N': {stack_norm}");

    let writer = EventWriter::new(store.conn());
    writer.write(&event).unwrap();
    let retrieved = writer.query_by_id(&event.event_id).unwrap().unwrap();
    assert!(retrieved.stacktrace.unwrap().contains("42"));
    assert!(!retrieved.stack_norm.unwrap().contains("42"));
}

/// Dropped-event counter is durable across `Store` opens (so `doctor` in another
/// process/invocation can see it).
#[test]
fn dropped_counter_persists_and_is_readable_via_writer() {
    let tmpdir = TempDir::new().unwrap();
    let db_path = tmpdir.path().join("witslog.db");
    let store = Store::open_or_create(&db_path).unwrap();
    let writer = EventWriter::new(store.conn());

    assert_eq!(writer.dropped_count().unwrap(), 0);
    writer.bump_dropped(3).unwrap();
    assert_eq!(writer.dropped_count().unwrap(), 3);

    drop(store);
    let store2 = Store::open_or_create(&db_path).unwrap();
    let writer2 = EventWriter::new(store2.conn());
    assert_eq!(writer2.dropped_count().unwrap(), 3);
}
