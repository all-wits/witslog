use crate::error::{McpError, Result};
use crate::schema;
use crate::tools::Tool;
use rusqlite::Connection;
use serde_json::{json, Value};
use witslog_query::{AggregateEngine, CorrelationEngine, Filters, SearchEngine};
use witslog_store::{DbConnection, DeleteFilter, EventWriter};

/// Dispatches MCP tool calls against a single project DB.
///
/// Read-only by construction unless `allow_write` is set, in which case
/// `witslog_delete` becomes callable (it still only ever deletes resolved
/// events unless `force:true` is passed). `attached` enables `search_all`
/// across those extra DB paths.
pub struct ToolRegistry<'a> {
    db: &'a DbConnection,
    allow_write: bool,
    attached: Vec<std::path::PathBuf>,
    /// FR-P9-004: env var name holding the metadata-encryption key, or
    /// `None` if not configured. See `ServerConfig::crypto_key_env`.
    crypto_key_env: Option<String>,
}

impl<'a> ToolRegistry<'a> {
    pub fn new(db: &'a DbConnection) -> Self {
        ToolRegistry {
            db,
            allow_write: false,
            attached: Vec::new(),
            crypto_key_env: None,
        }
    }

    pub fn with_allow_write(mut self, allow_write: bool) -> Self {
        self.allow_write = allow_write;
        self
    }

    pub fn with_attached(mut self, attached: Vec<std::path::PathBuf>) -> Self {
        self.attached = attached;
        self
    }

    pub fn with_crypto_key_env(mut self, crypto_key_env: Option<String>) -> Self {
        self.crypto_key_env = crypto_key_env;
        self
    }

    /// Resolves `crypto_key_env` to a `FieldCipher` for this call, or `None`
    /// if encryption isn't configured / the key can't be resolved right now.
    /// Unlike the write path, this is never fail-closed — a reader missing
    /// the key still gets the rest of the event; `event_detail` falls back
    /// to the `"<encrypted>"` placeholder for `metadata` alone.
    fn resolve_cipher(&self) -> Option<witslog_core::FieldCipher> {
        let var = self.crypto_key_env.as_ref()?;
        witslog_core::FieldCipher::from_env(var).ok().flatten()
    }

    /// List all available tools — `search_all` only when DBs are attached,
    /// `witslog_delete` only when write is allowed.
    pub fn list_tools(&self) -> Vec<Tool> {
        let mut tools = Tool::builtin_tools();
        if !self.attached.is_empty() {
            tools.push(Tool::search_all_tool());
        }
        if self.allow_write {
            tools.push(Tool::witslog_delete_tool());
        }
        tools
    }

    /// Call a tool by name with params. Validates params against the tool's
    /// JSON Schema first (FR-P5-003) before dispatching.
    pub fn call_tool(&self, name: &str, params: Value) -> Result<Value> {
        let tool = self
            .list_tools()
            .into_iter()
            .find(|t| t.name == name)
            .ok_or_else(|| McpError::MethodNotFound(name.to_string()))?;

        schema::validate(&tool.input_schema, &params).map_err(McpError::InvalidParams)?;

        match name {
            "search_errors" => self.search_errors(params),
            "latest_errors" => self.latest_errors(params),
            "summarize_errors" => self.summarize_errors(params),
            "statistics" => self.statistics(params),
            "classify_error" => self.classify_error(params),
            "explain_error" => self.explain_error(params),
            "similar_errors" => self.similar_errors(params),
            "list_categories" => self.list_categories(params),
            "timeline" => self.timeline(params),
            "top_failures" => self.top_failures(params),
            "mttr" => self.mttr(params),
            "list_traces" => self.list_traces(params),
            "get_event" => self.get_event(params),
            "search_all" => self.search_all(params),
            "witslog_delete" => self.witslog_delete(params),
            _ => Err(McpError::MethodNotFound(name.to_string())),
        }
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.conn()
    }

