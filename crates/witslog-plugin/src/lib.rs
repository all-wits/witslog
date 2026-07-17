//! P9: extension points for witslog. Each trait is a narrow hook invoked by
//! the core/store/mcp layers; the `PluginRegistry` holds static registrations
//! and isolates plugin panics so a misbehaving plugin never corrupts the DB
//! or crashes the core write path (FR-P9-001/002, non-functional isolation).

use serde_json::Value as JsonValue;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("plugin '{name}' failed: {message}")]
    Failed { name: String, message: String },
    #[error("plugin '{name}' panicked")]
    Panicked { name: String },
}

/// A suggested classification from a plugin, mirroring `witslog-core`'s
/// taxonomy output without depending on that crate (this crate stays a leaf).
#[derive(Debug, Clone, Default)]
pub struct PluginClassification {
    pub canonical: Option<String>,
    pub tags: Vec<String>,
}

/// Extension point: custom auto-classification rules beyond the builtin
/// taxonomy engine (FR-P9-001).
pub trait TaxonomyRule: Send + Sync {
    fn name(&self) -> &str;
    fn classify(
        &self,
        message: &str,
        exception: Option<&str>,
        error_code: Option<&str>,
    ) -> Option<PluginClassification>;
}

/// Extension point: export logged events to an external sink/format.
pub trait Exporter: Send + Sync {
    fn name(&self) -> &str;
    fn export(&self, events: &[JsonValue]) -> Result<(), String>;
}

/// Extension point: add extra fields to `context`/`metadata` before write.
pub trait Enricher: Send + Sync {
    fn name(&self) -> &str;
    fn enrich(&self, context: &mut JsonValue);
}

/// Extension point: an alternate/additional storage backend an event is
/// mirrored to (e.g. a webhook-backed store, a second DB).
pub trait StorageBackend: Send + Sync {
    fn name(&self) -> &str;
    fn write_event(&self, event: &JsonValue) -> Result<(), String>;
}

/// Extension point: fire on new events (webhook/desktop/file notifiers).
pub trait Notifier: Send + Sync {
    fn name(&self) -> &str;
    fn notify(&self, event: &JsonValue) -> Result<(), String>;
}

/// Extension point: an additional MCP tool beyond the builtin 12 (P5).
pub trait McpTool: Send + Sync {
    fn name(&self) -> &str;
    fn call(&self, params: JsonValue) -> Result<JsonValue, String>;
}

/// Static registry of plugins, one `Vec` per extension point. Dynamic
/// (`.so`/`.dll`) loading is deliberately out of scope here — plugins are
/// compiled in and registered by the host binary; this keeps the surface
/// small and avoids ABI-stability guarantees across a dynamic boundary.
#[derive(Default)]
pub struct PluginRegistry {
    taxonomy_rules: Vec<Arc<dyn TaxonomyRule>>,
    exporters: Vec<Arc<dyn Exporter>>,
    enrichers: Vec<Arc<dyn Enricher>>,
    storage_backends: Vec<Arc<dyn StorageBackend>>,
    notifiers: Vec<Arc<dyn Notifier>>,
    mcp_tools: Vec<Arc<dyn McpTool>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_taxonomy_rule(&mut self, plugin: Arc<dyn TaxonomyRule>) {
        self.taxonomy_rules.push(plugin);
    }

    pub fn register_exporter(&mut self, plugin: Arc<dyn Exporter>) {
        self.exporters.push(plugin);
    }

    pub fn register_enricher(&mut self, plugin: Arc<dyn Enricher>) {
        self.enrichers.push(plugin);
    }

    pub fn register_storage_backend(&mut self, plugin: Arc<dyn StorageBackend>) {
        self.storage_backends.push(plugin);
    }

    pub fn register_notifier(&mut self, plugin: Arc<dyn Notifier>) {
        self.notifiers.push(plugin);
    }

    pub fn register_mcp_tool(&mut self, plugin: Arc<dyn McpTool>) {
        self.mcp_tools.push(plugin);
    }

    pub fn taxonomy_rules(&self) -> &[Arc<dyn TaxonomyRule>] {
        &self.taxonomy_rules
    }

