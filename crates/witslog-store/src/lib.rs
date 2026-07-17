pub mod audit;
pub mod conn;
pub mod error;
pub mod migrate;
pub mod sink;
pub mod taxonomy;
pub mod writer;

pub use audit::{AuditBreak, AuditVerifyResult};
pub use conn::DbConnection;
pub use error::{Result, StoreError};
pub use migrate::{Migrator, CURRENT_SCHEMA_VERSION};
pub use sink::StoreSink;
pub use writer::{DeleteFilter, EventWriter};

use std::path::{Path, PathBuf};

pub struct Store {
    db_path: PathBuf,
    conn: DbConnection,
}

impl Store {
    pub fn open_or_create(db_path: impl AsRef<Path>) -> Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();

        let conn = DbConnection::open(&db_path)?;
        conn.migrate()?;

        Ok(Store { db_path, conn })
    }

    pub fn conn(&self) -> &DbConnection {
        &self.conn
    }

    pub fn path(&self) -> &Path {
        &self.db_path
    }
}
