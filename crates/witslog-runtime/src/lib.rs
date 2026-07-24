//! witslog-runtime — the ambient "Provider" for witslog.
//!
//! Instead of repeating the full `EventBuilder` chain (resolve db path, load
//! config, rebuild redactor/enricher/classifier/buffer) at every callsite, an
//! application *mounts* witslog once at its entrypoint:
//!
//! ```no_run
//! fn main() {
//!     let _guard = witslog_runtime::init_default();
//!     // ... rest of the program. Panics, `tracing` error/warn events, and
//!     // `Result::log_err()` boundaries are now captured ambiently.
//! }
//! ```
//!
//! This mirrors a TanStack-Devtools / Next.js `<Provider>` mounted at the app
//! root. The guard installs a panic hook and flushes any buffered events on
//! drop. The same runtime is re-exposed through the C ABI (`witslog-ffi`), so
//! any language/framework can mount it at its own entrypoint.
//!
//! Everything here is *additive* — `witslog_core::EventBuilder`, the CLI, and
//! the existing FFI `witslog_log` continue to work unchanged.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use witslog_config::Config;
use witslog_core::{
    error as build_error, exception as build_exception, info as build_info, warn as build_warn,
    AsyncBuffer, BufferConfig, Classifier, EnrichConfig, Event, EventBuilder, FieldCipher, Redactor,
    Severity,
};
use witslog_store::{EventWriter, Store, StoreSink};

mod notify;

#[cfg(feature = "tracing")]
mod tracing_layer;
#[cfg(feature = "tracing")]
pub use tracing_layer::WitslogLayer;

/// Errors from the stateless capture pipeline. The ambient path swallows these
/// (a logger must never take down the app it observes); callers of
/// [`build_and_write`] get them surfaced.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("invalid redaction pattern: {0}")]
    Redact(#[from] witslog_core::RedactError),
    #[error("invalid custom rules file: {0}")]
    CustomRules(String),
    #[error(transparent)]
    Store(#[from] witslog_store::StoreError),
    /// FR-P9-004 fail-closed: `crypto.key_env` names an env var, but it's
    /// unset or not valid 64-char hex. The write is refused rather than
    /// silently persisting `metadata` in plaintext.
    #[error("metadata encryption is configured (key_env={0:?}) but the key is unavailable: {1}")]
    MissingEncryptionKey(String, String),
}

/// `(registry, min_severity_rank)` — `None` when `[notify]` is disabled or
/// has no `path` configured.
type NotifyState = Option<(Arc<witslog_plugin::PluginRegistry>, i32)>;

/// The resolved, mount-once state. Redactor/classifier/enrich config are built
/// a single time here rather than per-callsite.
struct Runtime {
    enrich: EnrichConfig,
    redactor: Arc<Redactor>,
    classifier: Arc<Classifier>,
    auto_classify: bool,
    buffer_cfg: BufferConfig,
    db_path: PathBuf,
    notify: NotifyState,
    /// FR-P9-004: env var name holding the metadata-encryption key, or `None`
    /// if encryption is off. Resolved to a `FieldCipher` fresh at each write
    /// (not cached) — cheap `env::var` lookup, and means the check is always
    /// against the *current* env, not a snapshot taken at mount time.
    crypto_key_env: Option<String>,
}

/// Cheap, clonable view of the runtime taken under the lock, so the actual
/// write happens without holding it.
#[derive(Clone)]
struct Snapshot {
    enrich: EnrichConfig,
    redactor: Arc<Redactor>,
    classifier: Arc<Classifier>,
    auto_classify: bool,
    buffer_cfg: BufferConfig,
    db_path: PathBuf,
    notify: NotifyState,
    crypto_key_env: Option<String>,
}

static RUNTIME: OnceLock<RwLock<Runtime>> = OnceLock::new();
static BUFFER: OnceLock<Mutex<Option<AsyncBuffer<StoreSink>>>> = OnceLock::new();
static PANIC_HOOK_INSTALLED: OnceLock<()> = OnceLock::new();

fn snapshot() -> Option<Snapshot> {
    let rt = RUNTIME.get()?.read().unwrap();
    Some(Snapshot {
        enrich: rt.enrich.clone(),
        redactor: Arc::clone(&rt.redactor),
        classifier: Arc::clone(&rt.classifier),
        auto_classify: rt.auto_classify,
        buffer_cfg: rt.buffer_cfg.clone(),
        db_path: rt.db_path.clone(),
        notify: rt.notify.clone(),
        crypto_key_env: rt.crypto_key_env.clone(),
    })
}

// ---------------------------------------------------------------------------
// Mounting
// ---------------------------------------------------------------------------

/// RAII handle returned by [`init`]. Dropping it flushes any buffered events
/// (joining the background flush thread), the same guarantee `AsyncBuffer`'s
/// own `Drop` provides. Keep it alive for the lifetime of the program:
/// `let _guard = witslog_runtime::init_default();`.
#[must_use = "dropping the guard immediately un-mounts witslog and flushes; bind it to a name that lives for the program"]
pub struct Guard {
    _private: (),
}

impl Drop for Guard {
    fn drop(&mut self) {
        flush();
    }
}

/// Mount witslog with an explicit config and return a [`Guard`]. Installs the
/// panic hook (chained after any previously-installed hook).
pub fn init(config: Config) -> Guard {
    arm(config);
    Guard { _private: () }
}

/// Mount witslog resolving config from the current directory (`.witslog/config.toml`
/// if present, else defaults). Convenience wrapper over [`init`].
pub fn init_default() -> Guard {
    init(load_default_config())
}

/// Set up the process-global runtime and install the panic hook **without**
/// returning a guard — for hosts that have no RAII `Drop` (the C ABI). Flush
/// must then be driven explicitly via [`flush`].
pub fn arm(config: Config) {
    let db_path = resolve_db_path(&config);
    let rt = build_runtime(&config, db_path);

    match RUNTIME.get() {
        Some(lock) => *lock.write().unwrap() = rt,
        None => {
            let _ = RUNTIME.set(RwLock::new(rt));
        }
    }

    // Rebuild the buffer on next capture under the (possibly new) config.
    if let Some(buf) = BUFFER.get() {
        *buf.lock().unwrap() = None;
    }

    install_panic_hook();
}

fn load_default_config() -> Config {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    Config::load_or_default(&cwd)
}

fn resolve_db_path(config: &Config) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    config.resolve_db_path(&cwd)
}

