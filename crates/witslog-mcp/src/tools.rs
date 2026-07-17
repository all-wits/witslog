use serde_json::{json, Value};

/// Tool definition in MCP format.
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl Tool {
    /// Builtin tools list per PHASES.md § P5 / PLAN.md § 5.
    ///
    /// `search_all` and `witslog_delete` are NOT in this list — they are
    /// opt-in (`--attach`) / write-gated (`--allow-write`) respectively, and
    /// appended by `ToolRegistry::list_tools` only when enabled.
    pub fn builtin_tools() -> Vec<Self> {
        vec![
            Tool {
                name: "search_errors".to_string(),
                description: "Lexical + structured search over logged error events.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "FTS5 expression"},
                        "application": {"type": "string"},
                        "category": {"type": "string"},
                        "severity_min": {"type": "string"},
                        "from": {"type": "string"},
                        "to": {"type": "string"},
                        "resolved": {"type": "boolean", "description": "true = resolved only, false = unresolved backlog"},
                        "order": {"type": "string", "enum": ["rank", "time"], "default": "rank"},
                        "limit": {"type": "integer", "default": 20, "maximum": 200},
                        "cursor": {"type": "string"}
                    },
                    "required": ["query"]
                }),
            },
            Tool {
                name: "latest_errors".to_string(),
                description: "Most recent failures.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "application": {"type": "string"},
                        "severity_min": {"type": "string"},
                        "resolved": {"type": "boolean", "description": "true = resolved only, false = unresolved backlog"},
                        "limit": {"type": "integer", "default": 20, "maximum": 200},
                        "cursor": {"type": "string"}
                    }
                }),
            },
            Tool {
                name: "summarize_errors".to_string(),
                description: "Aggregate roll-up (counts by group) for a window/filter.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "application": {"type": "string"},
                        "severity_min": {"type": "string"},
                        "category": {"type": "string"},
                        "from": {"type": "string"},
                        "to": {"type": "string"},
                        "group_by": {
                            "type": "array",
                            "items": {"type": "string", "enum": ["category", "subsystem", "version", "fingerprint", "host"]}
                        }
                    }
                }),
            },
            Tool {
                name: "classify_error".to_string(),
                description: "Assign taxonomy to raw error text.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "message": {"type": "string"},
                        "exception": {"type": "string"},
                        "error_code": {"type": "string"}
                    },
                    "required": ["message"]
                }),
            },
            Tool {
                name: "explain_error".to_string(),
                description: "Full dossier on one error: root cause, chain, recurrence, category path.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "event_id": {"type": "string"},
                        "fingerprint": {"type": "string"}
                    }
                }),
            },
            Tool {
                name: "similar_errors".to_string(),
                description: "Find recurrences / near-duplicates of an error.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "event_id": {"type": "string"},
                        "fingerprint": {"type": "string"},
                        "mode": {"type": "string", "enum": ["fingerprint", "lexical"], "default": "fingerprint"},
                        "limit": {"type": "integer", "default": 20, "maximum": 200}
                    }
                }),
            },
            Tool {
                name: "list_categories".to_string(),
                description: "Taxonomy tree + counts.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "include_counts": {"type": "boolean"}
                    }
                }),
            },
            Tool {
                name: "statistics".to_string(),
                description: "Headline metrics over a window.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "application": {"type": "string"},
                        "severity_min": {"type": "string"},
                        "resolved": {"type": "boolean"},
                        "from": {"type": "string"},
                        "to": {"type": "string"}
                    }
                }),
            },
            Tool {
                name: "timeline".to_string(),
                description: "Bucketed counts over time.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "application": {"type": "string"},
                        "severity_min": {"type": "string"},
                        "category": {"type": "string"},
                        "resolved": {"type": "boolean"},
                        "from": {"type": "string"},
                        "to": {"type": "string"},
                        "bucket": {"type": "string", "enum": ["hour", "day", "week"], "default": "day"}
                    }
                }),
            },
            Tool {
                name: "top_failures".to_string(),
                description: "Ranked recurring failures.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "application": {"type": "string"},
                        "severity_min": {"type": "string"},
                        "category": {"type": "string"},
                        "resolved": {"type": "boolean"},
                        "from": {"type": "string"},
                        "to": {"type": "string"},
                        "by": {"type": "string", "enum": ["count", "recency", "severity"], "default": "count"},
                        "limit": {"type": "integer", "default": 10, "maximum": 100}
                    }
                }),
            },
            Tool {
                name: "mttr".to_string(),
                description: "Fingerprint-level mean time-to-resolution: time from first sighting to first fix.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "application": {"type": "string"},
                        "severity_min": {"type": "string"},
                        "category": {"type": "string"},
                        "from": {"type": "string"},
                        "to": {"type": "string"}
                    }
                }),
            },
            Tool {
                name: "list_traces".to_string(),
                description: "Events for a correlation/request id, or a caused-by chain from a root event.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "correlation_id": {"type": "string"},
                        "root_event_id": {"type": "string"}
                    }
                }),
            },
        ]
    }

    /// Schema for the opt-in `search_all` tool (only registered when the
    /// server is started with `--attach`).
    pub fn search_all_tool() -> Self {
        Tool {
            name: "search_all".to_string(),
            description: "Search across multiple attached project DBs (opt-in, --attach only)."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "default": 20, "maximum": 200}
                },
                "required": ["query"]
            }),
        }
    }

    /// Schema for the write-capable `witslog_delete` tool (only registered
    /// when the server is started with `--allow-write`).
    pub fn witslog_delete_tool() -> Self {
        Tool {
            name: "witslog_delete".to_string(),
            description:
                "Delete stale/resolved error(s). Requires resolved_at IS NOT NULL unless force:true. dry_run defaults true."
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "event_id": {"type": "string"},
                    "fingerprint": {"type": "string"},
                    "resolved_before": {"type": "string"},
                    "force": {"type": "boolean", "default": false},
                    "dry_run": {"type": "boolean", "default": true}
                }
            }),
        }
    }
}
