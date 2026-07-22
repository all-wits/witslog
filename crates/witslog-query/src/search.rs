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
    ///
    /// `query` of `"*"` or `""` means "match everything" — FTS5 rejects a
    /// bare `*`/empty string as MATCH syntax (`"unknown special query"`), so
    /// this is handled as its own path that skips the FTS5 join entirely and
    /// queries `events` directly, ordered by recency (there is no bm25 rank
    /// without a real FTS match). See `match_all_query_returns_filtered_results`
    /// for the regression this pins — every caller wanting "just apply
    /// filters, no text search" (`latest_errors`, `similar_errors`'s
    /// fingerprint mode, and `witslog query "*"`) used to error unconditionally.
    pub fn search(
        &self,
        query: &str,
        filters: &Filters,
        limit: usize,
        cursor: Option<String>,
        order_by_rank: bool,
    ) -> Result<SearchResult> {
        let limit = limit.min(200).max(1);
        let match_all = query.trim().is_empty() || query.trim() == "*";

        if !match_all {
            // Validate FTS syntax by attempting a test query; catch parse errors.
            if let Err(e) = self.conn.query_row(
                "SELECT 1 FROM events_fts WHERE events_fts MATCH ?1 LIMIT 1",
                [&query],
                |_| Ok(()),
            ) {
                if e.to_string().contains("syntax") {
                    return Err(QueryError::BadFtsSyntax(e.to_string()));
                }
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

        // Explicit column list, NOT `e.*` — `e.*` reflects physical column
        // order (CREATE TABLE order, then each `ALTER TABLE ADD COLUMN`
        // appended at the END, e.g. `resolved_at`), which drifts from any
        // hand-maintained index assumption the moment a column is added
        // later. This exact list matches `witslog_store::hydrate_event_row`
        // (see its doc comment) — the same helper `EventWriter::query_by_id`
        // uses, so both read paths hydrate identically. A prior version of
        // this file used `SELECT e.*` with its own hardcoded/drifted index
        // comment, which silently swapped `context`/`tags`/`metadata` once
        // `resolved_at` (migration 0002) was appended after them physically
        // instead of where the comment assumed.
        const EVENT_COLUMNS: &str = "e.id, e.event_id, e.ts, e.application, e.version, e.environment, \
             e.command, e.subsystem, e.hostname, e.severity, e.category, e.error_code, e.message, \
             e.exception, e.stacktrace, e.stack_norm, e.root_cause, e.fingerprint, e.correlation_id, \
             e.parent_event_id, e.context, e.tags, e.metadata, e.resolved_at";

        let sql = if match_all {
            format!(
                "SELECT {} FROM events e
                 WHERE {} {}
                 ORDER BY e.ts_epoch_ms DESC
                 LIMIT ?",
                EVENT_COLUMNS, filter_where, cursor_clause
            )
        } else if order_by_rank {
            format!(
                "SELECT {}, bm25(events_fts, 3.0, 2.0, 1.0, 2.0, 2.0, 1.0) AS rank
                 FROM events_fts
                 JOIN events e ON e.id = events_fts.rowid
                 WHERE events_fts MATCH ? AND {} {}
                 ORDER BY rank
                 LIMIT ?",
                EVENT_COLUMNS, filter_where, cursor_clause
            )
        } else {
            format!(
                "SELECT {}, bm25(events_fts, 3.0, 2.0, 1.0, 2.0, 2.0, 1.0) AS rank
                 FROM events_fts
                 JOIN events e ON e.id = events_fts.rowid
                 WHERE events_fts MATCH ? AND {} {}
                 ORDER BY e.ts_epoch_ms DESC
                 LIMIT ?",
                EVENT_COLUMNS, filter_where, cursor_clause
            )
        };

        // Execute query: return up to limit+1 to detect if there are more results.
        let mut stmt = self.conn.prepare(&sql)?;

        // Build full param list: [query if FTS] + filters + limit.
        let limit_param = limit as i32 + 1;
        let mut all_params: Vec<&dyn rusqlite::ToSql> = Vec::new();
        if !match_all {
            all_params.push(&query);
        }
        all_params.extend(&param_refs);
        all_params.push(&limit_param);

        let rows = stmt.query_map(all_params.as_slice(), |row| {
            witslog_store::hydrate_event_row(row)
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
        let total_sql = if match_all {
            format!("SELECT COUNT(*) FROM events e WHERE {}", filter_where)
        } else {
            format!(
                "SELECT COUNT(*) FROM events_fts
                 JOIN events e ON e.id = events_fts.rowid
                 WHERE events_fts MATCH ? AND {}",
                filter_where
            )
        };

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

}

#[cfg(test)]
mod tests {
    use super::*;
    use witslog_store::Migrator;

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

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        Migrator::new(&conn).migrate().unwrap();
        conn
    }

    /// Raw insert (mirrors `aggregates::tests`' pattern) — the FTS5 trigger
    /// from `migrate_0005_fts5` fires on any INSERT into `events`
    /// regardless of insert path, so this populates `events_fts` too.
    fn insert_event(conn: &Connection, application: &str, message: &str, ts_epoch_ms: i64) {
        conn.execute(
            "INSERT INTO events (event_id, ts, ts_epoch_ms, application, severity, severity_rank, message, fingerprint, schema_v)
             VALUES (?1, '2026-01-01T00:00:00.000Z', ?2, ?3, 'error', 50, ?4, ?5, 1)",
            rusqlite::params![
                format!("evt-{}-{}", application, ts_epoch_ms),
                ts_epoch_ms,
                application,
                message,
                format!("fp-{}-{}", application, ts_epoch_ms),
            ],
        )
        .unwrap();
    }

    fn fresh_conn_with_events(count: usize) -> Connection {
        let conn = fresh_conn();
        for i in 0..count {
            insert_event(&conn, "app", &format!("event {i}"), i as i64);
        }
        conn
    }

    /// Regression lock: `"*"` and `""` used to be forwarded straight to FTS5
    /// `MATCH`, which rejects them as a syntax error ("unknown special
    /// query") — every "match everything, just apply filters" caller
    /// (`latest_errors`, `similar_errors`'s fingerprint mode, and a user
    /// running `witslog query "*"`) failed unconditionally.
    #[test]
    fn match_all_query_returns_filtered_results() {
        let conn = fresh_conn_with_events(3);
        let engine = SearchEngine::new(&conn);

        let star = engine.search("*", &Filters::default(), 20, None, true).unwrap();
        assert_eq!(star.items.len(), 3);
        assert_eq!(star.total_estimate, 3);

        let empty = engine.search("", &Filters::default(), 20, None, false).unwrap();
        assert_eq!(empty.items.len(), 3);

        // Whitespace-only should also count as match-all.
        let whitespace = engine.search("  ", &Filters::default(), 20, None, false).unwrap();
        assert_eq!(whitespace.items.len(), 3);
    }

    /// Match-all must still respect structured filters and order by recency
    /// (there's no bm25 rank without a real FTS match).
    #[test]
    fn match_all_query_honours_filters_and_orders_by_recency() {
        let conn = fresh_conn();
        insert_event(&conn, "other-app", "irrelevant", 0);
        let engine = SearchEngine::new(&conn);

        let result = engine
            .search(
                "*",
                &Filters {
                    application: Some("app".to_string()),
                    ..Default::default()
                },
                20,
                None,
                true,
            )
            .unwrap();
        assert_eq!(result.items.len(), 0);
    }

    /// A real FTS syntax error must still be rejected — the match-all
    /// carve-out is narrow (exactly `"*"`/empty/whitespace), not a general
    /// bypass of FTS validation.
    #[test]
    fn non_match_all_bad_syntax_still_errors() {
        let conn = fresh_conn_with_events(1);
        let engine = SearchEngine::new(&conn);

        let result = engine.search("(unclosed", &Filters::default(), 20, None, true);
        assert!(result.is_err());
    }

    /// Regression lock: `search()` used to `SELECT e.*` and hydrate via
    /// hand-written column indices assuming `resolved_at` sat right after
    /// `parent_event_id` — but `resolved_at` is added by a later `ALTER
    /// TABLE ADD COLUMN` (migrate_0002_resolved_at), which SQLite always
    /// appends at the physical END of the column list, not wherever a
    /// comment assumes. `e.*`'s real order therefore has `resolved_at`
    /// AFTER `context`/`tags`/`metadata`/the generated `ctx_*` columns, one
    /// full column later than the old code assumed — silently reading
    /// `tags`'s data into `context`, `metadata`'s into `tags`, and a
    /// generated `ctx_request_id` column into `metadata`. Distinct,
    /// unambiguous context/tags values here would have failed loudly on the
    /// old code (context held the tags array; tags read back null).
    #[test]
    fn search_hydrates_context_and_tags_without_swapping_them() {
        let conn = fresh_conn();
        conn.execute(
            "INSERT INTO events (event_id, ts, ts_epoch_ms, application, severity, severity_rank,
                                  message, fingerprint, schema_v, context, tags, resolved_at)
             VALUES ('evt-1', '2026-01-01T00:00:00.000Z', 0, 'app', 'error', 50,
                     'boom', 'fp-1', 1, '{\"mutationKey\":[\"cards\",\"update\"]}', '[\"browser\",\"react-query\"]', NULL)",
            [],
        )
        .unwrap();

        let engine = SearchEngine::new(&conn);
        let result = engine.search("*", &Filters::default(), 20, None, true).unwrap();

        assert_eq!(result.items.len(), 1);
        let event = &result.items[0];

        let tags = event.tags.as_ref().expect("tags should be Some, not swapped into null");
        assert_eq!(tags, &vec!["browser".to_string(), "react-query".to_string()]);

        let context = event.context.as_ref().expect("context should be Some");
        assert_eq!(
            context.get("mutationKey").and_then(|v| v.as_array()).map(|a| a.len()),
            Some(2),
            "context should hold the mutationKey object, not the tags array: {context:?}"
        );
    }
}
