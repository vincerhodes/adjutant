//! Versioned migration runner gated on `PRAGMA user_version`.
//!
//! Migrations are embedded via `include_str!` so the binary is fully static —
//! no runtime file reads. Never edit an applied migration; add a new
//! `migrations/NNNN_name.sql` file instead.

use rusqlite::{Connection, OptionalExtension};

use super::DbError;

const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../../migrations/0001_init.sql")),
    (
        "0002_email",
        include_str!("../../migrations/0002_email.sql"),
    ),
    (
        "0003_calendar",
        include_str!("../../migrations/0003_calendar.sql"),
    ),
    (
        "0004_scratchpad",
        include_str!("../../migrations/0004_scratchpad.sql"),
    ),
];

pub fn run(conn: &Connection) -> Result<(), DbError> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    for (version, (name, sql)) in MIGRATIONS.iter().enumerate() {
        let version = i64::try_from(version + 1)
            .map_err(|_| DbError::Migration("version overflow".into()))?;
        if version <= current {
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)
            .map_err(|e| DbError::Migration(format!("{name}: {e}")))?;
        tx.pragma_update(None, "user_version", version)
            .map_err(|e| DbError::Migration(format!("{name}: bump user_version: {e}")))?;
        tx.commit()
            .map_err(|e| DbError::Migration(format!("{name}: commit: {e}")))?;
    }
    // Sanity: a database claiming a version we don't know about is newer than
    // this binary — refuse rather than silently misbehave.
    let after: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .optional()?
        .unwrap_or(0);
    if after > i64::try_from(MIGRATIONS.len()).unwrap_or(i64::MAX) {
        return Err(DbError::Migration(format!(
            "database user_version {after} is newer than this binary supports"
        )));
    }
    Ok(())
}
