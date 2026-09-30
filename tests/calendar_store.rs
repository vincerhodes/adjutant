//! Calendar store integration tests against an in-memory database (spec §10):
//! CRUD, CHECK constraints, trash/restore, exceptions, reminder state
//! machine, catch-up exactly-once, series split, triggers.
#![allow(clippy::unwrap_used)]

use adjutant::calendar::{CalendarStore, EventInput, ExceptionKind, OccurrenceChanges, Rsvp};
use adjutant::core::entity::{EntityRef, EntityType};
use adjutant::core::link::{LinkStore, Relation};
use adjutant::db::Db;
use chrono::{DateTime, Duration, NaiveDate, Utc};

fn mem_db() -> Db {
    Db::open(std::path::Path::new(":memory:")).expect("in-memory db")
}

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn timed_input(start: &str, end: &str) -> EventInput {
    EventInput {
        title: "Standup".into(),
        description: String::new(),
        location: "Room 1".into(),
        all_day: false,
        start_utc: Some(utc(start)),
        end_utc: Some(utc(end)),
        start_date: None,
        end_date: None,
        tz: "Europe/London".into(),
        rrule: None,
        color_idx: 0,
        attendees: vec![],
    }
}

fn all_day_input(start: &str, end: &str) -> EventInput {
    let mut i = timed_input("2026-01-01T09:00:00Z", "2026-01-01T10:00:00Z");
    i.all_day = true;
    i.start_utc = None;
    i.end_utc = None;
    i.start_date = Some(NaiveDate::parse_from_str(start, "%Y-%m-%d").unwrap());
    i.end_date = Some(NaiveDate::parse_from_str(end, "%Y-%m-%d").unwrap());
    i.tz = "UTC".into();
    i
}

// ── Schema ─────────────────────────────────────────────────────────────────

#[test]
fn migration_applies_user_version_3_and_all_triggers() {
    let db = mem_db();
    let v: i64 = db
        .conn()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, 3);
    for table in [
        "calendar_events",
        "calendar_attendees",
        "calendar_event_exceptions",
        "calendar_reminders",
        "calendar_reminder_fires",
    ] {
        let trigger: String = db
            .conn()
            .query_row(
                "SELECT name FROM sqlite_master
                 WHERE type = 'trigger' AND tbl_name = ?1
                 ORDER BY name LIMIT 1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(trigger, format!("trg_{table}_updated"));
        let schema: String = db
            .conn()
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            schema.contains("updated_at"),
            "{table} must have updated_at"
        );
    }
    // Shell-check equivalent: .schema calendar_events contains the trigger.
    let ev_schema: String = db
        .conn()
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'trg_calendar_events_updated'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(ev_schema.contains("trg_calendar_events_updated"));
}

#[test]
fn check_constraint_enforces_exactly_one_shape() {
    let db = mem_db();
    // Timed row with dates (violates shape) is rejected at the SQL level.
    let err = db
        .conn()
        .execute(
            "INSERT INTO calendar_events
                 (id, all_day, start_date, end_date, created_at, updated_at)
             VALUES ('x', 0, '2026-01-01', '2026-01-01', 't', 't')",
            [],
        )
        .unwrap_err();
    assert!(err.to_string().contains("CHECK"), "got: {err}");
    // All-day row without dates likewise.
    let err = db
        .conn()
        .execute(
            "INSERT INTO calendar_events (id, all_day, created_at, updated_at)
             VALUES ('y', 1, 't', 't')",
            [],
        )
        .unwrap_err();
    assert!(err.to_string().contains("CHECK"), "got: {err}");
}

// ── CRUD ───────────────────────────────────────────────────────────────────

