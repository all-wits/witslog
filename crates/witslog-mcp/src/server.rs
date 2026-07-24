//! Wires `JsonRpcTransport` + `ToolRegistry` into a serve loop implementing
//! MCP `tools/list` and `tools/call` over stdio (FR-P5-001).

use crate::error::McpError;
use crate::registry::ToolRegistry;
use crate::transport::JsonRpcTransport;
use serde_json::{json, Value};
use std::time::Duration;
use witslog_store::DbConnection;

/// Default per-call statement timeout (FR-P5-007).
pub const DEFAULT_STATEMENT_TIMEOUT: Duration = Duration::from_secs(2);

/// Returned as `instructions` in the `initialize` response — a workflow map with
/// worked examples so a lightweight/under-informed model can pattern-match a correct
/// first tool call instead of guessing from tool names alone. Every referenced tool
/// gets a literal `tool_name({field: value})` example matching the shape it actually
/// expects in `tools/call`'s `arguments`.
const INITIALIZE_INSTRUCTIONS: &str = "witslog stores structured error events. Typical workflow:\n\
1. List/browse: latest_errors({limit: 20}) - no query needed. search_errors requires a query string (FTS5 syntax, e.g. {query: \"timeout*\", severity_min: \"error\"}) - use latest_errors instead if you just want to browse.\n\
2. Investigate one error: get_event({event_id: \"...\"}) for the full raw payload, explain_error({event_id: \"...\"}) for a dossier (root cause + causality chain + recurrence), similar_errors({event_id: \"...\", mode: \"fingerprint\"}) for recurrences/duplicates.\n\
3. Aggregate/trend: statistics (headline counts), summarize_errors (grouped roll-up), top_failures({by: \"count\", limit: 10}) (ranked recurring issues), timeline({bucket: \"day\"}) (counts over time), mttr (resolution speed).\n\
4. Correlate a request: list_traces({correlation_id: \"...\"}) or list_traces({root_event_id: \"...\"}).\n\
5. Taxonomy: list_categories (browse), classify_error({message: \"...\"}) (classify raw text you already have, not a stored event).\n\
Timestamps are RFC3339 (e.g. \"2026-07-01T00:00:00Z\"). Read-only unless --allow-write is passed (adds witslog_delete, resolved-only by default).";

pub struct ServerConfig {
    pub allow_write: bool,
    pub attached: Vec<std::path::PathBuf>,
    pub statement_timeout: Duration,
    /// FR-P9-004: env var name holding the metadata-encryption key (from
    /// `[crypto] key_env` in `config.toml`), or `None` if not configured.
    /// When set and the env var actually holds a valid key at call time,
    /// `get_event`/`explain_error` decrypt `metadata`; otherwise readers see
    /// the `"<encrypted>"` placeholder (see `witslog_core::crypto::decrypt_metadata_for_display`).
    pub crypto_key_env: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            allow_write: false,
            attached: Vec::new(),
            statement_timeout: DEFAULT_STATEMENT_TIMEOUT,
            crypto_key_env: None,
        }
    }
}

/// Runs the JSON-RPC 2.0 stdio serve loop until stdin closes (EOF).
/// Each line is one JSON-RPC request; each request gets one line response.
pub fn serve_stdio(db: &DbConnection, config: ServerConfig) -> std::io::Result<()> {
    let mut transport = JsonRpcTransport::new();

    loop {
        let request = match transport.read_request() {
            Ok(Some(req)) => req,
            Ok(None) => break, // stdin closed (EOF)
            Err(e) => {
                // Malformed line: can't recover an id, so best-effort report
                // and keep serving subsequent lines.
                let resp = JsonRpcTransport::error_response(
                    None,
                    -32700,
                    "parse error",
                    Some(json!({"detail": e.to_string()})),
                );
                transport.write_response(resp)?;
                continue;
            }
        };

        let response = handle_request(db, &config, request);
        transport.write_response(response)?;
    }

    Ok(())
}

