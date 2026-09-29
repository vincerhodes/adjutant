//! Generic entity link layer: create/remove/query directed, typed links
//! between entities of any module.
//!
//! - `blocks` edges are cycle-guarded (rejected inside the same transaction
//!   as the insert); other relations may form cycles freely.
//! - Links to entities whose row no longer exists are tolerated (dangling),
//!   per spec — future modules render them as "unavailable".

use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::{Db, DbError};

use super::entity::EntityRef;

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("database error: {0}")]
    Db(#[from] DbError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("unknown entity type: {0}")]
    UnknownEntityType(String),
    #[error("linking {0} to itself is not allowed")]
    SelfLink(String),
    #[error("link already exists")]
    Duplicate,
    #[error("link not found")]
    NotFound,
    #[error("adding this 'blocks' link would create a cycle")]
    Cycle,
}

pub type Result<T> = std::result::Result<T, LinkError>;

/// Link relation vocabulary. Open enum by design — the DB column is
/// CHECK-less TEXT so new relations never require a migration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Relation(String);

impl Relation {
    pub const DERIVED_FROM: &'static str = "derived_from";
    pub const BLOCKS: &'static str = "blocks";
    pub const SCHEDULED_AS: &'static str = "scheduled_as";
    pub const MENTIONS: &'static str = "mentions";

    pub fn custom(name: impl Into<String>) -> Relation {
        Relation(name.into())
    }

    pub fn blocks() -> Relation {
        Relation(Self::BLOCKS.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_blocks(&self) -> bool {
        self.0 == Self::BLOCKS
    }
}

impl From<&str> for Relation {
    fn from(s: &str) -> Self {
        Relation(s.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub id: Uuid,
    pub source: EntityRef,
    pub target: EntityRef,
    pub relation: Relation,
    pub created_at: String,
}

pub struct LinkStore<'a> {
    db: &'a Db,
}

impl<'a> LinkStore<'a> {
    pub fn new(db: &'a Db) -> Self {
        LinkStore { db }
    }

    fn conn(&self) -> &Connection {
        self.db.conn()
    }

    pub fn link(&self, src: &EntityRef, tgt: &EntityRef, rel: &Relation) -> Result<()> {
        if src == tgt {
            return Err(LinkError::SelfLink(src.kind.as_str().to_string()));
        }
        let conn = self.conn();
        let tx = conn.unchecked_transaction()?;
        if rel.is_blocks() && Self::would_cycle_conn(&tx, src, tgt)? {
            return Err(LinkError::Cycle);
        }
        let id = Uuid::new_v4().to_string();
        let created = Utc::now().to_rfc3339();
        let n = tx.execute(
            "INSERT OR IGNORE INTO entity_links
                 (id, source_type, source_id, target_type, target_id, relation, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                id,
                src.kind.as_str(),
                src.id.to_string(),
                tgt.kind.as_str(),
                tgt.id.to_string(),
                rel.as_str(),
                created
            ],
        )?;
        if n == 0 {
            return Err(LinkError::Duplicate);
        }
        tx.commit()?;
        Ok(())
    }

    pub fn unlink(&self, src: &EntityRef, tgt: &EntityRef, rel: &Relation) -> Result<()> {
        let n = self.conn().execute(
            "DELETE FROM entity_links
             WHERE source_type = ?1 AND source_id = ?2
               AND target_type = ?3 AND target_id = ?4
               AND relation = ?5",
            rusqlite::params![
                src.kind.as_str(),
                src.id.to_string(),
                tgt.kind.as_str(),
                tgt.id.to_string(),
                rel.as_str()
            ],
        )?;
        if n == 0 {
            return Err(LinkError::NotFound);
        }
        Ok(())
    }

    pub fn links_from(&self, src: &EntityRef) -> Result<Vec<Link>> {
        self.query_links(
            "SELECT id, source_type, source_id, target_type, target_id, relation, created_at
             FROM entity_links WHERE source_type = ?1 AND source_id = ?2
             ORDER BY created_at",
            rusqlite::params![src.kind.as_str(), src.id.to_string()],
        )
    }

    pub fn links_to(&self, tgt: &EntityRef) -> Result<Vec<Link>> {
        self.query_links(
            "SELECT id, source_type, source_id, target_type, target_id, relation, created_at
             FROM entity_links WHERE target_type = ?1 AND target_id = ?2
             ORDER BY created_at",
            rusqlite::params![tgt.kind.as_str(), tgt.id.to_string()],
        )
    }

    /// Remove every link touching `entity` (both directions). Called in the
    /// same transaction as a hard delete of the entity row.
    pub fn delete_links_for(
        conn: &Connection,
        entity: &EntityRef,
    ) -> std::result::Result<usize, DbError> {
        Ok(conn.execute(
            "DELETE FROM entity_links
             WHERE (source_type = ?1 AND source_id = ?2)
                OR (target_type = ?1 AND target_id = ?2)",
            [entity.kind.as_str(), &entity.id.to_string()],
        )?)
    }

    /// Would adding `src -blocks-> tgt` create a cycle? True iff `src` is
    /// reachable from `tgt` over outgoing `blocks` edges (i.e. tgt blocks*
    /// src already).
    pub fn would_cycle(&self, src: &EntityRef, tgt: &EntityRef) -> Result<bool> {
        let conn = self.conn();
        Self::would_cycle_conn(conn, src, tgt)
    }

    fn would_cycle_conn(conn: &Connection, src: &EntityRef, tgt: &EntityRef) -> Result<bool> {
        // Recursive CTE walks tgt's outgoing 'blocks' edges; src reachable → cycle.
        let reachable: bool = conn.query_row(
            "WITH RECURSIVE walk (kind, id) AS (
                 SELECT ?3, ?4
                 UNION
                 SELECT e.target_type, e.target_id
                 FROM walk w JOIN entity_links e
                   ON e.relation = 'blocks'
                  AND e.source_type = w.kind
                  AND e.source_id = w.id
             )
             SELECT EXISTS(
                 SELECT 1 FROM walk WHERE kind = ?1 AND id = ?2
             )",
            [
                src.kind.as_str(),
                &src.id.to_string(),
                tgt.kind.as_str(),
                &tgt.id.to_string(),
            ],
            |row| row.get(0),
        )?;
        Ok(reachable)
    }

    fn query_links(&self, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<Vec<Link>> {
        let mut stmt = self.conn().prepare(sql)?;
        let rows = stmt.query_map(params, |row| {
            let source_type: String = row.get(1)?;
            let source_id: String = row.get(2)?;
            let target_type: String = row.get(3)?;
            let target_id: String = row.get(4)?;
            let relation: String = row.get(5)?;
            Ok(Link {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                source: EntityRef::new(
                    source_type.parse().map_err(to_sql_err)?,
                    parse_uuid(&source_id)?,
                ),
                target: EntityRef::new(
                    target_type.parse().map_err(to_sql_err)?,
                    parse_uuid(&target_id)?,
                ),
                relation: Relation::from(relation.as_str()),
                created_at: row.get(6)?,
            })
        })?;
        let mut links = Vec::new();
        for row in rows {
            links.push(row?);
        }
        Ok(links)
    }
}

fn parse_uuid(s: &str) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn to_sql_err(e: LinkError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}
