//! ReminderScheduler tests (spec §6): tick fires due reminders exactly once;
//! snooze re-fires in-app only; catch-up bounded to 24h and deduped;
//! trashed events never schedule; recurring events fire per occurrence.
//! Mock `Notifier` — zero dbus, zero network.
#![allow(clippy::unwrap_used)]

use std::cell::RefCell;

use adjutant::calendar::notify::Notifier;
use adjutant::calendar::reminders::ReminderScheduler;
use adjutant::calendar::CalendarStore;
use adjutant::db::Db;
use chrono::{DateTime, Utc};

#[derive(Default)]
struct MockNotifier {
    calls: RefCell<Vec<(String, String)>>,
}

impl Notifier for MockNotifier {
    fn notify(&self, title: &str, body: &str) {
        self.calls
            .borrow_mut()
            .push((title.to_string(), body.to_string()));
    }
}

fn mem_db() -> Db {
    Db::open(std::path::Path::new(":memory:")).expect("in-memory db")
}

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn daily_meeting(
    store: &CalendarStore,
    start: &str,
) -> (adjutant::calendar::EventInput, uuid::Uuid) {
    let input = adjutant::calendar::EventInput {
        title: "Standup".into(),
        description: String::new(),
        location: String::new(),
        all_day: false,
        start_utc: Some(utc(start)),
        end_utc: Some(utc(start) + chrono::Duration::minutes(30)),
        start_date: None,
        end_date: None,
        tz: "UTC".into(),
        rrule: Some("FREQ=DAILY".into()),
        color_idx: 0,
        attendees: vec![],
    };
    let id = store.create_event(&input).unwrap();
    store.add_reminder(id, 10).unwrap();
    (input, id)
}

#[test]
fn tick_fires_due_reminder_exactly_once() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    daily_meeting(&store, "2026-03-10T09:00:00Z");
    let sched = ReminderScheduler::new();
    let mut notifier = MockNotifier::default();

    // Before the fire instant: row created, nothing delivered.
    let toasts = sched
        .tick(&store, &mut notifier, utc("2026-03-10T08:00:00Z"))
        .unwrap();
    assert!(toasts.is_empty());
    assert!(notifier.calls.borrow().is_empty());

    // Past the fire instant (08:50): delivered once.
    let toasts = sched
        .tick(&store, &mut notifier, utc("2026-03-10T09:30:00Z"))
        .unwrap();
    assert_eq!(notifier.calls.borrow().len(), 1);
    assert_eq!(notifier.calls.borrow()[0].0, "Standup");
    assert_eq!(toasts, vec!["Reminder: Standup".to_string()]);

    // Later ticks do not re-fire the same occurrence.
    sched
        .tick(&store, &mut notifier, utc("2026-03-10T10:00:00Z"))
        .unwrap();
    sched
        .tick(&store, &mut notifier, utc("2026-03-10T23:00:00Z"))
        .unwrap();
    assert_eq!(notifier.calls.borrow().len(), 1);
}

#[test]
fn snooze_refires_in_app_only() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    daily_meeting(&store, "2026-03-10T09:00:00Z");
    let sched = ReminderScheduler::new();
    let mut notifier = MockNotifier::default();

    // Tick before the fire instant to create the ledger row.
    sched
        .tick(&store, &mut notifier, utc("2026-03-10T08:00:00Z"))
        .unwrap();
    let t_fire = utc("2026-03-10T09:30:00Z");
    let fire_id = store.due_reminders(t_fire).unwrap()[0].fire_id;
    sched.tick(&store, &mut notifier, t_fire).unwrap();
    assert_eq!(notifier.calls.borrow().len(), 1, "first fire is desktop");

    // User snoozes the fired reminder from the banner to 10:15.
    let snoozed_to = utc("2026-03-10T10:15:00Z");
    store.snooze_fire(fire_id, snoozed_to).unwrap();

    // Re-fires at the target — toast yes, desktop no.
    let toasts = sched.tick(&store, &mut notifier, snoozed_to).unwrap();
    assert_eq!(toasts.len(), 1, "snoozed re-fire toasts in-app");
    assert_eq!(
        notifier.calls.borrow().len(),
        1,
        "desktop notification fires once per occurrence"
    );

    // Snoozed row is consumed; later ticks are quiet.
    sched
        .tick(&store, &mut notifier, utc("2026-03-10T10:30:00Z"))
        .unwrap();
    assert_eq!(notifier.calls.borrow().len(), 1);
    assert!(store
        .due_reminders(utc("2026-03-10T10:30:00Z"))
        .unwrap()
        .is_empty());
}

