use serde_json::{json, Value};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum McpError {
    #[error("Invalid params: {0}")]
    InvalidParams(String),

    #[error("Method not found: {0}")]
    MethodNotFound(String),

    #[error("Database error: {0}")]
    DbError(String),

    #[error("Query error: {0}")]
    QueryError(String),

    #[error("Timed out")]
    Timeout,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, McpError>;

/// Standard JSON-RPC 2.0 error codes used by this server.
pub const JSONRPC_INVALID_PARAMS: i32 = -32602;
pub const JSONRPC_METHOD_NOT_FOUND: i32 = -32601;
pub const JSONRPC_DOMAIN_ERROR: i32 = -32000;

impl McpError {
    /// Map to a `(code, message, data)` triple suitable for a JSON-RPC error
    /// object. `message` never contains raw SQL — domain/db errors are
    /// flattened to a generic message with `data.retriable`.
    pub fn to_jsonrpc(&self) -> (i32, String, Option<Value>) {
        match self {
            McpError::InvalidParams(detail) => (
                JSONRPC_INVALID_PARAMS,
                "invalid params".to_string(),
                Some(json!({"detail": detail})),
            ),
            McpError::MethodNotFound(name) => (
                JSONRPC_METHOD_NOT_FOUND,
                format!("unknown tool: {}", name),
                None,
            ),
            McpError::DbError(_) => (
                JSONRPC_DOMAIN_ERROR,
                "a database error occurred".to_string(),
                Some(json!({"retriable": true})),
            ),
            McpError::QueryError(_) => (
                JSONRPC_DOMAIN_ERROR,
                "a query error occurred".to_string(),
                Some(json!({"retriable": true})),
            ),
            McpError::Timeout => (
                JSONRPC_DOMAIN_ERROR,
                "query timed out".to_string(),
                Some(json!({"retriable": true})),
            ),
            McpError::IoError(_) => (
                JSONRPC_DOMAIN_ERROR,
                "an io error occurred".to_string(),
                Some(json!({"retriable": true})),
            ),
            McpError::JsonError(_) => (
                JSONRPC_INVALID_PARAMS,
                "malformed JSON".to_string(),
                None,
            ),
        }
    }
}
