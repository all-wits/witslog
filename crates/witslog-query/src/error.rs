use thiserror::Error;
use witslog_store::error::StoreError;

#[derive(Error, Debug)]
pub enum QueryError {
    #[error("FTS query syntax error: {0}")]
    BadFtsSyntax(String),

    #[error("Invalid date range: from > to")]
    BadRange,

    #[error("Database error: {0}")]
    DbError(#[from] rusqlite::Error),

    #[error("Serialization error: {0}")]
    SerdeError(#[from] serde_json::Error),

    #[error("Store error: {0}")]
    Store(#[from] StoreError),
}

pub type Result<T> = std::result::Result<T, QueryError>;
