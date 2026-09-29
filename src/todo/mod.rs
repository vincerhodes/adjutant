//! Todo store: groups, todos, nesting, trash, reorder, completion rule.
//!
//! Every trash-state mutation covers the whole subtree in one transaction.
//! Every live query filters `deleted_at IS NULL`.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::core::entity::{EntityRef, EntityType};
use crate::core::link::LinkStore;
use crate::db::{Db, DbError};

pub mod model;
pub mod ui;

pub use model::{Priority, Status, Todo, TodoGroup, TodoNode};

#[derive(Debug, thiserror::Error)]
pub enum TodoError {
    #[error("database error: {0}")]
    Db(#[from] DbError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("todo or group not found: {0}")]
    NotFound(String),
    #[error("unknown status: {0}")]
    UnknownStatus(String),
    #[error("invalid priority: {0} (expected 0..=3)")]
    InvalidPriority(u8),
    #[error("todo has {0} open descendant(s); complete or cancel them first")]
    OpenDescendants(usize),
    #[error("todo is in the trash")]
    Trashed,
}

pub type Result<T> = std::result::Result<T, TodoError>;

/// Subtree purge grace period: trashed rows older than this are hard-deleted.
const PURGE_AGE: Duration = Duration::days(30);

pub struct TodoStore<'a> {
    db: &'a Db,
}

impl<'a> TodoStore<'a> {
    pub fn new(db: &'a Db) -> Self {
        TodoStore { db }
    }

    fn conn(&self) -> &Connection {
        self.db.conn()
    }

    // ── Groups ────────────────────────────────────────────────────────────

