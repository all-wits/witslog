use crate::error::Result;
use rusqlite::Connection;

/// Insert a category into the store. Best-effort: skips if already present (idempotent).
pub fn insert_category(
    conn: &Connection,
    canonical: &str,
    parent: Option<&str>,
    label: &str,
    builtin: bool,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO categories (canonical, parent, label, builtin) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![canonical, parent, label, if builtin { 1 } else { 0 }],
    )?;
    Ok(())
}

/// Insert a category alias. Best-effort: skips if already present.
pub fn insert_alias(conn: &Connection, alias: &str, canonical: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO category_aliases (alias, canonical) VALUES (?1, ?2)",
        rusqlite::params![alias, canonical],
    )?;
    Ok(())
}

/// Resolve an alias to its canonical form. Returns the canonical if it exists.
pub fn resolve_alias(conn: &Connection, alias_or_canonical: &str) -> Result<Option<String>> {
    // Try to look up as alias first
    let result: rusqlite::Result<String> = conn.query_row(
        "SELECT canonical FROM category_aliases WHERE alias = ?1",
        rusqlite::params![alias_or_canonical],
        |row| row.get(0),
    );

    match result {
        Ok(canonical) => Ok(Some(canonical)),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            // Not an alias, check if it's a canonical
            let canonical_check: rusqlite::Result<String> = conn.query_row(
                "SELECT canonical FROM categories WHERE canonical = ?1",
                rusqlite::params![alias_or_canonical],
                |row| row.get(0),
            );
            match canonical_check {
                Ok(c) => Ok(Some(c)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        }
        Err(e) => Err(e.into()),
    }
}

/// List all categories as a tree structure (parent → vec<children>).
pub fn list_categories(conn: &Connection) -> Result<Vec<(String, String, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT canonical, label, parent FROM categories ORDER BY canonical",
    )?;

    let categories = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;

    let mut result = Vec::new();
    for cat in categories {
        result.push(cat?);
    }

    Ok(result)
}

/// Count categories in the store.
pub fn count_categories(conn: &Connection) -> Result<usize> {
    let count: i64 =
        conn.query_row("SELECT COUNT(*) FROM categories", [], |row| row.get(0))?;
    Ok(count as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_db() -> rusqlite::Result<Connection> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            r#"
            CREATE TABLE categories (
              canonical  TEXT PRIMARY KEY,
              parent     TEXT REFERENCES categories(canonical),
              label      TEXT,
              builtin    INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE category_aliases (
              alias      TEXT PRIMARY KEY,
              canonical  TEXT NOT NULL REFERENCES categories(canonical)
            );
            "#,
        )?;
        Ok(conn)
    }

    #[test]
    fn insert_and_resolve_category() {
        let conn = setup_db().unwrap();
        insert_category(&conn, "infra.network", None, "Network", true).unwrap();
        let result = resolve_alias(&conn, "infra.network").unwrap();
        assert_eq!(result, Some("infra.network".to_string()));
    }

    #[test]
    fn alias_resolves_to_canonical() {
        let conn = setup_db().unwrap();
        insert_category(&conn, "infra.network", None, "Network", true).unwrap();
        insert_alias(&conn, "dns_error", "infra.network").unwrap();
        let result = resolve_alias(&conn, "dns_error").unwrap();
        assert_eq!(result, Some("infra.network".to_string()));
    }

    #[test]
    fn idempotent_insert() {
        let conn = setup_db().unwrap();
        insert_category(&conn, "app.error", None, "App Error", false).unwrap();
        insert_category(&conn, "app.error", None, "App Error", false).unwrap();
        assert_eq!(count_categories(&conn).unwrap(), 1);
    }

    #[test]
    fn list_categories_returns_all() {
        let conn = setup_db().unwrap();
        insert_category(&conn, "root", None, "Root", true).unwrap();
        insert_category(&conn, "root.child", Some("root"), "Child", true).unwrap();
        let cats = list_categories(&conn).unwrap();
        assert_eq!(cats.len(), 2);
    }
}
