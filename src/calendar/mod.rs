//! Calendar module: store, recurrence expansion, reminders, UI.
//!
//! `Notifier` (notify.rs) is the desktop-notification test seam. No live
//! network anywhere in this module. All SQL for the calendar module lives in
//! this file; UI files contain none. Expansion happens in Rust over base
//! rows (spec §5) — SQL returns rows only.

pub mod model;
pub mod notify;
pub mod reminders;
pub mod rrule;
pub mod ui;

use std::collections::HashMap;
use std::str::FromStr;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::core::entity::{EntityRef, EntityType};
use crate::core::link::LinkStore;
use crate::db::{Db, DbError};

pub use model::{
    Attendee, AttendeeInput, DueReminder, Event, EventInput, Exception, ExceptionKind, Freq,
    Occurrence, OccurrenceChanges, Recurrence, Reminder, Rsvp,
};

use rrule::RruleError;

#[derive(Debug, thiserror::Error)]
pub enum CalendarError {
    #[error("database error: {0}")]
    Db(#[from] DbError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("event not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("event is not recurring: {0}")]
    NotRecurring(String),
    #[error("recurrence error: {0}")]
    Rrule(#[from] RruleError),
    #[error("corrupt row: {0}")]
    Corrupt(String),
}

pub type Result<T> = std::result::Result<T, CalendarError>;

/// Trashed-event purge grace period, matching the todo rule (spec §3).
const PURGE_AGE: Duration = Duration::days(30);

pub struct CalendarStore<'a> {
    db: &'a Db,
}

impl<'a> CalendarStore<'a> {
    pub fn new(db: &'a Db) -> Self {
        CalendarStore { db }
    }

    fn conn(&self) -> &Connection {
        self.db.conn()
    }

    // ── CRUD ──────────────────────────────────────────────────────────────

    /// Create an event (and its attendees) in one transaction.
    pub fn create_event(&self, input: &EventInput) -> Result<Uuid> {
        validate_input(input)?;
        let tx = self.conn().unchecked_transaction()?;
        let id = insert_event(&tx, input)?;
        tx.commit()?;
        Ok(id)
    }

    /// Replace an event's fields and attendees (one transaction). Existing
    /// per-occurrence exceptions are kept only if their occurrence still
    /// exists in the (possibly new) series, else pruned (spec §5).
    pub fn update_event(&self, id: Uuid, input: &EventInput) -> Result<()> {
        validate_input(input)?;
        let tx = self.conn().unchecked_transaction()?;
        let n = tx.execute(
            "UPDATE calendar_events SET
                 title=?2, description=?3, location=?4, all_day=?5,
                 start_utc=?6, end_utc=?7, start_date=?8, end_date=?9,
                 tz=?10, rrule=?11, color_idx=?12
             WHERE id=?1",
            params![
                id.to_string(),
                input.title,
                input.description,
                input.location,
                input.all_day as i64,
                input.start_utc.map(|d| d.to_rfc3339()),
                input.end_utc.map(|d| d.to_rfc3339()),
                input.start_date.map(|d| d.to_string()),
                input.end_date.map(|d| d.to_string()),
                input.tz,
                input.rrule,
                input.color_idx,
            ],
        )?;
        if n == 0 {
            return Err(CalendarError::NotFound(id.to_string()));
        }
        tx.execute(
            "DELETE FROM calendar_attendees WHERE event_id = ?1",
            params![id.to_string()],
        )?;
        insert_attendees(&tx, id, input)?;
        prune_exceptions(&tx, id, input)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_event(&self, id: Uuid) -> Result<Event> {
        let mut events = self.base_events("id = ?1", params![id.to_string()])?;
        let event = events
            .pop()
            .ok_or_else(|| CalendarError::NotFound(id.to_string()))?;
        let mut events = vec![event];
        self.attach_relations(&mut events)?;
        events
            .pop()
            .ok_or_else(|| CalendarError::Corrupt("event vanished during load".into()))
    }

    pub fn trash_event(&self, id: Uuid) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE calendar_events SET trashed_at = ?2 WHERE id = ?1",
            params![id.to_string(), Utc::now().to_rfc3339()],
        )?;
        if n == 0 {
            return Err(CalendarError::NotFound(id.to_string()));
        }
        Ok(())
    }

