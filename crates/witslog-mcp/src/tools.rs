use serde_json::{json, Value};

/// Shared enum for `severity_min` across every tool schema that accepts it —
/// mirrors the severity taxonomy in `bindings/CONTRACT.md`'s `witslog_log`
/// payload table. Kept as a constant so every call site stays in sync.
const SEVERITY_ENUM: [&str; 7] = [
    "trace", "debug", "info", "warn", "error", "critical", "fatal",
];

fn severity_min_prop() -> Value {
    json!({
        "type": "string",
        "enum": SEVERITY_ENUM,
        "description": "Minimum severity floor (inclusive) — events at or above this level. Example: \"error\"."
    })
}

fn from_to_props() -> (Value, Value) {
    let prop = json!({
        "type": "string",
        "format": "date-time",
        "examples": ["2026-07-01T00:00:00Z"],
        "description": "RFC3339 timestamp."
    });
    (prop.clone(), prop)
}

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
        let (from_prop, to_prop) = from_to_props();
        vec![
            Tool {
                name: "search_errors".to_string(),
                description: "Lexical + structured search over logged error events. Requires a non-empty 'query' (FTS5 syntax: 'timeout*' prefix match, '\"exact phrase\"', 'db AND NOT retry' boolean). For browsing without a specific query, use latest_errors instead — it needs no query. Example: {query: \"timeout*\", severity_min: \"error\", limit: 20}.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "FTS5 expression.",
                            "examples": ["timeout*", "\"connection refused\"", "db AND NOT retry"]
                        },
                        "application": {"type": "string", "description": "Example: \"my-api\"."},
                        "category": {"type": "string", "description": "Taxonomy leaf, e.g. \"infrastructure.network.timeout\"."},
                        "severity_min": severity_min_prop(),
                        "from": from_prop.clone(),
                        "to": to_prop.clone(),
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
                description: "Most recent failures, newest first. No query needed — the default choice for 'show me recent errors' / 'list the errors'. Example: {limit: 20, severity_min: \"error\"}.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "application": {"type": "string", "description": "Example: \"my-api\"."},
                        "severity_min": severity_min_prop(),
                        "resolved": {"type": "boolean", "description": "true = resolved only, false = unresolved backlog"},
                        "limit": {"type": "integer", "default": 20, "maximum": 200},
                        "cursor": {"type": "string"}
                    }
                }),
            },
            Tool {
                name: "summarize_errors".to_string(),
                description: "Aggregate roll-up: total count plus counts grouped by category/severity, and top recurring fingerprints — a broad-strokes overview, not a list of individual events. Example: {application: \"my-api\", from: \"2026-07-01T00:00:00Z\"}.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "application": {"type": "string", "description": "Example: \"my-api\"."},
                        "severity_min": severity_min_prop(),
                        "category": {"type": "string"},
                        "from": from_prop.clone(),
                        "to": to_prop.clone(),
                        "group_by": {
                            "type": "array",
                            "items": {"type": "string", "enum": ["category", "subsystem", "version", "fingerprint", "host"]},
                            "description": "Example: [\"category\", \"fingerprint\"]."
                        }
                    }
                }),
            },
            Tool {
                name: "classify_error".to_string(),
                description: "Assign a taxonomy category to raw error text you already have in hand (not a stored event — for that use get_event/explain_error instead). Example: {message: \"connection timed out\", exception: \"TimeoutError\"}.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "message": {"type": "string", "description": "Example: \"connection timed out\"."},
                        "exception": {"type": "string", "description": "Example: \"TimeoutError\"."},
                        "error_code": {"type": "string", "description": "Example: \"ETIMEDOUT\"."}
                    },
                    "required": ["message"]
                }),
            },
            Tool {
                name: "explain_error".to_string(),
                description: "Full investigative dossier on one error: the event itself, its root cause, its full causality chain, recurrence stats, and category path — the richest single-error view. For just the raw stored fields use get_event; for other occurrences of the same/similar error use similar_errors; for other events in the same request use list_traces. Example: {event_id: \"01H...\"}.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "event_id": {"type": "string", "description": "Example: \"01H8X2QK3Z7VYB2R4M6N9P0ABC\"."},
                        "fingerprint": {"type": "string"}
                    }
                }),
            },
            Tool {
                name: "similar_errors".to_string(),
                description: "Find recurrences / near-duplicates of one error — either the same fingerprint (default) or a lexical match on message text. Not for a single error's full detail (use get_event/explain_error) or a request's other events (use list_traces). Example: {event_id: \"01H...\", mode: \"fingerprint\", limit: 20}.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "event_id": {"type": "string", "description": "Example: \"01H8X2QK3Z7VYB2R4M6N9P0ABC\"."},
                        "fingerprint": {"type": "string"},
                        "mode": {"type": "string", "enum": ["fingerprint", "lexical"], "default": "fingerprint"},
                        "limit": {"type": "integer", "default": 20, "maximum": 200}
                    }
                }),
            },
            Tool {
                name: "list_categories".to_string(),
                description: "Browse the taxonomy tree (e.g. infrastructure.network.timeout, application.validation). Example: {include_counts: true}.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "include_counts": {"type": "boolean"}
                    }
                }),
            },
            Tool {
                name: "statistics".to_string(),
                description: "Headline metrics for a window: total, by-severity, by-category, error rate/day, unique fingerprints, top hosts — single numbers/small breakdowns, not a ranked list (for that use top_failures). Example: {from: \"2026-07-01T00:00:00Z\", severity_min: \"error\"}.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "application": {"type": "string", "description": "Example: \"my-api\"."},
                        "severity_min": severity_min_prop(),
                        "resolved": {"type": "boolean"},
                        "from": from_prop.clone(),
                        "to": to_prop.clone()
                    }
                }),
            },
            Tool {
                name: "timeline".to_string(),
                description: "Bucketed event counts over time (hour/day/week) — a time series, for spotting trends/spikes. Example: {bucket: \"day\", from: \"2026-07-01T00:00:00Z\"}.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "application": {"type": "string", "description": "Example: \"my-api\"."},
                        "severity_min": severity_min_prop(),
                        "category": {"type": "string"},
                        "resolved": {"type": "boolean"},
                        "from": from_prop.clone(),
                        "to": to_prop.clone(),
                        "bucket": {"type": "string", "enum": ["hour", "day", "week"], "default": "day"}
                    }
                }),
            },
            Tool {
                name: "top_failures".to_string(),
                description: "Ranked list of recurring failures (by count/recency/severity) — the 'what's breaking the most' view, one row per fingerprint. Example: {by: \"count\", limit: 10}.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "application": {"type": "string", "description": "Example: \"my-api\"."},
                        "severity_min": severity_min_prop(),
                        "category": {"type": "string"},
                        "resolved": {"type": "boolean"},
                        "from": from_prop.clone(),
                        "to": to_prop.clone(),
                        "by": {"type": "string", "enum": ["count", "recency", "severity"], "default": "count"},
                        "limit": {"type": "integer", "default": 10, "maximum": 100}
                    }
                }),
            },
            Tool {
                name: "mttr".to_string(),
                description: "Fingerprint-level mean time-to-resolution: time from first sighting to first fix, averaged across fingerprints in the window — a single 'how fast do we fix things' number, not per-event data. Example: {from: \"2026-07-01T00:00:00Z\"}.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "application": {"type": "string", "description": "Example: \"my-api\"."},
                        "severity_min": severity_min_prop(),
                        "category": {"type": "string"},
                        "from": from_prop.clone(),
                        "to": to_prop.clone()
                    }
                }),
            },
            Tool {
                name: "list_traces".to_string(),
                description: "Events for a correlation/request id, or a caused-by chain from a root event — 'what else happened in this request/incident', not duplicates of the same error (use similar_errors for that). Requires exactly one of correlation_id or root_event_id. Example: {correlation_id: \"req-abc123\"}.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "correlation_id": {"type": "string", "description": "Example: \"req-abc123\"."},
                        "root_event_id": {"type": "string", "description": "Example: \"01H8X2QK3Z7VYB2R4M6N9P0ABC\"."}
                    }
                }),
            },
            Tool {
                name: "get_event".to_string(),
                description: "Full event payload by id — every field (exception, stacktrace, error_code, root_cause, context, tags, metadata), not just the search-result summary. Read-only. Use this when you already have an event_id (e.g. from search_errors/latest_errors) and need the raw detail; for an investigative dossier with root-cause/chain/recurrence use explain_error instead. Example: {event_id: \"01H8X2QK3Z7VYB2R4M6N9P0ABC\"}.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "event_id": {"type": "string", "description": "Example: \"01H8X2QK3Z7VYB2R4M6N9P0ABC\"."}
                    },
                    "required": ["event_id"]
                }),
            },
        ]
    }

    /// Schema for the opt-in `search_all` tool (only registered when the
    /// server is started with `--attach`).
    pub fn search_all_tool() -> Self {
        Tool {
            name: "search_all".to_string(),
            description: "Search across multiple attached project DBs (opt-in, --attach only). Same FTS5 query syntax as search_errors. Example: {query: \"timeout*\", limit: 20}."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "examples": ["timeout*", "\"connection refused\"", "db AND NOT retry"]
                    },
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
                "Delete stale/resolved error(s). Requires resolved_at IS NOT NULL unless force:true. dry_run defaults true — call once with dry_run:true to preview, then dry_run:false to actually delete. Provide exactly one of event_id/fingerprint/resolved_before. Example: {resolved_before: \"2026-01-01T00:00:00Z\", dry_run: true}."
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "event_id": {"type": "string", "description": "Example: \"01H8X2QK3Z7VYB2R4M6N9P0ABC\"."},
                    "fingerprint": {"type": "string"},
                    "resolved_before": {
                        "type": "string",
                        "format": "date-time",
                        "examples": ["2026-01-01T00:00:00Z"],
                        "description": "RFC3339 timestamp — delete events resolved before this time."
                    },
                    "force": {"type": "boolean", "default": false, "description": "Delete even if not resolved."},
                    "dry_run": {"type": "boolean", "default": true, "description": "Preview only, no rows deleted."}
                }
            }),
        }
    }
}