#[test]
fn create_get_update_roundtrip_with_attendees() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    let mut input = timed_input("2026-03-09T09:00:00Z", "2026-03-09T09:30:00Z");
    input.attendees.push(adjutant::calendar::AttendeeInput {
        name: "Sam".into(),
        email: "sam@example.com".into(),
        rsvp: Rsvp::Accepted,
    });
    let id = store.create_event(&input).unwrap();

    let e = store.get_event(id).unwrap();
    assert_eq!(e.title, "Standup");
    assert_eq!(e.tz, "Europe/London");
    assert_eq!(e.attendees.len(), 1);
    assert_eq!(e.attendees[0].email, "sam@example.com");
    assert_eq!(e.attendees[0].rsvp, Rsvp::Accepted);

    let mut updated = timed_input("2026-03-09T10:00:00Z", "2026-03-09T10:30:00Z");
    updated.title = "Standup moved".into();
    store.update_event(id, &updated).unwrap();
    let e = store.get_event(id).unwrap();
    assert_eq!(e.title, "Standup moved");
    assert_eq!(e.start_utc, Some(utc("2026-03-09T10:00:00Z")));
    assert!(e.attendees.is_empty(), "attendees replaced on update");
}

#[test]
fn create_validates_shape_and_tz() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    // end before start
    let bad = timed_input("2026-03-09T10:00:00Z", "2026-03-09T09:00:00Z");
    assert!(store.create_event(&bad).is_err());
    // bad timezone
    let mut bad_tz = timed_input("2026-03-09T09:00:00Z", "2026-03-09T10:00:00Z");
    bad_tz.tz = "Mars/Olympus_Mons".into();
    assert!(store.create_event(&bad_tz).is_err());
    // bad rrule
    let mut bad_rrule = timed_input("2026-03-09T09:00:00Z", "2026-03-09T10:00:00Z");
    bad_rrule.rrule = Some("FREQ=FORTNIGHTLY".into());
    assert!(store.create_event(&bad_rrule).is_err());
    // all-day end before start
    let bad_day = all_day_input("2026-01-10", "2026-01-09");
    assert!(store.create_event(&bad_day).is_err());
    // missing instants on timed event
    let mut no_start = timed_input("2026-03-09T09:00:00Z", "2026-03-09T10:00:00Z");
    no_start.start_utc = None;
    assert!(store.create_event(&no_start).is_err());
}

// ── Trash / restore / hard delete ──────────────────────────────────────────

#[test]
fn trash_restore_and_hard_delete_purges_links() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    let links = LinkStore::new(&db);
    let id = store
        .create_event(&timed_input("2026-03-09T09:00:00Z", "2026-03-09T09:30:00Z"))
        .unwrap();

    let todos = adjutant::todo::TodoStore::new(&db);
    let group = todos.create_group("G").unwrap();
    let todo = todos.create(group.id, None, "related").unwrap();
    links
        .link(
            &EntityRef::new(EntityType::Event, id),
            &EntityRef::new(EntityType::Todo, todo.id),
            &Relation::from(Relation::SCHEDULED_AS),
        )
        .unwrap();

    store.trash_event(id).unwrap();
    assert!(store.get_event(id).unwrap().trashed_at.is_some());
    // Trashed events do not appear in window queries.
    assert!(store
        .events_in_window(utc("2026-03-01T00:00:00Z"), utc("2026-03-31T00:00:00Z"))
        .unwrap()
        .is_empty());

    store.restore_event(id).unwrap();
    assert_eq!(
        store
            .events_in_window(utc("2026-03-01T00:00:00Z"), utc("2026-03-31T00:00:00Z"))
            .unwrap()
            .len(),
        1
    );

    store.delete_event(id).unwrap();
    assert!(store.get_event(id).is_err());
    // Link rows are purged with the hard delete.
    let remaining = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM entity_links
             WHERE (source_type = 'event' AND source_id = ?1)
                OR (target_type = 'event' AND target_id = ?1)",
            [id.to_string()],
            |r| r.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(remaining, 0);
}

// ── Window queries ─────────────────────────────────────────────────────────

