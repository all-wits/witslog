use thiserror::Error;

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("migration failed: {0}")]
    Migration(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid event: {0}")]
    InvalidEvent(String),

    #[error("schema version mismatch: {0}")]
    SchemaVersionMismatch(String),

    #[error("category '{0}' collides with an existing builtin category")]
    CategoryCollision(String),

    #[error("alias targets unknown canonical '{0}'")]
    UnknownCanonical(String),
}
