use tempfile::TempDir;
use witslog_core::{EventBuilder, Severity};
use witslog_query::{
    AggregateEngine, CorrelationEngine, Filters, SearchEngine,
};
use witslog_store::{Store, EventWriter};

fn setup_test_db() -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let store = Store::open_or_create(&db_path).unwrap();
    (tmp, store)
}

#[test]
fn fts5_table_created() {
    let (_tmp, store) = setup_test_db();
    let conn = store.conn().conn();

    let result: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='events_fts'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);

    assert!(result, "events_fts table should exist after migration");
}

#[test]
fn fts5_triggers_working() {
    let (_tmp, store) = setup_test_db();
    let writer = EventWriter::new(store.conn());

    let event = EventBuilder::new("test-app", "connection timeout")
        .severity(Severity::Error)
        .exception("ETIMEDOUT")
        .build();

    writer.write(&event).unwrap();

    let conn = store.conn().conn();
    let count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM events_fts WHERE events_fts MATCH 'timeout'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(count, 1, "FTS trigger should index event");
}

#[test]
fn search_prefix_query() {
    let (_tmp, store) = setup_test_db();
    let writer = EventWriter::new(store.conn());

    EventBuilder::new("app1", "timeout waiting for response")
        .severity(Severity::Error)
        .build();
    writer.write(&EventBuilder::new("app1", "timeout waiting for response").build()).unwrap();

    EventBuilder::new("app1", "timed out on request")
        .severity(Severity::Error)
        .build();
    writer.write(&EventBuilder::new("app1", "timed out on request").build()).unwrap();

    let conn = store.conn().conn();
    let search = SearchEngine::new(&conn);

    let result = search.search(
        "time*",
        &Filters::default(),
        20,
        None,
        true,
    ).unwrap();

    assert!(result.items.len() >= 2, "Prefix search should find both events");
}

#[test]
fn search_with_filters() {
    let (_tmp, store) = setup_test_db();
    let writer = EventWriter::new(store.conn());

    let event1 = EventBuilder::new("app1", "error in handler")
        .severity(Severity::Error)
        .category("application.handler")
        .build();
    writer.write(&event1).unwrap();

    let event2 = EventBuilder::new("app2", "network error")
        .severity(Severity::Error)
        .category("infrastructure.network")
        .build();
    writer.write(&event2).unwrap();

    let conn = store.conn().conn();
    let search = SearchEngine::new(&conn);

    let filters = Filters {
        application: Some("app1".to_string()),
        ..Default::default()
    };

    let result = search.search(
        "error",
        &filters,
        20,
        None,
        false,
    ).unwrap();

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].application, "app1");
}

#[test]
fn pagination_with_cursor() {
    let (_tmp, store) = setup_test_db();
    let writer = EventWriter::new(store.conn());

    for i in 0..30 {
        let event = EventBuilder::new("app", &format!("error {}", i))
            .severity(Severity::Error)
            .build();
        writer.write(&event).unwrap();
    }

    let conn = store.conn().conn();
    let search = SearchEngine::new(&conn);

    let page1 = search.search(
        "error",
        &Filters::default(),
        10,
        None,
        false,
    ).unwrap();

    assert_eq!(page1.items.len(), 10);
    assert!(page1.next_cursor.is_some(), "Should have next_cursor for more results");

    let page2 = search.search(
        "error",
        &Filters::default(),
        10,
        page1.next_cursor,
        false,
    ).unwrap();

    assert_eq!(page2.items.len(), 10);
    // Check that page2 items are different from page1.
    let page1_ids: Vec<_> = page1.items.iter().map(|e| &e.event_id).collect();
    let page2_ids: Vec<_> = page2.items.iter().map(|e| &e.event_id).collect();
    assert_ne!(page1_ids, page2_ids, "Pagination should return different items");
}

#[test]
fn statistics_aggregation() {
    let (_tmp, store) = setup_test_db();
    let writer = EventWriter::new(store.conn());

    EventBuilder::new("app", "error 1")
        .severity(Severity::Error)
        .category("infrastructure.network")
        .build();
    writer.write(&EventBuilder::new("app", "error 1")
        .severity(Severity::Error)
        .category("infrastructure.network")
        .build()).unwrap();

    EventBuilder::new("app", "warn 1")
        .severity(Severity::Warn)
        .category("application.validation")
        .build();
    writer.write(&EventBuilder::new("app", "warn 1")
        .severity(Severity::Warn)
        .category("application.validation")
        .build()).unwrap();

    let conn = store.conn().conn();
    let agg = AggregateEngine::new(&conn);

    let stats = agg.statistics(&Filters::default()).unwrap();

    assert_eq!(stats.total, 2);
    assert_eq!(stats.by_severity.get("error"), Some(&1));
    assert_eq!(stats.by_severity.get("warn"), Some(&1));
    assert_eq!(stats.by_category.get("infrastructure.network"), Some(&1));
}

#[test]
fn top_failures_ranking() {
    let (_tmp, store) = setup_test_db();
    let writer = EventWriter::new(store.conn());

    // Log same error 5 times (same fingerprint).
    for _ in 0..5 {
        writer.write(&EventBuilder::new("app", "connection refused")
            .severity(Severity::Error)
            .build()).unwrap();
    }

    // Log different error 2 times.
    for _ in 0..2 {
        writer.write(&EventBuilder::new("app", "timeout")
            .severity(Severity::Error)
            .build()).unwrap();
    }

    let conn = store.conn().conn();
    let agg = AggregateEngine::new(&conn);

    let top = agg.top_failures(&Filters::default(), "count", 10).unwrap();

    assert!(top.len() >= 2);
    assert_eq!(top[0].count, 5, "Most frequent error should rank first");
}

#[test]
fn correlation_by_correlation_id() {
    let (_tmp, store) = setup_test_db();
    let writer = EventWriter::new(store.conn());

    let corr_id = "trace-123";
    let event1 = EventBuilder::new("app", "started processing")
        .severity(Severity::Info)
        .correlation_id(corr_id.to_string())
        .build();
    writer.write(&event1).unwrap();

    let event2 = EventBuilder::new("app", "error in handler")
        .severity(Severity::Error)
        .correlation_id(corr_id.to_string())
        .build();
    writer.write(&event2).unwrap();

    let conn = store.conn().conn();
    let corr = CorrelationEngine::new(&conn);

    let trace = corr.by_correlation_id(corr_id).unwrap();

    assert_eq!(trace.ordered_events.len(), 2, "Should find both events in trace");
    assert_eq!(trace.ordered_events[0].message, "started processing");
    assert_eq!(trace.ordered_events[1].message, "error in handler");
}

#[test]
fn correlation_by_root_event_id() {
    let (_tmp, store) = setup_test_db();
    let writer = EventWriter::new(store.conn());

    let event1 = EventBuilder::new("app", "root cause")
        .severity(Severity::Error)
        .build();
    let event1_id = event1.event_id.clone();
    writer.write(&event1).unwrap();

    let event2 = EventBuilder::new("app", "child error")
        .severity(Severity::Error)
        .parent_event_id(event1_id.clone())
        .build();
    writer.write(&event2).unwrap();

    let conn = store.conn().conn();
    let corr = CorrelationEngine::new(&conn);

    let trace = corr.by_root_event_id(&event1_id).unwrap();

    assert_eq!(trace.ordered_events.len(), 2);
    assert_eq!(trace.ordered_events[0].message, "root cause");
    assert_eq!(trace.ordered_events[1].message, "child error");
}
