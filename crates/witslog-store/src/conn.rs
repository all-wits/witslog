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
