use crate::conn::DbConnection;
use crate::error::Result;
use rusqlite::OptionalExtension;
use witslog_core::Event;

pub struct EventWriter<'a> {
    conn: &'a DbConnection,
}

impl<'a> EventWriter<'a> {
    pub fn new(conn: &'a DbConnection) -> Self {
        EventWriter { conn }
    }

    pub fn write(&self, event: &Event) -> Result<i64> {
        let conn = self.conn.conn();
        write_event(&conn, event)
    }

    /// Write a batch of events in a single transaction (bench/high-throughput path).
    pub fn write_batch(&self, events: &[Event]) -> Result<()> {
        self.conn.transaction(|tx| {
            for event in events {
                write_event(tx, event)?;
            }
            Ok(())
        })
    }

    /// Best-effort bump of the lifetime dropped-events counter (mirrored from an
    /// in-process atomic — see `witslog-core::buffer`). Never surfaces failure to
    /// the caller beyond the `Result`; callers on the FFI/buffer hot path should
    /// swallow errors from this too.
    pub fn bump_dropped(&self, n: u64) -> Result<()> {
        let conn = self.conn.conn();
        conn.execute(
            "UPDATE runtime_stats SET value = value + ?1 WHERE key = 'dropped_events'",
            rusqlite::params![n as i64],
        )?;
        Ok(())
    }

    pub fn dropped_count(&self) -> Result<u64> {
        let conn = self.conn.conn();
        let value: i64 = conn.query_row(
            "SELECT value FROM runtime_stats WHERE key = 'dropped_events'",
            [],
            |row| row.get(0),
        )?;
        Ok(value.max(0) as u64)
    }

