//! Conformance tests for P5 (MCP server) — PHASES.md § P5.
//!
//! Drives `ToolRegistry` directly (the same dispatch surface
//! `witslog_mcp::server::serve_stdio` calls into for `tools/call`), plus a
//! couple of `Tool::builtin_tools()` shape checks that stand in for
//! `tools/list`. This avoids spawning a subprocess while still exercising
//! the exact JSON contract external MCP clients depend on.

use serde_json::json;
use witslog_core::{EventBuilder, Severity};
use witslog_mcp::{tools::Tool, McpError, ToolRegistry};
use witslog_store::{DbConnection, EventWriter};

fn setup_db_with_events() -> DbConnection {
    let db = DbConnection::open(":memory:").unwrap();
    db.migrate().unwrap();

    let writer = EventWriter::new(&db);

    for i in 0..3 {
        let event = EventBuilder::new("checkout-svc", format!("payment gateway timeout #{}", i))
            .severity(Severity::Error)
            .category("infrastructure.network.timeout")
            .build();
        writer.write(&event).unwrap();
    }

    let resolved = EventBuilder::new("checkout-svc", "stale cache entry")
        .severity(Severity::Warn)
        .build();
    writer.write(&resolved).unwrap();
    writer.mark_resolved(&resolved.event_id, false).unwrap();

    db
}

#[test]
fn tools_list_reports_all_required_tools_with_valid_schemas() {
    let required = [
        "search_errors",
        "latest_errors",
        "summarize_errors",
        "classify_error",
        "explain_error",
        "similar_errors",
        "list_categories",
        "statistics",
        "timeline",
        "top_failures",
        "list_traces",
    ];

    let tools = Tool::builtin_tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();

    for r in required {
        assert!(names.contains(&r), "missing required tool: {}", r);
    }

    for tool in &tools {
        assert!(tool.input_schema.is_object(), "{} schema must be an object", tool.name);
        assert_eq!(
            tool.input_schema.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "{} schema type must be 'object'",
            tool.name
        );
        assert!(!tool.description.is_empty(), "{} needs a description", tool.name);
    }
}

#[test]
fn search_all_hidden_unless_attached_witslog_delete_hidden_unless_allow_write() {
    let db = setup_db_with_events();

    let default_registry = ToolRegistry::new(&db);
    let default_names: Vec<String> = default_registry.list_tools().into_iter().map(|t| t.name).collect();
    assert!(!default_names.contains(&"search_all".to_string()));
    assert!(!default_names.contains(&"witslog_delete".to_string()));

    let write_registry = ToolRegistry::new(&db).with_allow_write(true);
    let write_names: Vec<String> = write_registry.list_tools().into_iter().map(|t| t.name).collect();
    assert!(write_names.contains(&"witslog_delete".to_string()));

    let attach_registry = ToolRegistry::new(&db).with_attached(vec!["other.db".into()]);
    let attach_names: Vec<String> = attach_registry.list_tools().into_iter().map(|t| t.name).collect();
    assert!(attach_names.contains(&"search_all".to_string()));
}

#[test]
fn search_errors_returns_ranked_paginated_shape() {
    let db = setup_db_with_events();
    let registry = ToolRegistry::new(&db);

    let result = registry
        .call_tool("search_errors", json!({"query": "timeout", "limit": 10}))
        .expect("search_errors should succeed");

    let items = result["items"].as_array().expect("items must be an array");
    assert!(!items.is_empty(), "expected at least one match for 'timeout'");
    for item in items {
        assert!(item["event_id"].is_string());
        assert!(item["message"].is_string());
        assert!(item["severity"].is_string());
    }
    assert!(result.get("next_cursor").is_some());
    assert!(result.get("total_estimate").is_some());
}

#[test]
fn explain_error_dossier_has_recurrence_and_category_path() {
    let db = setup_db_with_events();
    let writer = EventWriter::new(&db);

    // Grab a real event_id via a direct query through the writer (the same
    // one search_errors would surface) rather than hardcoding one.
    let registry = ToolRegistry::new(&db);
    let found = registry
        .call_tool("search_errors", json!({"query": "timeout", "limit": 1}))
        .unwrap();
    let event_id = found["items"][0]["event_id"].as_str().unwrap().to_string();
    assert!(writer.query_by_id(&event_id).unwrap().is_some());

    let dossier = registry
        .call_tool("explain_error", json!({"event_id": event_id}))
        .expect("explain_error should succeed");

    assert_eq!(dossier["event"]["event_id"], json!(event_id));
    assert!(dossier.get("chain").is_some());
    assert!(dossier.get("recurrence").is_some());
    assert_eq!(dossier["category_path"], json!("infrastructure.network.timeout"));
}