    pub fn mcp_tools(&self) -> &[Arc<dyn McpTool>] {
        &self.mcp_tools
    }

    /// First non-`None` classification across registered taxonomy-rule
    /// plugins, in registration order. A panicking plugin is isolated:
    /// treated as "no match" rather than propagating.
    pub fn classify(
        &self,
        message: &str,
        exception: Option<&str>,
        error_code: Option<&str>,
    ) -> Option<PluginClassification> {
        for plugin in &self.taxonomy_rules {
            let plugin = plugin.clone();
            let msg = message.to_string();
            let exc = exception.map(|s| s.to_string());
            let code = error_code.map(|s| s.to_string());
            let result = catch_unwind(AssertUnwindSafe(|| {
                plugin.classify(&msg, exc.as_deref(), code.as_deref())
            }));
            if let Ok(Some(classification)) = result {
                return Some(classification);
            }
        }
        None
    }

    /// Runs every registered enricher, isolating panics per-plugin so one
    /// bad enricher doesn't stop the others or the write path.
    pub fn run_enrichers(&self, context: &mut JsonValue) {
        for plugin in &self.enrichers {
            let plugin = plugin.clone();
            let mut ctx_clone = context.clone();
            let result = catch_unwind(AssertUnwindSafe(|| {
                plugin.enrich(&mut ctx_clone);
                ctx_clone
            }));
            if let Ok(updated) = result {
                *context = updated;
            }
        }
    }

    /// Fans an event out to every registered storage backend + notifier.
    /// Failures (including panics) are collected, never short-circuit a
    /// sibling plugin, and never propagate as an error from the core write
    /// path (FR-P9 non-functional: plugin failures isolated).
    pub fn dispatch_event(&self, event: &JsonValue) -> Vec<PluginError> {
        let mut errors = Vec::new();

        for plugin in &self.storage_backends {
            let name = plugin.name().to_string();
            let plugin = plugin.clone();
            let event = event.clone();
            match catch_unwind(AssertUnwindSafe(|| plugin.write_event(&event))) {
                Ok(Ok(())) => {}
                Ok(Err(message)) => errors.push(PluginError::Failed { name, message }),
                Err(_) => errors.push(PluginError::Panicked { name }),
            }
        }

        for plugin in &self.notifiers {
            let name = plugin.name().to_string();
            let plugin = plugin.clone();
            let event = event.clone();
            match catch_unwind(AssertUnwindSafe(|| plugin.notify(&event))) {
                Ok(Ok(())) => {}
                Ok(Err(message)) => errors.push(PluginError::Failed { name, message }),
                Err(_) => errors.push(PluginError::Panicked { name }),
            }
        }

        errors
    }

