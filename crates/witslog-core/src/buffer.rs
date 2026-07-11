use crate::event::Event;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    #[error("sink write failed: {0}")]
    Write(String),
}

/// Destination for a batch of buffered events. Implemented by `witslog-store`'s
/// `StoreSink`; kept here as a trait so `witslog-core` stays storage-agnostic.
pub trait Sink: Send + Sync {
    fn write_batch(&self, events: &[Event]) -> Result<(), SinkError>;
}

/// Writes synchronously, one event at a time — used when buffering is disabled.
pub struct SyncSink<S: Sink> {
    inner: S,
}

impl<S: Sink> SyncSink<S> {
    pub fn new(inner: S) -> Self {
        SyncSink { inner }
    }

    pub fn write(&self, event: Event) -> Result<(), SinkError> {
        self.inner.write_batch(std::slice::from_ref(&event))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BufferConfig {
    pub enabled: bool,
    pub batch_size: usize,
    pub flush_interval_ms: u64,
    pub queue_capacity: usize,
}

impl Default for BufferConfig {
    fn default() -> Self {
        BufferConfig {
            enabled: false,
            batch_size: 50,
            flush_interval_ms: 1000,
            queue_capacity: 1024,
        }
    }
}

/// Enqueues events off the caller's thread and flushes them in batches on a
/// background thread. `enqueue` never blocks the caller and never panics: a full
/// queue or a disconnected flush thread just increments the dropped counter.
pub struct AsyncBuffer<S: Sink + 'static> {
    sender: SyncSender<Event>,
    dropped: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    _sink: std::marker::PhantomData<S>,
}

impl<S: Sink + 'static> AsyncBuffer<S> {
    pub fn new(sink: S, cfg: BufferConfig) -> Self {
        let (sender, receiver) = sync_channel::<Event>(cfg.queue_capacity.max(1));
        let dropped = Arc::new(AtomicU64::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));

        let flush_dropped = Arc::clone(&dropped);
        let flush_shutdown = Arc::clone(&shutdown);
        let batch_size = cfg.batch_size.max(1);
        let flush_interval = Duration::from_millis(cfg.flush_interval_ms.max(1));

        let handle = std::thread::spawn(move || {
            let sink = sink;
            let mut batch: Vec<Event> = Vec::with_capacity(batch_size);

            loop {
                match receiver.recv_timeout(flush_interval) {
                    Ok(event) => {
                        batch.push(event);
                        if batch.len() >= batch_size {
                            flush_batch(&sink, &mut batch, &flush_dropped);
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if !batch.is_empty() {
                            flush_batch(&sink, &mut batch, &flush_dropped);
                        }
                        if flush_shutdown.load(Ordering::Acquire) {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        // Drain any remaining queued events before exiting.
                        while let Ok(event) = receiver.try_recv() {
                            batch.push(event);
                        }
                        if !batch.is_empty() {
                            flush_batch(&sink, &mut batch, &flush_dropped);
                        }
                        break;
                    }
                }
            }
        });

        AsyncBuffer {
            sender,
            dropped,
            shutdown,
            handle: Some(handle),
            _sink: std::marker::PhantomData,
        }
    }

    /// Enqueue an event for background flush. Never blocks; on a full queue the
    /// event is dropped and the dropped counter incremented.
    pub fn enqueue(&self, event: Event) {
        if self.sender.try_send(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl<S: Sink + 'static> Drop for AsyncBuffer<S> {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Writes `batch` via `sink`, retrying once on failure before dropping it.
/// A panicking `Sink` is caught so the flush thread never dies silently.
fn flush_batch<S: Sink>(sink: &S, batch: &mut Vec<Event>, dropped: &Arc<AtomicU64>) {
    if batch.is_empty() {
        return;
    }

    let attempt = |b: &[Event]| -> bool {
        catch_unwind(AssertUnwindSafe(|| sink.write_batch(b)))
            .map(|r| r.is_ok())
            .unwrap_or(false)
    };

    if !attempt(batch) && !attempt(batch) {
        dropped.fetch_add(batch.len() as u64, Ordering::Relaxed);
    }

    batch.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MockSink {
        calls: Arc<Mutex<Vec<usize>>>,
        fail_times: Arc<AtomicU64>,
    }

    impl Sink for MockSink {
        fn write_batch(&self, events: &[Event]) -> Result<(), SinkError> {
            if self.fail_times.load(Ordering::Relaxed) > 0 {
                self.fail_times.fetch_sub(1, Ordering::Relaxed);
                return Err(SinkError::Write("mock failure".to_string()));
            }
            self.calls.lock().unwrap().push(events.len());
            Ok(())
        }
    }

    fn sample_event() -> Event {
        crate::event::EventBuilder::new("app", "msg").build()
    }

    #[test]
    fn batches_exactly_at_batch_size() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let sink = MockSink {
            calls: Arc::clone(&calls),
            fail_times: Arc::new(AtomicU64::new(0)),
        };
        let cfg = BufferConfig {
            enabled: true,
            batch_size: 50,
            flush_interval_ms: 50,
            queue_capacity: 1024,
        };
        let buffer = AsyncBuffer::new(sink, cfg);

        for _ in 0..50 {
            buffer.enqueue(sample_event());
        }

        // Give the flush thread time to process the full batch.
        std::thread::sleep(Duration::from_millis(300));

        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0], 50);
    }

    #[test]
    fn drop_flushes_partial_batch() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let sink = MockSink {
            calls: Arc::clone(&calls),
            fail_times: Arc::new(AtomicU64::new(0)),
        };
        let cfg = BufferConfig {
            enabled: true,
            batch_size: 50,
            flush_interval_ms: 5000,
            queue_capacity: 1024,
        };
        let buffer = AsyncBuffer::new(sink, cfg);

        for _ in 0..5 {
            buffer.enqueue(sample_event());
        }
        drop(buffer);

        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.iter().sum::<usize>(), 5);
    }

    #[test]
    fn failing_sink_drops_and_counts_without_panic() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let sink = MockSink {
            calls: Arc::clone(&calls),
            fail_times: Arc::new(AtomicU64::new(2)), // fails both attempts
        };
        let cfg = BufferConfig {
            enabled: true,
            batch_size: 3,
            flush_interval_ms: 50,
            queue_capacity: 1024,
        };
        let buffer = AsyncBuffer::new(sink, cfg);

        for _ in 0..3 {
            buffer.enqueue(sample_event());
        }
        std::thread::sleep(Duration::from_millis(300));

        assert_eq!(buffer.dropped_count(), 3);
        assert!(calls.lock().unwrap().is_empty());
    }
}
