//! Versioned schema migrations tracked with `PRAGMA user_version`.

use rusqlite::Connection;

/// Ordered, append-only. Never edit a released migration; add a new one.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/0001_initial.sql")),
    (2, include_str!("../migrations/0002_snapshot_metadata.sql")),
];

pub fn latest_schema_version() -> i64 {
    MIGRATIONS.last().map_or(0, |(v, _)| *v)
}

/// Applies every migration newer than the database, each in its own transaction.
pub(crate) fn apply(conn: &mut Connection) -> rusqlite::Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v > current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}
