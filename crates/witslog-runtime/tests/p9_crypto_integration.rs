//! FR-P9-004 wiring integration tests: metadata encryption end-to-end through
//! `build_and_write` (the CLI `witslog log` / one-shot path). Covers the
//! round-trip, fail-closed write, and the metadata-only invariant (search on
//! `message` is unaffected by encrypting `metadata`) — see
//! docs/security-review-v0.1.7.md and CLAUDE.md for the design rationale.

use std::sync::Mutex;

use witslog_config::Config;
use witslog_core::{decrypt_metadata_for_display, FieldCipher};
use witslog_store::{EventWriter, Store};

// `FieldCipher::from_env`/`resolve_cipher` read process-wide env vars —
// serialize tests that set/unset them so they don't race each other.
static LOCK: Mutex<()> = Mutex::new(());

const TEST_VAR: &str = "WITSLOG_TEST_P9_CRYPTO_KEY";

fn test_key_hex() -> String {
    "07".repeat(32) // 64 hex chars = 32 bytes
}

fn temp_config(key_env: Option<&str>) -> (tempfile::TempDir, Config, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("witslog.db");
    let mut cfg = Config::default_project();
    cfg.db_path = Some(db.clone());
    cfg.crypto.key_env = key_env.map(|s| s.to_string());
    (dir, cfg, db)
}

#[test]
fn round_trip_write_with_key_then_read_back() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var(TEST_VAR, test_key_hex());

    let (_dir, cfg, db) = temp_config(Some(TEST_VAR));
    let builder = witslog_core::error("app", "boom")
        .metadata(serde_json::json!({"user_email": "x@y.com"}));
    let event = witslog_runtime::build_and_write(&cfg, &db, builder).expect("write succeeds");

    // Stored value is the encrypted envelope, not plaintext.
    let stored = event.metadata.clone().expect("metadata set");
    assert!(stored.get("__witslog_enc").is_some(), "metadata stored as envelope: {stored}");

    // Read back through the store and decrypt with the same key.
    let store = Store::open_or_create(&db).unwrap();
    let writer = EventWriter::new(store.conn());
    let read_back = writer.query_by_id(&event.event_id).unwrap().expect("row exists");

    let cipher = FieldCipher::from_env(TEST_VAR).unwrap().unwrap();
    let displayed = decrypt_metadata_for_display(read_back.metadata, Some(&cipher));
    assert_eq!(displayed, Some(serde_json::json!({"user_email": "x@y.com"})));

    std::env::remove_var(TEST_VAR);
}

#[test]
fn placeholder_when_reading_without_the_key() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var(TEST_VAR, test_key_hex());

    let (_dir, cfg, db) = temp_config(Some(TEST_VAR));
    let builder =
        witslog_core::error("app", "boom").metadata(serde_json::json!({"secret": "value"}));
    let event = witslog_runtime::build_and_write(&cfg, &db, builder).expect("write succeeds");

    std::env::remove_var(TEST_VAR);

    let store = Store::open_or_create(&db).unwrap();
    let writer = EventWriter::new(store.conn());
    let read_back = writer.query_by_id(&event.event_id).unwrap().expect("row exists");

    // No key available now — display-only decrypt falls back to the placeholder,
    // never crashes, never leaks ciphertext or the raw envelope.
    let displayed = decrypt_metadata_for_display(read_back.metadata, None);
    assert_eq!(displayed, Some(serde_json::Value::String("<encrypted>".to_string())));
}

#[test]
fn fail_closed_when_key_env_configured_but_var_unset() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var(TEST_VAR); // ensure genuinely unset

    let (_dir, cfg, db) = temp_config(Some(TEST_VAR));
    let builder = witslog_core::error("app", "boom").metadata(serde_json::json!({"a": 1}));
    let result = witslog_runtime::build_and_write(&cfg, &db, builder);

    assert!(result.is_err(), "write must be refused, not silently written in plaintext");

    // Nothing was persisted — fail-closed means no row, not a plaintext fallback.
    let store = Store::open_or_create(&db).unwrap();
    let count: i64 = store
        .conn()
        .conn()
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn encryption_off_by_default_metadata_stays_plaintext() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, cfg, db) = temp_config(None);

    let builder =
        witslog_core::error("app", "boom").metadata(serde_json::json!({"a": 1}));
    let event = witslog_runtime::build_and_write(&cfg, &db, builder).expect("write succeeds");

    assert_eq!(event.metadata, Some(serde_json::json!({"a": 1})));
}

#[test]
fn metadata_only_invariant_fts_index_and_message_unaffected() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var(TEST_VAR, test_key_hex());

    let (_dir, cfg, db) = temp_config(Some(TEST_VAR));
    let builder = witslog_core::error("app", "distinctivesearchtermxyz")
        .metadata(serde_json::json!({"user_email": "x@y.com"}));
    let event = witslog_runtime::build_and_write(&cfg, &db, builder).expect("write succeeds");

    std::env::remove_var(TEST_VAR);

    // `message` (what FTS5 indexes) stays plaintext regardless of metadata
    // encryption — only `metadata` is ever touched by the cipher — and the
    // row is still reachable via the FTS5 virtual table directly, proving
    // encrypting metadata didn't disturb the index.
    let store = Store::open_or_create(&db).unwrap();
    let conn = store.conn().conn();

    let message: String = conn
        .query_row("SELECT message FROM events WHERE event_id = ?1", [&event.event_id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(message, "distinctivesearchtermxyz");

    let fts_hit: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events_fts WHERE events_fts MATCH 'distinctivesearchtermxyz'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(fts_hit, 1, "FTS5 index still finds the event by message");
}
