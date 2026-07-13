//! A `tracing_subscriber::Layer` that siphons `error!`/`warn!` events emitted
//! anywhere in the app into witslog. Rust-only (behind the `tracing` feature);
//! this is the piece that does **not** cross the C ABI.
//!
//! ```no_run
//! # #[cfg(feature = "tracing")]
//! # {
//! use tracing_subscriber::prelude::*;
//! let _guard = witslog_runtime::init_default();
//! tracing_subscriber::registry()
//!     .with(witslog_runtime::WitslogLayer::new("my-app"))
//!     .init();
//! tracing::error!("db connection failed"); // captured by witslog
//! # }
//! ```

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use witslog_core::{error as build_error, warn as build_warn, Severity};

/// Captures `tracing` events at or above WARN into the mounted witslog runtime.
pub struct WitslogLayer {
    app: String,
}

impl WitslogLayer {
    /// Create a layer that tags captured events with `app` as the application
    /// name. Only WARN and ERROR events are captured; lower levels are ignored.
    pub fn new(app: impl Into<String>) -> Self {
        WitslogLayer { app: app.into() }
    }
}

impl<S: Subscriber> Layer<S> for WitslogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let level = *event.metadata().level();

        // `tracing::Level` orders ERROR < WARN < INFO < DEBUG < TRACE, so
        // "at or above WARN in severity" is `level <= WARN`.
        if level > Level::WARN {
            return;
        }

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let message = visitor
            .message
            .unwrap_or_else(|| event.metadata().name().to_string());

        let builder = match level {
            Level::ERROR => build_error(&self.app, message).severity(Severity::Error),
            _ => build_warn(&self.app, message).severity(Severity::Warn),
        };
        let builder = builder.subsystem(event.metadata().target());

        let _ = crate::capture(builder);
    }
}

/// Extracts the `message` field (the positional format string of a
/// `tracing::error!(...)` call) from an event's fields.
#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" && self.message.is_none() {
            self.message = Some(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }
}
