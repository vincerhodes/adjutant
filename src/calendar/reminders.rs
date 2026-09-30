//! Reminder scheduler (spec §6): app-session, tick-driven delivery.
//!
//! `ReminderScheduler::tick(now)` runs from `app.rs` every ~15s (plus one
//! `catch_up(now)` on launch). No background process — while the app is
//! closed, reminders queue up as `calendar_reminder_fires` ledger rows and
//! `catch_up` fires them exactly once, bounded to the last 24h.
//!
//! Exactly-once: one ledger row per (reminder, occurrence); firing marks
//! the row fired. Snooze re-opens the row with a new target and re-fires
//! **in-app only** — the desktop notification fires once per occurrence,
//! at first fire.
//!
//! No SQL here — all store access goes through `CalendarStore`. `Notifier`
//! is the Step 1 test seam; tests inject a mock and assert the call log.

use chrono::{DateTime, Duration, Utc};

use super::model::Occurrence;
use super::notify::Notifier;
use super::{CalendarStore, Result};

/// How far ahead of `now` ticks pre-create ledger rows.
pub const TICK_LOOKAHEAD: Duration = Duration::hours(24);
/// Catch-up horizon: missed fires older than this are never delivered.
pub const CATCH_UP_HORIZON: Duration = Duration::hours(24);
/// App-side cadence (app.rs asks the scheduler every TICK_INTERVAL).
pub const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

pub struct ReminderScheduler;

impl ReminderScheduler {
    pub fn new() -> Self {
        ReminderScheduler
    }

    /// Regular tick: ensure ledger rows exist, then deliver everything due
    /// by `now`. Returns toast lines for the in-app toast system.
    pub fn tick(
        &self,
        store: &CalendarStore,
        notifier: &mut dyn Notifier,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>> {
        self.ensure_fire_rows(store, now)?;
        self.fire_due(store, notifier, now, false)
    }

    /// Launch catch-up: same as [`tick`](Self::tick) but only fires missed
    /// fires within the last 24h (the due scan is left to the next tick).
    pub fn catch_up(
        &self,
        store: &CalendarStore,
        notifier: &mut dyn Notifier,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>> {
        self.ensure_fire_rows(store, now)?;
        self.fire_due(store, notifier, now, true)
    }

    /// Pre-create ledger rows for every (reminder, occurrence) whose fire
    /// instant falls in `[now - CATCH_UP_HORIZON, now + TICK_LOOKAHEAD]`.
    /// An event whose RRULE fails to expand is skipped (logged) so one bad
    /// row can't stall the whole tick.
    pub fn ensure_fire_rows(&self, store: &CalendarStore, now: DateTime<Utc>) -> Result<usize> {
        let mut ensured = 0;
        for (event, reminders) in store.events_with_reminders()? {
            let max_offset = Duration::minutes(
                reminders
                    .iter()
                    .map(|r| r.offset_minutes)
                    .max()
                    .unwrap_or(0)
                    .max(0),
            );
            let from = now - CATCH_UP_HORIZON - max_offset - Duration::days(1);
            let to = now + TICK_LOOKAHEAD + max_offset + Duration::days(1);
            let occs = match super::rrule::expand(&event, from, to) {
                Ok(o) => o,
                Err(e) => {
                    eprintln!(
                        "adjutant: reminder scan skipped event {} ({:?}): {e}",
                        event.id, event.title
                    );
                    continue;
                }
            };
            for occ in occs {
                let Some(start) = occurrence_start(&occ) else {
                    continue;
                };
                for reminder in &reminders {
                    let fire_at = start - Duration::minutes(reminder.offset_minutes);
                    if fire_at < now - CATCH_UP_HORIZON || fire_at > now + TICK_LOOKAHEAD {
                        continue;
                    }
                    store.upsert_fire(reminder.id, start, fire_at)?;
                    ensured += 1;
                }
            }
        }
        Ok(ensured)
    }

    /// Deliver due (or, for catch-up, missed) fires: desktop notification
    /// on first fire of an occurrence, in-app toast always, then mark the
    /// ledger row fired. Notification failure is already swallowed by the
    /// notifier — the ledger update below is unconditional.
    fn fire_due(
        &self,
        store: &CalendarStore,
        notifier: &mut dyn Notifier,
        now: DateTime<Utc>,
        catch_up: bool,
    ) -> Result<Vec<String>> {
        let due = if catch_up {
            store.catch_up_due(now, CATCH_UP_HORIZON)?
        } else {
            store.due_reminders(now)?
        };
        let mut toasts = Vec::new();
        for d in due {
            let first_desktop_fire = d.snoozed_to_utc.is_none();
            if first_desktop_fire {
                let body = format!(
                    "{} at {}",
                    d.title,
                    d.occurrence_utc.format("%Y-%m-%d %H:%M")
                );
                notifier.notify(&d.title, &body);
            }
            store.mark_fired(d.fire_id, now)?;
            toasts.push(format!("Reminder: {}", d.title));
        }
        Ok(toasts)
    }
}

impl Default for ReminderScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Effective start instant of an occurrence: timed events use their UTC
/// instants; all-day events anchor at midnight UTC (their tz column is
/// UTC — dates are tz-free, spec §3).
fn occurrence_start(o: &Occurrence) -> Option<DateTime<Utc>> {
    o.start_utc.or_else(|| {
        o.start_date
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|n| n.and_utc())
    })
}
