use crate::error::{Result, StoreError};
use rusqlite::Connection;

/// Highest schema version this binary knows how to read/write (FR-P8-007).
/// Bump alongside adding a new `migrate_000N_*` step.
pub const CURRENT_SCHEMA_VERSION: i32 = 5;

pub struct Migrator<'a> {
    conn: &'a Connection,
}

impl<'a> Migrator<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Migrator { conn }
    }

    pub fn migrate(&mut self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS schema_meta (
              key   TEXT PRIMARY KEY,
              value TEXT NOT NULL
            );
            "#,
        )?;

        let current_version: i32 = self
            .conn
            .query_row(
                "SELECT COALESCE(value, '0') FROM schema_meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_else(|_| "0".to_string())
            .parse()
            .unwrap_or(0);

        // Version-compatibility guard (FR-P8-007): a DB stamped by a newer
        // binary carries schema features this binary doesn't understand.
        // Refuse rather than silently truncating/corrupting on write.
        if current_version > CURRENT_SCHEMA_VERSION {
            return Err(StoreError::SchemaVersionMismatch(format!(
                "database schema version {current_version} is newer than this binary supports (max {CURRENT_SCHEMA_VERSION}); upgrade witslog"
            )));
        }

        if current_version < 1 {
            self.migrate_0001_init()?;
            self.record_migration(1, "init")?;
        }

        if current_version < 2 {
            self.migrate_0002_resolved_at()?;
            self.record_migration(2, "resolved_at")?;
        }

        if current_version < 3 {
            self.migrate_0003_dropped_counter()?;
            self.record_migration(3, "dropped_counter")?;
        }

        if current_version < 4 {
            self.migrate_0004_seed_taxonomy()?;
            self.record_migration(4, "seed_taxonomy")?;
        }

        if current_version < 5 {
            self.migrate_0005_fts5()?;
            self.record_migration(5, "fts5")?;
        }

        Ok(())
    }

    fn migrate_0001_init(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS migrations (
              version     INTEGER PRIMARY KEY,
              name        TEXT NOT NULL,
              applied_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
              checksum    TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS events (
              id            INTEGER PRIMARY KEY,
              event_id      TEXT NOT NULL UNIQUE,
              ts            TEXT NOT NULL,
              ts_epoch_ms   INTEGER NOT NULL,

              application   TEXT NOT NULL,
              version       TEXT,
              environment   TEXT,
              command       TEXT,
              subsystem     TEXT,
              hostname      TEXT,

              severity      TEXT NOT NULL,
              severity_rank INTEGER NOT NULL,
              category      TEXT,
              error_code    TEXT,

              message       TEXT NOT NULL,
              exception     TEXT,
              stacktrace    TEXT,
              stack_norm    TEXT,
              root_cause    TEXT,

              fingerprint   TEXT NOT NULL,
              correlation_id TEXT,
              parent_event_id TEXT,

              context       TEXT,
              tags          TEXT,
              metadata      TEXT,

              ctx_request_id TEXT GENERATED ALWAYS AS (json_extract(context,'$.request_id')) VIRTUAL,
              ctx_git_commit TEXT GENERATED ALWAYS AS (json_extract(context,'$.git_commit')) VIRTUAL,
              ctx_pid        INTEGER GENERATED ALWAYS AS (json_extract(context,'$.pid')) VIRTUAL,
              ctx_duration_ms INTEGER GENERATED ALWAYS AS (json_extract(context,'$.duration_ms')) VIRTUAL,

              ingest_source TEXT DEFAULT 'lib',
              schema_v      INTEGER NOT NULL,

              CHECK (json_valid(context) OR context IS NULL),
              CHECK (json_valid(tags)    OR tags    IS NULL),
              CHECK (json_valid(metadata)OR metadata IS NULL)
            ) STRICT;

            CREATE INDEX IF NOT EXISTS ix_events_ts ON events(ts_epoch_ms DESC);
            CREATE INDEX IF NOT EXISTS ix_events_fp_ts ON events(fingerprint, ts_epoch_ms DESC);
            CREATE INDEX IF NOT EXISTS ix_events_cat_ts ON events(category, ts_epoch_ms DESC);
            CREATE INDEX IF NOT EXISTS ix_events_sub_ts ON events(subsystem, ts_epoch_ms DESC);
            CREATE INDEX IF NOT EXISTS ix_events_sev_ts ON events(severity_rank, ts_epoch_ms DESC);
            CREATE INDEX IF NOT EXISTS ix_events_app_ver_ts ON events(application, version, ts_epoch_ms DESC);
            CREATE INDEX IF NOT EXISTS ix_events_cmd_ts ON events(command, ts_epoch_ms DESC);
            CREATE INDEX IF NOT EXISTS ix_events_host_ts ON events(hostname, ts_epoch_ms DESC);
            CREATE INDEX IF NOT EXISTS ix_events_corr ON events(correlation_id) WHERE correlation_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS ix_events_parent ON events(parent_event_id) WHERE parent_event_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS ix_events_code_ts ON events(error_code, ts_epoch_ms DESC) WHERE error_code IS NOT NULL;
            CREATE INDEX IF NOT EXISTS ix_events_reqid ON events(ctx_request_id) WHERE ctx_request_id IS NOT NULL;

            -- FTS5 deferred to M4; placeholder for now
            -- CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(...);

            CREATE TABLE IF NOT EXISTS categories (
              canonical  TEXT PRIMARY KEY,
              parent     TEXT REFERENCES categories(canonical),
              label      TEXT,
              builtin    INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS category_aliases (
              alias      TEXT PRIMARY KEY,
              canonical  TEXT NOT NULL REFERENCES categories(canonical)
            );

            CREATE TABLE IF NOT EXISTS fingerprints (
              fingerprint  TEXT PRIMARY KEY,
              first_seen   TEXT NOT NULL,
              last_seen    TEXT NOT NULL,
              count        INTEGER NOT NULL DEFAULT 1,
              sample_event_id TEXT NOT NULL,
              category     TEXT,
              title        TEXT
            );

            CREATE INDEX IF NOT EXISTS ix_fp_last ON fingerprints(last_seen DESC);
            CREATE INDEX IF NOT EXISTS ix_fp_count ON fingerprints(count DESC);

            CREATE TABLE IF NOT EXISTS error_edges (
              src_event_id TEXT NOT NULL,
              dst_event_id TEXT NOT NULL,
              rel          TEXT NOT NULL,
              PRIMARY KEY (src_event_id, dst_event_id, rel)
            );

            CREATE INDEX IF NOT EXISTS ix_edges_dst ON error_edges(dst_event_id, rel);
            "#,
        )?;

        Ok(())
    }

    fn migrate_0002_resolved_at(&self) -> Result<()> {
        let has_column: bool = self
            .conn
            .prepare("SELECT 1 FROM pragma_table_info('events') WHERE name = 'resolved_at'")?
            .exists([])?;

        if !has_column {
            self.conn
                .execute_batch("ALTER TABLE events ADD COLUMN resolved_at TEXT;")?;
        }

        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS ix_events_unresolved ON events(ts_epoch_ms DESC) WHERE resolved_at IS NULL;",
        )?;

        Ok(())
    }

    fn migrate_0003_dropped_counter(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS runtime_stats (
              key   TEXT PRIMARY KEY,
              value INTEGER NOT NULL
            );
            INSERT OR IGNORE INTO runtime_stats (key, value) VALUES ('dropped_events', 0);
            "#,
        )?;

        Ok(())
    }

    fn migrate_0004_seed_taxonomy(&self) -> Result<()> {
        // Seed builtin categories from witslog_core::builtin_categories().
        // All inserted with builtin=1, idempotent via INSERT OR IGNORE.
        let categories = witslog_core::builtin_categories();

        for cat in categories {
            self.conn.execute(
                "INSERT OR IGNORE INTO categories (canonical, parent, label, builtin) VALUES (?1, ?2, ?3, 1)",
                rusqlite::params![&cat.canonical, &cat.parent, &cat.label],
            )?;
        }

        Ok(())
    }

    fn migrate_0005_fts5(&self) -> Result<()> {
        let fts_exists: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='events_fts'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !fts_exists {
            self.conn.execute_batch(
                r#"
                CREATE VIRTUAL TABLE events_fts USING fts5(
                  message, exception, stack_norm, root_cause, tags_text, category,
                  content='events', content_rowid='id',
                  tokenize = "unicode61 remove_diacritics 2 tokenchars '._:/-'"
                );
                "#,
            )?;

            self.conn.execute_batch(
                r#"
                CREATE TRIGGER events_ai AFTER INSERT ON events BEGIN
                  INSERT INTO events_fts(rowid,message,exception,stack_norm,root_cause,tags_text,category)
                  VALUES (new.id,new.message,new.exception,new.stack_norm,new.root_cause,
                          (SELECT group_concat(value,' ') FROM json_each(new.tags)), new.category);
                END;
                "#,
            )?;

            self.conn.execute_batch(
                r#"
                CREATE TRIGGER events_ad AFTER DELETE ON events BEGIN
                  INSERT INTO events_fts(events_fts,rowid,message,exception,stack_norm,root_cause,tags_text,category)
                  VALUES ('delete',old.id,old.message,old.exception,old.stack_norm,old.root_cause,'',old.category);
                END;
                "#,
            )?;

            // Backfill existing events into FTS index.
            self.conn.execute_batch(
                r#"
                INSERT INTO events_fts(rowid,message,exception,stack_norm,root_cause,tags_text,category)
                SELECT events.id,events.message,events.exception,events.stack_norm,events.root_cause,
                       COALESCE(group_concat(value,' '),''),events.category
                FROM events
                LEFT JOIN json_each(events.tags)
                GROUP BY events.id;
                "#,
            )?;
        }

        Ok(())
    }

    fn record_migration(&self, version: i32, name: &str) -> Result<()> {
        let checksum = format!("init-{}", version);
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

        self.conn.execute(
            "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', ?1)",
            rusqlite::params![version.to_string()],
        )?;

        self.conn.execute(
            "INSERT INTO migrations (version, name, applied_at, checksum) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![version, name, now, checksum],
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_db_migrates_to_current_version() {
        let conn = Connection::open_in_memory().unwrap();
        let mut migrator = Migrator::new(&conn);
        migrator.migrate().unwrap();

        let version: i32 = conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn re_running_migrate_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        Migrator::new(&conn).migrate().unwrap();
        // Second run must not error or duplicate migrations rows.
        Migrator::new(&conn).migrate().unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM migrations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, CURRENT_SCHEMA_VERSION as i64);
    }

    #[test]
    fn schema_newer_than_binary_is_refused() {
        let conn = Connection::open_in_memory().unwrap();
        Migrator::new(&conn).migrate().unwrap();

        // Simulate a DB stamped by a future binary.
        conn.execute(
            "UPDATE schema_meta SET value = ?1 WHERE key = 'schema_version'",
            rusqlite::params![(CURRENT_SCHEMA_VERSION + 1).to_string()],
        )
        .unwrap();

        let err = Migrator::new(&conn).migrate().unwrap_err();
        match err {
            StoreError::SchemaVersionMismatch(msg) => {
                assert!(msg.contains("newer than this binary supports"));
            }
            other => panic!("expected SchemaVersionMismatch, got {:?}", other),
        }
    }
}