    fn common_filters(&self, params: &Value) -> Filters {
        let application = str_field(params, "application");
        let category = str_field(params, "category");
        let severity_min = str_field(params, "severity_min");
        let from = str_field(params, "from").and_then(|s| parse_time(&s));
        let to = str_field(params, "to").and_then(|s| parse_time(&s));
        let resolved = params.get("resolved").and_then(|v| v.as_bool());

        Filters {
            application,
            category,
            severity_min,
            from,
            to,
            resolved,
            ..Default::default()
        }
    }

    fn search_errors(&self, params: Value) -> Result<Value> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing 'query'".to_string()))?;

        let limit = limit_field(&params, "limit", 20, 200);
        let cursor = str_field(&params, "cursor");
        let order_by_rank = str_field(&params, "order").as_deref() != Some("time");

        let filters = self.common_filters(&params);

        let conn = self.conn();
        let search = SearchEngine::new(&conn);
        let result = search
            .search(query, &filters, limit, cursor, order_by_rank)
            .map_err(|e| McpError::QueryError(e.to_string()))?;

        Ok(json!({
            "items": result.items.iter().map(event_summary).collect::<Vec<_>>(),
            "next_cursor": result.next_cursor,
            "total_estimate": result.total_estimate,
            "cursor_warning": result.cursor_warning
        }))
    }

    fn latest_errors(&self, params: Value) -> Result<Value> {
        let limit = limit_field(&params, "limit", 20, 200);
        let cursor = str_field(&params, "cursor");
        let filters = self.common_filters(&params);

        let conn = self.conn();
        let search = SearchEngine::new(&conn);

        // Use "*" to match all — filters + time ordering do the real work.
        let result = search
            .search("*", &filters, limit, cursor, false)
            .map_err(|e| McpError::QueryError(e.to_string()))?;

        Ok(json!({
            "items": result.items.iter().map(event_summary).collect::<Vec<_>>(),
            "next_cursor": result.next_cursor
        }))
    }

    fn summarize_errors(&self, params: Value) -> Result<Value> {
        let filters = self.common_filters(&params);

        let conn = self.conn();
        let agg = AggregateEngine::new(&conn);
        let stats = agg
            .statistics(&filters)
            .map_err(|e| McpError::QueryError(e.to_string()))?;
        let top = agg
            .top_failures(&filters, "count", 10)
            .map_err(|e| McpError::QueryError(e.to_string()))?;

        Ok(json!({
            "total": stats.total,
            "by_group": {
                "category": stats.by_category,
                "severity": stats.by_severity,
            },
            "top_fingerprints": top.iter().map(|f| json!({
                "fingerprint": f.fingerprint,
                "count": f.count,
                "last_seen": f.last_seen
            })).collect::<Vec<_>>()
        }))
    }

    fn statistics(&self, params: Value) -> Result<Value> {
        let filters = self.common_filters(&params);

        let conn = self.conn();
        let agg = AggregateEngine::new(&conn);
        let stats = agg
            .statistics(&filters)
            .map_err(|e| McpError::QueryError(e.to_string()))?;

        Ok(json!({
            "total": stats.total,
            "by_severity": stats.by_severity,
            "by_category": stats.by_category,
            "error_rate_per_day": stats.error_rate_per_day,
            "unique_fingerprints": stats.unique_fingerprints,
            "top_hosts": stats.top_hosts
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

    fn explain_error(&self, params: Value) -> Result<Value> {
        let event_id = self.resolve_event_id(&params)?;

        let conn = self.conn();
        let corr = CorrelationEngine::new(&conn);
        let trace = corr
            .by_root_event_id(&event_id)
            .map_err(|e| McpError::QueryError(e.to_string()))?;

        let event = trace
            .ordered_events
            .iter()
            .find(|e| e.event_id == event_id)
            .cloned()
            .ok_or_else(|| McpError::InvalidParams("event not found".to_string()))?;

        let agg = AggregateEngine::new(&conn);
        let recurrence = agg
            .fingerprint_stats(&event.fingerprint)
            .map_err(|e| McpError::QueryError(e.to_string()))?;

        let root_cause = trace
            .ordered_events
            .iter()
            .find(|e| e.parent_event_id.is_none())
            .map(event_summary);

        Ok(json!({
            "event": event_detail(&event, self.resolve_cipher().as_ref()),
            "root_cause": root_cause,
            "chain": trace.ordered_events.iter().map(event_summary).collect::<Vec<_>>(),
            "edges": trace.edges.iter().map(|e| json!({
                "src_event_id": e.src_event_id,
                "dst_event_id": e.dst_event_id,
                "rel": e.rel
            })).collect::<Vec<_>>(),
            "recurrence": recurrence.as_ref().map(|r| json!({
                "count": r.count,
                "first_seen": r.first_seen,
                "last_seen": r.last_seen
            })),
            "category_path": event.category,
            "context": event.context
        }))
    }

    fn similar_errors(&self, params: Value) -> Result<Value> {
        let event_id = self.resolve_event_id(&params)?;
        let mode = str_field(&params, "mode").unwrap_or_else(|| "fingerprint".to_string());
        let limit = limit_field(&params, "limit", 20, 200);

        let source = EventWriter::new(self.db)
            .query_by_id(&event_id)
            .map_err(|e| McpError::DbError(e.to_string()))?
            .ok_or_else(|| McpError::InvalidParams("event not found".to_string()))?;

        let conn = self.conn();
        let search = SearchEngine::new(&conn);
        let result = if mode == "lexical" {
            search
                .search(&fts_safe(&source.message), &Filters::default(), limit, None, true)
                .map_err(|e| McpError::QueryError(e.to_string()))?
        } else {
            let filters = Filters {
                fingerprint: Some(source.fingerprint.clone()),
                ..Default::default()
            };
            search
                .search("*", &filters, limit, None, false)
                .map_err(|e| McpError::QueryError(e.to_string()))?
        };

        Ok(json!({
            "items": result.items.iter().filter(|e| e.event_id != event_id).map(event_summary).collect::<Vec<_>>(),
            "grouping": mode
        }))
    }

    fn list_categories(&self, _params: Value) -> Result<Value> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT canonical, label, parent FROM categories ORDER BY canonical")
            .map_err(|e| McpError::DbError(e.to_string()))?;

        let categories = stmt
            .query_map([], |row| {
                Ok(json!({
                    "canonical": row.get::<_, String>(0)?,
                    "label": row.get::<_, Option<String>>(1)?,
                    "parent": row.get::<_, Option<String>>(2)?
                }))
            })
            .map_err(|e| McpError::DbError(e.to_string()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| McpError::DbError(e.to_string()))?;

        Ok(json!({
            "tree": categories,
            "aliases": []
        }))
    }

    fn timeline(&self, params: Value) -> Result<Value> {
        let bucket = str_field(&params, "bucket").unwrap_or_else(|| "day".to_string());
        let filters = self.common_filters(&params);

        let conn = self.conn();
        let agg = AggregateEngine::new(&conn);
        let buckets = agg
            .timeline(&filters, &bucket)
            .map_err(|e| McpError::QueryError(e.to_string()))?;

        Ok(json!({
            "buckets": buckets.iter().map(|b| json!({"t": b.timestamp, "count": b.count})).collect::<Vec<_>>()
        }))
    }

    fn top_failures(&self, params: Value) -> Result<Value> {
        let by = str_field(&params, "by").unwrap_or_else(|| "count".to_string());
        let limit = limit_field(&params, "limit", 10, 100);

        // Regression lock (top_failures_honours_caller_filters): this used to
        // hardcode Filters::default(), silently ignoring every filter param
        // the caller passed (application/category/resolved/from/to).
        let filters = self.common_filters(&params);
        let conn = self.conn();
        let agg = AggregateEngine::new(&conn);
        let results = agg
            .top_failures(&filters, &by, limit)
            .map_err(|e| McpError::QueryError(e.to_string()))?;

        Ok(json!({
            "items": results.iter().map(|f| {
                json!({
                    "fingerprint": f.fingerprint,
                    "title": f.title,
                    "count": f.count,
                    "last_seen": f.last_seen,
                    "category": f.category,
                    "sample_event_id": f.sample_event_id
                })
            }).collect::<Vec<_>>()
        }))
    }

    /// Read-only. Fingerprint-level, not event-level (see `AggregateEngine::mttr`
    /// doc comment) — no MCP write tool for resolution exists (PLAN.md §5
    /// deliberately made `witslog_delete` the only write tool; a resolve tool
    /// would let an agent silently qualify rows for `witslog_delete`'s
    /// `resolved_at IS NOT NULL` default filter).
    fn mttr(&self, params: Value) -> Result<Value> {
        let filters = self.common_filters(&params);

        let conn = self.conn();
        let agg = AggregateEngine::new(&conn);
        let mttr = agg
            .mttr(&filters)
            .map_err(|e| McpError::QueryError(e.to_string()))?;

        Ok(json!({
            "fingerprints_resolved": mttr.fingerprints_resolved,
            "fingerprints_unresolved": mttr.fingerprints_unresolved,
            "mean_seconds": mttr.mean_seconds
        }))
    }

    fn list_traces(&self, params: Value) -> Result<Value> {
        let correlation_id = str_field(&params, "correlation_id");
        let root_event_id = str_field(&params, "root_event_id");

        if correlation_id.is_none() && root_event_id.is_none() {
            return Err(McpError::InvalidParams(
                "one of 'correlation_id' or 'root_event_id' is required".to_string(),
            ));
        }

        let conn = self.conn();
        let corr = CorrelationEngine::new(&conn);

        let trace = if let Some(cid) = correlation_id {
            corr.by_correlation_id(&cid)
        } else {
            corr.by_root_event_id(&root_event_id.unwrap())
        }
        .map_err(|e| McpError::QueryError(e.to_string()))?;

        Ok(json!({
            "ordered_events": trace.ordered_events.iter().map(event_summary).collect::<Vec<_>>(),
            "edges": trace.edges.iter().map(|e| json!({
                "src_event_id": e.src_event_id,
                "dst_event_id": e.dst_event_id,
                "rel": e.rel
            })).collect::<Vec<_>>()
        }))
    }

    fn search_all(&self, params: Value) -> Result<Value> {
        if self.attached.is_empty() {
            return Err(McpError::MethodNotFound("search_all".to_string()));
        }

        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing 'query'".to_string()))?;
        let limit = limit_field(&params, "limit", 20, 200);

        let mut items = Vec::new();
        let mut by_project = Vec::new();

        // Query the primary DB first.
        {
            let conn = self.conn();
            let search = SearchEngine::new(&conn);
            if let Ok(result) = search.search(query, &Filters::default(), limit, None, true) {
                by_project.push(json!({"source_db": "primary", "count": result.items.len()}));
                for e in &result.items {
                    let mut v = event_summary(e);
                    v["source_db"] = json!("primary");
                    items.push(v);
                }
            }
        }

        // Each attached DB is opened read-only, queried independently, and
        // closed — avoids holding N connections open for the server's
        // lifetime and keeps failures in one DB from poisoning the rest.
        for path in &self.attached {
            let label = path.display().to_string();
            let opened = Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
            );
            let Ok(conn) = opened else {
                by_project.push(json!({"source_db": label, "error": "could not open"}));
                continue;
            };
            let search = SearchEngine::new(&conn);
            match search.search(query, &Filters::default(), limit, None, true) {
                Ok(result) => {
                    by_project.push(json!({"source_db": label, "count": result.items.len()}));
                    for e in &result.items {
                        let mut v = event_summary(e);
                        v["source_db"] = json!(label.clone());
                        items.push(v);
                    }
                }
                Err(_) => {
                    by_project.push(json!({"source_db": label, "error": "query failed"}));
                }
            }
        }

        Ok(json!({
            "items": items,
            "by_project": by_project
        }))
    }

    fn witslog_delete(&self, params: Value) -> Result<Value> {
        if !self.allow_write {
            return Err(McpError::MethodNotFound("witslog_delete".to_string()));
        }

        let event_id = str_field(&params, "event_id");
        let fingerprint = str_field(&params, "fingerprint");
        let resolved_before = str_field(&params, "resolved_before");
        let force = params
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let dry_run = params
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if event_id.is_none() && fingerprint.is_none() && resolved_before.is_none() {
            return Err(McpError::InvalidParams(
                "one of 'event_id', 'fingerprint', or 'resolved_before' is required".to_string(),
            ));
        }

        let filter = DeleteFilter {
            event_id,
            fingerprint,
            resolved_before,
            force,
        };

        if dry_run {
            let preview = self.preview_delete(&filter)?;
            return Ok(json!({
                "deleted_count": 0,
                "deleted_ids": [],
                "would_delete_count": preview.len(),
                "would_delete_ids": preview,
                "dry_run": true
            }));
        }

        let writer = EventWriter::new(self.db);
        let ids = writer
            .delete_resolved(&filter)
            .map_err(|e| McpError::DbError(e.to_string()))?;

        Ok(json!({
            "deleted_count": ids.len(),
            "deleted_ids": ids,
            "dry_run": false
        }))
    }

    /// Read-only preview of what `delete_resolved` would delete, mirroring
    /// its WHERE clause exactly without mutating anything.
    fn preview_delete(&self, filter: &DeleteFilter) -> Result<Vec<String>> {
        let conn = self.conn();

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

        let sql = format!("SELECT event_id FROM events WHERE {}", clauses.join(" AND "));
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sql).map_err(|e| McpError::DbError(e.to_string()))?;
        let ids = stmt
            .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
            .map_err(|e| McpError::DbError(e.to_string()))?
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|e| McpError::DbError(e.to_string()))?;

        Ok(ids)
    }

    /// Full event payload by id — every field, unlike `event_summary`. The
    /// only MCP tool that returns a complete `Event` (mirrors CLI `get
    /// --json`); every other tool intentionally stays on the lean summary
    /// (see `event_summary` doc comment) so lists don't bloat with
    /// stacktraces/metadata.
    fn get_event(&self, params: Value) -> Result<Value> {
        let event_id = str_field(&params, "event_id")
            .ok_or_else(|| McpError::InvalidParams("missing 'event_id'".to_string()))?;

        let event = EventWriter::new(self.db)
            .query_by_id(&event_id)
            .map_err(|e| McpError::DbError(e.to_string()))?
            .ok_or_else(|| McpError::InvalidParams("event not found".to_string()))?;

        Ok(event_detail(&event, self.resolve_cipher().as_ref()))
    }

    /// Resolve `event_id`/`fingerprint` params to a concrete `event_id`,
    /// used by `explain_error` and `similar_errors`.
    fn resolve_event_id(&self, params: &Value) -> Result<String> {
        if let Some(id) = str_field(params, "event_id") {
            return Ok(id);
        }
        if let Some(fp) = str_field(params, "fingerprint") {
            let conn = self.conn();
            let agg = AggregateEngine::new(&conn);
            let stats = agg
                .fingerprint_stats(&fp)
                .map_err(|e| McpError::QueryError(e.to_string()))?
                .ok_or_else(|| McpError::InvalidParams("unknown fingerprint".to_string()))?;
            return Ok(stats.sample_event_id);
        }
        Err(McpError::InvalidParams(
            "one of 'event_id' or 'fingerprint' is required".to_string(),
        ))
    }
}