    pub fn create_group(&self, name: &str) -> Result<TodoGroup> {
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        let pos: i64 = self.conn().query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM todo_groups",
            [],
            |row| row.get(0),
        )?;
        self.conn().execute(
            "INSERT INTO todo_groups (id, name, position, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.to_string(), name, pos, now, now],
        )?;
        Ok(TodoGroup {
            id,
            name: name.to_string(),
            position: pos,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn list_groups(&self) -> Result<Vec<TodoGroup>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id, name, position, created_at, updated_at
                      FROM todo_groups ORDER BY position, created_at")?;
        let rows = stmt.query_map([], |row| {
            Ok(TodoGroup {
                id: parse_uuid(&row.get::<_, String>(0)?)?,
                name: row.get(1)?,
                position: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        collect(rows)
    }

    pub fn rename_group(&self, id: Uuid, name: &str) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE todo_groups SET name = ?2 WHERE id = ?1",
            params![id.to_string(), name],
        )?;
        if n == 0 {
            return Err(TodoError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Hard-delete a group and everything in it (rows cascade via FK; links
    /// are removed explicitly for every todo in the group, in one tx).
    pub fn delete_group(&self, id: Uuid) -> Result<()> {
        let conn = self.conn();
        let tx = conn.unchecked_transaction()?;
        let ids: Vec<String> = {
            let mut stmt = tx.prepare("SELECT id FROM todos WHERE group_id = ?1")?;
            let rows = stmt.query_map(params![id.to_string()], |row| row.get(0))?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };
        for raw in &ids {
            let ent = EntityRef::new(EntityType::Todo, parse_uuid(raw)?);
            LinkStore::delete_links_for(&tx, &ent)?;
        }
        tx.execute("DELETE FROM todo_groups WHERE id = ?1", params![id.to_string()])?;
        tx.commit()?;
        Ok(())
    }

    // ── Todos: CRUD ───────────────────────────────────────────────────────

    pub fn create(
        &self,
        group_id: Uuid,
        parent_id: Option<Uuid>,
        title: &str,
    ) -> Result<Todo> {
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        let pos: i64 = self.conn().query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM todos
             WHERE group_id = ?1 AND parent_id IS ?2 AND deleted_at IS NULL",
            params![group_id.to_string(), parent_id.map(|p| p.to_string())],
            |row| row.get(0),
        )?;
        let n = self.conn().execute(
            "INSERT INTO todos
                 (id, group_id, parent_id, title, notes, status, priority,
                  position, due_date, deleted_at, created_at, updated_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, '', 'open', ?5, ?6, NULL, NULL, ?7, ?7, NULL)",
            params![
                id.to_string(),
                group_id.to_string(),
                parent_id.map(|p| p.to_string()),
                title,
                Priority::default().value(),
                pos,
                now
            ],
        )?;
        if n == 0 {
            return Err(TodoError::NotFound(group_id.to_string()));
        }
        Ok(Todo {
            id,
            group_id,
            parent_id,
            title: title.to_string(),
            notes: String::new(),
            status: Status::Open,
            priority: Priority::default(),
            position: pos,
            due_date: None,
            deleted_at: None,
            created_at: now.clone(),
            updated_at: now,
            completed_at: None,
        })
    }

    pub fn get(&self, id: Uuid) -> Result<Todo> {
        self.conn()
            .query_row(
                "SELECT id, group_id, parent_id, title, notes, status, priority,
                        position, due_date, deleted_at, created_at, updated_at, completed_at
                 FROM todos WHERE id = ?1",
                params![id.to_string()],
                |row| self.todo_from_row(row),
            )
            .optional()?
            .ok_or_else(|| TodoError::NotFound(id.to_string()))
    }

    /// Update title/notes/priority/due_date. `None` fields are left unchanged.
    pub fn update(
        &self,
        id: Uuid,
        title: Option<&str>,
        notes: Option<&str>,
        priority: Option<Priority>,
        due_date: Option<Option<NaiveDate>>,
    ) -> Result<Todo> {
        let current = self.get(id)?;
        if current.deleted_at.is_some() {
            return Err(TodoError::Trashed);
        }
        let title = title.unwrap_or(&current.title);
        let notes = notes.unwrap_or(&current.notes);
        let priority = priority.unwrap_or(current.priority);
        let due_date = due_date.unwrap_or(current.due_date);
        let n = self.conn().execute(
            "UPDATE todos SET title = ?2, notes = ?3, priority = ?4, due_date = ?5
             WHERE id = ?1",
            params![
                id.to_string(),
                title,
                notes,
                priority.value(),
                due_date.map(|d| d.to_string())
            ],
        )?;
        if n == 0 {
            return Err(TodoError::NotFound(id.to_string()));
        }
        self.get(id)
    }

    /// Set status. Enforces the completion rule: a todo may become `done`
    /// only when all its descendants are terminal (done/cancelled).
    pub fn set_status(&self, id: Uuid, status: Status) -> Result<Todo> {
        let current = self.get(id)?;
        if current.deleted_at.is_some() {
            return Err(TodoError::Trashed);
        }
        if status == Status::Done {
            let open = self.open_descendant_count(id)?;
            if open > 0 {
                return Err(TodoError::OpenDescendants(open));
            }
        }
        let completed_at = if status.is_terminal() {
            Some(Utc::now().to_rfc3339())
        } else {
            None
        };
        self.conn().execute(
            "UPDATE todos SET status = ?2, completed_at = ?3 WHERE id = ?1",
            params![id.to_string(), status.as_str(), completed_at],
        )?;
        self.get(id)
    }

    // ── Todos: ordering ───────────────────────────────────────────────────

    /// Move `id` one position up (`up = true`) or down among its live
    /// siblings. No-op (Ok) when already at the edge.
    pub fn reorder(&self, id: Uuid, up: bool) -> Result<()> {
        let todo = self.get(id)?;
        if todo.deleted_at.is_some() {
            return Err(TodoError::Trashed);
        }
        let conn = self.conn();
        let tx = conn.unchecked_transaction()?;
        let mut siblings = Vec::new();
        {
            let mut stmt = tx.prepare(
                "SELECT id, position FROM todos
                 WHERE group_id = ?1 AND parent_id IS ?2 AND deleted_at IS NULL
                 ORDER BY position, created_at, id",
            )?;
            let rows = stmt.query_map(
                params![todo.group_id.to_string(), todo.parent_id.map(|p| p.to_string())],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )?;
            for r in rows {
                siblings.push(r?);
            }
        }
        let idx = siblings
            .iter()
            .position(|(sid, _)| *sid == id.to_string())
            .ok_or_else(|| TodoError::NotFound(id.to_string()))?;
        let swap_with = if up {
            idx.checked_sub(1)
        } else {
            if idx + 1 < siblings.len() {
                Some(idx + 1)
            } else {
                None
            }
        };
        if let Some(other) = swap_with {
            let (a_id, a_pos) = &siblings[idx];
            let (b_id, b_pos) = &siblings[other];
            tx.execute("UPDATE todos SET position = ?2 WHERE id = ?1", params![b_id, a_pos])?;
            tx.execute("UPDATE todos SET position = ?2 WHERE id = ?1", params![a_id, b_pos])?;
        }
        tx.commit()?;
        Ok(())
    }

    // ── Todos: tree ───────────────────────────────────────────────────────

    /// Live todos of a group as a nested structure, siblings ordered by
    /// position. Trashed items are excluded entirely.
    pub fn tree(&self, group_id: Uuid) -> Result<Vec<TodoNode>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, group_id, parent_id, title, notes, status, priority,
                    position, due_date, deleted_at, created_at, updated_at, completed_at
             FROM todos
             WHERE group_id = ?1 AND deleted_at IS NULL
             ORDER BY position, created_at, id",
        )?;
        let rows = stmt.query_map(params![group_id.to_string()], |row| self.todo_from_row(row))?;
        let all = collect(rows)?;
        Ok(build_tree(&all))
    }

    /// Todos due before today, live, non-terminal (across all groups).
    pub fn overdue(&self) -> Result<Vec<Todo>> {
        let today = Utc::now().date_naive().to_string();
        let mut stmt = self.conn().prepare(
            "SELECT id, group_id, parent_id, title, notes, status, priority,
                    position, due_date, deleted_at, created_at, updated_at, completed_at
             FROM todos
             WHERE due_date IS NOT NULL AND due_date < ?1
               AND status NOT IN ('done','cancelled') AND deleted_at IS NULL
             ORDER BY due_date",
        )?;
        let rows = stmt.query_map(params![today], |row| self.todo_from_row(row))?;
        collect(rows)
    }

    /// Top-level trashed items (for the Trash view). Children of trashed
    /// parents are not listed separately — restoring the parent restores them.
    pub fn list_trash(&self) -> Result<Vec<Todo>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, group_id, parent_id, title, notes, status, priority,
                    position, due_date, deleted_at, created_at, updated_at, completed_at
             FROM todos
             WHERE deleted_at IS NOT NULL
               AND (parent_id IS NULL OR parent_id NOT IN
                    (SELECT id FROM todos WHERE deleted_at IS NOT NULL))
             ORDER BY deleted_at DESC",
        )?;
        let rows = stmt.query_map([], |row| self.todo_from_row(row))?;
        collect(rows)
    }

    // ── Todos: trash ──────────────────────────────────────────────────────

    /// Soft-delete a todo and all its descendants in one tx. Descendants
    /// inherit the parent's `deleted_at` timestamp. Links are retained.
    pub fn trash(&self, id: Uuid) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn();
        let tx = conn.unchecked_transaction()?;
        let ids = descendant_ids(&tx, &id.to_string())?;
        {
            let mut stmt = tx.prepare("UPDATE todos SET deleted_at = ?2 WHERE id = ?1")?;
            for raw in &ids {
                stmt.execute(params![raw, now])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Restore a trashed todo and all its descendants (same tx). Group,
    /// position and links are untouched by trash/restore by design.
    pub fn restore(&self, id: Uuid) -> Result<()> {
        let conn = self.conn();
        let tx = conn.unchecked_transaction()?;
        let ids = descendant_ids(&tx, &id.to_string())?;
        {
            let mut stmt = tx.prepare("UPDATE todos SET deleted_at = NULL WHERE id = ?1")?;
            for raw in &ids {
                stmt.execute(params![raw])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Hard-delete a todo and all its descendants. Row deletes and link
    /// deletes happen in one transaction.
    pub fn delete_permanent(&self, id: Uuid) -> Result<()> {
        let conn = self.conn();
        let tx = conn.unchecked_transaction()?;
        let ids = descendant_ids(&tx, &id.to_string())?;
        for raw in &ids {
            let ent = EntityRef::new(EntityType::Todo, parse_uuid(raw)?);
            LinkStore::delete_links_for(&tx, &ent)?;
        }
        let mut stmt = tx.prepare("DELETE FROM todos WHERE id = ?1")?;
        for raw in &ids {
            stmt.execute(params![raw])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(())
    }

    /// Hard-delete trashed rows older than 30 days (run at startup). Returns
    /// the number of todos purged. Links are removed in the same tx.
    pub fn purge_expired(&self) -> Result<usize> {
        let cutoff: DateTime<Utc> = Utc::now() - PURGE_AGE;
        let cutoff = cutoff.to_rfc3339();
        let conn = self.conn();
        let tx = conn.unchecked_transaction()?;
        // Expired roots plus anything still attached beneath them.
        let ids: Vec<String> = {
            let mut stmt = tx.prepare(
                "WITH RECURSIVE expired (id) AS (
                     SELECT id FROM todos WHERE deleted_at IS NOT NULL AND deleted_at < ?1
                     UNION
                     SELECT t.id FROM todos t JOIN expired e ON t.parent_id = e.id
                 )
                 SELECT id FROM expired",
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
                let ent = EntityRef::new(EntityType::Todo, uuid);
                LinkStore::delete_links_for(&tx, &ent)?;
            }
        }
        {
            let mut stmt = tx.prepare("DELETE FROM todos WHERE id = ?1")?;
            for raw in &ids {
                stmt.execute(params![raw])?;
            }
        }
        tx.commit()?;
        Ok(ids.len())
    }

    // ── internals ─────────────────────────────────────────────────────────

    /// Count of live, non-terminal descendants (excluding `id` itself).
    fn open_descendant_count(&self, id: Uuid) -> Result<usize> {
        let n: i64 = self.conn().query_row(
            "WITH RECURSIVE sub (id, status) AS (
                 SELECT id, status FROM todos WHERE parent_id = ?1 AND deleted_at IS NULL
                 UNION ALL
                 SELECT t.id, t.status FROM todos t JOIN sub s ON t.parent_id = s.id
                 WHERE t.deleted_at IS NULL
             )
             SELECT COUNT(*) FROM sub WHERE status NOT IN ('done','cancelled')",
            params![id.to_string()],
            |row| row.get(0),
        )?;
        usize::try_from(n).map_err(|_| TodoError::NotFound(id.to_string()))
    }

    /// Live descendant count including `id` itself (UI delete confirm).
    pub fn live_subtree_count(&self, id: Uuid) -> Result<usize> {
        Ok(self.open_subtree_count(id)?)
    }

    fn open_subtree_count(&self, id: Uuid) -> Result<usize> {
        let n: i64 = self.conn().query_row(
            "WITH RECURSIVE sub (id) AS (
                 SELECT id FROM todos WHERE id = ?1
                 UNION ALL
                 SELECT t.id FROM todos t JOIN sub s ON t.parent_id = s.id
             )
             SELECT COUNT(*) FROM sub",
            params![id.to_string()],
            |row| row.get(0),
        )?;
        usize::try_from(n).map_err(|_| TodoError::NotFound(id.to_string()))
    }

    fn todo_from_row(&self, row: &rusqlite::Row<'_>) -> rusqlite::Result<Todo> {
        let status: String = row.get(5)?;
        let priority: i64 = row.get(6)?;
        let due: Option<String> = row.get(8)?;
        Ok(Todo {
            id: parse_uuid(&row.get::<_, String>(0)?)?,
            group_id: parse_uuid(&row.get::<_, String>(1)?)?,
            parent_id: row
                .get::<_, Option<String>>(2)?
                .map(|s| parse_uuid(&s))
                .transpose()?,
            title: row.get(3)?,
            notes: row.get(4)?,
            status: status.parse().map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(TodoError::UnknownStatus(status.clone())),
                )
            })?,
            priority: Priority::new(u8::try_from(priority).unwrap_or(u8::MAX)).map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Integer,
                    Box::new(TodoError::InvalidPriority(priority as u8)),
                )
            })?,
            position: row.get(7)?,
            due_date: due
                .map(|d| NaiveDate::parse_from_str(&d, "%Y-%m-%d"))
                .transpose()
                .map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        8,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
            deleted_at: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
            completed_at: row.get(12)?,
        })
    }
}

