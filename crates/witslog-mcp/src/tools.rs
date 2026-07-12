use serde_json::{json, Value};

/// Tool definition in MCP format.
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl Tool {
    /// Builtin tools list per PHASES.md § P5.
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
                        "limit": {"type": "integer", "default": 20, "maximum": 200},
                        "cursor": {"type": "string"}
                    }
                }),
            },
            Tool {
                name: "latest_errors".to_string(),
                description: "Most recent failures.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "limit": {"type": "integer", "default": 20, "maximum": 200}
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
                        "severity_min": {"type": "string"}
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
                name: "list_categories".to_string(),
                description: "Taxonomy tree + counts.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            Tool {
                name: "top_failures".to_string(),
                description: "Ranked recurring failures.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "by": {"type": "string", "enum": ["count", "recency", "severity"], "default": "count"},
                        "limit": {"type": "integer", "default": 10, "maximum": 100}
                    }
                }),
            },
        ]
    }
}
