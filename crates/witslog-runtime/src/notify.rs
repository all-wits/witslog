//! P10c: builtin `Notifier` implementations wired into the runtime's write
//! path. Webhook/desktop notifiers are deliberately NOT offered here —
//! `witslog-runtime` links into `witslog-ffi`, which is `dlopen`'d into every
//! host process (Python/Node/PHP), so adding an HTTP client dependency there
//! was rejected. `witslog_plugin::Notifier` is already the extension point
//! for anyone who wants a webhook.

use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use witslog_plugin::Notifier;

/// Appends one NDJSON line per event. Microsecond-order cost, same order as
/// the SQLite insert that already happened — safe to dispatch synchronously.
pub struct FileNotifier {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileNotifier {
    pub fn new(path: PathBuf) -> Self {
        FileNotifier {
            path,
            lock: Mutex::new(()),
        }
    }
}

impl Notifier for FileNotifier {
    fn name(&self) -> &str {
        "file"
    }

    fn notify(&self, event: &JsonValue) -> Result<(), String> {
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| e.to_string())?;
        let mut line = serde_json::to_string(event).map_err(|e| e.to_string())?;
        line.push('\n');
        file.write_all(line.as_bytes()).map_err(|e| e.to_string())
    }
}

/// Wraps another `Notifier`, suppressing repeat calls for the same
/// `fingerprint` within `interval` — the difference between a notify feature
/// and a self-inflicted log flood on a hot recurring failure.
pub struct ThrottledNotifier {
    inner: Arc<dyn Notifier>,
    interval: Duration,
    last_fired: Mutex<HashMap<String, Instant>>,
}

impl ThrottledNotifier {
    pub fn new(inner: Arc<dyn Notifier>, interval: Duration) -> Self {
        ThrottledNotifier {
            inner,
            interval,
            last_fired: Mutex::new(HashMap::new()),
        }
    }
}

impl Notifier for ThrottledNotifier {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn notify(&self, event: &JsonValue) -> Result<(), String> {
        let fingerprint = event.get("fingerprint").and_then(|v| v.as_str()).unwrap_or("");
        {
            let mut last_fired = self.last_fired.lock().map_err(|e| e.to_string())?;
            if let Some(last) = last_fired.get(fingerprint) {
                if last.elapsed() < self.interval {
                    return Ok(());
                }
            }
            last_fired.insert(fingerprint.to_string(), Instant::now());
        }
        self.inner.notify(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_notifier_appends_ndjson_line() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let notifier = FileNotifier::new(path.clone());

        notifier.notify(&serde_json::json!({"message": "boom"})).unwrap();
        notifier.notify(&serde_json::json!({"message": "bang"})).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("boom"));
        assert!(lines[1].contains("bang"));
    }

    #[test]
    fn file_notifier_failure_is_reported_not_panicked() {
        // A directory as the "file" path can never be opened for append.
        let tmp = tempfile::TempDir::new().unwrap();
        let notifier = FileNotifier::new(tmp.path().to_path_buf());
        assert!(notifier.notify(&serde_json::json!({"message": "boom"})).is_err());
    }

    struct CountingNotifier {
        calls: Mutex<usize>,
    }
    impl Notifier for CountingNotifier {
        fn name(&self) -> &str {
            "counting"
        }
        fn notify(&self, _event: &JsonValue) -> Result<(), String> {
            *self.calls.lock().unwrap() += 1;
            Ok(())
        }
    }

    #[test]
    fn throttle_suppresses_repeat_within_interval() {
        let inner = Arc::new(CountingNotifier { calls: Mutex::new(0) });
        let throttled = ThrottledNotifier::new(inner.clone(), Duration::from_secs(3600));

        let event = serde_json::json!({"fingerprint": "fp-1"});
        throttled.notify(&event).unwrap();
        throttled.notify(&event).unwrap();
        throttled.notify(&event).unwrap();

        assert_eq!(*inner.calls.lock().unwrap(), 1);
    }

    #[test]
    fn throttle_does_not_suppress_different_fingerprints() {
        let inner = Arc::new(CountingNotifier { calls: Mutex::new(0) });
        let throttled = ThrottledNotifier::new(inner.clone(), Duration::from_secs(3600));

        throttled.notify(&serde_json::json!({"fingerprint": "fp-1"})).unwrap();
        throttled.notify(&serde_json::json!({"fingerprint": "fp-2"})).unwrap();

        assert_eq!(*inner.calls.lock().unwrap(), 2);
    }
}
