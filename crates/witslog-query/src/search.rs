use crate::error::{QueryError, Result};
use crate::filters::Filters;
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use witslog_core::Event;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cursor {
    ts_epoch_ms: i64,
    id: i64,
}

impl Cursor {
    pub fn encode(&self) -> String {
        let json = serde_json::to_string(&self).unwrap();
        STANDARD.encode(&json)
    }

    pub fn decode(encoded: &str) -> Option<Self> {
        let bytes = STANDARD.decode(encoded).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub items: Vec<Event>,
    pub next_cursor: Option<String>,
    pub total_estimate: usize,
    /// Set when the supplied cursor was undecodable and the query fell back
    /// to the first page instead of erroring.
    #[serde(default)]
    pub cursor_warning: Option<String>,
}

pub struct SearchEngine<'a> {
    conn: &'a Connection,
}

impl<'a> SearchEngine<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        SearchEngine { conn }
    }

    /// Search events by FTS query + structured filters.
    /// Returns paginated results with bm25 ranking.
    pub fn search(
        &self,
        query: &str,
        filters: &Filters,
        limit: usize,
        cursor: Option<String>,
        order_by_rank: bool,
    ) -> Result<SearchResult> {
        let limit = limit.min(200).max(1);

        // Validate FTS syntax by attempting a test query.
        self.conn
            .query_row(
                "SELECT 1 FROM events_fts WHERE events_fts MATCH ?1 LIMIT 1",
                [&query],
                |_| Ok(()),
            )
            .ok(); // Ignore "no results" error, but catch parse errors.

        if let Err(e) = self.conn.query_row(
            "SELECT 1 FROM events_fts WHERE events_fts MATCH ?1 LIMIT 1",
            [&query],
            |_| Ok(()),
        ) {
            if e.to_string().contains("syntax") {
                return Err(QueryError::BadFtsSyntax(e.to_string()));
            }
        }

        // Validate date range.
        if let (Some(from), Some(to)) = (&filters.from, &filters.to) {
            if from > to {
                return Err(QueryError::BadRange);
            }
        }

        // Build filters SQL.
        let (filter_where, filter_params) = filters.to_sql();

        // Convert filter params to refs for the query.
        let param_refs: Vec<&dyn rusqlite::ToSql> = filter_params.iter().map(|p| p.as_ref()).collect();

        // Keyset pagination: if cursor provided, start after that row.
        // A tampered/undecodable cursor is ignored — fall back to page 1
        // rather than erroring — and surfaced via `SearchResult::cursor_warning`
        // so the caller (CLI/MCP) can report it however it likes.
        let mut cursor_warning = None;
        let cursor_clause = if let Some(cursor_str) = cursor {
            if let Some(cur) = Cursor::decode(&cursor_str) {
                format!("AND (ts_epoch_ms < {} OR (ts_epoch_ms = {} AND id < {}))",
                    cur.ts_epoch_ms, cur.ts_epoch_ms, cur.id)
            } else {
                cursor_warning = Some("cursor could not be decoded; returning first page".to_string());
                String::new()
            }
        } else {
            String::new()
        };

        // Search query: FTS + filters + bm25 ranking.
        let sql = if order_by_rank {
            format!(
                "SELECT e.*, bm25(events_fts, 3.0, 2.0, 1.0, 2.0, 2.0, 1.0) AS rank
                 FROM events_fts
                 JOIN events e ON e.id = events_fts.rowid
                 WHERE events_fts MATCH ? AND {} {}
                 ORDER BY rank
                 LIMIT ?",
                filter_where, cursor_clause
            )
        } else {
            format!(
                "SELECT e.*, bm25(events_fts, 3.0, 2.0, 1.0, 2.0, 2.0, 1.0) AS rank
                 FROM events_fts
                 JOIN events e ON e.id = events_fts.rowid
                 WHERE events_fts MATCH ? AND {} {}
                 ORDER BY e.ts_epoch_ms DESC
                 LIMIT ?",
                filter_where, cursor_clause
            )
        };

        // Execute query: return up to limit+1 to detect if there are more results.
        let mut stmt = self.conn.prepare(&sql)?;

        // Build full param list: query + filters + limit.
        let limit_param = limit as i32 + 1;
        let mut all_params: Vec<&dyn rusqlite::ToSql> = vec![&query];
        all_params.extend(&param_refs);
        all_params.push(&limit_param);

        let rows = stmt.query_map(all_params.as_slice(), |row| {
            self.hydrate_event(row)
        })?;

        let mut items = Vec::new();
        let mut has_more = false;

        for (idx, row_result) in rows.enumerate() {
            if idx >= limit {
                has_more = true;
                break;
            }
            items.push(row_result?);
        }

        // Get estimate of total matching rows (for frontend UI).
        let total_sql = format!(
            "SELECT COUNT(*) FROM events_fts
             JOIN events e ON e.id = events_fts.rowid
             WHERE events_fts MATCH ? AND {}",
            filter_where
        );

        let total_estimate: usize = self.conn.query_row(
            &total_sql,
            all_params[..all_params.len() - 1].as_ref(), // Exclude limit param.
            |row| row.get(0),
        ).unwrap_or(0);

        let next_cursor = if has_more && !items.is_empty() {
            let last = &items[items.len() - 1];
            Some(Cursor {
                ts_epoch_ms: last.timestamp.timestamp_millis(),
                id: last.id,
            }.encode())
        } else {
            None
        };

        Ok(SearchResult {
            items,
            next_cursor,
            total_estimate,
            cursor_warning,
        })
    }

    /// Hydrate an Event from a query result row.
    /// SELECT e.* from events gives columns in this order:
    /// id(0), event_id(1), ts(2), ts_epoch_ms(3), application(4), version(5),
    /// environment(6), command(7), subsystem(8), hostname(9), severity(10), severity_rank(11),
    /// category(12), error_code(13), message(14), exception(15), stacktrace(16), stack_norm(17),
    /// root_cause(18), fingerprint(19), correlation_id(20), parent_event_id(21), resolved_at(22),
    /// context(23), tags(24), metadata(25), ...generated columns, ingest_source, schema_v
    fn hydrate_event(&self, row: &rusqlite::Row) -> rusqlite::Result<Event> {
        let ts_str: String = row.get(2)?;
        let ts = chrono::DateTime::parse_from_rfc3339(&ts_str)
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|| chrono::Utc::now());

        let context = row.get::<_, Option<String>>(23)?
            .and_then(|j| serde_json::from_str(&j).ok());
        let tags = row.get::<_, Option<String>>(24)?
            .and_then(|j| serde_json::from_str(&j).ok());
        let metadata = row.get::<_, Option<String>>(25)?
            .and_then(|j| serde_json::from_str(&j).ok());

        let severity_str: String = row.get(10)?;
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

        let resolved_at = row.get::<_, Option<String>>(22)?
            .and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&chrono::Utc))
            });

        Ok(Event {
            id: row.get(0)?,
            event_id: row.get(1)?,
            timestamp: ts,
            application: row.get(4)?,
            version: row.get(5)?,
            environment: row.get(6)?,
            command: row.get(7)?,
            subsystem: row.get(8)?,
            hostname: row.get(9)?,
            severity,
            category: row.get(12)?,
            error_code: row.get(13)?,
            message: row.get(14)?,
            exception: row.get(15)?,
            stacktrace: row.get(16)?,
            stack_norm: row.get(17)?,
            root_cause: row.get(18)?,
            fingerprint: row.get(19)?,
            correlation_id: row.get(20)?,
            parent_event_id: row.get(21)?,
            resolved_at,
            context,
            tags,
            metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_encode_decode() {
        let cursor = Cursor {
            ts_epoch_ms: 1234567890,
            id: 42,
        };
        let encoded = cursor.encode();
        let decoded = Cursor::decode(&encoded).unwrap();
        assert_eq!(decoded.ts_epoch_ms, cursor.ts_epoch_ms);
        assert_eq!(decoded.id, cursor.id);
    }
}
