use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Default)]
pub struct Filters {
    pub application: Option<String>,
    pub version: Option<String>,
    pub environment: Option<String>,
    pub command: Option<String>,
    pub subsystem: Option<String>,
    pub hostname: Option<String>,
    pub severity_min: Option<String>,
    pub category: Option<String>,
    pub error_code: Option<String>,
    pub correlation_id: Option<String>,
    pub fingerprint: Option<String>,
    pub tags: Option<Vec<String>>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

impl Filters {
    /// Convert filters to SQL WHERE clause + bound params.
    /// Returns (where_clause, params) tuple.
    /// Uses positional ? placeholders (not numbered) to work with rusqlite.
    pub fn to_sql(&self) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
        let mut clauses = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref app) = self.application {
            clauses.push("application = ?".to_string());
            params.push(Box::new(app.clone()));
        }

        if let Some(ref ver) = self.version {
            clauses.push("version = ?".to_string());
            params.push(Box::new(ver.clone()));
        }

        if let Some(ref env) = self.environment {
            clauses.push("environment = ?".to_string());
            params.push(Box::new(env.clone()));
        }

        if let Some(ref cmd) = self.command {
            clauses.push("command = ?".to_string());
            params.push(Box::new(cmd.clone()));
        }

        if let Some(ref sub) = self.subsystem {
            clauses.push("subsystem = ?".to_string());
            params.push(Box::new(sub.clone()));
        }

        if let Some(ref host) = self.hostname {
            clauses.push("hostname = ?".to_string());
            params.push(Box::new(host.clone()));
        }

        if let Some(ref cat) = self.category {
            clauses.push("category = ?".to_string());
            params.push(Box::new(cat.clone()));
        }

        if let Some(ref code) = self.error_code {
            clauses.push("error_code = ?".to_string());
            params.push(Box::new(code.clone()));
        }

        if let Some(ref corr) = self.correlation_id {
            clauses.push("correlation_id = ?".to_string());
            params.push(Box::new(corr.clone()));
        }

        if let Some(ref fp) = self.fingerprint {
            clauses.push("fingerprint = ?".to_string());
            params.push(Box::new(fp.clone()));
        }

        if let Some(ref sev) = self.severity_min {
            let rank = severity_to_rank(sev);
            clauses.push("severity_rank >= ?".to_string());
            params.push(Box::new(rank as i32));
        }

        if let Some(ref from) = self.from {
            let ms = from.timestamp_millis();
            clauses.push("ts_epoch_ms >= ?".to_string());
            params.push(Box::new(ms));
        }

        if let Some(ref to) = self.to {
            let ms = to.timestamp_millis();
            clauses.push("ts_epoch_ms <= ?".to_string());
            params.push(Box::new(ms));
        }

        if let Some(ref tag_list) = self.tags {
            // Tags as JSON array — check if json_each finds any match.
            // For simplicity: OR together tag conditions.
            if !tag_list.is_empty() {
                let mut tag_clauses = Vec::new();
                for tag in tag_list {
                    tag_clauses.push("json_extract(tags, '$[*]') LIKE ?".to_string());
                    params.push(Box::new(format!("%{}%", tag)));
                }
                clauses.push(format!("({})", tag_clauses.join(" OR ")));
            }
        }

        (
            if clauses.is_empty() {
                "1=1".to_string()
            } else {
                clauses.join(" AND ")
            },
            params,
        )
    }
}

fn severity_to_rank(sev: &str) -> u32 {
    match sev {
        "trace" => 10,
        "debug" => 20,
        "info" => 30,
        "warn" => 40,
        "error" => 50,
        "critical" => 60,
        "fatal" => 70,
        _ => 30,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filters_to_sql_empty() {
        let f = Filters::default();
        let (where_clause, params) = f.to_sql();
        assert_eq!(where_clause, "1=1");
        assert_eq!(params.len(), 0);
    }

    #[test]
    fn test_filters_to_sql_application() {
        let f = Filters {
            application: Some("myapp".to_string()),
            ..Default::default()
        };
        let (where_clause, params) = f.to_sql();
        assert!(where_clause.contains("application = ?"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_filters_to_sql_severity() {
        let f = Filters {
            severity_min: Some("error".to_string()),
            ..Default::default()
        };
        let (where_clause, params) = f.to_sql();
        assert!(where_clause.contains("severity_rank >= ?"));
        assert_eq!(params.len(), 1);
    }
}