#[test]
fn catch_up_is_bounded_to_24h_and_deduped() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    // Series started long ago; app opens at 2026-03-10 12:00.
    daily_meeting(&store, "2026-03-01T09:00:00Z");
    let sched = ReminderScheduler::new();
    let mut notifier = MockNotifier::default();
    let now = utc("2026-03-10T12:00:00Z");

    let toasts = sched.catch_up(&store, &mut notifier, now).unwrap();
    assert_eq!(
        toasts.len(),
        1,
        "only the in-horizon missed fire (03-10 08:50)"
    );
    assert_eq!(notifier.calls.borrow().len(), 1);

    // Exactly once: a second catch-up finds nothing.
    let toasts = sched.catch_up(&store, &mut notifier, now).unwrap();
    assert!(toasts.is_empty());
    assert_eq!(notifier.calls.borrow().len(), 1);

    // The 03-09 fire instant (before now-24h) never had a row ensured and
    // is never delivered; only 03-10 and 03-11 are in the ensure window.
    let fires: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM calendar_reminder_fires", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(fires, 2);
}

#[test]
fn trashed_events_never_schedule() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    let (_, id) = daily_meeting(&store, "2026-03-09T09:00:00Z");
    store.trash_event(id).unwrap();
    assert!(store.events_with_reminders().unwrap().is_empty());

    let sched = ReminderScheduler::new();
    let mut notifier = MockNotifier::default();
    let toasts = sched
        .tick(&store, &mut notifier, utc("2026-03-10T09:30:00Z"))
        .unwrap();
    assert!(toasts.is_empty());
    assert!(notifier.calls.borrow().is_empty());
}

#[test]
fn recurring_event_fires_per_occurrence_not_per_reminder_row() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    // One reminder row on a daily event.
    daily_meeting(&store, "2026-03-08T09:00:00Z");
    let sched = ReminderScheduler::new();
    let mut notifier = MockNotifier::default();

    // Early tick pre-creates rows for the next 24h of fire instants.
    sched
        .tick(&store, &mut notifier, utc("2026-03-08T08:00:00Z"))
        .unwrap();
    assert!(notifier.calls.borrow().is_empty());

    // By midday two occurrences are pending: each fires once.
    let toasts = sched
        .tick(&store, &mut notifier, utc("2026-03-09T12:00:00Z"))
        .unwrap();
    assert_eq!(
        notifier.calls.borrow().len(),
        2,
        "one desktop fire per occurrence"
    );
    assert_ne!(
        notifier.calls.borrow()[0].1,
        notifier.calls.borrow()[1].1,
        "bodies identify distinct occurrences"
    );
    assert_eq!(toasts.len(), 2);

    // Both consumed.
    sched
        .tick(&store, &mut notifier, utc("2026-03-09T13:00:00Z"))
        .unwrap();
    assert_eq!(notifier.calls.borrow().len(), 2);
}

#[test]
fn all_day_event_reminder_anchors_at_midnight_utc() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    let input = adjutant::calendar::EventInput {
        title: "Trip".into(),
        description: String::new(),
        location: String::new(),
        all_day: true,
        start_utc: None,
        end_utc: None,
        start_date: Some(chrono::NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()),
        end_date: Some(chrono::NaiveDate::from_ymd_opt(2026, 4, 3).unwrap()),
        tz: "UTC".into(),
        rrule: Some("FREQ=DAILY".into()),
        color_idx: 0,
        attendees: vec![],
    };
    let id = store.create_event(&input).unwrap();
    store.add_reminder(id, 30).unwrap();

    let sched = ReminderScheduler::new();
    let mut notifier = MockNotifier::default();
    // Fire instant for the 04-01 occurrence: midnight - 30min = 03-31 23:30.
    let toasts = sched
        .tick(&store, &mut notifier, utc("2026-04-01T00:00:00Z"))
        .unwrap();
    assert_eq!(notifier.calls.borrow().len(), 1);
    assert_eq!(toasts, vec!["Reminder: Trip".to_string()]);
}
