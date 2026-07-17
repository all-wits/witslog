//! FR-P9-006/007: tamper-evident audit hash chain over `events`. Each row's
//! `audit_hash` = sha256(prev_hash || event_id || ts || message || fingerprint),
//! chained to the previous row in insertion (id) order. `verify_chain`
//! recomputes the chain and reports the first row where it diverges from
//! what's stored — i.e. a tampered/edited row, or one deleted-and-reinserted
//! out of the original position.

use crate::error::Result;
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

const GENESIS: &str = "genesis";

fn compute_hash(prev_hash: &str, event_id: &str, ts: &str, message: &str, fingerprint: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(b"|");
    hasher.update(event_id.as_bytes());
    hasher.update(b"|");
    hasher.update(ts.as_bytes());
    hasher.update(b"|");
    hasher.update(message.as_bytes());
    hasher.update(b"|");
    hasher.update(fingerprint.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn last_hash(conn: &Connection) -> Result<String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM audit_meta WHERE key = 'last_hash'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value.unwrap_or_else(|| GENESIS.to_string()))
}

fn set_last_hash(conn: &Connection, hash: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO audit_meta (key, value) VALUES ('last_hash', ?1)",
        rusqlite::params![hash],
    )?;
    Ok(())
}

/// Called by the write path right after a row is inserted: extends the
/// chain by one link and stamps `audit_hash` on that row. O(1) overhead
/// (non-functional target in PHASES.md P9).
pub(crate) fn append(
    conn: &Connection,
    row_id: i64,
    event_id: &str,
    ts: &str,
    message: &str,
    fingerprint: &str,
) -> Result<()> {
    let prev = last_hash(conn)?;
    let hash = compute_hash(&prev, event_id, ts, message, fingerprint);

    conn.execute(
        "UPDATE events SET audit_hash = ?1 WHERE id = ?2",
        rusqlite::params![hash, row_id],
    )?;
    set_last_hash(conn, &hash)?;

    Ok(())
}

