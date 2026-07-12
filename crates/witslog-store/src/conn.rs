use crate::error::Result;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::sync::Mutex;

pub struct DbConnection {
    conn: Mutex<Connection>,
}

impl DbConnection {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_URI;

        let conn = Connection::open_with_flags(path, flags)?;

        Self::setup_pragmas(&conn)?;

        Ok(DbConnection {
            conn: Mutex::new(conn),
        })
    }

    fn setup_pragmas(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            PRAGMA busy_timeout = 5000;
            PRAGMA temp_store = MEMORY;
            PRAGMA cache_size = -8000;
            PRAGMA wal_autocheckpoint = 1000;
            PRAGMA mmap_size = 268435456;
            "#,
        )?;

        Ok(())
    }

    /// Open an existing DB read-only (FR-P5-005). Never creates the file —
    /// the path must already exist. Used by `witslog serve-mcp` unless
    /// `--allow-write` is passed.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;

        let conn = Connection::open_with_flags(path, flags)?;

        // A subset of pragmas apply to read-only connections; harmless
        // no-ops otherwise (WAL mode is set once by whoever created the DB).
        conn.execute_batch(
            r#"
            PRAGMA busy_timeout = 5000;
            PRAGMA temp_store = MEMORY;
            "#,
        )?;

        Ok(DbConnection {
            conn: Mutex::new(conn),
        })
    }

    /// Install a per-call statement timeout (FR-P5-007): SQLite invokes the
    /// progress handler periodically while executing; once `timeout` has
    /// elapsed we return non-zero, which aborts the running statement with
    /// `SQLITE_INTERRUPT`.
    pub fn set_statement_timeout(&self, timeout: std::time::Duration) {
        let conn = self.conn();
        let deadline = std::time::Instant::now() + timeout;
        conn.progress_handler(1000, Some(move || std::time::Instant::now() >= deadline));
    }

    /// Remove any previously installed progress handler.
    pub fn clear_statement_timeout(&self) {
        let conn = self.conn();
        conn.progress_handler(0, None::<fn() -> bool>);
    }

    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }

    pub fn transaction<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let result = f(&tx)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn migrate(&self) -> crate::error::Result<()> {
        let sqlite_conn = self.conn();
        let mut migrator = crate::migrate::Migrator::new(&sqlite_conn);
        migrator.migrate()?;
        Ok(())
    }
}