/// All IDs in the subtree rooted at `root_id` (including the root), walking
/// over both live and trashed rows so purge/restore see the whole subtree.
fn descendant_ids(conn: &Connection, root_id: &str) -> std::result::Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE sub (id) AS (
             SELECT id FROM todos WHERE id = ?1
             UNION ALL
             SELECT t.id FROM todos t JOIN sub s ON t.parent_id = s.id
         )
         SELECT id FROM sub",
    )?;
    let rows = stmt.query_map(params![root_id], |row| row.get(0))?;
    let mut v = Vec::new();
    for r in rows {
        v.push(r?);
    }
    Ok(v)
}

/// Assemble a flat, live-only todo list (parents before children per the
/// position ordering of the tree query) into nested nodes.
fn build_tree(all: &[Todo]) -> Vec<TodoNode> {
    fn attach(all: &[Todo], parent_id: Option<Uuid>) -> Vec<TodoNode> {
        all.iter()
            .filter(|t| t.parent_id == parent_id)
            .map(|t| TodoNode {
                todo: t.clone(),
                children: attach(all, Some(t.id)),
            })
            .collect()
    }
    attach(all, None)
}

fn parse_uuid(s: &str) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(s)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}

fn collect<T>(rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>) -> Result<Vec<T>> {
    let mut v = Vec::new();
    for r in rows {
        v.push(r?);
    }
    Ok(v)
}