#[test]
fn events_in_window_includes_recurring_base_rows() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    let mut input = timed_input("2026-03-09T09:00:00Z", "2026-03-09T09:30:00Z");
    input.rrule = Some("FREQ=WEEKLY;BYDAY=MO".into());
    store.create_event(&input).unwrap();
    store
        .create_event(&timed_input("2026-03-11T14:00:00Z", "2026-03-11T15:00:00Z"))
        .unwrap();

    let mut events = store
        .events_in_window(utc("2026-03-23T00:00:00Z"), utc("2026-03-30T00:00:00Z"))
        .unwrap();
    assert_eq!(
        events.len(),
        1,
        "only the recurring event has a March-23 occurrence"
    );
    events.retain(|e| e.rrule.is_some());
    assert_eq!(events.len(), 1);
}

#[test]
fn upcoming_occurrences_orders_and_limits() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    store
        .create_event(&timed_input("2026-03-11T09:00:00Z", "2026-03-11T10:00:00Z"))
        .unwrap();
    store
        .create_event(&timed_input("2026-03-10T09:00:00Z", "2026-03-10T10:00:00Z"))
        .unwrap();
    let occs = store
        .upcoming_occurrences(utc("2026-03-01T00:00:00Z"), 5)
        .unwrap();
    assert_eq!(occs.len(), 2);
    assert!(occs[0].start_utc.unwrap() < occs[1].start_utc.unwrap());
    // Event-level view dedups.
    assert_eq!(
        store
            .upcoming_events(utc("2026-03-01T00:00:00Z"), 5)
            .unwrap()
            .len(),
        2
    );
}

// ── Exceptions ─────────────────────────────────────────────────────────────

#[test]
fn exception_upsert_and_uniqueness() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    let mut input = timed_input("2026-01-05T09:00:00Z", "2026-01-05T09:30:00Z");
    input.rrule = Some("FREQ=WEEKLY;BYDAY=MO".into());
    let id = store.create_event(&input).unwrap();

    let occ = utc("2026-01-12T09:00:00Z");
    store.add_exception(id, occ, ExceptionKind::Skip).unwrap();
    store
        .modify_occurrence(
            id,
            occ,
            &OccurrenceChanges {
                title: Some("Changed".into()),
                start_utc: Some(utc("2026-01-12T11:00:00Z")),
                end_utc: None,
                tz: None,
                location: None,
                description: None,
            },
        )
        .unwrap();

    // Same (event, occurrence): upserted, not duplicated.
    let e = store.get_event(id).unwrap();
    assert_eq!(e.exceptions.len(), 1);
    assert_eq!(e.exceptions[0].kind, ExceptionKind::Modify);
    let changes = e.exceptions[0].changes.as_ref().unwrap();
    assert_eq!(changes.title.as_deref(), Some("Changed"));

    // Expansion honors the modify.
    let occs = adjutant::calendar::rrule::expand(
        &e,
        utc("2026-01-11T00:00:00Z"),
        utc("2026-01-13T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(occs.len(), 1);
    assert_eq!(occs[0].title, "Changed");
    assert_eq!(occs[0].start_utc, Some(utc("2026-01-12T11:00:00Z")));
}

#[test]
fn series_update_prunes_stale_exceptions() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    let mut input = timed_input("2026-01-05T09:00:00Z", "2026-01-05T09:30:00Z");
    input.rrule = Some("FREQ=DAILY".into());
    let id = store.create_event(&input).unwrap();
    store
        .add_exception(id, utc("2026-01-06T09:00:00Z"), ExceptionKind::Skip)
        .unwrap();

    // Move the series to weekly: Jan 6 (a Tuesday) is no longer generated.
    let mut updated = timed_input("2026-01-05T09:00:00Z", "2026-01-05T09:30:00Z");
    updated.rrule = Some("FREQ=WEEKLY;BYDAY=MO".into());
    store.update_event(id, &updated).unwrap();
    assert!(store.get_event(id).unwrap().exceptions.is_empty());

    // An exception on a still-generated occurrence survives.
    store
        .add_exception(id, utc("2026-01-12T09:00:00Z"), ExceptionKind::Skip)
        .unwrap();
    store.update_event(id, &updated).unwrap();
    assert_eq!(store.get_event(id).unwrap().exceptions.len(), 1);
}

// ── Series split ───────────────────────────────────────────────────────────