    pub fn query_by_id(&self, event_id: &str) -> Result<Option<Event>> {
        let conn = self.conn.conn();

        let result = conn.query_row(
            "SELECT id, event_id, ts, application, version, environment, command, subsystem,
                    hostname, severity, category, error_code, message, exception,
                    stacktrace, stack_norm, root_cause, fingerprint, correlation_id,
                    parent_event_id, context, tags, metadata, resolved_at
             FROM events WHERE event_id = ?1",
            [event_id],
            |row| hydrate_event_row(row),
        );

        match result {
            Ok(event) => Ok(Some(event)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Marks an event resolved. Guards `resolved_at IS NULL` unless `force`,
    /// so the first resolution wins and MTTR can't be moved by a re-resolve.
    /// Returns `false` when no row matched (unknown id, or already resolved
    /// without `force`).
    pub fn mark_resolved(&self, event_id: &str, force: bool) -> Result<bool> {
        let conn = self.conn.conn();
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

        let sql = if force {
            "UPDATE events SET resolved_at = ?1 WHERE event_id = ?2"
        } else {
            "UPDATE events SET resolved_at = ?1 WHERE event_id = ?2 AND resolved_at IS NULL"
        };

        let rows = conn.execute(sql, rusqlite::params![now, event_id])?;
        Ok(rows > 0)
    }

    /// Stream all events within an optional time range, ordered by ts ascending.
    /// Used by `export` — bounded memory via callback rather than a `Vec`.
    pub fn for_each_event<F>(&self, from_ms: Option<i64>, to_ms: Option<i64>, mut f: F) -> Result<()>
    where
        F: FnMut(Event),
    {
        let conn = self.conn.conn();

        let mut clauses = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(from) = from_ms {
            clauses.push(format!("ts_epoch_ms >= ?{}", params.len() + 1));
            params.push(Box::new(from));
        }
        if let Some(to) = to_ms {
            clauses.push(format!("ts_epoch_ms <= ?{}", params.len() + 1));
            params.push(Box::new(to));
        }

        let where_clause = if clauses.is_empty() {
            "1=1".to_string()
        } else {
            clauses.join(" AND ")
        };

        let sql = format!(
            "SELECT id, event_id, ts, application, version, environment, command, subsystem,
                    hostname, severity, category, error_code, message, exception,
                    stacktrace, stack_norm, root_cause, fingerprint, correlation_id,
                    parent_event_id, context, tags, metadata, resolved_at
             FROM events WHERE {} ORDER BY ts_epoch_ms ASC",
            where_clause
        );

        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| hydrate_event_row(row))?;

        for row_result in rows {
            f(row_result?);
        }

        Ok(())
    }

    /// Insert an event if its `event_id` doesn't already exist (idempotent import).
    /// Returns true if inserted, false if it was a duplicate.
    pub fn write_if_absent(&self, event: &Event) -> Result<bool> {
        let conn = self.conn.conn();

        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM events WHERE event_id = ?1",
                [&event.event_id],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if exists {
            return Ok(false);
        }

        write_event(&conn, event)?;
        Ok(true)
    }

    pub fn delete_resolved(&self, filter: &DeleteFilter) -> Result<Vec<String>> {
        let conn = self.conn.conn();

        let mut clauses: Vec<String> = vec!["resolved_at IS NOT NULL".to_string()];
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if filter.force {
            clauses[0] = "1=1".to_string();
        }

        if let Some(event_id) = &filter.event_id {
            clauses.push(format!("event_id = ?{}", params.len() + 1));
            params.push(Box::new(event_id.clone()));
        }

        if let Some(fingerprint) = &filter.fingerprint {
            clauses.push(format!("fingerprint = ?{}", params.len() + 1));
            params.push(Box::new(fingerprint.clone()));
        }

        if let Some(resolved_before) = &filter.resolved_before {
            clauses.push(format!("resolved_at <= ?{}", params.len() + 1));
            params.push(Box::new(resolved_before.clone()));
        }

        let sql = format!(
            "SELECT event_id FROM events WHERE {}",
            clauses.join(" AND ")
        );

        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let ids: Vec<String> = {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<String>>>()?
        };

        if ids.is_empty() {
            return Ok(ids);
        }

        delete_events_by_id(&conn, &ids)?;

        Ok(ids)
    }

    /// Deletes events by id, same tombstone-then-delete path as
    /// `delete_resolved`. Used by `prune`/`archive` so every row-removal path
    /// in the codebase keeps the audit chain bridgeable (FR-P10-001) — see
    /// `delete_events_by_id`.
    pub fn delete_by_ids(&self, event_ids: &[String]) -> Result<()> {
        let conn = self.conn.conn();
        delete_events_by_id(&conn, event_ids)
    }
}

/// Records a tombstone (the row's `audit_hash` at time of deletion) for each
/// event, then deletes the row and its `error_edges`. Recording the hash
/// before deleting lets `audit::verify_chain` bridge the id gap this leaves
/// behind, instead of reporting every subsequent row as tampered — this is
/// the single path all of `delete_resolved`/`prune`/`archive` must use (see
/// PLAN.md §1's "no component reaches around the store layer" rule).
pub(crate) fn delete_events_by_id(conn: &rusqlite::Connection, event_ids: &[String]) -> Result<()> {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

    for event_id in event_ids {
        let row: Option<(i64, Option<String>)> = conn
            .query_row(
                "SELECT id, audit_hash FROM events WHERE event_id = ?1",
                [event_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;

        let Some((row_id, audit_hash)) = row else {
            continue;
        };

        conn.execute(
            "INSERT OR REPLACE INTO audit_tombstones (row_id, event_id, audit_hash, deleted_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![row_id, event_id, audit_hash.unwrap_or_default(), now],
        )?;

        conn.execute("DELETE FROM events WHERE id = ?1", [row_id])?;
        conn.execute(
            "DELETE FROM error_edges WHERE src_event_id = ?1 OR dst_event_id = ?1",
            [event_id],
        )?;
    }

    Ok(())
}

/// Hydrates an `Event` from a row produced by the canonical column list:
/// id, event_id, ts, application, version, environment, command, subsystem,
/// hostname, severity, category, error_code, message, exception,
/// stacktrace, stack_norm, root_cause, fingerprint, correlation_id,
/// parent_event_id, context, tags, metadata, resolved_at
pub(crate) fn hydrate_event_row(row: &rusqlite::Row) -> rusqlite::Result<Event> {
    let ts_str: String = row.get(2)?;
    let ts = chrono::DateTime::parse_from_rfc3339(&ts_str)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    let severity_str: String = row.get(9)?;
    let severity = match severity_str.as_str() {
        "trace" => witslog_core::Severity::Trace,
        "debug" => witslog_core::Severity::Debug,
        "info" => witslog_core::Severity::Info,
        "warn" => witslog_core::Severity::Warn,
        "error" => witslog_core::Severity::Error,
        "critical" => witslog_core::Severity::Critical,
        "fatal" => witslog_core::Severity::Fatal,
        _ => witslog_core::Severity::Error,
    };

    let context = row
        .get::<_, Option<String>>(20)?
        .and_then(|j| serde_json::from_str(&j).ok());
    let tags = row
        .get::<_, Option<String>>(21)?
        .and_then(|j| serde_json::from_str(&j).ok());
    let metadata = row
        .get::<_, Option<String>>(22)?
        .and_then(|j| serde_json::from_str(&j).ok());
    let resolved_at = row.get::<_, Option<String>>(23)?.and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(&s)
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Utc))
    });

