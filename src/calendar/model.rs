//! Calendar domain types: events, attendees, recurrence, exceptions, reminders.
//!
//! Timestamps are `DateTime<Utc>` (ISO-8601 text in the DB); all-day events
//! use `NaiveDate` ('YYYY-MM-DD' text). `OccurrenceChanges` is the only type
//! serialized to JSON (stored in `calendar_event_exceptions.changes_json`).

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A calendar event row. Timed events carry `start_utc`/`end_utc`; all-day
/// events carry `start_date`/`end_date` — exactly one shape is non-NULL per
/// row (CHECK constraint). `tz` is the IANA wall-clock anchor.
#[derive(Debug, Clone)]
pub struct Event {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub location: String,
    pub all_day: bool,
    pub start_utc: Option<DateTime<Utc>>,
    pub end_utc: Option<DateTime<Utc>>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub tz: String,
    /// Raw RRULE subset text; `None` = non-recurring.
    pub rrule: Option<String>,
    pub color_idx: i64,
    pub trashed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub attendees: Vec<Attendee>,
    pub exceptions: Vec<Exception>,
}

/// Input shape for create/update: an `Event` sans id, timestamps, trash
/// state, and loaded relations.
#[derive(Debug, Clone)]
pub struct EventInput {
    pub title: String,
    pub description: String,
    pub location: String,
    pub all_day: bool,
    pub start_utc: Option<DateTime<Utc>>,
    pub end_utc: Option<DateTime<Utc>>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub tz: String,
    pub rrule: Option<String>,
    pub color_idx: i64,
    pub attendees: Vec<AttendeeInput>,
}

#[derive(Debug, Clone)]
pub struct AttendeeInput {
    pub name: String,
    pub email: String,
    pub rsvp: Rsvp,
}

#[derive(Debug, Clone)]
pub struct Attendee {
    pub id: Uuid,
    pub event_id: Uuid,
    pub name: String,
    pub email: String,
    pub rsvp: Rsvp,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rsvp {
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
}

impl Rsvp {
    pub fn as_str(&self) -> &'static str {
        match self {
            Rsvp::NeedsAction => "needs_action",
            Rsvp::Accepted => "accepted",
            Rsvp::Declined => "declined",
            Rsvp::Tentative => "tentative",
        }
    }
}

impl std::str::FromStr for Rsvp {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "needs_action" => Ok(Rsvp::NeedsAction),
            "accepted" => Ok(Rsvp::Accepted),
            "declined" => Ok(Rsvp::Declined),
            "tentative" => Ok(Rsvp::Tentative),
            _ => Err(format!("unknown rsvp: {s}")),
        }
    }
}

/// A parsed RRULE (subset per spec §5). `byday`/`bymonthday` are filters —
/// when both are present they INTERSECT (RFC 5545): a candidate day must
/// match a BYDAY weekday AND a BYMONTHDAY day-of-month.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recurrence {
    pub freq: Freq,
    pub interval: u32,
    pub byday: Vec<chrono::Weekday>,
    pub bymonthday: Vec<i32>,
    pub until: Option<DateTime<Utc>>,
    pub count: Option<u32>,
    pub wkst: chrono::Weekday,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// Per-occurrence exception of a recurring event. `changes` is `Some` only
/// for `Modify` (kind='skip' stores NULL).
#[derive(Debug, Clone)]
pub struct Exception {
    pub id: Uuid,
    pub event_id: Uuid,
    pub occurrence_utc: DateTime<Utc>,
    pub kind: ExceptionKind,
    pub changes: Option<OccurrenceChanges>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionKind {
    /// Occurrence cancelled.
    Skip,
    /// Occurrence fields overridden by `changes`.
    Modify,
}

/// JSON payload of a `Modify` exception: only the fields that changed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OccurrenceChanges {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_utc: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_utc: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Reminder {
    pub id: Uuid,
    pub event_id: Uuid,
    pub offset_minutes: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A reminder fire that is due (or pending): ledger row joined with its
/// reminder and event. `due_utc` is the effective fire instant
/// (`snoozed_to_utc` when set, else `fire_at_utc`).
#[derive(Debug, Clone)]
pub struct DueReminder {
    pub fire_id: Uuid,
    pub reminder_id: Uuid,
    pub event_id: Uuid,
    pub occurrence_utc: DateTime<Utc>,
    pub fire_at_utc: DateTime<Utc>,
    pub snoozed_to_utc: Option<DateTime<Utc>>,
    /// Set once the reminder has fired (banner shows it as fired-today).
    pub fired_at: Option<DateTime<Utc>>,
    pub offset_minutes: i64,
    pub title: String,
}

impl DueReminder {
    pub fn due_utc(&self) -> DateTime<Utc> {
        self.snoozed_to_utc.unwrap_or(self.fire_at_utc)
    }
}

/// One expanded occurrence of an event inside a query window. All-day
/// occurrences carry shifted `start_date`/`end_date`; timed occurrences carry
/// `start_utc`/`end_utc`.
#[derive(Debug, Clone)]
pub struct Occurrence {
    pub event_id: Uuid,
    pub title: String,
    pub description: String,
    pub location: String,
    pub all_day: bool,
    pub start_utc: Option<DateTime<Utc>>,
    pub end_utc: Option<DateTime<Utc>>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub tz: String,
    pub color_idx: i64,
}
