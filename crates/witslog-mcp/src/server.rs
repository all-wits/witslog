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

pub struct ServerConfig {
    pub allow_write: bool,
    pub attached: Vec<std::path::PathBuf>,
    pub statement_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            allow_write: false,
            attached: Vec::new(),
            statement_timeout: DEFAULT_STATEMENT_TIMEOUT,
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
    let registry = ToolRegistry::new(db)
        .with_allow_write(config.allow_write)
        .with_attached(config.attached.clone());

    match method {
        "tools/list" => {
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