fn event_summary(e: &witslog_core::Event) -> Value {
    json!({
        "event_id": e.event_id,
        "application": e.application,
        "message": e.message,
        "severity": e.severity.as_str(),
        "timestamp": e.timestamp,
        "category": e.category,
        "fingerprint": e.fingerprint,
        "correlation_id": e.correlation_id,
        "parent_event_id": e.parent_event_id,
        "resolved_at": e.resolved_at
    })
}

/// Full event payload — every field, including exception/stacktrace/
/// error_code/root_cause/context/tags/metadata that `event_summary` drops.
/// Used by `get_event` and `explain_error`'s focal event; mirrors CLI `get
/// --json` (`witslog-cli/src/main.rs`'s `get_event`, `serde_json::to_string_pretty(&event)`).
///
/// FR-P9-004: `metadata` is decrypted in place when `cipher` is given and the
/// stored value is an encrypted envelope; otherwise (no cipher, or a cipher
/// that can't decrypt it) it's replaced with the `"<encrypted>"` placeholder
/// — see `witslog_core::crypto::decrypt_metadata_for_display`.
fn event_detail(e: &witslog_core::Event, cipher: Option<&witslog_core::FieldCipher>) -> Value {
    let mut value = serde_json::to_value(e).unwrap_or_else(|_| event_summary(e));
    if let Some(obj) = value.as_object_mut() {
        let displayed = witslog_core::decrypt_metadata_for_display(e.metadata.clone(), cipher);
        match displayed {
            Some(v) => {
                obj.insert("metadata".to_string(), v);
            }
            None => {
                obj.insert("metadata".to_string(), Value::Null);
            }
        }
    }
    value
}

