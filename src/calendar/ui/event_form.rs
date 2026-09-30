//! Event form (spec §7 + §8.3): create/edit modal — timed/all-day toggle,
//! hand-rolled date/time entry, IANA timezone picker, recurrence section,
//! attendees, reminders, color tag, validation. Recurring events offer the
//! this-occurrence / this-and-future / whole-series choice (spec §5).
//!
//! Implemented in the "event form and recurrence editing" step; this
//! placeholder keeps the Step 4 view commits compiling.

use chrono::{DateTime, NaiveDate, Utc};
use egui::Context;

use crate::calendar::model::{Event, Occurrence};
use crate::db::Db;

/// Which rows an edit applies to (spec §5 recurring-edit chooser).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Creating a new event.
    Single,
    /// One occurrence of a recurring event.
    This,
    /// This occurrence onward (series split).
    Future,
    /// The whole series.
    Series,
}

pub struct EventForm {
    pub scope: Scope,
}

impl EventForm {
    pub fn new(_preset_date: Option<NaiveDate>, _default_tz: Option<String>) -> EventForm {
        EventForm {
            scope: Scope::Single,
        }
    }

    pub fn edit(_event: &Event, _occurrence: Option<(&Occurrence, DateTime<Utc>)>) -> EventForm {
        EventForm {
            scope: Scope::Series,
        }
    }

    /// Renders the modal window; returns false when the form should close.
    pub fn show(&mut self, _ctx: &Context, _db: &Db, _toasts: &mut Vec<String>) -> bool {
        false
    }
}
