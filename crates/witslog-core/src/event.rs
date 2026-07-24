use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Trace = 10,
    Debug = 20,
    Info = 30,
    Warn = 40,
    Error = 50,
    Critical = 60,
    Fatal = 70,
}

impl Severity {
    pub fn rank(&self) -> i32 {
        *self as i32
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Trace => "trace",
            Severity::Debug => "debug",
            Severity::Info => "info",
            Severity::Warn => "warn",
            Severity::Error => "error",
            Severity::Critical => "critical",
            Severity::Fatal => "fatal",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Event {
    /// SQLite rowid (`events.id`). `0` for events not yet persisted/hydrated
    /// with a rowid (e.g. freshly built by `EventBuilder`).
    #[serde(default)]
    pub id: i64,
    pub event_id: String,
    pub timestamp: DateTime<Utc>,
    pub application: String,
    pub version: Option<String>,
    pub environment: Option<String>,
    pub command: Option<String>,
    pub subsystem: Option<String>,
    pub hostname: Option<String>,
    pub severity: Severity,
    pub category: Option<String>,
    pub error_code: Option<String>,
    pub message: String,
    pub exception: Option<String>,
    pub stacktrace: Option<String>,
    pub stack_norm: Option<String>,
    pub root_cause: Option<String>,
    pub fingerprint: String,
    pub correlation_id: Option<String>,
    pub parent_event_id: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub context: Option<JsonValue>,
    pub tags: Option<Vec<String>>,
    pub metadata: Option<JsonValue>,
}

pub struct EventBuilder {
    event_id: String,
    timestamp: DateTime<Utc>,
    application: String,
    version: Option<String>,
    environment: Option<String>,
    command: Option<String>,
    subsystem: Option<String>,
    pub(crate) hostname: Option<String>,
    severity: Severity,
    category: Option<String>,
    error_code: Option<String>,
    message: String,
    exception: Option<String>,
    stacktrace: Option<String>,
    stack_norm: Option<String>,
    root_cause: Option<String>,
    fingerprint: Option<String>,
    correlation_id: Option<String>,
    parent_event_id: Option<String>,
    pub(crate) context: Option<JsonValue>,
    tags: Option<Vec<String>>,
    metadata: Option<JsonValue>,
}

impl EventBuilder {
    pub fn new(application: impl Into<String>, message: impl Into<String>) -> Self {
        let app_str = application.into();
        let msg_str = message.into();

        let now = Utc::now();
        let ts = uuid::Timestamp::now(uuid::NoContext);
        let event_id = Uuid::new_v7(ts).to_string();

        EventBuilder {
            event_id,
            timestamp: now,
            application: app_str,
            version: None,
            environment: None,
            command: None,
            subsystem: None,
            hostname: None,
            severity: Severity::Error,
            category: None,
            error_code: None,
            message: msg_str,
            exception: None,
            stacktrace: None,
            stack_norm: None,
            root_cause: None,
            fingerprint: None,
            correlation_id: None,
            parent_event_id: None,
            context: None,
            tags: None,
            metadata: None,
        }
    }

    pub fn severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn environment(mut self, env: impl Into<String>) -> Self {
        self.environment = Some(env.into());
        self
    }

    pub fn command(mut self, cmd: impl Into<String>) -> Self {
        self.command = Some(cmd.into());
        self
    }

    pub fn subsystem(mut self, subsys: impl Into<String>) -> Self {
        self.subsystem = Some(subsys.into());
        self
    }

    pub fn hostname(mut self, host: impl Into<String>) -> Self {
        self.hostname = Some(host.into());
        self
    }

    pub fn category(mut self, cat: impl Into<String>) -> Self {
        self.category = Some(cat.into());
        self
    }

    pub fn error_code(mut self, code: impl Into<String>) -> Self {
        self.error_code = Some(code.into());
        self
    }

    pub fn exception(mut self, exc: impl Into<String>) -> Self {
        self.exception = Some(exc.into());
        self
    }

    pub fn stacktrace(mut self, trace: impl Into<String>) -> Self {
        let trace_str = trace.into();
        let normalized = normalize_stacktrace(&trace_str);
        self.stacktrace = Some(trace_str);
        self.stack_norm = Some(normalized);
        self
    }

    pub fn root_cause(mut self, cause: impl Into<String>) -> Self {
        self.root_cause = Some(cause.into());
        self
    }

    pub fn correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    pub fn parent_event_id(mut self, id: impl Into<String>) -> Self {
        self.parent_event_id = Some(id.into());
        self
    }

    pub fn context(mut self, ctx: JsonValue) -> Self {
        self.context = Some(ctx);
        self
    }

    pub fn tags(mut self, tags: Vec<String>) -> Self {
        self.tags = Some(tags);
        self
    }

    pub fn metadata(mut self, meta: JsonValue) -> Self {
        self.metadata = Some(meta);
        self
    }

    /// Mutable access to `metadata` for extension points (e.g. `crypto::FieldCipher`).
    pub(crate) fn metadata_mut(&mut self) -> Option<&mut JsonValue> {
        self.metadata.as_mut()
    }

    /// Auto-populate hostname/pid/cwd/argv/git_commit/allow-listed env vars per `cfg`.
    /// Best-effort: a missing/unreadable field is silently skipped, never errors.
    pub fn enrich(self, cfg: &crate::enrich::EnrichConfig) -> Self {
        crate::enrich::enrich(self, cfg)
    }

    /// Apply secret redaction to message/exception/stacktrace/context/metadata.
    /// **Does not touch `tags`** — tags are caller-set/classifier-suggested,
    /// low-cardinality labels, not a place secrets are expected to land; if a
    /// caller ever puts free-form/user-controlled text into a tag, it will
    /// persist un-redacted (and, unlike `metadata`, tags are never encrypted
    /// either — see `crypto::FieldCipher`/FR-P9-004).
    pub fn redact(mut self, redactor: &crate::redact::Redactor) -> Self {
        self.message = redactor.redact(&self.message);
        if let Some(exc) = &self.exception {
            self.exception = Some(redactor.redact(exc));
        }
        if let Some(trace) = &self.stacktrace {
            let redacted = redactor.redact(trace);
            self.stack_norm = Some(normalize_stacktrace(&redacted));
            self.stacktrace = Some(redacted);
        }
        if let Some(ctx) = &mut self.context {
            redactor.redact_json(ctx);
        }
        if let Some(meta) = &mut self.metadata {
            redactor.redact_json(meta);
        }
        self
    }

    /// Auto-classify an event if category is not already set.
    /// Only updates category/tags if classification succeeds and category was null.
    pub fn classify(mut self, classifier: &crate::taxonomy::Classifier) -> Self {
        if self.category.is_none() {
            let classification = classifier.classify(
                &self.message,
                self.exception.as_deref(),
                self.error_code.as_deref(),
            );

            if let Some(canonical) = classification.canonical {
                self.category = Some(canonical);
            }

            // Add suggested tags to existing tags.
            if !classification.suggested_tags.is_empty() {
                let mut tags = self.tags.unwrap_or_default();
                tags.extend(classification.suggested_tags);
                self.tags = Some(tags);
            }
        }
        self
    }

    pub fn build(mut self) -> Event {
        if self.fingerprint.is_none() {
            self.fingerprint = Some(compute_fingerprint(
                &self.message,
                self.exception.as_deref(),
                self.stack_norm.as_deref(),
                self.category.as_deref(),
            ));
        }

        Event {
            id: 0,
            event_id: self.event_id,
            timestamp: self.timestamp,
            application: self.application,
            version: self.version,
            environment: self.environment,
            command: self.command,
            subsystem: self.subsystem,
            hostname: self.hostname,
            severity: self.severity,
            category: self.category,
            error_code: self.error_code,
            message: self.message,
            exception: self.exception,
            stacktrace: self.stacktrace,
            stack_norm: self.stack_norm,
            root_cause: self.root_cause,
            fingerprint: self.fingerprint.unwrap(),
            correlation_id: self.correlation_id,
            parent_event_id: self.parent_event_id,
            resolved_at: None,
            context: self.context,
            tags: self.tags,
            metadata: self.metadata,
        }
    }
}

/// Convenience constructor: starts an `EventBuilder` at `Severity::Error`.
pub fn error(application: impl Into<String>, message: impl Into<String>) -> EventBuilder {
    EventBuilder::new(application, message).severity(Severity::Error)
}

/// Convenience constructor: starts an `EventBuilder` at `Severity::Warn`.
pub fn warn(application: impl Into<String>, message: impl Into<String>) -> EventBuilder {
    EventBuilder::new(application, message).severity(Severity::Warn)
}

/// Convenience constructor: starts an `EventBuilder` at `Severity::Info`.
pub fn info(application: impl Into<String>, message: impl Into<String>) -> EventBuilder {
    EventBuilder::new(application, message).severity(Severity::Info)
}

/// Convenience constructor for a caught error: captures the error's message and,
/// via its `source()` chain, a synthetic stacktrace (masked into `stack_norm` like
/// any other stacktrace). Rust errors don't carry a portable stack trace, so this
/// is the best obtainable equivalent without requiring callers to enable
/// `std::backtrace` themselves.
pub fn exception(
    application: impl Into<String>,
    err: &dyn std::error::Error,
) -> EventBuilder {
    let message = err.to_string();
    let mut builder = EventBuilder::new(application, message)
        .severity(Severity::Error)
        .exception(err.to_string());

    let mut chain = Vec::new();
    let mut cur = err.source();
    while let Some(e) = cur {
        chain.push(e.to_string());
        cur = e.source();
    }
    if !chain.is_empty() {
        builder = builder.stacktrace(chain.join("\n"));
    }
    builder
}

fn normalize_stacktrace(trace: &str) -> String {
    trace
        .lines()
        .map(|line| {
            let mut normalized = line.to_string();
            normalized = normalized.replace(|c: char| c.is_numeric(), "N");
            normalized
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn compute_fingerprint(
    message: &str,
    exception: Option<&str>,
    stack_norm: Option<&str>,
    category: Option<&str>,
) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    message.hash(&mut hasher);
    if let Some(exc) = exception {
        exc.hash(&mut hasher);
    }
    if let Some(stack) = stack_norm {
        stack.split('\n').take(5).for_each(|line| {
            line.hash(&mut hasher);
        });
    }
    if let Some(cat) = category {
        cat.hash(&mut hasher);
    }

    format!("{:x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_builder() {
        let event = EventBuilder::new("test-app", "something failed")
            .severity(Severity::Error)
            .version("1.0.0")
            .category("application.error")
            .build();

        assert_eq!(event.application, "test-app");
        assert_eq!(event.message, "something failed");
        assert_eq!(event.severity, Severity::Error);
        assert_eq!(event.version, Some("1.0.0".to_string()));
        assert_eq!(event.category, Some("application.error".to_string()));
    }

    #[test]
    fn test_fingerprint_deterministic() {
        let fp1 = compute_fingerprint("error message", None, None, None);
        let fp2 = compute_fingerprint("error message", None, None, None);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn severity_presets() {
        assert_eq!(error("app", "m").build().severity, Severity::Error);
        assert_eq!(warn("app", "m").build().severity, Severity::Warn);
        assert_eq!(info("app", "m").build().severity, Severity::Info);
    }

    #[derive(Debug)]
    struct Cause;
    impl std::fmt::Display for Cause {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "root cause")
        }
    }
    impl std::error::Error for Cause {}

    #[derive(Debug)]
    struct Wrapper;
    impl std::fmt::Display for Wrapper {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "wrapper failed")
        }
    }
    impl std::error::Error for Wrapper {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&Cause)
        }
    }

    #[test]
    fn exception_captures_message_and_source_chain() {
        let err = Wrapper;
        let event = exception("app", &err).build();
        assert_eq!(event.message, "wrapper failed");
        assert_eq!(event.exception, Some("wrapper failed".to_string()));
        assert!(event.stacktrace.unwrap().contains("root cause"));
        assert!(event.stack_norm.is_some());
    }
}
