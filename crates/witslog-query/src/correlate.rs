use crate::error::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use witslog_core::Event;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub src_event_id: String,
    pub dst_event_id: String,
    pub rel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceResult {
    pub ordered_events: Vec<Event>,
    pub edges: Vec<Edge>,
}

pub struct CorrelationEngine<'a> {
    conn: &'a Connection,
}

impl<'a> CorrelationEngine<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        CorrelationEngine { conn }
    }

    /// Get all events for a correlation_id, ordered by timestamp.
    pub fn by_correlation_id(&self, correlation_id: &str) -> Result<TraceResult> {
        let mut events = Vec::new();

        let mut stmt = self.conn.prepare(
            "SELECT id, event_id, ts, application, version, environment, command, subsystem,
                    hostname, severity, category, error_code, message, exception,
                    stacktrace, stack_norm, root_cause, fingerprint, correlation_id,
                    parent_event_id, context, tags, metadata, resolved_at
             FROM events WHERE correlation_id = ?1 ORDER BY ts ASC"
        )?;

        let rows = stmt.query_map([correlation_id], |row| {
            self.hydrate_event(row)
        })?;

        for row_result in rows {
            events.push(row_result?);
        }

        // Fetch edges involving these events.
        let edges = self.fetch_edges_for_correlation_id(correlation_id)?;

        Ok(TraceResult { ordered_events: events, edges })
    }

    /// Get chain of caused-by events starting from a root event.
    pub fn by_root_event_id(&self, event_id: &str) -> Result<TraceResult> {
        let mut events = Vec::new();
        let mut edges = Vec::new();

        // Fetch root event.
        if let Some(root) = self.fetch_event_by_id(event_id)? {
            events.push(root);
        }

        // Walk the parent_event_id chain backwards (upwards).
        let mut current_id = event_id.to_string();
        loop {
            let parent_id: Option<String> = self.conn.query_row(
                "SELECT parent_event_id FROM events WHERE event_id = ?1",
                [&current_id],
                |row| row.get(0),
            ).ok().flatten();

            if let Some(parent) = parent_id {
                if let Some(parent_event) = self.fetch_event_by_id(&parent)? {
                    edges.push(Edge {
                        src_event_id: parent.clone(),
                        dst_event_id: current_id.clone(),
                        rel: "caused_by".to_string(),
                    });
                    events.insert(0, parent_event);
                    current_id = parent;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Walk children (reverse parent_event_id references).
        self.walk_children(event_id, &mut events, &mut edges)?;

        // Also fetch explicit error_edges.
        let explicit_edges = self.fetch_edges_for_event_id(event_id)?;
        edges.extend(explicit_edges);

        // Sort events by timestamp.
        events.sort_by_key(|e| e.timestamp);

        Ok(TraceResult { ordered_events: events, edges })
    }

    pub(crate) fn fetch_event_by_id(&self, event_id: &str) -> Result<Option<Event>> {
        let result = self.conn.query_row(
            "SELECT id, event_id, ts, application, version, environment, command, subsystem,
                    hostname, severity, category, error_code, message, exception,
                    stacktrace, stack_norm, root_cause, fingerprint, correlation_id,
                    parent_event_id, context, tags, metadata, resolved_at
             FROM events WHERE event_id = ?1",
            [event_id],
            |row| self.hydrate_event(row),
        );

        match result {
            Ok(event) => Ok(Some(event)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn walk_children(
        &self,
        parent_id: &str,
        events: &mut Vec<Event>,
        edges: &mut Vec<Edge>,
    ) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id FROM events WHERE parent_event_id = ?1 ORDER BY ts ASC"
        )?;

        let child_ids: Vec<String> = stmt.query_map([parent_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        for child_id in child_ids {
            if let Some(child_event) = self.fetch_event_by_id(&child_id)? {
                edges.push(Edge {
                    src_event_id: parent_id.to_string(),
                    dst_event_id: child_id.clone(),
                    rel: "caused_by".to_string(),
                });
                events.push(child_event);
                self.walk_children(&child_id, events, edges)?;
            }
        }

        Ok(())
    }

    fn fetch_edges_for_correlation_id(&self, correlation_id: &str) -> Result<Vec<Edge>> {
        let mut stmt = self.conn.prepare(
            "SELECT src_event_id, dst_event_id, rel FROM error_edges
             WHERE src_event_id IN (SELECT event_id FROM events WHERE correlation_id = ?1)
                OR dst_event_id IN (SELECT event_id FROM events WHERE correlation_id = ?1)"
        )?;

        let rows = stmt.query_map([correlation_id], |row| {
            Ok(Edge {
                src_event_id: row.get(0)?,
                dst_event_id: row.get(1)?,
                rel: row.get(2)?,
            })
        })?;

        let mut edges = Vec::new();
        for row_result in rows {
            edges.push(row_result?);
        }

        Ok(edges)
    }

    pub(crate) fn fetch_edges_for_event_id(&self, event_id: &str) -> Result<Vec<Edge>> {
        let mut stmt = self.conn.prepare(
            "SELECT src_event_id, dst_event_id, rel FROM error_edges
             WHERE src_event_id = ?1 OR dst_event_id = ?1"
        )?;

        let rows = stmt.query_map([event_id], |row| {
            Ok(Edge {
                src_event_id: row.get(0)?,
                dst_event_id: row.get(1)?,
                rel: row.get(2)?,
            })
        })?;

        let mut edges = Vec::new();
        for row_result in rows {
            edges.push(row_result?);
        }

        Ok(edges)
    }

    fn hydrate_event(&self, row: &rusqlite::Row) -> rusqlite::Result<Event> {
        let ts_str: String = row.get(2)?;
        let ts = chrono::DateTime::parse_from_rfc3339(&ts_str)
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|| chrono::Utc::now());

        let context = row.get::<_, Option<String>>(20)?
            .and_then(|j| serde_json::from_str(&j).ok());
        let tags = row.get::<_, Option<String>>(21)?
            .and_then(|j| serde_json::from_str(&j).ok());
        let metadata = row.get::<_, Option<String>>(22)?
            .and_then(|j| serde_json::from_str(&j).ok());

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

        let resolved_at = row.get::<_, Option<String>>(23)?
            .and_then(|s| {
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
}
