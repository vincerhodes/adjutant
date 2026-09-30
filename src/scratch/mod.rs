//! Scratch module: plain-text capture pads.
//!
//! All SQL for the scratch module lives in this file; UI files contain
//! none. Pads are link-graph citizens via `EntityType::Note` (the variant
//! and the entity_links CHECK pre-date this module — no core change).

pub mod model;
pub mod ui;

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::core::entity::{EntityRef, EntityType};
use crate::core::link::LinkStore;
use crate::db::{Db, DbError};

pub use model::{ScratchPad, ScratchPadInput};

/// Trashed-pad purge grace period, matching the todo/calendar rule.
const PURGE_AGE: Duration = Duration::days(30);

#[derive(Debug, thiserror::Error)]
pub enum ScratchError {
    #[error("database error: {0}")]
    Db(#[from] DbError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("pad not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("corrupt row: {0}")]
    Corrupt(String),
}

pub type Result<T> = std::result::Result<T, ScratchError>;

pub struct ScratchStore<'a> {
    db: &'a Db,
}

impl<'a> ScratchStore<'a> {
    pub fn new(db: &'a Db) -> Self {
        ScratchStore { db }
    }

    fn conn(&self) -> &Connection {
        self.db.conn()
    }

    pub fn create(&self, input: &ScratchPadInput) -> Result<ScratchPad> {
        validate_input(input)?;
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        self.conn().execute(
            "INSERT INTO scratch_notes (id, body, pinned, color_idx, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?5)",
            params![
                id.to_string(),
                input.body,
                input.pinned as i64,
                input.color_idx,
                now,
            ],
        )?;
        self.get(id)
    }