    pub fn export_all(&self, exporter_name: &str, events: &[JsonValue]) -> Result<(), PluginError> {
        let plugin = self
            .exporters
            .iter()
            .find(|p| p.name() == exporter_name)
            .ok_or_else(|| PluginError::Failed {
                name: exporter_name.to_string(),
                message: "no such exporter registered".to_string(),
            })?
            .clone();

        match catch_unwind(AssertUnwindSafe(|| plugin.export(events))) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(PluginError::Failed {
                name: exporter_name.to_string(),
                message,
            }),
            Err(_) => Err(PluginError::Panicked {
                name: exporter_name.to_string(),
            }),
        }
    }

    pub fn call_mcp_tool(&self, tool_name: &str, params: JsonValue) -> Result<JsonValue, PluginError> {
        let plugin = self
            .mcp_tools
            .iter()
            .find(|p| p.name() == tool_name)
            .ok_or_else(|| PluginError::Failed {
                name: tool_name.to_string(),
                message: "no such MCP tool registered".to_string(),
            })?
            .clone();

        match catch_unwind(AssertUnwindSafe(|| plugin.call(params))) {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(message)) => Err(PluginError::Failed {
                name: tool_name.to_string(),
                message,
            }),
            Err(_) => Err(PluginError::Panicked {
                name: tool_name.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DnsRule;
    impl TaxonomyRule for DnsRule {
        fn name(&self) -> &str {
            "dns_rule"
        }
        fn classify(
            &self,
            message: &str,
            _exception: Option<&str>,
            _error_code: Option<&str>,
        ) -> Option<PluginClassification> {
            if message.contains("dns") {
                Some(PluginClassification {
                    canonical: Some("custom.dns".to_string()),
                    tags: vec!["plugin".to_string()],
                })
            } else {
                None
            }
        }
    }

    struct StampEnricher;
    impl Enricher for StampEnricher {
        fn name(&self) -> &str {
            "stamp"
        }
        fn enrich(&self, context: &mut JsonValue) {
            context["plugin_stamp"] = JsonValue::from("stamped");
        }
    }

    struct RecordingStorage {
        calls: std::sync::Mutex<Vec<JsonValue>>,
    }
    impl StorageBackend for RecordingStorage {
        fn name(&self) -> &str {
            "recorder"
        }
        fn write_event(&self, event: &JsonValue) -> Result<(), String> {
            self.calls.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    struct PanicNotifier;
    impl Notifier for PanicNotifier {
        fn name(&self) -> &str {
            "panics"
        }
        fn notify(&self, _event: &JsonValue) -> Result<(), String> {
            panic!("boom");
        }
    }

    struct EchoExporter;
    impl Exporter for EchoExporter {
        fn name(&self) -> &str {
            "echo"
        }
        fn export(&self, events: &[JsonValue]) -> Result<(), String> {
            if events.is_empty() {
                Err("no events".to_string())
            } else {
                Ok(())
            }
        }
    }

    struct EchoTool;
    impl McpTool for EchoTool {
        fn name(&self) -> &str {
            "echo_tool"
        }
        fn call(&self, params: JsonValue) -> Result<JsonValue, String> {
            Ok(params)
        }
    }

    #[test]
    fn taxonomy_rule_plugin_classifies() {
        let mut registry = PluginRegistry::new();
        registry.register_taxonomy_rule(Arc::new(DnsRule));

        let result = registry.classify("dns lookup failed", None, None).unwrap();
        assert_eq!(result.canonical.as_deref(), Some("custom.dns"));

        assert!(registry.classify("unrelated", None, None).is_none());
    }

    #[test]
    fn enricher_plugin_mutates_context() {
        let mut registry = PluginRegistry::new();
        registry.register_enricher(Arc::new(StampEnricher));

        let mut ctx = serde_json::json!({});
        registry.run_enrichers(&mut ctx);
        assert_eq!(ctx["plugin_stamp"], "stamped");
    }

    #[test]
    fn storage_backend_plugin_observes_event() {
        let mut registry = PluginRegistry::new();
        let storage = Arc::new(RecordingStorage {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        registry.register_storage_backend(storage.clone());

        let event = serde_json::json!({"message": "boom"});
        let errors = registry.dispatch_event(&event);
        assert!(errors.is_empty());
        assert_eq!(storage.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn notifier_panic_is_isolated_not_propagated_as_panic() {
        let mut registry = PluginRegistry::new();
        registry.register_notifier(Arc::new(PanicNotifier));

        let event = serde_json::json!({"message": "boom"});
        // Must not unwind out of dispatch_event; must report as an error.
        let errors = registry.dispatch_event(&event);
        assert_eq!(errors.len(), 1);
        match &errors[0] {
            PluginError::Panicked { name } => assert_eq!(name, "panics"),
            other => panic!("expected Panicked, got {:?}", other),
        }
    }

    #[test]
    fn exporter_plugin_runs_by_name() {
        let mut registry = PluginRegistry::new();
        registry.register_exporter(Arc::new(EchoExporter));

        let events = vec![serde_json::json!({"a": 1})];
        assert!(registry.export_all("echo", &events).is_ok());
        assert!(registry.export_all("echo", &[]).is_err());
        assert!(registry.export_all("missing", &events).is_err());
    }

    #[test]
    fn mcp_tool_plugin_runs_by_name() {
        let mut registry = PluginRegistry::new();
        registry.register_mcp_tool(Arc::new(EchoTool));

        let params = serde_json::json!({"x": 1});
        let result = registry.call_mcp_tool("echo_tool", params.clone()).unwrap();
        assert_eq!(result, params);
        assert!(registry.call_mcp_tool("missing", params).is_err());
    }
}
