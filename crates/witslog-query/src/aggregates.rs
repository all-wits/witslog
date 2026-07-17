use crate::error::Result;
use crate::filters::Filters;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Statistics {
    pub total: usize,
    pub by_severity: HashMap<String, usize>,
    pub by_category: HashMap<String, usize>,
    pub error_rate_per_day: f64,
    pub unique_fingerprints: usize,
    pub top_hosts: Vec<(String, usize)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineBucket {
    pub timestamp: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopFailure {
    pub fingerprint: String,
    pub title: String,
    pub count: usize,
    pub last_seen: String,
    pub category: Option<String>,
    pub sample_event_id: String,
}

/// Fingerprint-level MTTR — "time from first sighting to first fix", one row
/// per distinct failure. Deliberately NOT event-level: a fingerprint firing
/// hundreds of times before one fix would otherwise measure error volume and
/// call it recovery time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MttrStats {
    pub fingerprints_resolved: usize,
    pub fingerprints_unresolved: usize,
    /// `None` when no fingerprint has been resolved yet.
    pub mean_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FingerprintStats {
    pub fingerprint: String,
    pub title: String,
    pub count: usize,
    pub first_seen: String,
    pub last_seen: String,
    pub category: Option<String>,
    pub sample_event_id: String,
}

pub struct AggregateEngine<'a> {
    conn: &'a Connection,
}

impl<'a> AggregateEngine<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        AggregateEngine { conn }
    }

    /// Recurrence stats for a single fingerprint (used by `explain_error`).
    pub fn fingerprint_stats(&self, fingerprint: &str) -> Result<Option<FingerprintStats>> {
        let result = self.conn.query_row(
            "SELECT fingerprint, title, count, first_seen, last_seen, category, sample_event_id
             FROM fingerprints WHERE fingerprint = ?1",
            [fingerprint],
            |row| {
                Ok(FingerprintStats {
                    fingerprint: row.get(0)?,
                    title: row.get(1)?,
                    count: row.get(2)?,
                    first_seen: row.get(3)?,
                    last_seen: row.get(4)?,
                    category: row.get(5)?,
                    sample_event_id: row.get(6)?,
                })
            },
        );

        match result {
            Ok(stats) => Ok(Some(stats)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn statistics(&self, filters: &Filters) -> Result<Statistics> {
        let (filter_where, filter_params) = filters.to_sql();
        let param_refs: Vec<&dyn rusqlite::ToSql> = filter_params.iter().map(|p| p.as_ref()).collect();

        // Total count.
        let total: usize = self
            .conn
            .query_row(
                &format!("SELECT COUNT(*) FROM events WHERE {}", filter_where),
                param_refs.as_slice(),
                |row| row.get(0),
            )
            .unwrap_or(0);

        // By severity.
        let mut by_severity = HashMap::new();
        let mut stmt = self.conn.prepare(
            &format!(
                "SELECT severity, COUNT(*) FROM events WHERE {} GROUP BY severity",
                filter_where
            ),
        )?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
        })?;
        for row_result in rows {
            let (sev, count) = row_result?;
            by_severity.insert(sev, count);
        }

        // By category.
        let mut by_category = HashMap::new();
        let mut stmt = self.conn.prepare(
            &format!(
                "SELECT category, COUNT(*) FROM events WHERE {} AND category IS NOT NULL GROUP BY category",
                filter_where
            ),
        )?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
        })?;
        for row_result in rows {
            let (cat, count) = row_result?;
            by_category.insert(cat, count);
        }

        // Error rate per day (total / # of unique days).
        let days: usize = self
            .conn
            .query_row(
                &format!(
                    "SELECT COUNT(DISTINCT DATE(ts)) FROM events WHERE {}",
                    filter_where
                ),
                param_refs.as_slice(),
                |row| row.get(0),
            )
            .unwrap_or(1);

        let error_rate_per_day = if days > 0 {
            total as f64 / days as f64
        } else {
            0.0
        };

        // Unique fingerprints.
        let unique_fingerprints: usize = self
            .conn
            .query_row(
                &format!("SELECT COUNT(DISTINCT fingerprint) FROM events WHERE {}", filter_where),
                param_refs.as_slice(),
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Top hosts.
        let mut top_hosts = Vec::new();
        let mut stmt = self.conn.prepare(
            &format!(
                "SELECT hostname, COUNT(*) FROM events WHERE {} AND hostname IS NOT NULL GROUP BY hostname ORDER BY COUNT(*) DESC LIMIT 5",
                filter_where
            ),
        )?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
        })?;
        for row_result in rows {
            top_hosts.push(row_result?);
        }

        Ok(Statistics {
            total,
            by_severity,
            by_category,
            error_rate_per_day,
            unique_fingerprints,
            top_hosts,
        })
    }

    pub fn timeline(&self, filters: &Filters, bucket: &str) -> Result<Vec<TimelineBucket>> {
        let (filter_where, filter_params) = filters.to_sql();
        let param_refs: Vec<&dyn rusqlite::ToSql> = filter_params.iter().map(|p| p.as_ref()).collect();

        let time_format = match bucket {
            "hour" => "%Y-%m-%dT%H:00:00Z",
            "day" => "%Y-%m-%d",
            "week" => "%Y-W%W",
            _ => "%Y-%m-%d",
        };

        let sql = format!(
            "SELECT strftime('{}', ts), COUNT(*) FROM events WHERE {} GROUP BY strftime('{}', ts) ORDER BY ts",
            time_format, filter_where, time_format
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(TimelineBucket {
                timestamp: row.get(0)?,
                count: row.get(1)?,
            })
        })?;

        let mut result = Vec::new();
        for row_result in rows {
            result.push(row_result?);
        }

        Ok(result)
    }

    /// Fingerprint-level MTTR: for each fingerprint, `MIN(resolved_at) −
    /// MIN(ts)` among events matching `filters` — "time from first sighting
    /// to first fix". A fingerprint counts as resolved if any of its events
    /// (within the filter) has `resolved_at` set. Duration is computed in
    /// Rust from the parsed RFC3339 timestamps rather than SQL `julianday`,
    /// since `ts`/`resolved_at` are TEXT with no epoch-ms mirror.
    pub fn mttr(&self, filters: &Filters) -> Result<MttrStats> {
        let (filter_where, filter_params) = filters.to_sql();
        let param_refs: Vec<&dyn rusqlite::ToSql> = filter_params.iter().map(|p| p.as_ref()).collect();

        let sql = format!(
            "SELECT MIN(ts), MIN(CASE WHEN resolved_at IS NOT NULL THEN resolved_at END)
             FROM events WHERE {}
             GROUP BY fingerprint",
            filter_where
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;

        let mut fingerprints_resolved = 0usize;
        let mut fingerprints_unresolved = 0usize;
        let mut total_seconds = 0f64;

        for row_result in rows {
            let (first_seen, first_resolved) = row_result?;
            match first_resolved {
                Some(resolved) => {
                    fingerprints_resolved += 1;
                    if let (Ok(seen), Ok(res)) = (
                        chrono::DateTime::parse_from_rfc3339(&first_seen),
                        chrono::DateTime::parse_from_rfc3339(&resolved),
                    ) {
                        total_seconds += (res - seen).num_milliseconds() as f64 / 1000.0;
                    }
                }
                None => fingerprints_unresolved += 1,
            }
        }

        let mean_seconds = if fingerprints_resolved > 0 {
            Some(total_seconds / fingerprints_resolved as f64)
        } else {
            None
        };

        Ok(MttrStats {
            fingerprints_resolved,
            fingerprints_unresolved,
            mean_seconds,
        })
    }

    pub fn top_failures(&self, filters: &Filters, by: &str, limit: usize) -> Result<Vec<TopFailure>> {
        let (filter_where, filter_params) = filters.to_sql();
        let param_refs: Vec<&dyn rusqlite::ToSql> = filter_params.iter().map(|p| p.as_ref()).collect();

        let limit = limit.min(100).max(1);
        let limit_param = limit as i32;

        let order_by = match by {
            "count" => "COUNT(*) DESC",
            "recency" => "MAX(e.ts_epoch_ms) DESC",
            "severity" => "MAX(e.severity_rank) DESC",
            _ => "COUNT(*) DESC",
        };

        let sql = format!(
            "SELECT fp.fingerprint, fp.title, COUNT(*) as cnt, fp.last_seen, fp.category, fp.sample_event_id
             FROM fingerprints fp
             JOIN events e ON fp.fingerprint = e.fingerprint
             WHERE {}
             GROUP BY fp.fingerprint
             ORDER BY {}
             LIMIT ?",
            filter_where, order_by
        );

        let mut stmt = self.conn.prepare(&sql)?;

        // Add limit param.
        let mut all_params = param_refs;
        all_params.push(&limit_param);

        let rows = stmt.query_map(all_params.as_slice(), |row| {
            Ok(TopFailure {
                fingerprint: row.get(0)?,
                title: row.get(1)?,
                count: row.get(2)?,
                last_seen: row.get(3)?,
                category: row.get(4)?,
                sample_event_id: row.get(5)?,
            })
        })?;

        let mut result = Vec::new();
        for row_result in rows {
            result.push(row_result?);
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use witslog_store::Migrator;

    #[test]
    fn test_timeline_bucket() {
        let bucket = TimelineBucket {
            timestamp: "2026-07-12".to_string(),
            count: 42,
        };
        assert_eq!(bucket.count, 42);
    }

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        Migrator::new(&conn).migrate().unwrap();
        conn
    }

    fn insert_event(conn: &Connection, event_id: &str, fingerprint: &str, ts: &str, resolved_at: Option<&str>) {
        conn.execute(
            "INSERT INTO events (event_id, ts, ts_epoch_ms, application, severity, severity_rank, message, fingerprint, resolved_at, schema_v)
             VALUES (?1, ?2, 0, 'app', 'error', 50, 'boom', ?3, ?4, 1)",
            params![event_id, ts, fingerprint, resolved_at],
        )
        .unwrap();
    }

    /// Regression lock: MTTR is fingerprint-level, not event-level — a
    /// fingerprint that fires many times before a single fix should not
    /// count as many resolutions, and unresolved fingerprints must not
    /// pollute the mean.
    #[test]
    fn mttr_excludes_unresolved_fingerprints() {
        let conn = fresh_conn();
        // fp-1 fires 3x, fixed 1 hour after first sighting.
        insert_event(&conn, "e1", "fp-1", "2026-01-01T00:00:00.000Z", None);
        insert_event(&conn, "e2", "fp-1", "2026-01-01T00:30:00.000Z", None);
        insert_event(
            &conn,
            "e3",
            "fp-1",
            "2026-01-01T00:45:00.000Z",
            Some("2026-01-01T01:00:00.000Z"),
        );
        // fp-2 never resolved.
        insert_event(&conn, "e4", "fp-2", "2026-01-01T00:00:00.000Z", None);

        let agg = AggregateEngine::new(&conn);
        let mttr = agg.mttr(&Filters::default()).unwrap();

        assert_eq!(mttr.fingerprints_resolved, 1);
        assert_eq!(mttr.fingerprints_unresolved, 1);
        assert_eq!(mttr.mean_seconds, Some(3600.0));
    }

    #[test]
    fn mttr_mean_is_none_when_nothing_resolved() {
        let conn = fresh_conn();
        insert_event(&conn, "e1", "fp-1", "2026-01-01T00:00:00.000Z", None);

        let agg = AggregateEngine::new(&conn);
        let mttr = agg.mttr(&Filters::default()).unwrap();

        assert_eq!(mttr.fingerprints_resolved, 0);
        assert_eq!(mttr.fingerprints_unresolved, 1);
        assert_eq!(mttr.mean_seconds, None);
    }
}