#[test]
fn edit_future_splits_series_with_until_boundary() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    let mut input = timed_input("2026-01-05T09:00:00Z", "2026-01-05T09:30:00Z");
    input.rrule = Some("FREQ=WEEKLY;BYDAY=MO".into());
    let id = store.create_event(&input).unwrap();

    // Split at the 2026-01-19 occurrence: original gets UNTIL = 2026-01-12
    // 09:00 UTC (occurrence minus one weekly period, wall-clock); the clone
    // starts at the modified occurrence.
    let mut clone_input = timed_input("2026-01-19T10:00:00Z", "2026-01-19T10:30:00Z");
    clone_input.title = "Standup (new series)".into();
    clone_input.rrule = Some("FREQ=WEEKLY;BYDAY=MO".into());
    let new_id = store
        .edit_future(id, utc("2026-01-19T09:00:00Z"), &clone_input)
        .unwrap();
    assert_ne!(new_id, id);

    let original = store.get_event(id).unwrap();
    assert_eq!(
        original.rrule.as_deref(),
        Some("FREQ=WEEKLY;BYDAY=MO;UNTIL=20260112T090000Z")
    );
    let original_occs = adjutant::calendar::rrule::expand(
        &original,
        utc("2026-01-01T00:00:00Z"),
        utc("2026-03-01T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(original_occs.len(), 2, "Jan 5 and Jan 12 only");
    assert_eq!(
        original_occs[1].start_utc,
        Some(utc("2026-01-12T09:00:00Z"))
    );

    let clone = store.get_event(new_id).unwrap();
    assert_eq!(clone.title, "Standup (new series)");
    let clone_occs = adjutant::calendar::rrule::expand(
        &clone,
        utc("2026-01-01T00:00:00Z"),
        utc("2026-03-01T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(clone_occs[0].start_utc, Some(utc("2026-01-19T10:00:00Z")));
    assert_eq!(clone_occs[1].start_utc, Some(utc("2026-01-26T10:00:00Z")));
}

#[test]
fn edit_future_rejects_non_recurring() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    let id = store
        .create_event(&timed_input("2026-01-05T09:00:00Z", "2026-01-05T09:30:00Z"))
        .unwrap();
    let r = store.edit_future(
        id,
        utc("2026-01-05T09:00:00Z"),
        &timed_input("2026-01-05T09:00:00Z", "2026-01-05T09:30:00Z"),
    );
    assert!(r.is_err());
}

// ── Reminder state machine ─────────────────────────────────────────────────

#[test]
fn reminder_fire_snooze_dismiss_cycle() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    let id = store
        .create_event(&timed_input("2026-03-09T09:00:00Z", "2026-03-09T09:30:00Z"))
        .unwrap();
    let reminder_id = store.add_reminder(id, 10).unwrap();
    assert_eq!(store.reminders_for_event(id).unwrap().len(), 1);

    let now = utc("2026-03-09T09:00:00Z");
    // Not yet due: fire instant is 08:50; now is 08:30.
    let early = utc("2026-03-09T08:30:00Z");
    let fire_id = store
        .upsert_fire(reminder_id, now, now - Duration::minutes(10))
        .unwrap();
    assert!(store.due_reminders(early).unwrap().is_empty());
    assert_eq!(store.due_reminders(now).unwrap().len(), 1);
    assert_eq!(store.due_reminders(now).unwrap()[0].title, "Standup");
    assert_eq!(store.due_reminders(now).unwrap()[0].offset_minutes, 10);

    // Idempotent upsert: same (reminder, occurrence) returns same id.
    let again = store
        .upsert_fire(reminder_id, now, now - Duration::minutes(10))
        .unwrap();
    assert_eq!(again, fire_id);

    // Fire once, then it is no longer due.
    store.mark_fired(fire_id, now).unwrap();
    assert!(store.due_reminders(now).unwrap().is_empty());

    // Snooze re-fires at the target (in-app), then dismiss silences it.
    let snoozed_to = now + Duration::minutes(15);
    store.snooze_fire(fire_id, snoozed_to).unwrap();
    // mark_fired cleared fired_at? No — snooze after fire is a no-op path;
    // use a second occurrence to exercise snooze.
    let occ2 = now + Duration::days(1);
    let fire2 = store
        .upsert_fire(reminder_id, occ2, occ2 - Duration::minutes(10))
        .unwrap();
    store.snooze_fire(fire2, snoozed_to).unwrap();
    assert!(store.due_reminders(now).unwrap().is_empty());
    let due = store.due_reminders(snoozed_to).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].fire_id, fire2);
    assert_eq!(due[0].due_utc(), snoozed_to);

    store.dismiss_fire(fire2).unwrap();
    assert!(store
        .due_reminders(snoozed_to + Duration::hours(1))
        .unwrap()
        .is_empty());

    store.remove_reminder(reminder_id).unwrap();
    assert!(store.reminders_for_event(id).unwrap().is_empty());
}