fn str_field(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn limit_field(params: &Value, key: &str, default: usize, max: usize) -> usize {
    params
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(default)
        .min(max)
}

fn parse_time(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Escape an arbitrary message for use as an FTS5 MATCH query: wrap in
/// double quotes as a phrase and escape embedded quotes, so punctuation in
/// the source message can't be interpreted as FTS syntax.
fn fts_safe(message: &str) -> String {
    format!("\"{}\"", message.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> DbConnection {
        let db = DbConnection::open(":memory:").unwrap();
        db.migrate().unwrap();
        db
    }

    #[test]
    fn classify_error_uses_error_code_not_stacktrace_slot() {
        let db = setup();
        let registry = ToolRegistry::new(&db);

        let result = registry
            .call_tool("classify_error", json!({"message": "boom", "error_code": "ETIMEDOUT"}))
            .unwrap();

        assert_eq!(result["category"], "infrastructure.network.timeout");
        assert!(result["matched_rules"].as_array().unwrap().contains(&json!("builtin_etimedout")));
    }

    #[test]
    fn classify_error_no_match_returns_null_category() {
        let db = setup();
        let registry = ToolRegistry::new(&db);

        let result = registry
            .call_tool("classify_error", json!({"message": "something weird"}))
            .unwrap();

        assert!(result["category"].is_null());
    }

    #[test]
    fn invalid_params_rejected_before_dispatch() {
        let db = setup();
        let registry = ToolRegistry::new(&db);

        let err = registry.call_tool("classify_error", json!({})).unwrap_err();
        match err {
            McpError::InvalidParams(detail) => assert!(detail.contains("message")),
            other => panic!("expected InvalidParams, got {:?}", other),
        }
    }

    #[test]
    fn witslog_delete_absent_without_allow_write() {
        let db = setup();
        let registry = ToolRegistry::new(&db);

        assert!(!registry.list_tools().iter().any(|t| t.name == "witslog_delete"));

        let err = registry.call_tool("witslog_delete", json!({"event_id": "x"})).unwrap_err();
        assert!(matches!(err, McpError::MethodNotFound(_)));
    }

    #[test]
    fn witslog_delete_present_with_allow_write() {
        let db = setup();
        let registry = ToolRegistry::new(&db).with_allow_write(true);

        assert!(registry.list_tools().iter().any(|t| t.name == "witslog_delete"));
    }

    #[test]
    fn search_all_absent_without_attach() {
        let db = setup();
        let registry = ToolRegistry::new(&db);
        assert!(!registry.list_tools().iter().any(|t| t.name == "search_all"));
    }
}