    /// Text edits only — pin/color have their own ops (spec §4). Bumps
    /// `updated_at` via trigger so the pad floats to the top of its group.
    pub fn update(&self, id: Uuid, body: &str) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE scratch_notes SET body = ?2 WHERE id = ?1",
            params![id.to_string(), body],
        )?;
        if n == 0 {
            return Err(ScratchError::NotFound(id.to_string()));
        }
        Ok(())
    }

    pub fn set_pinned(&self, id: Uuid, pinned: bool) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE scratch_notes SET pinned = ?2 WHERE id = ?1",
            params![id.to_string(), pinned as i64],
        )?;
        if n == 0 {
            return Err(ScratchError::NotFound(id.to_string()));
        }
        Ok(())
    }

    pub fn set_color(&self, id: Uuid, color_idx: i64) -> Result<()> {
        if !(0..=5).contains(&color_idx) {
            return Err(ScratchError::InvalidInput(format!(
                "color_idx {color_idx} out of range (0..=5)"
            )));
        }
        let n = self.conn().execute(
            "UPDATE scratch_notes SET color_idx = ?2 WHERE id = ?1",
            params![id.to_string(), color_idx],
        )?;
        if n == 0 {
            return Err(ScratchError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Live pads: pinned first, then `updated_at` desc within each group.
    pub fn list(&self) -> Result<Vec<ScratchPad>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, body, pinned, color_idx, trashed_at, created_at, updated_at
             FROM scratch_notes
             WHERE trashed_at IS NULL
             ORDER BY pinned DESC, updated_at DESC",
        )?;
        let rows = stmt.query_map([], map_pad_row)?;
        collect(rows)
    }

    /// Trashed pads, newest trash first (Trash view).
    pub fn list_trashed(&self) -> Result<Vec<ScratchPad>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, body, pinned, color_idx, trashed_at, created_at, updated_at
             FROM scratch_notes
             WHERE trashed_at IS NOT NULL
             ORDER BY trashed_at DESC",
        )?;
        let rows = stmt.query_map([], map_pad_row)?;
        collect(rows)
    }

    pub fn get(&self, id: Uuid) -> Result<ScratchPad> {
        let mut stmt = self.conn().prepare(
            "SELECT id, body, pinned, color_idx, trashed_at, created_at, updated_at
             FROM scratch_notes WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id.to_string()], map_pad_row)?;
        rows.next()
            .transpose()?
            .ok_or_else(|| ScratchError::NotFound(id.to_string()))
    }

    pub fn trash(&self, id: Uuid) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE scratch_notes SET trashed_at = ?2 WHERE id = ?1",
            params![id.to_string(), Utc::now().to_rfc3339()],
        )?;
        if n == 0 {
            return Err(ScratchError::NotFound(id.to_string()));
        }
        Ok(())
    }

    pub fn restore(&self, id: Uuid) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE scratch_notes SET trashed_at = NULL WHERE id = ?1",
            params![id.to_string()],
        )?;
        if n == 0 {
            return Err(ScratchError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Hard delete: row + link purge (both directions), one transaction.
    pub fn delete(&self, id: Uuid) -> Result<()> {
        let tx = self.conn().unchecked_transaction()?;
        let ent = EntityRef::new(EntityType::Note, id);
        LinkStore::delete_links_for(&tx, &ent)?;
        let n = tx.execute(
            "DELETE FROM scratch_notes WHERE id = ?1",
            params![id.to_string()],
        )?;
        if n == 0 {
            return Err(ScratchError::NotFound(id.to_string()));
        }
        tx.commit()?;
        Ok(())
    }

    /// Hard-delete pads trashed more than 30 days ago (startup purge, same
    /// rule as todos/calendar). Links are purged per row in the same tx.
    pub fn purge_expired(&self) -> Result<usize> {
        let cutoff = (Utc::now() - PURGE_AGE).to_rfc3339();
        let tx = self.conn().unchecked_transaction()?;
        let ids: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM scratch_notes WHERE trashed_at IS NOT NULL AND trashed_at < ?1",
            )?;
            let rows = stmt.query_map(params![cutoff], |row| row.get(0))?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };
        for raw in &ids {
            if let Ok(uuid) = Uuid::parse_str(raw) {
                let ent = EntityRef::new(EntityType::Note, uuid);
                LinkStore::delete_links_for(&tx, &ent)?;
            }
        }
        {
            let mut stmt = tx.prepare("DELETE FROM scratch_notes WHERE id = ?1")?;
            for raw in &ids {
                stmt.execute(params![raw])?;
            }
        }
        tx.commit()?;
        Ok(ids.len())
    }

    /// Body substring search (link picker). Returns id + first line.
    pub fn search(&self, needle: &str, limit: usize) -> Result<Vec<(Uuid, String)>> {
        let pattern = format!("%{}%", needle.replace('%', "_"));
        let mut stmt = self.conn().prepare(
            "SELECT id, body FROM scratch_notes
             WHERE trashed_at IS NULL AND body LIKE ?1
             ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map(params![pattern], |row| {
            let id = parse_uuid(&row.get::<_, String>(0)?)?;
            let body: String = row.get(1)?;
            let title = body
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("(empty pad)")
                .to_string();
            Ok((id, title))
        })?;
        let mut out = Vec::new();
        for r in rows {
            if out.len() >= limit {
                break;
            }
            out.push(r?);
        }
        Ok(out)
    }
}

fn validate_input(input: &ScratchPadInput) -> Result<()> {
    if !(0..=5).contains(&input.color_idx) {
        return Err(ScratchError::InvalidInput(format!(
            "color_idx {} out of range (0..=5)",
            input.color_idx
        )));
    }
    Ok(())
}

fn map_pad_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScratchPad> {
    Ok(ScratchPad {
        id: parse_uuid(&row.get::<_, String>(0)?)?,
        body: row.get(1)?,
        pinned: row.get::<_, i64>(2)? != 0,
        color_idx: row.get(3)?,
        trashed_at: row
            .get::<_, Option<String>>(4)?
            .map(|s| dt(&s))
            .transpose()?,
        created_at: dt(&row.get::<_, String>(5)?)?,
        updated_at: dt(&row.get::<_, String>(6)?)?,
    })
}

fn dt(s: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("bad timestamp {s:?}: {e}").into(),
            )
        })
}

fn parse_uuid(s: &str) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn collect<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    let mut v = Vec::new();
    for r in rows {
        v.push(r?);
    }
    Ok(v)
}
