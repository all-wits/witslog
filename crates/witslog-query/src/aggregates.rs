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

pub struct AggregateEngine<'a> {
    conn: &'a Connection,
}

impl<'a> AggregateEngine<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        AggregateEngine { conn }
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

    #[test]
    fn test_timeline_bucket() {
        let bucket = TimelineBucket {
            timestamp: "2026-07-12".to_string(),
            count: 42,
        };
        assert_eq!(bucket.count, 42);
    }
}
