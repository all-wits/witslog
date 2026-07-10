use crate::error::Result;
use rusqlite::Connection;

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

        if current_version < 1 {
            self.migrate_0001_init()?;
            self.record_migration(1, "init")?;
        }

        if current_version < 2 {
            self.migrate_0002_resolved_at()?;
            self.record_migration(2, "resolved_at")?;
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