#[test]
fn catch_up_is_bounded_and_exactly_once() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    let id = store
        .create_event(&timed_input("2026-03-09T09:00:00Z", "2026-03-09T09:30:00Z"))
        .unwrap();
    let reminder_id = store.add_reminder(id, 10).unwrap();
    let now = utc("2026-03-09T12:00:00Z");

    // Missed while the app was closed (fired 09:50... actually 08:50 — the
    // point is it is pending and in the past).
    let missed = store
        .upsert_fire(reminder_id, now, now - Duration::hours(2))
        .unwrap();
    // One older than the 24h horizon.
    let stale = store
        .upsert_fire(
            reminder_id,
            now - Duration::hours(30),
            now - Duration::hours(31),
        )
        .unwrap();
    // One in the future.
    let future = store
        .upsert_fire(
            reminder_id,
            now + Duration::hours(1),
            now + Duration::minutes(50),
        )
        .unwrap();

    let due = store.catch_up_due(now, Duration::hours(24)).unwrap();
    assert_eq!(due.len(), 1, "only the in-horizon missed fire");
    assert_eq!(due[0].fire_id, missed);

    // Exactly once: mark fired, catch-up returns nothing for it.
    store.mark_fired(missed, now).unwrap();
    assert!(store
        .catch_up_due(now, Duration::hours(24))
        .unwrap()
        .is_empty());

    // Sanity: stale and future rows still untouched.
    let all_pending: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM calendar_reminder_fires WHERE fired_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(all_pending, 2);
    let _ = (stale, future);
}

#[test]
fn reminders_skip_trashed_events() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    let id = store
        .create_event(&timed_input("2026-03-09T09:00:00Z", "2026-03-09T09:30:00Z"))
        .unwrap();
    let reminder_id = store.add_reminder(id, 10).unwrap();
    let now = utc("2026-03-09T09:00:00Z");
    store
        .upsert_fire(reminder_id, now, now - Duration::minutes(10))
        .unwrap();

    store.trash_event(id).unwrap();
    assert!(store.due_reminders(now).unwrap().is_empty());
    assert!(store
        .catch_up_due(now, Duration::hours(24))
        .unwrap()
        .is_empty());
}

#[test]
fn recurring_event_fires_per_occurrence() {
    let db = mem_db();
    let store = CalendarStore::new(&db);
    let mut input = timed_input("2026-03-09T09:00:00Z", "2026-03-09T09:30:00Z");
    input.rrule = Some("FREQ=DAILY".into());
    let id = store.create_event(&input).unwrap();
    let reminder_id = store.add_reminder(id, 10).unwrap();

    // Two occurrences → two distinct ledger rows, both due later.
    let now = utc("2026-03-10T09:00:00Z");
    let f1 = store
        .upsert_fire(
            reminder_id,
            utc("2026-03-09T09:00:00Z"),
            utc("2026-03-09T08:50:00Z"),
        )
        .unwrap();
    let f2 = store
        .upsert_fire(
            reminder_id,
            utc("2026-03-10T09:00:00Z"),
            utc("2026-03-10T08:50:00Z"),
        )
        .unwrap();
    assert_ne!(f1, f2);
    let due = store.due_reminders(now).unwrap();
    assert_eq!(due.len(), 2, "one fire per occurrence");
}
