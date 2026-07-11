use crate::conn::DbConnection;
use crate::error::Result;
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
            "SELECT event_id, ts, application, version, environment, command, subsystem,
                    hostname, severity, category, error_code, message, exception,
                    stacktrace, stack_norm, root_cause, fingerprint, correlation_id,
                    parent_event_id, context, tags, metadata, resolved_at
             FROM events WHERE event_id = ?1",
            [event_id],
            |row| {
                let ts_str: String = row.get(1)?;
                let ts = chrono::DateTime::parse_from_rfc3339(&ts_str)
                    .ok()
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                Ok((
                    row.get::<_, String>(0)?,
                    ts,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, String>(16)?,
                    row.get::<_, Option<String>>(17)?,
                    row.get::<_, Option<String>>(18)?,
                    row.get::<_, Option<String>>(19)?,
                    row.get::<_, Option<String>>(20)?,
                    row.get::<_, Option<String>>(21)?,
                    row.get::<_, Option<String>>(22)?,
                ))
            },
        );

        match result {
            Ok((
                event_id,
                Some(timestamp),
                application,
                version,
                environment,
                command,
                subsystem,
                hostname,
                severity_str,
                category,
                error_code,
                message,
                exception,
                stacktrace,
                stack_norm,
                root_cause,
                fingerprint,
                correlation_id,
                parent_event_id,
                context_json,
                tags_json,
                metadata_json,
                resolved_at_str,
            )) => {
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

                let context = context_json.and_then(|j| serde_json::from_str(&j).ok());
                let tags = tags_json.and_then(|j| serde_json::from_str(&j).ok());
                let metadata = metadata_json.and_then(|j| serde_json::from_str(&j).ok());
                let resolved_at = resolved_at_str.and_then(|s| {
                    chrono::DateTime::parse_from_rfc3339(&s)
                        .ok()
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                });

                Ok(Some(Event {
                    event_id,
                    timestamp,
                    application,
                    version,
                    environment,
                    command,
                    subsystem,
                    hostname,
                    severity,
                    category,
                    error_code,
                    message,
                    exception,
                    stacktrace,
                    stack_norm,
                    root_cause,
                    fingerprint,
                    correlation_id,
                    parent_event_id,
                    resolved_at,
                    context,
                    tags,
                    metadata,
                }))
            }
            Ok(_) => Ok(None),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn mark_resolved(&self, event_id: &str) -> Result<()> {
        let conn = self.conn.conn();
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

        conn.execute(
            "UPDATE events SET resolved_at = ?1 WHERE event_id = ?2",
            rusqlite::params![now, event_id],
        )?;

        Ok(())
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

        for id in &ids {
            conn.execute("DELETE FROM events WHERE event_id = ?1", [id])?;
            conn.execute(
                "DELETE FROM error_edges WHERE src_event_id = ?1 OR dst_event_id = ?1",
                [id],
            )?;
        }

        Ok(ids)
    }
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
            event.category,
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
        &event.category,
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
