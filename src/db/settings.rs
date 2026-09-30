//! Key-value settings over the `settings` table. Values are JSON-encoded.

use rusqlite::OptionalExtension;
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::{Db, DbError};

impl Db {
    pub fn get_setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, DbError> {
        let raw: Option<String> = self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        match raw {
            Some(raw) => serde_json::from_str(&raw)
                .map(Some)
                .map_err(|e| DbError::Settings(format!("key {key:?}: {e}"))),
            None => Ok(None),
        }
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<(), DbError> {
        let raw = serde_json::to_string(value)
            .map_err(|e| DbError::Settings(format!("key {key:?}: {e}")))?;
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [key, &raw],
        )?;
        Ok(())
    }
}