fn build_runtime(config: &Config, db_path: PathBuf) -> Runtime {
    // A bad redaction pattern falls back to built-ins rather than failing the
    // mount — the app must not fail to start because of a logger config typo.
    let redactor = Redactor::new(&config.redact.custom_patterns)
        .unwrap_or_else(|_| Redactor::built_in());
    let classifier = build_classifier(config).unwrap_or_else(|_| Classifier::built_in());

    Runtime {
        enrich: enrich_from(config),
        redactor: Arc::new(redactor),
        classifier: Arc::new(classifier),
        auto_classify: config.taxonomy.auto_classify_enabled,
        buffer_cfg: buffer_cfg_from(config),
        db_path,
        notify: build_notify_registry(config),
        crypto_key_env: config.crypto.key_env.clone(),
    }
}

/// Resolves `crypto.key_env` to a `FieldCipher`, fail-closed (FR-P9-004): if
/// `key_env` names a var that's unset or not valid hex, this returns an
/// error rather than silently falling back to plaintext `metadata`.
/// `key_env: None` (encryption off) always returns `Ok(None)`.
fn resolve_cipher(key_env: &Option<String>) -> Result<Option<FieldCipher>, RuntimeError> {
    let Some(var) = key_env else {
        return Ok(None);
    };
    match FieldCipher::from_env(var) {
        Ok(Some(cipher)) => Ok(Some(cipher)),
        Ok(None) => Err(RuntimeError::MissingEncryptionKey(
            var.clone(),
            "environment variable is not set".to_string(),
        )),
        Err(e) => Err(RuntimeError::MissingEncryptionKey(var.clone(), e.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Ambient capture
// ---------------------------------------------------------------------------

/// Capture an event through the mounted runtime. Returns the inserted row id
/// (or `0` when buffered), or `None` if witslog isn't mounted or the write
/// failed. Never panics.
pub fn capture(builder: EventBuilder) -> Option<i64> {
    let snap = snapshot()?;
    write_via_snapshot(&snap, builder, false)
}

/// Like [`capture`] but forces a synchronous write, bypassing the buffer. Used
/// by the panic hook, where the process may abort before a background flush.
fn capture_sync(builder: EventBuilder) -> Option<i64> {
    let snap = snapshot()?;
    write_via_snapshot(&snap, builder, true)
}

/// One-line ambient error. `witslog_runtime::log_error("app", format!("x={x}"))`.
pub fn log_error(app: impl Into<String>, message: impl Into<String>) -> Option<i64> {
    capture(build_error(app, message))
}

/// One-line ambient warning.
pub fn log_warn(app: impl Into<String>, message: impl Into<String>) -> Option<i64> {
    capture(build_warn(app, message))
}

/// One-line ambient info.
pub fn log_info(app: impl Into<String>, message: impl Into<String>) -> Option<i64> {
    capture(build_info(app, message))
}

fn write_via_snapshot(snap: &Snapshot, builder: EventBuilder, force_sync: bool) -> Option<i64> {
    let classifier = if snap.auto_classify {
        Some(snap.classifier.as_ref())
    } else {
        None
    };
    // Fail-closed (FR-P9-004): if encryption is configured but the key can't
    // be resolved, drop the write rather than persist metadata in plaintext.
    // This silently drops the event, matching every other failure in this
    // ambient path (store-open failure, buffer-full) — `capture`/`capture_sync`
    // already document "never panics" over "never loses an event".
    let cipher = resolve_cipher(&snap.crypto_key_env).ok()?;
    let event = apply_pipeline(builder, &snap.enrich, &snap.redactor, classifier, cipher.as_ref());

    let result = if snap.buffer_cfg.enabled && !force_sync {
        enqueue_buffered(&snap.db_path, &snap.buffer_cfg, event.clone())
    } else {
        let store = Store::open_or_create(&snap.db_path).ok()?;
        let writer = EventWriter::new(store.conn());
        writer.write(&event).ok()
    };

    // Hard rule: never dispatch from the panic-hook's forced-sync path — a
    // panic may precede process abort, and a notifier (even a "fast" one)
    // doing I/O inside a panic handler is the one place a stall is
    // unacceptable. `force_sync` is exactly that path (see `capture_sync`).
    if result.is_some() && !force_sync {
        dispatch_notify(&snap.notify, &event);
    }

    result
}

fn enqueue_buffered(db_path: &Path, cfg: &BufferConfig, event: Event) -> Option<i64> {
    let buf_lock = BUFFER.get_or_init(|| Mutex::new(None));
    let mut guard = buf_lock.lock().unwrap();
    if guard.is_none() {
        let store = Store::open_or_create(db_path).ok()?;
        *guard = Some(AsyncBuffer::new(StoreSink::new(store), cfg.clone()));
    }
    if let Some(buffer) = guard.as_ref() {
        buffer.enqueue(event);
    }
    Some(0)
}

/// Flush and tear down any buffered events, joining the background flush
/// thread. Idempotent. Called automatically by [`Guard`]'s `Drop`; call it
/// directly from a C-ABI host's shutdown/atexit path.
pub fn flush() {
    if let Some(buf) = BUFFER.get() {
        // Dropping the AsyncBuffer joins its flush thread, guaranteeing queued
        // events are persisted (or counted as dropped) before we return.
        *buf.lock().unwrap() = None;
    }
}

// ---------------------------------------------------------------------------
// Stateless pipeline (shared by the ambient path and one-shot CLI callers)
// ---------------------------------------------------------------------------

/// Run the full enrich → redact → (classify) → build → write pipeline for a
/// single event against an explicit `config`/`db_path`, returning the built
/// `Event` (whose `event_id`/`fingerprint` the caller may want to print).
///
/// This is the single home for the logic that `witslog log` and the ambient
/// runtime used to duplicate. Unlike [`capture`], it rebuilds the redactor /
/// classifier each call — fine for one-shot CLI processes.
pub fn build_and_write(
    config: &Config,
    db_path: &Path,
    builder: EventBuilder,
) -> Result<Event, RuntimeError> {
    let redactor = Redactor::new(&config.redact.custom_patterns)?;
    let classifier = if config.taxonomy.auto_classify_enabled {
        Some(build_classifier(config)?)
    } else {
        None
    };
    // Fail-closed (FR-P9-004): propagate as a real error here (unlike the
    // ambient ...`write_via_snapshot` path) since `build_and_write` callers
    // (CLI `witslog log`, FFI) can and should surface it as a failed write.
    let cipher = resolve_cipher(&config.crypto.key_env)?;
    let event = apply_pipeline(
        builder,
        &enrich_from(config),
        &redactor,
        classifier.as_ref(),
        cipher.as_ref(),
    );

    let store = Store::open_or_create(db_path)?;

    if config.buffer.enabled {
        let sink = StoreSink::new(store);
        let buffer = AsyncBuffer::new(sink, buffer_cfg_from(config));
        buffer.enqueue(event.clone());
        // Drop joins the flush thread, guaranteeing persistence before return.
        drop(buffer);
    } else {
        let writer = EventWriter::new(store.conn());
        writer.write(&event)?;
    }

    dispatch_notify(&build_notify_registry(config), &event);

    Ok(event)
}

fn apply_pipeline(
    builder: EventBuilder,
    enrich: &EnrichConfig,
    redactor: &Redactor,
    classifier: Option<&Classifier>,
    cipher: Option<&FieldCipher>,
) -> Event {
    let builder = builder.enrich(enrich).redact(redactor);
    let builder = match classifier {
        Some(c) => builder.classify(c),
        None => builder,
    };
    // FR-P9-004: encrypt `metadata` last — after redact (so any pattern-shaped
    // secret elsewhere in metadata's plaintext still gets a redaction pass)
    // and before build (so the stored Event carries the envelope, not raw JSON).
    let builder = match cipher {
        Some(c) => builder.encrypt_metadata(c),
        None => builder,
    };
    builder.build()
}

fn enrich_from(config: &Config) -> EnrichConfig {
    EnrichConfig {
        hostname: config.enrich.hostname,
        pid: config.enrich.pid,
        cwd: config.enrich.cwd,
        argv: config.enrich.argv,
        git_commit: config.enrich.git_commit,
        env_allowlist: config.enrich.env_allowlist.clone(),
    }
}

fn buffer_cfg_from(config: &Config) -> BufferConfig {
    BufferConfig {
        enabled: config.buffer.enabled,
        batch_size: config.buffer.batch_size,
        flush_interval_ms: config.buffer.flush_interval_ms,
        queue_capacity: config.buffer.queue_capacity,
    }
}

/// Builds the notifier `PluginRegistry` from `[notify]`. `None` when disabled
/// or no `path` is configured (the only builtin notifier is file-based).
fn build_notify_registry(config: &Config) -> NotifyState {
    if !config.notify.enabled {
        return None;
    }
    let path = config.notify.path.clone()?;

    let mut registry = witslog_plugin::PluginRegistry::new();
    let file_notifier: Arc<dyn witslog_plugin::Notifier> = Arc::new(notify::FileNotifier::new(path));
    let notifier: Arc<dyn witslog_plugin::Notifier> = match config.notify.once_per_fingerprint_secs {
        Some(secs) if secs > 0 => Arc::new(notify::ThrottledNotifier::new(
            file_notifier,
            std::time::Duration::from_secs(secs),
        )),
        _ => file_notifier,
    };
    registry.register_notifier(notifier);

    Some((Arc::new(registry), severity_rank(&config.notify.min_severity)))
}

/// Dispatches to the notifier registry if configured and the event meets
/// `min_severity`. Failures (including plugin panics) are already isolated
/// and swallowed by `PluginRegistry::dispatch_event` — never fails the write.
fn dispatch_notify(notify: &NotifyState, event: &Event) {
    let Some((registry, min_rank)) = notify else {
        return;
    };
    if event.severity.rank() < *min_rank {
        return;
    }
    if let Ok(json) = serde_json::to_value(event) {
        let _ = registry.dispatch_event(&json);
    }
}

fn severity_rank(s: &str) -> i32 {
    match s {
        "trace" => 10,
        "debug" => 20,
        "info" => 30,
        "warn" => 40,
        "error" => 50,
        "critical" => 60,
        "fatal" => 70,
        _ => 50,
    }
}

fn build_classifier(config: &Config) -> Result<Classifier, RuntimeError> {
    match &config.taxonomy.custom_rules_file {
        Some(path) => {
            let rules = witslog_core::load_custom_rules(path)
                .map_err(|e| RuntimeError::CustomRules(e.to_string()))?;
            Ok(Classifier::built_in_with_custom(rules))
        }
        None => Ok(Classifier::built_in()),
    }
}

// ---------------------------------------------------------------------------
// Panic hook
// ---------------------------------------------------------------------------

/// Install a panic hook that captures each panic as a `Fatal` event before
/// chaining to the previously-installed hook. Installed once per process
/// (subsequent calls are no-ops), so re-`arm`-ing does not stack hooks.
fn install_panic_hook() {
    if PANIC_HOOK_INSTALLED.set(()).is_err() {
        return;
    }

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        capture_panic(info);
        prev(info);
    }));
}

fn capture_panic(info: &std::panic::PanicHookInfo<'_>) {
    let message = panic_message(info);
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
    let backtrace = std::backtrace::Backtrace::force_capture().to_string();
    let stacktrace = match location {
        Some(loc) => format!("panicked at {loc}\n{backtrace}"),
        None => backtrace,
    };

    let builder = build_error(app_name(), message)
        .severity(Severity::Fatal)
        .error_code("panic")
        .stacktrace(stacktrace);

    // Synchronous write: a panic may precede process abort, so never buffer it.
    let _ = capture_sync(builder);
}

fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic (non-string payload)".to_string()
    }
}