    Ok(Event {
        id: row.get(0)?,
        event_id: row.get(1)?,
        timestamp: ts,
        application: row.get(3)?,
        version: row.get(4)?,
        environment: row.get(5)?,
        command: row.get(6)?,
        subsystem: row.get(7)?,
        hostname: row.get(8)?,
        severity,
        category: row.get(10)?,
        error_code: row.get(11)?,
        message: row.get(12)?,
        exception: row.get(13)?,
        stacktrace: row.get(14)?,
        stack_norm: row.get(15)?,
        root_cause: row.get(16)?,
        fingerprint: row.get(17)?,
        correlation_id: row.get(18)?,
        parent_event_id: row.get(19)?,
        resolved_at,
        context,
        tags,
        metadata,
    })
}

/// Inserts one event and rolls it into the `fingerprints` table. Shared by
/// `EventWriter::write` (single-write path) and `StoreSink::write_batch`
/// (buffered/transactional batch path) so both go through identical SQL.
pub(crate) fn write_event(conn: &rusqlite::Connection, event: &Event) -> Result<i64> {
    let ts_epoch_ms = event.timestamp.timestamp_millis();
    let severity_rank = event.severity.rank();

    let context_json = event
        .context
        .as_ref()
        .map(|c| c.to_string())
        .or_else(|| Some("{}".to_string()));

    let tags_json = event
        .tags
        .as_ref()
        .map(|t| serde_json::to_string(t).unwrap_or_default());

    let metadata_json = event.metadata.as_ref().map(|m| m.to_string());

    // Resolve a category alias to its canonical form before persisting
    // (FR-P2-004). A category that's neither an alias nor a known canonical
    // is stored as-is — resolution is best-effort, not validation.
    let category: Option<String> = match &event.category {
        Some(cat) => Some(
            crate::taxonomy::resolve_alias(conn, cat)
                .ok()
                .flatten()
                .unwrap_or_else(|| cat.clone()),
        ),
        None => None,
    };

    conn.execute(
        "INSERT INTO events (
            event_id, ts, ts_epoch_ms, application, version, environment,
            command, subsystem, hostname, severity, severity_rank, category,
            error_code, message, exception, stacktrace, stack_norm, root_cause,
            fingerprint, correlation_id, parent_event_id, context, tags,
            metadata, ingest_source, schema_v
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
            ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26
        )",
        rusqlite::params![
            event.event_id,
            event.timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            ts_epoch_ms,
            event.application,
            event.version,
            event.environment,
            event.command,
            event.subsystem,
            event.hostname,
            event.severity.as_str(),
            severity_rank,
            category,
            event.error_code,
            event.message,
            event.exception,
            event.stacktrace,
            event.stack_norm,
            event.root_cause,
            event.fingerprint,
            event.correlation_id,
            event.parent_event_id,
            context_json,
            tags_json,
            metadata_json,
            "lib",
            1,
        ],
    )?;

    let row_id = conn.last_insert_rowid();

    update_fingerprint(
        conn,
        &event.fingerprint,
        &event.event_id,
        &event.message,
        &category,
    )?;

    let ts_str = event.timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    crate::audit::append(
        conn,
        row_id,
        &event.event_id,
        &ts_str,
        &event.message,
        &event.fingerprint,
    )?;

    Ok(row_id)
}

fn update_fingerprint(
    conn: &rusqlite::Connection,
    fingerprint: &str,
    event_id: &str,
    message: &str,
    category: &Option<String>,
) -> Result<()> {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

    conn.execute(
        "INSERT INTO fingerprints (fingerprint, first_seen, last_seen, sample_event_id, category, title)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(fingerprint) DO UPDATE SET
            last_seen = ?3, count = count + 1",
        rusqlite::params![fingerprint, now, now, event_id, category, message],
    )?;

    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct DeleteFilter {
    pub event_id: Option<String>,
    pub fingerprint: Option<String>,
    pub resolved_before: Option<String>,
    pub force: bool,
}
