use crate::error::{McpError, Result};
use crate::tools::Tool;
use rusqlite::Connection;
use serde_json::{json, Value};
use witslog_query::{AggregateEngine, Filters, SearchEngine};

pub struct ToolRegistry<'a> {
    conn: &'a Connection,
}

impl<'a> ToolRegistry<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        ToolRegistry { conn }
    }

    /// List all available tools.
    pub fn list_tools(&self) -> Vec<Tool> {
        Tool::builtin_tools()
    }

    /// Call a tool by name with params.
    pub fn call_tool(&self, name: &str, params: Value) -> Result<Value> {
        match name {
            "search_errors" => self.search_errors(params),
            "latest_errors" => self.latest_errors(params),
            "statistics" => self.statistics(params),
            "classify_error" => self.classify_error(params),
            "list_categories" => self.list_categories(params),
            "top_failures" => self.top_failures(params),
            _ => Err(McpError::MethodNotFound(name.to_string())),
        }
    }

    fn search_errors(&self, params: Value) -> Result<Value> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing 'query'".to_string()))?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .min(200) as usize;

        let cursor = params.get("cursor").and_then(|v| v.as_str()).map(|s| s.to_string());

        let application = params.get("application").and_then(|v| v.as_str()).map(|s| s.to_string());
        let severity_min = params.get("severity_min").and_then(|v| v.as_str()).map(|s| s.to_string());
        let category = params.get("category").and_then(|v| v.as_str()).map(|s| s.to_string());

        let filters = Filters {
            application,
            category,
            severity_min,
            ..Default::default()
        };

        let search = SearchEngine::new(&self.conn);
        let result = search.search(query, &filters, limit, cursor, true)
            .map_err(|e| McpError::QueryError(e.to_string()))?;

        Ok(json!({
            "items": result.items.iter().map(|e| {
                json!({
                    "event_id": e.event_id,
                    "application": e.application,
                    "message": e.message,
                    "severity": e.severity.as_str(),
                    "timestamp": e.timestamp,
                    "category": e.category
                })
            }).collect::<Vec<_>>(),
            "next_cursor": result.next_cursor,
            "total_estimate": result.total_estimate,
            "cursor_warning": result.cursor_warning
        }))
    }

    fn latest_errors(&self, params: Value) -> Result<Value> {
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .min(200) as usize;

        let filters = Filters::default();
        let search = SearchEngine::new(&self.conn);

        // Use "1=1" query to match all
        let result = search.search("*", &filters, limit, None, false)
            .map_err(|e| McpError::QueryError(e.to_string()))?;

        Ok(json!({
            "items": result.items.iter().map(|e| {
                json!({
                    "event_id": e.event_id,
                    "application": e.application,
                    "message": e.message,
                    "severity": e.severity.as_str(),
                    "timestamp": e.timestamp
                })
            }).collect::<Vec<_>>()
        }))
    }

    fn statistics(&self, params: Value) -> Result<Value> {
        let application = params.get("application").and_then(|v| v.as_str()).map(|s| s.to_string());
        let severity_min = params.get("severity_min").and_then(|v| v.as_str()).map(|s| s.to_string());

        let filters = Filters {
            application,
            severity_min,
            ..Default::default()
        };

        let agg = AggregateEngine::new(&self.conn);
        let stats = agg.statistics(&filters)
            .map_err(|e| McpError::QueryError(e.to_string()))?;

        Ok(json!({
            "total": stats.total,
            "by_severity": stats.by_severity,
            "by_category": stats.by_category,
            "error_rate_per_day": stats.error_rate_per_day,
            "unique_fingerprints": stats.unique_fingerprints
        }))
    }

    fn classify_error(&self, params: Value) -> Result<Value> {
        let message = params
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing 'message'".to_string()))?;

        let exception = params.get("exception").and_then(|v| v.as_str());
        let error_code = params.get("error_code").and_then(|v| v.as_str());

        let classifier = witslog_core::Classifier::built_in();
        let classification = classifier.classify(message, exception, error_code);

        Ok(json!({
            "category": classification.canonical,
            "matched_rules": classification.rule_ids,
            "suggested_tags": classification.suggested_tags
        }))
    }

    fn list_categories(&self, _params: Value) -> Result<Value> {
        let mut stmt = self.conn.prepare(
            "SELECT canonical, label, parent FROM categories ORDER BY canonical"
        ).map_err(|e| McpError::DbError(e.to_string()))?;

        let categories = stmt.query_map([], |row| {
            Ok(json!({
                "canonical": row.get::<_, String>(0)?,
                "label": row.get::<_, Option<String>>(1)?,
                "parent": row.get::<_, Option<String>>(2)?
            }))
        }).map_err(|e| McpError::DbError(e.to_string()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| McpError::DbError(e.to_string()))?;

        Ok(json!({
            "categories": categories
        }))
    }

    fn top_failures(&self, params: Value) -> Result<Value> {
        let by = params
            .get("by")
            .and_then(|v| v.as_str())
            .unwrap_or("count");

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(100) as usize;

        let filters = Filters::default();
        let agg = AggregateEngine::new(&self.conn);
        let results = agg.top_failures(&filters, by, limit)
            .map_err(|e| McpError::QueryError(e.to_string()))?;

        Ok(json!({
            "items": results.iter().map(|f| {
                json!({
                    "fingerprint": f.fingerprint,
                    "title": f.title,
                    "count": f.count,
                    "last_seen": f.last_seen,
                    "category": f.category
                })
            }).collect::<Vec<_>>()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_error_uses_error_code_not_stacktrace_slot() {
        let conn = Connection::open_in_memory().unwrap();
        let registry = ToolRegistry::new(&conn);

        let result = registry
            .call_tool("classify_error", json!({"message": "boom", "error_code": "ETIMEDOUT"}))
            .unwrap();

        assert_eq!(result["category"], "infrastructure.network.timeout");
        assert!(result["matched_rules"].as_array().unwrap().contains(&json!("builtin_etimedout")));
    }

    #[test]
    fn classify_error_no_match_returns_null_category() {
        let conn = Connection::open_in_memory().unwrap();
        let registry = ToolRegistry::new(&conn);

        let result = registry
            .call_tool("classify_error", json!({"message": "something weird"}))
            .unwrap();

        assert!(result["category"].is_null());
    }
}