/// Chains any rows missing `audit_hash` (e.g. rows written before this
/// migration ran) in `id` order, continuing from whatever `last_hash`
/// currently is. Idempotent: a fully-chained DB is a no-op.
pub(crate) fn backfill_chain(conn: &Connection) -> Result<()> {
    let mut prev = last_hash(conn)?;

    let mut stmt = conn.prepare(
        "SELECT id, event_id, ts, message, fingerprint FROM events
         WHERE audit_hash IS NULL ORDER BY id ASC",
    )?;
    let rows: Vec<(i64, String, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    for (id, event_id, ts, message, fingerprint) in rows {
        let hash = compute_hash(&prev, &event_id, &ts, &message, &fingerprint);
        conn.execute(
            "UPDATE events SET audit_hash = ?1 WHERE id = ?2",
            rusqlite::params![hash, id],
        )?;
        prev = hash;
    }

    set_last_hash(conn, &prev)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditBreak {
    pub row_id: i64,
    pub event_id: String,
    pub expected_hash: String,
    pub actual_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditVerifyResult {
    Ok { rows_checked: i64, tombstones_bridged: i64 },
    Broken(AuditBreak),
}

/// Looks up the tombstoned `audit_hash` for a deleted row, if any (FR-P10-001).
fn tombstone_hash(conn: &Connection, row_id: i64) -> Result<Option<String>> {
    let hash: Option<String> = conn
        .query_row(
            "SELECT audit_hash FROM audit_tombstones WHERE row_id = ?1",
            [row_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(hash)
}

/// Walks `events` in `id` order, recomputing the chain from genesis and
/// comparing against the stored `audit_hash` on each row. When the `id`
/// sequence gaps (a row was removed by `delete`/`prune`/`archive`), bridges
/// the gap using the tombstoned hash of the row immediately preceding it
/// (`id - 1`) rather than reporting the gap as tampering — that hash already
/// encodes the full chain up to that point, regardless of how many rows in
/// the gap were removed. A gap with no matching tombstone is still reported
/// as `Broken`: undocumented row removal stays indistinguishable from
/// tampering, which is the point of the chain.
///
/// Returns the first genuine divergence found (FR-P9-007: `doctor
/// --verify-audit` reports the offending row + expected/actual hash).
pub fn verify_chain(conn: &Connection) -> Result<AuditVerifyResult> {
    let mut prev = GENESIS.to_string();
    let mut prev_id = 0i64;
    let mut rows_checked = 0i64;
    let mut tombstones_bridged = 0i64;

    let mut stmt = conn.prepare(
        "SELECT id, event_id, ts, message, fingerprint, audit_hash FROM events ORDER BY id ASC",
    )?;
    let rows: Vec<(i64, String, String, String, String, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    for (id, event_id, ts, message, fingerprint, stored) in rows {
        if id != prev_id + 1 {
            match tombstone_hash(conn, id - 1)? {
                Some(bridge) => {
                    prev = bridge;
                    tombstones_bridged += 1;
                }
                None => {
                    return Ok(AuditVerifyResult::Broken(AuditBreak {
                        row_id: id,
                        event_id,
                        expected_hash: String::new(),
                        actual_hash: stored,
                    }));
                }
            }
        }

        let expected = compute_hash(&prev, &event_id, &ts, &message, &fingerprint);
        if stored.as_deref() != Some(expected.as_str()) {
            return Ok(AuditVerifyResult::Broken(AuditBreak {
                row_id: id,
                event_id,
                expected_hash: expected,
                actual_hash: stored,
            }));
        }
        prev = expected;
        prev_id = id;
        rows_checked += 1;
    }

    Ok(AuditVerifyResult::Ok { rows_checked, tombstones_bridged })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::Migrator;

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        Migrator::new(&conn).migrate().unwrap();
        conn
    }

    #[test]
    fn chain_verifies_clean_after_inserts() {
        let conn = fresh_conn();
        for i in 0..3 {
            let event_id = format!("evt-{i}");
            conn.execute(
                "INSERT INTO events (event_id, ts, ts_epoch_ms, application, severity, severity_rank, message, fingerprint, schema_v)
                 VALUES (?1, '2026-01-01T00:00:00.000Z', 0, 'app', 'error', 60, ?2, ?3, 1)",
                rusqlite::params![event_id, format!("msg {i}"), format!("fp-{i}")],
            )
            .unwrap();
            let row_id = conn.last_insert_rowid();
            append(&conn, row_id, &event_id, "2026-01-01T00:00:00.000Z", &format!("msg {i}"), &format!("fp-{i}")).unwrap();
        }

        match verify_chain(&conn).unwrap() {
            AuditVerifyResult::Ok { rows_checked, tombstones_bridged } => {
                assert_eq!(rows_checked, 3);
                assert_eq!(tombstones_bridged, 0);
            }
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn tampered_row_breaks_chain_at_that_row() {
        let conn = fresh_conn();
        for i in 0..3 {
            let event_id = format!("evt-{i}");
            conn.execute(
                "INSERT INTO events (event_id, ts, ts_epoch_ms, application, severity, severity_rank, message, fingerprint, schema_v)
                 VALUES (?1, '2026-01-01T00:00:00.000Z', 0, 'app', 'error', 60, ?2, ?3, 1)",
                rusqlite::params![event_id, format!("msg {i}"), format!("fp-{i}")],
            )
            .unwrap();
            let row_id = conn.last_insert_rowid();
            append(&conn, row_id, &event_id, "2026-01-01T00:00:00.000Z", &format!("msg {i}"), &format!("fp-{i}")).unwrap();
        }

        // Tamper with the middle row's message without updating audit_hash.
        conn.execute(
            "UPDATE events SET message = 'tampered' WHERE event_id = 'evt-1'",
            [],
        )
        .unwrap();

        match verify_chain(&conn).unwrap() {
            AuditVerifyResult::Broken(b) => assert_eq!(b.event_id, "evt-1"),
            other => panic!("expected Broken, got {:?}", other),
        }
    }

    #[test]
    fn backfill_chains_rows_written_before_migration() {
        let conn = Connection::open_in_memory().unwrap();
        // Simulate a pre-v6 DB: migrate to v5 schema shape manually is
        // impractical here, so instead migrate fully then insert a row
        // with a NULL audit_hash (as if it predated the chain) and confirm
        // backfill picks it up.
        Migrator::new(&conn).migrate().unwrap();
        conn.execute(
            "INSERT INTO events (event_id, ts, ts_epoch_ms, application, severity, severity_rank, message, fingerprint, schema_v)
             VALUES ('evt-legacy', '2026-01-01T00:00:00.000Z', 0, 'app', 'error', 60, 'legacy msg', 'fp-legacy', 1)",
            [],
        )
        .unwrap();

        backfill_chain(&conn).unwrap();

        match verify_chain(&conn).unwrap() {
            AuditVerifyResult::Ok { rows_checked, .. } => assert_eq!(rows_checked, 1),
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    fn insert_chained(conn: &Connection, event_id: &str, msg: &str, fp: &str) -> i64 {
        conn.execute(
            "INSERT INTO events (event_id, ts, ts_epoch_ms, application, severity, severity_rank, message, fingerprint, schema_v)
             VALUES (?1, '2026-01-01T00:00:00.000Z', 0, 'app', 'error', 60, ?2, ?3, 1)",
            rusqlite::params![event_id, msg, fp],
        )
        .unwrap();
        let row_id = conn.last_insert_rowid();
        append(conn, row_id, event_id, "2026-01-01T00:00:00.000Z", msg, fp).unwrap();
        row_id
    }

    /// Regression lock (FR-P10-001): deleting a row and recording its
    /// tombstone must keep `verify_chain` reporting `Ok` — this is the fix
    /// for the bug where any `delete`/`prune`/`archive` permanently broke
    /// `doctor --verify-audit` for every row after the deleted one.
    #[test]
    fn deleting_a_row_keeps_verify_chain_ok() {
        let conn = fresh_conn();
        let ids: Vec<i64> = (0..3)
            .map(|i| insert_chained(&conn, &format!("evt-{i}"), &format!("msg {i}"), &format!("fp-{i}")))
            .collect();

        // Tombstone + delete the middle row, mirroring
        // `writer::delete_events_by_id`.
        let hash: String = conn
            .query_row(
                "SELECT audit_hash FROM events WHERE id = ?1",
                [ids[1]],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO audit_tombstones (row_id, event_id, audit_hash, deleted_at) VALUES (?1, 'evt-1', ?2, '2026-01-01T00:00:00.000Z')",
            rusqlite::params![ids[1], hash],
        )
        .unwrap();
        conn.execute("DELETE FROM events WHERE id = ?1", [ids[1]]).unwrap();

        match verify_chain(&conn).unwrap() {
            AuditVerifyResult::Ok { rows_checked, tombstones_bridged } => {
                assert_eq!(rows_checked, 2);
                assert_eq!(tombstones_bridged, 1);
            }
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    /// The other half of the regression lock: a gap with no recorded
    /// tombstone must still report `Broken` — undocumented row removal stays
    /// indistinguishable from tampering.
    #[test]
    fn deleted_row_without_tombstone_still_breaks_chain() {
        let conn = fresh_conn();
        let ids: Vec<i64> = (0..3)
            .map(|i| insert_chained(&conn, &format!("evt-{i}"), &format!("msg {i}"), &format!("fp-{i}")))
            .collect();

        // Delete the middle row WITHOUT recording a tombstone.
        conn.execute("DELETE FROM events WHERE id = ?1", [ids[1]]).unwrap();

        match verify_chain(&conn).unwrap() {
            AuditVerifyResult::Broken(b) => assert_eq!(b.event_id, "evt-2"),
            other => panic!("expected Broken, got {:?}", other),
        }
    }
}