#[test]
fn statistics_and_list_categories_return_valid_shapes() {
    let db = setup_db_with_events();
    let registry = ToolRegistry::new(&db);

    let stats = registry.call_tool("statistics", json!({})).unwrap();
    assert!(stats["total"].as_u64().unwrap() >= 3);
    assert!(stats.get("by_severity").is_some());
    assert!(stats.get("by_category").is_some());

    let categories = registry.call_tool("list_categories", json!({})).unwrap();
    let tree = categories["tree"].as_array().expect("tree must be an array");
    assert!(!tree.is_empty());
}

#[test]
fn invalid_params_return_invalid_params_error_with_no_sql_leakage() {
    let db = setup_db_with_events();
    let registry = ToolRegistry::new(&db);

    // classify_error requires 'message'.
    let err = registry.call_tool("classify_error", json!({})).unwrap_err();
    match err {
        McpError::InvalidParams(detail) => {
            let lower = detail.to_lowercase();
            assert!(!lower.contains("select"), "detail must not leak SQL: {}", detail);
            assert!(!lower.contains("from events"), "detail must not leak SQL: {}", detail);
        }
        other => panic!("expected InvalidParams, got {:?}", other),
    }

    let (code, message, _data) = err_to_jsonrpc(&db);
    assert_eq!(code, -32602);
    assert!(!message.to_lowercase().contains("select"));
}

/// Re-derive the same InvalidParams case through the public `to_jsonrpc`
/// mapping to assert the wire-level error code, independent of the
/// `McpError` variant match above.
fn err_to_jsonrpc(db: &DbConnection) -> (i32, String, Option<serde_json::Value>) {
    let registry = ToolRegistry::new(db);
    let err = registry
        .call_tool("classify_error", json!({}))
        .expect_err("must fail validation");
    err.to_jsonrpc()
}

#[test]
fn witslog_delete_absent_by_default_present_and_gated_with_allow_write() {
    let db = setup_db_with_events();

    let default_registry = ToolRegistry::new(&db);
    let err = default_registry
        .call_tool("witslog_delete", json!({"event_id": "does-not-exist"}))
        .unwrap_err();
    assert!(matches!(err, McpError::MethodNotFound(_)));

    // With write allowed: dry_run defaults true, so nothing is actually
    // deleted, and only resolved events are eligible without force:true.
    let write_registry = ToolRegistry::new(&db).with_allow_write(true);

    let resolved_search = write_registry
        .call_tool("search_errors", json!({"query": "cache"}))
        .unwrap();
    let resolved_id = resolved_search["items"][0]["event_id"].as_str().unwrap().to_string();

    let dry_run_result = write_registry
        .call_tool("witslog_delete", json!({"event_id": resolved_id, "dry_run": true}))
        .unwrap();
    assert_eq!(dry_run_result["dry_run"], json!(true));
    assert_eq!(dry_run_result["deleted_count"], json!(0));
    assert_eq!(dry_run_result["would_delete_count"], json!(1));

    // The dry run must not have deleted anything.
    let writer = EventWriter::new(&db);
    assert!(writer.query_by_id(&resolved_id).unwrap().is_some());

    let real_result = write_registry
        .call_tool("witslog_delete", json!({"event_id": resolved_id, "dry_run": false}))
        .unwrap();
    assert_eq!(real_result["deleted_count"], json!(1));
    assert!(writer.query_by_id(&resolved_id).unwrap().is_none());
}

#[test]
fn witslog_delete_rejects_unresolved_event_without_force() {
    let db = setup_db_with_events();
    let write_registry = ToolRegistry::new(&db).with_allow_write(true);

    let search = write_registry
        .call_tool("search_errors", json!({"query": "timeout", "limit": 1}))
        .unwrap();
    let unresolved_id = search["items"][0]["event_id"].as_str().unwrap().to_string();

    let result = write_registry
        .call_tool(
            "witslog_delete",
            json!({"event_id": unresolved_id, "dry_run": false}),
        )
        .unwrap();

    // Not resolved and no force:true -> nothing eligible for deletion.
    assert_eq!(result["deleted_count"], json!(0));

    let writer = EventWriter::new(&db);
    assert!(writer.query_by_id(&unresolved_id).unwrap().is_some());
}
