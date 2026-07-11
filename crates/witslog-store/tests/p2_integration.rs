use tempfile::TempDir;
use witslog_core::{error, Classifier};
use witslog_store::Store;

#[test]
fn p2_migration_seeds_builtin_categories() {
    let tmpdir = TempDir::new().unwrap();
    let db_path = tmpdir.path().join("test.db");

    let store = Store::open_or_create(&db_path).unwrap();
    let conn = store.conn().conn();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM categories WHERE builtin = 1", [], |row| {
            row.get(0)
        })
        .unwrap();

    assert!(count > 20, "Expected >20 builtin categories, got {}", count);

    // Check specific categories exist
    let has_dns: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM categories WHERE canonical = 'infrastructure.network.dns'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(has_dns, 1);
}

#[test]
fn p2_classify_error_code_returns_category() {
    let classifier = Classifier::built_in();
    let result = classifier.classify("connection timed out", None, Some("ETIMEDOUT"));

    assert_eq!(
        result.canonical,
        Some("infrastructure.network.timeout".to_string())
    );
    assert!(result.suggested_tags.contains(&"timeout".to_string()));
}

#[test]
fn p2_classify_exception_returns_category() {
    let classifier = Classifier::built_in();
    let result = classifier.classify("panic in main thread", Some("panic"), None);

    assert_eq!(result.canonical, Some("runtime.panic".to_string()));
    assert!(result.suggested_tags.contains(&"panic".to_string()));
}

#[test]
fn p2_classify_message_keyword_case_insensitive() {
    let classifier = Classifier::built_in();
    let result = classifier.classify("ERROR: DISK FULL", None, None);

    assert_eq!(
        result.canonical,
        Some("infrastructure.storage.disk".to_string())
    );
}

#[test]
fn p2_no_match_returns_unclassified_tag() {
    let classifier = Classifier::built_in();
    let result = classifier.classify("something unexpected", None, None);

    assert_eq!(result.canonical, None);
    assert!(result.suggested_tags.contains(&"unclassified".to_string()));
}

#[test]
fn p2_classify_in_builder_chain() {
    let classifier = Classifier::built_in();

    let event = error("test-app", "connection timeout")
        .error_code("ETIMEDOUT")
        .classify(&classifier)
        .build();

    assert_eq!(event.category, Some("infrastructure.network.timeout".to_string()));
    assert!(event.tags.is_some());
}

#[test]
fn p2_classify_respects_explicit_category() {
    let classifier = Classifier::built_in();

    let event = error("test-app", "connection timeout")
        .error_code("ETIMEDOUT")
        .category("custom.category")
        .classify(&classifier) // Should not override explicit category
        .build();

    assert_eq!(event.category, Some("custom.category".to_string()));
}

#[test]
fn p2_deterministic_classification() {
    let classifier = Classifier::built_in();

    let input = ("connection failed", Some("TimeoutError"), None);
    let result1 = classifier.classify(input.0, input.1, input.2);
    let result2 = classifier.classify(input.0, input.1, input.2);

    assert_eq!(result1.canonical, result2.canonical);
    assert_eq!(result1.suggested_tags, result2.suggested_tags);
}
