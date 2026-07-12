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

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, McpError>;