    pub fn restore_event(&self, id: Uuid) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE calendar_events SET trashed_at = NULL WHERE id = ?1",
            params![id.to_string()],
        )?;
        if n == 0 {
            return Err(CalendarError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Hard delete: event row (relations cascade) + link purge, one tx.
    pub fn delete_event(&self, id: Uuid) -> Result<()> {
        let tx = self.conn().unchecked_transaction()?;
        let ent = EntityRef::new(EntityType::Event, id);
        LinkStore::delete_links_for(&tx, &ent)?;
        let n = tx.execute(
            "DELETE FROM calendar_events WHERE id = ?1",
            params![id.to_string()],
        )?;
        if n == 0 {
            return Err(CalendarError::NotFound(id.to_string()));
        }
        tx.commit()?;
        Ok(())
    }

    // ── Window queries (expansion in Rust; SQL returns base rows) ─────────

    /// Live events with at least one occurrence in the window. Recurring
    /// events are included as base rows; the caller expands via `rrule`.
    pub fn events_in_window(
        &self,
        from_utc: DateTime<Utc>,
        to_utc: DateTime<Utc>,
    ) -> Result<Vec<Event>> {
        let mut events = self.base_events("trashed_at IS NULL", &[])?;
        self.attach_relations(&mut events)?;
        Ok(events
            .iter()
            .filter(|e| {
                rrule::expand(e, from_utc, to_utc)
                    .map(|o| !o.is_empty())
                    .unwrap_or(false)
            })
            .cloned()
            .collect())
    }

    /// Next `limit` events with upcoming occurrences, soonest first
    /// (deduped; see `upcoming_occurrences` for the occurrence-level view).
    pub fn upcoming_events(&self, now: DateTime<Utc>, limit: usize) -> Result<Vec<Event>> {
        let occs = self.upcoming_occurrences(now, limit)?;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for occ in occs {
            if seen.insert(occ.event_id) {
                out.push(self.get_event(occ.event_id)?);
            }
        }
        Ok(out)
    }

    /// Next `limit` occurrences across all live events, soonest first.
    /// Window grows geometrically (7d → 28d → …, capped at 2y) until enough
    /// occurrences or the cap is reached.
    pub fn upcoming_occurrences(
        &self,
        now: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<Occurrence>> {
        let mut events = self.base_events("trashed_at IS NULL", &[])?;
        self.attach_relations(&mut events)?;
        let mut horizon = Duration::days(7);
        let mut all: Vec<Occurrence> = loop {
            let to = now + horizon;
            let occs: Vec<Occurrence> = events
                .iter()
                .filter_map(|e| rrule::expand(e, now, to).ok())
                .flatten()
                .collect();
            if occs.len() >= limit || horizon >= Duration::days(730) {
                break occs;
            }
            horizon = horizon * 4;
        };
        all.sort_by_key(occurrence_sort_key);
        all.truncate(limit);
        Ok(all)
    }

    // ── Recurring edits ───────────────────────────────────────────────────

    /// Record a per-occurrence exception (`skip` = cancelled). Upserts on
    /// (event_id, occurrence_utc).
    pub fn add_exception(
        &self,
        event_id: Uuid,
        occurrence: DateTime<Utc>,
        kind: ExceptionKind,
    ) -> Result<()> {
        self.ensure_event(event_id)?;
        let kind_str = match kind {
            ExceptionKind::Skip => "skip",
            ExceptionKind::Modify => "modify",
        };
        self.conn().execute(
            "INSERT INTO calendar_event_exceptions
                 (id, event_id, occurrence_utc, kind, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?5)
             ON CONFLICT(event_id, occurrence_utc) DO UPDATE SET
                 kind = excluded.kind, changes_json = NULL",
            params![
                Uuid::new_v4().to_string(),
                event_id.to_string(),
                occurrence.to_rfc3339(),
                kind_str,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Record (upsert) a per-occurrence field override.
    pub fn modify_occurrence(
        &self,
        event_id: Uuid,
        occurrence: DateTime<Utc>,
        changes: &OccurrenceChanges,
    ) -> Result<()> {
        self.ensure_event(event_id)?;
        let changes_json = serde_json::to_string(changes)?;
        self.conn().execute(
            "INSERT INTO calendar_event_exceptions
                 (id, event_id, occurrence_utc, kind, changes_json, created_at, updated_at)
             VALUES (?1,?2,?3,'modify',?4,?5,?5)
             ON CONFLICT(event_id, occurrence_utc) DO UPDATE SET
                 kind = 'modify', changes_json = excluded.changes_json",
            params![
                Uuid::new_v4().to_string(),
                event_id.to_string(),
                occurrence.to_rfc3339(),
                changes_json,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Series split ("this and future", spec §5): the original event gets
    /// `UNTIL = occurrence - one period` (computed in TZ wall-clock; COUNT
    /// is dropped since the series is shortened); a NEW event is created
    /// from `input` (carrying the modified occurrence as its start and the
    /// same RRULE) and its id returned. Original update + clone happen in
    /// one transaction.
    pub fn edit_future(
        &self,
        event_id: Uuid,
        from_occurrence: DateTime<Utc>,
        input: &EventInput,
    ) -> Result<Uuid> {
        validate_input(input)?;
        let original = self.get_event(event_id)?;
        let rrule_text = original
            .rrule
            .clone()
            .ok_or_else(|| CalendarError::NotRecurring(event_id.to_string()))?;
        let rule = rrule::parse_rrule(&rrule_text)?;
        let tz: Tz = original
            .tz
            .parse()
            .map_err(|_| RruleError::UnknownTz(original.tz.clone()))?;
        let boundary = rrule::prev_in_wall_clock(&tz, &rule, from_occurrence)?;
        let mut shortened = rule;
        shortened.until = Some(boundary);
        shortened.count = None;

        let tx = self.conn().unchecked_transaction()?;
        let n = tx.execute(
            "UPDATE calendar_events SET rrule = ?2 WHERE id = ?1",
            params![event_id.to_string(), rrule::format_rrule(&shortened)],
        )?;
        if n == 0 {
            return Err(CalendarError::NotFound(event_id.to_string()));
        }
        let new_id = insert_event(&tx, input)?;
        tx.commit()?;
        Ok(new_id)
    }

    // ── Reminders ─────────────────────────────────────────────────────────

    pub fn reminders_for_event(&self, event_id: Uuid) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, event_id, offset_minutes, created_at, updated_at
             FROM calendar_reminders WHERE event_id = ?1 ORDER BY offset_minutes",
        )?;
        let rows = stmt.query_map(params![event_id.to_string()], map_reminder_row)?;
        collect(rows)
    }

    /// Live events that have at least one reminder, with their reminders —
    /// the scheduler's working set. Trashed events are excluded (spec §8.5).
    pub fn events_with_reminders(&self) -> Result<Vec<(Event, Vec<Reminder>)>> {
        let mut events = self.base_events(
            "trashed_at IS NULL AND id IN (SELECT DISTINCT event_id FROM calendar_reminders)",
            &[],
        )?;
        self.attach_relations(&mut events)?;
        let ids: Vec<String> = events.iter().map(|e| e.id.to_string()).collect();
        let mut by_event: HashMap<Uuid, Vec<Reminder>> = HashMap::new();
        if !ids.is_empty() {
            let placeholders = repeat_vars(ids.len());
            let sql = format!(
                "SELECT id, event_id, offset_minutes, created_at, updated_at
                 FROM calendar_reminders WHERE event_id IN ({placeholders})"
            );
            let mut stmt = self.conn().prepare(&sql)?;
            let rows = stmt.query_map(
                rusqlite::params_from_iter(ids.iter().map(|s| s.as_str())),
                map_reminder_row,
            )?;
            for r in collect(rows)? {
                by_event.entry(r.event_id).or_default().push(r);
            }
        }
        Ok(events
            .into_iter()
            .map(|e| {
                let mut rs = by_event.remove(&e.id).unwrap_or_default();
                rs.sort_by_key(|r| r.offset_minutes);
                (e, rs)
            })
            .collect())
    }

    pub fn add_reminder(&self, event_id: Uuid, offset_minutes: i64) -> Result<Uuid> {
        self.ensure_event(event_id)?;
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        self.conn().execute(
            "INSERT INTO calendar_reminders (id, event_id, offset_minutes, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?4)",
            params![id.to_string(), event_id.to_string(), offset_minutes, now],
        )?;
        Ok(id)
    }

    pub fn remove_reminder(&self, id: Uuid) -> Result<()> {
        let n = self.conn().execute(
            "DELETE FROM calendar_reminders WHERE id = ?1",
            params![id.to_string()],
        )?;
        if n == 0 {
            return Err(CalendarError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Pending fires (never fired, not dismissed, live event) whose
    /// effective fire instant (snooze target when set, else scheduled) is
    /// due by `now`.
    pub fn due_reminders(&self, now: DateTime<Utc>) -> Result<Vec<DueReminder>> {
        self.query_due(
            "f.fired_at IS NULL AND f.dismissed = 0 AND e.trashed_at IS NULL
             AND COALESCE(f.snoozed_to_utc, f.fire_at_utc) <= ?1",
            params![now.to_rfc3339()],
        )
    }

    /// Missed fires in `[now - horizon, now]`, at most once each (the ledger
    /// row is the dedupe key — the caller marks them fired).
    pub fn catch_up_due(&self, now: DateTime<Utc>, horizon: Duration) -> Result<Vec<DueReminder>> {
        self.query_due(
            "f.fired_at IS NULL AND f.dismissed = 0 AND e.trashed_at IS NULL
             AND COALESCE(f.snoozed_to_utc, f.fire_at_utc) <= ?1
             AND f.fire_at_utc >= ?2",
            params![now.to_rfc3339(), (now - horizon).to_rfc3339()],
        )
    }

    fn query_due(
        &self,
        filter: &str,
        query_params: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<DueReminder>> {
        let sql = format!(
            "SELECT f.id, f.reminder_id, r.event_id, f.occurrence_utc,
                    f.fire_at_utc, f.snoozed_to_utc, f.fired_at,
                    r.offset_minutes, e.title
             FROM calendar_reminder_fires f
             JOIN calendar_reminders r ON r.id = f.reminder_id
             JOIN calendar_events e ON e.id = r.event_id
             WHERE {filter}
             ORDER BY COALESCE(f.snoozed_to_utc, f.fire_at_utc)"
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(query_params, |row| {
            Ok(DueReminder {
                fire_id: parse_uuid(&row.get::<_, String>(0)?)?,
                reminder_id: parse_uuid(&row.get::<_, String>(1)?)?,
                event_id: parse_uuid(&row.get::<_, String>(2)?)?,
                occurrence_utc: dt(&row.get::<_, String>(3)?)?,
                fire_at_utc: dt(&row.get::<_, String>(4)?)?,
                snoozed_to_utc: row
                    .get::<_, Option<String>>(5)?
                    .map(|s| dt(&s))
                    .transpose()?,
                fired_at: row
                    .get::<_, Option<String>>(6)?
                    .map(|s| dt(&s))
                    .transpose()?,
                offset_minutes: row.get(7)?,
                title: row.get(8)?,
            })
        })?;
        collect(rows)
    }

    pub fn mark_fired(&self, fire_id: Uuid, at: DateTime<Utc>) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE calendar_reminder_fires
             SET fired_at = ?2, snoozed_to_utc = NULL
             WHERE id = ?1",
            params![fire_id.to_string(), at.to_rfc3339()],
        )?;
        if n == 0 {
            return Err(CalendarError::NotFound(fire_id.to_string()));
        }
        Ok(())
    }

    pub fn snooze_fire(&self, fire_id: Uuid, to: DateTime<Utc>) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE calendar_reminder_fires
             SET snoozed_to_utc = ?2, fired_at = NULL
             WHERE id = ?1",
            params![fire_id.to_string(), to.to_rfc3339()],
        )?;
        if n == 0 {
            return Err(CalendarError::NotFound(fire_id.to_string()));
        }
        Ok(())
    }

    pub fn dismiss_fire(&self, fire_id: Uuid) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE calendar_reminder_fires SET dismissed = 1 WHERE id = ?1",
            params![fire_id.to_string()],
        )?;
        if n == 0 {
            return Err(CalendarError::NotFound(fire_id.to_string()));
        }
        Ok(())
    }

    /// Idempotent ledger write: one row per (reminder, occurrence). An
    /// existing row (fired/snoozed/dismissed) is never disturbed — this is
    /// the exactly-once guarantee.
    pub fn upsert_fire(
        &self,
        reminder_id: Uuid,
        occurrence_utc: DateTime<Utc>,
        fire_at_utc: DateTime<Utc>,
    ) -> Result<Uuid> {
        self.conn().execute(
            "INSERT INTO calendar_reminder_fires
                 (id, reminder_id, occurrence_utc, fire_at_utc, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?5)
             ON CONFLICT(reminder_id, occurrence_utc) DO NOTHING",
            params![
                Uuid::new_v4().to_string(),
                reminder_id.to_string(),
                occurrence_utc.to_rfc3339(),
                fire_at_utc.to_rfc3339(),
                Utc::now().to_rfc3339(),
            ],
        )?;
        self.conn()
            .query_row(
                "SELECT id FROM calendar_reminder_fires
                 WHERE reminder_id = ?1 AND occurrence_utc = ?2",
                params![reminder_id.to_string(), occurrence_utc.to_rfc3339()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|s| parse_uuid_str(&s))
            .transpose()?
            .ok_or_else(|| CalendarError::Corrupt("fire row vanished after upsert".into()))
    }

    /// Title search over live events (link picker). Relations are not
    /// attached — the picker only needs id + title.
    pub fn search_events(&self, needle: &str, limit: usize) -> Result<Vec<Event>> {
        let pattern = format!("%{}%", needle.replace('%', "_"));
        let events = self.base_events("trashed_at IS NULL AND title LIKE ?1", params![pattern])?;
        Ok(events.into_iter().take(limit).collect())
    }

    /// A single reminder row by id (read-only link resolution).
    pub fn get_reminder(&self, id: Uuid) -> Result<Reminder> {
        let mut stmt = self.conn().prepare(
            "SELECT id, event_id, offset_minutes, created_at, updated_at
             FROM calendar_reminders WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id.to_string()], map_reminder_row)?;
        let row = rows
            .next()
            .transpose()?
            .ok_or_else(|| CalendarError::NotFound(id.to_string()))?;
        Ok(row)
    }

    /// Banner working set (spec §6): pending-due fires plus fired-within-
    /// `horizon` rows — the user can Dismiss/Snooze either kind. Trashed
    /// events and dismissed rows are excluded.
    pub fn banner_items(&self, now: DateTime<Utc>, horizon: Duration) -> Result<Vec<DueReminder>> {
        self.query_due(
            "f.dismissed = 0 AND e.trashed_at IS NULL AND (
                 (f.fired_at IS NULL AND COALESCE(f.snoozed_to_utc, f.fire_at_utc) <= ?1)
                 OR f.fired_at >= ?2
             )",
            params![now.to_rfc3339(), (now - horizon).to_rfc3339()],
        )
    }

    /// Hard-delete events trashed more than 30 days ago (startup purge,
    /// same rule as todos). Links are purged in the same transaction;
    /// attendees/exceptions/reminders/fires cascade via FK.
    pub fn purge_expired(&self) -> Result<usize> {
        let cutoff = (Utc::now() - PURGE_AGE).to_rfc3339();
        let tx = self.conn().unchecked_transaction()?;
        let ids: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM calendar_events WHERE trashed_at IS NOT NULL AND trashed_at < ?1",
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
                let ent = EntityRef::new(EntityType::Event, uuid);
                LinkStore::delete_links_for(&tx, &ent)?;
            }
        }
        {
            let mut stmt = tx.prepare("DELETE FROM calendar_events WHERE id = ?1")?;
            for raw in &ids {
                stmt.execute(params![raw])?;
            }
        }
        tx.commit()?;
        Ok(ids.len())
    }

    // ── internals ─────────────────────────────────────────────────────────

    fn ensure_event(&self, id: Uuid) -> Result<()> {
        let found: Option<String> = self
            .conn()
            .query_row(
                "SELECT id FROM calendar_events WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if found.is_none() {
            return Err(CalendarError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Base event rows (no attendees/exceptions), filtered by a WHERE
    /// fragment, ordered by start.
    fn base_events(
        &self,
        filter: &str,
        query_params: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<Event>> {
        let sql = format!(
            "SELECT id, title, description, location, all_day,
                    start_utc, end_utc, start_date, end_date, tz, rrule,
                    color_idx, trashed_at, created_at, updated_at
             FROM calendar_events
             WHERE {filter}"
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(query_params, |row| {
            Ok(Event {
                id: parse_uuid(&row.get::<_, String>(0)?)?,
                title: row.get(1)?,
                description: row.get(2)?,
                location: row.get(3)?,
                all_day: row.get::<_, i64>(4)? != 0,
                start_utc: row
                    .get::<_, Option<String>>(5)?
                    .map(|s| dt(&s))
                    .transpose()?,
                end_utc: row
                    .get::<_, Option<String>>(6)?
                    .map(|s| dt(&s))
                    .transpose()?,
                start_date: row
                    .get::<_, Option<String>>(7)?
                    .map(|s| NaiveDate::from_str(&s).map_err(|e| corrupt(&s, &e)))
                    .transpose()?,
                end_date: row
                    .get::<_, Option<String>>(8)?
                    .map(|s| NaiveDate::from_str(&s).map_err(|e| corrupt(&s, &e)))
                    .transpose()?,
                tz: row.get(9)?,
                rrule: row.get(10)?,
                color_idx: row.get(11)?,
                trashed_at: row
                    .get::<_, Option<String>>(12)?
                    .map(|s| dt(&s))
                    .transpose()?,
                created_at: dt(&row.get::<_, String>(13)?)?,
                updated_at: dt(&row.get::<_, String>(14)?)?,
                attendees: Vec::new(),
                exceptions: Vec::new(),
            })
        })?;
        let mut events = collect(rows)?;
        events.sort_by_key(event_sort_key);
        Ok(events)
    }

    /// Batch-load attendees and exceptions for the given events.
    fn attach_relations(&self, events: &mut [Event]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let ids: Vec<String> = events.iter().map(|e| e.id.to_string()).collect();
        let placeholders = repeat_vars(ids.len());

        let mut by_event: HashMap<Uuid, Vec<Attendee>> = HashMap::new();
        {
            let sql = format!(
                "SELECT id, event_id, name, email, rsvp, created_at, updated_at
                 FROM calendar_attendees WHERE event_id IN ({placeholders})"
            );
            let mut stmt = self.conn().prepare(&sql)?;
            let rows = stmt.query_map(
                rusqlite::params_from_iter(ids.iter().map(|s| s.as_str())),
                |row| {
                    Ok(Attendee {
                        id: parse_uuid(&row.get::<_, String>(0)?)?,
                        event_id: parse_uuid(&row.get::<_, String>(1)?)?,
                        name: row.get(2)?,
                        email: row.get(3)?,
                        rsvp: row.get::<_, String>(4)?.parse::<Rsvp>().map_err(|e| {
                            rusqlite::Error::FromSqlConversionFailure(
                                4,
                                rusqlite::types::Type::Text,
                                e.into(),
                            )
                        })?,
                        created_at: dt(&row.get::<_, String>(5)?)?,
                        updated_at: dt(&row.get::<_, String>(6)?)?,
                    })
                },
            )?;
            for a in collect(rows)? {
                by_event.entry(a.event_id).or_default().push(a);
            }
        }

        let mut exc_by_event: HashMap<Uuid, Vec<Exception>> = HashMap::new();
        {
            let sql = format!(
                "SELECT id, event_id, occurrence_utc, kind, changes_json, created_at, updated_at
                 FROM calendar_event_exceptions WHERE event_id IN ({placeholders})"
            );
            let mut stmt = self.conn().prepare(&sql)?;
            let rows = stmt.query_map(
                rusqlite::params_from_iter(ids.iter().map(|s| s.as_str())),
                |row| {
                    let kind_str: String = row.get(3)?;
                    let changes_json: Option<String> = row.get(4)?;
                    let kind = match kind_str.as_str() {
                        "skip" => ExceptionKind::Skip,
                        "modify" => ExceptionKind::Modify,
                        _ => {
                            return Err(rusqlite::Error::FromSqlConversionFailure(
                                3,
                                rusqlite::types::Type::Text,
                                format!("unknown exception kind: {kind_str}").into(),
                            ))
                        }
                    };
                    Ok(Exception {
                        id: parse_uuid(&row.get::<_, String>(0)?)?,
                        event_id: parse_uuid(&row.get::<_, String>(1)?)?,
                        occurrence_utc: dt(&row.get::<_, String>(2)?)?,
                        kind,
                        changes: changes_json
                            .map(|s| serde_json::from_str(&s).map_err(corrupt_json))
                            .transpose()?,
                        created_at: dt(&row.get::<_, String>(5)?)?,
                        updated_at: dt(&row.get::<_, String>(6)?)?,
                    })
                },
            )?;
            for ex in collect(rows)? {
                exc_by_event.entry(ex.event_id).or_default().push(ex);
            }
        }

        for e in events.iter_mut() {
            e.attendees = by_event.remove(&e.id).unwrap_or_default();
            e.exceptions = exc_by_event.remove(&e.id).unwrap_or_default();
        }
        Ok(())
    }
}

// ── free helpers (no SQL below this line except inside fns above) ──────────

fn validate_input(input: &EventInput) -> Result<()> {
    input
        .tz
        .parse::<Tz>()
        .map_err(|_| CalendarError::InvalidInput(format!("unknown timezone: {}", input.tz)))?;
    if !(0..=5).contains(&input.color_idx) {
        return Err(CalendarError::InvalidInput(format!(
            "color_idx {} out of range (0..=5)",
            input.color_idx
        )));
    }
    if input.all_day {
        match (input.start_date, input.end_date) {
            (Some(s), Some(e)) if e >= s => {}
            (Some(_), Some(_)) => {
                return Err(CalendarError::InvalidInput(
                    "all-day end_date before start_date".into(),
                ))
            }
            _ => {
                return Err(CalendarError::InvalidInput(
                    "all-day event requires start_date and end_date".into(),
                ))
            }
        }
    } else {
        match (input.start_utc, input.end_utc) {
            (Some(s), Some(e)) if e > s => {}
            (Some(_), Some(_)) => {
                return Err(CalendarError::InvalidInput("end before start".into()))
            }
            _ => {
                return Err(CalendarError::InvalidInput(
                    "timed event requires start_utc and end_utc".into(),
                ))
            }
        }
    }
    if let Some(text) = &input.rrule {
        rrule::parse_rrule(text)?;
    }
    Ok(())
}

fn insert_event(tx: &Connection, input: &EventInput) -> Result<Uuid> {
    let id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();
    tx.execute(
        "INSERT INTO calendar_events
             (id, title, description, location, all_day,
              start_utc, end_utc, start_date, end_date, tz, rrule,
              color_idx, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13)",
        params![
            id.to_string(),
            input.title,
            input.description,
            input.location,
            input.all_day as i64,
            input.start_utc.map(|d| d.to_rfc3339()),
            input.end_utc.map(|d| d.to_rfc3339()),
            input.start_date.map(|d| d.to_string()),
            input.end_date.map(|d| d.to_string()),
            input.tz,
            input.rrule,
            input.color_idx,
            now,
        ],
    )?;
    insert_attendees(tx, id, input)?;
    Ok(id)
}

fn insert_attendees(tx: &Connection, event_id: Uuid, input: &EventInput) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    for a in &input.attendees {
        tx.execute(
            "INSERT INTO calendar_attendees (id, event_id, name, email, rsvp, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?6)",
            params![
                Uuid::new_v4().to_string(),
                event_id.to_string(),
                a.name,
                a.email,
                a.rsvp.as_str(),
                now,
            ],
        )?;
    }
    Ok(())
}

/// Drop per-occurrence exceptions whose occurrence no longer exists in the
/// event's (new) series. Membership is computed WITHOUT applying exceptions,
/// against the exact generated instants (spec §5, "entire series" edit).
/// Failures (e.g. iteration cap) leave exceptions untouched.
fn prune_exceptions(tx: &Connection, event_id: Uuid, input: &EventInput) -> Result<()> {
    let excs: Vec<(String, String)> = {
        let mut stmt = tx.prepare(
            "SELECT id, occurrence_utc FROM calendar_event_exceptions WHERE event_id = ?1",
        )?;
        let rows = stmt.query_map(params![event_id.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        collect(rows)?
    };
    if excs.is_empty() {
        return Ok(());
    }
    let probe = Event {
        id: event_id,
        title: input.title.clone(),
        description: input.description.clone(),
        location: input.location.clone(),
        all_day: input.all_day,
        start_utc: input.start_utc,
        end_utc: input.end_utc,
        start_date: input.start_date,
        end_date: input.end_date,
        tz: input.tz.clone(),
        rrule: input.rrule.clone(),
        color_idx: input.color_idx,
        trashed_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        attendees: Vec::new(),
        exceptions: Vec::new(),
    };
    for (exc_id, occ_str) in excs {
        let keep = match dt(&occ_str) {
            Ok(occ) => {
                // ±1s window around the exception instant: an occurrence
                // starting exactly at `occ` intersects it.
                let expanded = rrule::expand(
                    &probe,
                    occ - Duration::seconds(1),
                    occ + Duration::seconds(1),
                );
                match expanded {
                    Ok(occs) => occs.iter().any(|o| o.start_utc == Some(occ)),
                    Err(_) => true,
                }
            }
            Err(_) => false,
        };
        if !keep {
            tx.execute(
                "DELETE FROM calendar_event_exceptions WHERE id = ?1",
                params![exc_id],
            )?;
        }
    }
    Ok(())
}

fn event_sort_key(e: &Event) -> DateTime<Utc> {
    e.start_utc.unwrap_or_else(|| {
        e.start_date
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|n| n.and_utc())
            .unwrap_or(DateTime::<Utc>::MIN_UTC)
    })
}

fn occurrence_sort_key(o: &Occurrence) -> DateTime<Utc> {
    o.start_utc.unwrap_or_else(|| {
        o.start_date
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|n| n.and_utc())
            .unwrap_or(DateTime::<Utc>::MIN_UTC)
    })
}

fn repeat_vars(n: usize) -> String {
    std::iter::repeat_n("?", n).collect::<Vec<_>>().join(",")
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

fn map_reminder_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Reminder> {
    Ok(Reminder {
        id: parse_uuid(&row.get::<_, String>(0)?)?,
        event_id: parse_uuid(&row.get::<_, String>(1)?)?,
        offset_minutes: row.get(2)?,
        created_at: dt(&row.get::<_, String>(3)?)?,
        updated_at: dt(&row.get::<_, String>(4)?)?,
    })
}

fn parse_uuid(s: &str) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn parse_uuid_str(s: &str) -> Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| CalendarError::Corrupt(format!("bad uuid {s:?}: {e}")))
}

fn corrupt(s: &str, e: &chrono::ParseError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("bad date {s:?}: {e}").into(),
    )
}

fn corrupt_json(e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
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