fn handle_request(db: &DbConnection, config: &ServerConfig, request: Value) -> Value {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");

    // Per-call statement timeout (FR-P5-007): installed fresh for every
    // request and cleared afterward so it can't leak into unrelated calls.
    db.set_statement_timeout(config.statement_timeout);
    let result = dispatch(db, config, method, request.get("params").cloned());
    db.clear_statement_timeout();

    match result {
        Ok(value) => JsonRpcTransport::success_response(id, value),
        Err(e) => {
            let (code, message, data) = e.to_jsonrpc();
            JsonRpcTransport::error_response(id, code, &message, data)
        }
    }
}

fn dispatch(
    db: &DbConnection,
    config: &ServerConfig,
    method: &str,
    params: Option<Value>,
) -> crate::error::Result<Value> {
    match method {
        "initialize" => {
            Ok(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "witslog",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": INITIALIZE_INSTRUCTIONS
            }))
        }
        "tools/list" => {
            let registry = ToolRegistry::new(db)
                .with_allow_write(config.allow_write)
                .with_attached(config.attached.clone())
                .with_crypto_key_env(config.crypto_key_env.clone());
            let tools: Vec<Value> = registry
                .list_tools()
                .into_iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": t.input_schema
                    })
                })
                .collect();
            Ok(json!({"tools": tools}))
        }
        "tools/call" => {
            let registry = ToolRegistry::new(db)
                .with_allow_write(config.allow_write)
                .with_attached(config.attached.clone())
                .with_crypto_key_env(config.crypto_key_env.clone());
            let params = params.unwrap_or(Value::Null);
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| McpError::InvalidParams("missing 'name'".to_string()))?;
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            let result = registry.call_tool(name, arguments)?;
            Ok(json!({
                "content": [{"type": "text", "text": result.to_string()}],
                "structuredContent": result
            }))
        }
        "" => Err(McpError::InvalidParams("missing 'method'".to_string())),
        other => Err(McpError::MethodNotFound(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression lock for the "MCP works but no AI assistant knows the
    /// workflow" gap: `initialize` must carry non-empty `instructions` with
    /// at least one worked `tool_name({...})` example a weak model can
    /// pattern-match, not just prose.
    #[test]
    fn initialize_response_includes_worked_example_instructions() {
        let db = DbConnection::open(":memory:").unwrap();
        db.migrate().unwrap();
        let config = ServerConfig::default();

        let result = dispatch(&db, &config, "initialize", None).unwrap();
        let instructions = result["instructions"]
            .as_str()
            .expect("instructions must be a string");

        assert!(!instructions.is_empty());
        assert!(
            instructions.contains("latest_errors({"),
            "instructions must contain a worked tool_name({{...}}) example"
        );
        assert!(
            instructions.contains("search_errors"),
            "instructions must mention search_errors vs. latest_errors disambiguation"
        );
    }

    /// Regression lock: every tool description must carry a worked
    /// `Example: {...}` clause, not regress to a bare one-liner a weak model
    /// can't pattern-match input shapes from.
    #[test]
    fn every_tool_description_has_a_worked_example() {
        use crate::tools::Tool;

        for tool in Tool::builtin_tools() {
            assert!(
                tool.description.contains('{') && tool.description.to_lowercase().contains("example"),
                "{} description is missing a worked Example: {{...}} clause: {}",
                tool.name,
                tool.description
            );
        }
    }

    /// Regression lock: `severity_min` must be a closed enum (matching
    /// bindings/CONTRACT.md's severity taxonomy), not a free-form string a
    /// model could guess wrong on.
    #[test]
    fn severity_min_is_a_closed_enum() {
        use crate::tools::Tool;

        for tool in Tool::builtin_tools() {
            let Some(prop) = tool.input_schema["properties"]["severity_min"].as_object() else {
                continue;
            };
            let enum_values = prop
                .get("enum")
                .and_then(|v| v.as_array())
                .unwrap_or_else(|| panic!("{} severity_min must have an enum", tool.name));
            assert!(enum_values.iter().any(|v| v == "error"));
            assert!(enum_values.iter().any(|v| v == "fatal"));
        }
    }
}
