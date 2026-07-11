use crate::writer::write_event;
use crate::Store;
use witslog_core::{Event, Sink, SinkError};

/// Adapts a project DB to `witslog-core`'s `Sink` trait so an `AsyncBuffer` can
/// flush batches into it — one transaction per batch via `DbConnection::transaction`.
/// Owns the `Store` (rather than borrowing) so it can be moved into the buffer's
/// background thread, which requires `Sink + 'static`.
pub struct StoreSink {
    store: Store,
}

impl StoreSink {
    pub fn new(store: Store) -> Self {
        StoreSink { store }
    }
}

impl Sink for StoreSink {
    fn write_batch(&self, events: &[Event]) -> Result<(), SinkError> {
        self.store
            .conn()
            .transaction(|tx| {
                for event in events {
                    write_event(tx, event)?;
                }
                Ok(())
            })
            .map_err(|e| SinkError::Write(e.to_string()))
    }
}