/// Best-effort application name for auto-captured events: the current
/// executable's file stem, falling back to `"witslog"`.
fn app_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "witslog".to_string())
}

// ---------------------------------------------------------------------------
// Result extension trait
// ---------------------------------------------------------------------------

/// Capture the `Err` arm of a `Result` at a boundary with one chained call,
/// then pass the `Result` through unchanged:
///
/// ```no_run
/// use witslog_runtime::LogErr;
/// # fn do_io() -> std::io::Result<()> { Ok(()) }
/// let _ = do_io().log_err("my-app");
/// ```
pub trait LogErr {
    /// On `Err`, capture the error (and its `source()` chain) via the mounted
    /// runtime, then return `self`.
    fn log_err(self, app: &str) -> Self;
}

impl<T, E: std::error::Error> LogErr for Result<T, E> {
    fn log_err(self, app: &str) -> Self {
        if let Err(e) = &self {
            let _ = capture(build_exception(app, e));
        }
        self
    }
}

// ---------------------------------------------------------------------------
// Macros
// ---------------------------------------------------------------------------

/// `witslog_runtime::error!("app", "failed: {code}")` — ambient error capture
/// with `format!`-style args. Expands to [`log_error`].
#[macro_export]
macro_rules! error {
    ($app:expr, $($arg:tt)*) => {
        $crate::log_error($app, format!($($arg)*))
    };
}

/// `witslog_runtime::warn!("app", "slow: {ms}ms")` — see [`error!`].
#[macro_export]
macro_rules! warn {
    ($app:expr, $($arg:tt)*) => {
        $crate::log_warn($app, format!($($arg)*))
    };
}

/// `witslog_runtime::info!("app", "started")` — see [`error!`].
#[macro_export]
macro_rules! info {
    ($app:expr, $($arg:tt)*) => {
        $crate::log_info($app, format!($($arg)*))
    };
}
