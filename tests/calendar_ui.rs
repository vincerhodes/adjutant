//! Calendar UI tests (kittest, spec §10): view switcher, dashboard/upcoming
//! content, event form validation, reminder banner dismiss/snooze, link
//! picker entries.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use adjutant::app::AdjutantApp;
use adjutant::calendar::{CalendarStore, EventInput};
use adjutant::db::Db;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

fn temp_db_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "adjutant-calendar-ui-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("adjutant.db")
}

fn base_input(title: &str) -> EventInput {
    EventInput {
        title: title.to_string(),
        description: String::new(),
        location: "Room 1".to_string(),
        all_day: false,
        start_utc: None,
        end_utc: None,
        start_date: None,
        end_date: None,
        tz: "UTC".into(),
        rrule: None,
        color_idx: 0,
        attendees: vec![],
    }
}

fn fixture(name: &str) -> PathBuf {
    let path = temp_db_path(name);
    let db = Db::open(&path).unwrap();
    let store = CalendarStore::new(&db);
    // All events safely in the future regardless of when the test runs.
    let today = chrono::Utc::now().date_naive();
    let at = |day_offset: i64, h: u32| {
        (today + chrono::Duration::days(day_offset))
            .and_hms_opt(h, 0, 0)
            .unwrap()
            .and_utc()
    };
    store
        .create_event(&EventInput {
            start_utc: Some(at(1, 9)),
            end_utc: Some(at(1, 10)),
            ..base_input("Alpha standup")
        })
        .unwrap();
    store
        .create_event(&EventInput {
            start_utc: Some(at(2, 14)),
            end_utc: Some(at(2, 15)),
            ..base_input("Beta review")
        })
        .unwrap();
    store
        .create_event(&EventInput {
            all_day: true,
            start_utc: None,
            end_utc: None,
            start_date: Some(today + chrono::Duration::days(3)),
            end_date: Some(today + chrono::Duration::days(3)),
            location: String::new(),
            ..base_input("Gamma offsite")
        })
        .unwrap();
    drop(db);
    path
}

fn boot(path: &Path) -> Harness<'static, AdjutantApp> {
    let db = Db::open(path).unwrap();
    Harness::builder()
        .with_size([1400.0, 900.0])
        .build_eframe(move |cc| AdjutantApp::new_with_engine(db, cc, None))
}

fn open_calendar(h: &mut Harness<'static, AdjutantApp>) {
    h.run_steps(2);
    h.get_by_label("Calendar").click();
    h.run_steps(3);
}

#[test]
fn view_switcher_routes_dashboard_week_upcoming() {
    let path = fixture("switcher");
    let mut h = boot(&path);
    open_calendar(&mut h);

    // Dashboard default: the upcoming strip shows the nearest events.
    assert_eq!(h.query_all_by_label_contains("Alpha standup").count(), 1);
    assert_eq!(h.query_all_by_label_contains("Gamma offsite").count(), 1);

    // Week view: weekday headers render (event blocks are painter-drawn).
    h.get_by_label("Week").click();
    h.run_steps(3);
    let weekday_headers = ["Mon ", "Tue ", "Wed ", "Thu ", "Fri "]
        .iter()
        .map(|d| h.query_all_by_label_contains(d).count())
        .sum::<usize>();
    assert!(weekday_headers >= 5, "weekday column headers visible");

    // Upcoming view lists every upcoming event as a card.
    h.get_by_label("Upcoming").click();
    h.run_steps(3);
    assert_eq!(h.query_all_by_label_contains("Alpha standup").count(), 1);
    assert_eq!(h.query_all_by_label_contains("Beta review").count(), 1);
    assert_eq!(h.query_all_by_label_contains("Gamma offsite").count(), 1);

    // Back to dashboard.
    h.get_by_label("Dashboard").click();
    h.run_steps(2);
    assert_eq!(h.query_all_by_label_contains("Alpha standup").count(), 1);
}

#[test]
fn empty_calendar_shows_designed_empty_state() {
    let path = temp_db_path("empty");
    let db = Db::open(&path).unwrap();
    drop(db);
    let mut h = boot(&path);
    open_calendar(&mut h);
    h.get_by_label("Upcoming").click();
    h.run_steps(3);
    assert_eq!(h.query_all_by_label_contains("Nothing upcoming").count(), 1);
}

#[test]
fn new_event_form_creates_event() {
    let path = temp_db_path("new-event");
    let db = Db::open(&path).unwrap();
    drop(db);
    let mut h = boot(&path);
    open_calendar(&mut h);

    h.get_by_label_contains("New event").click();
    h.run_steps(3);
    // TextInputs in layout order: Title, Location, Date, Start, End, Reminder.
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    assert!(inputs.len() >= 5, "form fields visible: {}", inputs.len());
    inputs[0].focus();
    h.run_steps(1);
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    inputs[0].type_text("Delta sync");
    h.run_steps(1);
    let day = (chrono::Utc::now().date_naive() + chrono::Duration::days(5)).to_string();
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    inputs[2].focus();
    h.run_steps(1);
    for _ in 0..12 {
        h.key_press(egui::Key::Backspace);
    }
    h.run_steps(1);
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    inputs[2].type_text(&day);
    h.run_steps(1);

    h.get_by_label("Save").click();
    h.run_steps(3);

    let db = Db::open(&path).unwrap();
    let store = CalendarStore::new(&db);
    let now = chrono::Utc::now();
    let events = store
        .events_in_window(now, now + chrono::Duration::days(10))
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].title, "Delta sync");
}

#[test]
fn form_validation_blocks_bad_dates() {
    let path = temp_db_path("form-validation");
    let db = Db::open(&path).unwrap();
    drop(db);
    let mut h = boot(&path);
    open_calendar(&mut h);

    h.get_by_label_contains("New event").click();
    h.run_steps(3);
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    inputs[0].focus();
    h.run_steps(1);
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    inputs[0].type_text("Bad date event");
    h.run_steps(1);
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    inputs[2].focus();
    h.run_steps(1);
    for _ in 0..12 {
        h.key_press(egui::Key::Backspace);
    }
    h.run_steps(1);
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    inputs[2].type_text("not-a-date");
    h.run_steps(1);

    h.get_by_label("Save").click();
    h.run_steps(3);

    // The error is shown inline and nothing is persisted.
    assert_eq!(
        h.query_all_by_label_contains("Start date must be YYYY-MM-DD")
            .count(),
        1
    );
    let db = Db::open(&path).unwrap();
    let store = CalendarStore::new(&db);
    let now = chrono::Utc::now();
    assert!(store
        .events_in_window(now, now + chrono::Duration::days(30))
        .unwrap()
        .is_empty());
}

#[test]
fn recurring_event_edit_scope_this_creates_exception() {
    let path = temp_db_path("recurring-edit");
    let db = Db::open(&path).unwrap();
    let store = CalendarStore::new(&db);
    let tomorrow = chrono::Utc::now().date_naive() + chrono::Duration::days(1);
    let start = tomorrow.and_hms_opt(9, 0, 0).unwrap().and_utc();
    let event_id = store
        .create_event(&EventInput {
            start_utc: Some(start),
            end_utc: Some(start + chrono::Duration::minutes(30)),
            rrule: Some("FREQ=DAILY".into()),
            ..base_input("Daily sync")
        })
        .unwrap();
    drop(db);

    let mut h = boot(&path);
    open_calendar(&mut h);
    h.get_by_label("Upcoming").click();
    h.run_steps(3);
    // Open the recurring event from the upcoming list (one card per
    // occurrence — click the first).
    h.query_all_by_label_contains("Daily sync")
        .next()
        .expect("at least one occurrence card")
        .click();
    h.run_steps(3);

    // Recurring-edit chooser per spec §5, defaulting to This occurrence.
    assert_eq!(h.query_all_by_label_contains("This occurrence").count(), 1);
    assert_eq!(h.query_all_by_label_contains("This and future").count(), 1);
    assert_eq!(h.query_all_by_label_contains("Entire series").count(), 1);

    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    inputs[0].focus();
    h.run_steps(1);
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    inputs[0].type_text(" (moved)");
    h.run_steps(1);
    h.get_by_label("Save").click();
    h.run_steps(3);

    // Base series untouched; one modify exception on the clicked occurrence.
    let db = Db::open(&path).unwrap();
    let store = CalendarStore::new(&db);
    let event = store.get_event(event_id).unwrap();
    assert_eq!(event.title, "Daily sync");
    assert_eq!(event.exceptions.len(), 1);
    let occs = adjutant::calendar::rrule::expand(
        &event,
        start - chrono::Duration::hours(1),
        start + chrono::Duration::hours(1),
    )
    .unwrap();
    assert_eq!(occs.len(), 1);
    assert_eq!(occs[0].title, "Daily sync (moved)");
}

fn banner_fixture(name: &str) -> (PathBuf, uuid::Uuid, uuid::Uuid) {
    let path = temp_db_path(name);
    let db = Db::open(&path).unwrap();
    let store = CalendarStore::new(&db);
    let now = chrono::Utc::now();
    let start = now + chrono::Duration::hours(1);
    let event_id = store
        .create_event(&EventInput {
            start_utc: Some(start),
            end_utc: Some(start + chrono::Duration::minutes(30)),
            ..base_input("Banner meeting")
        })
        .unwrap();
    let reminder_id = store.add_reminder(event_id, 10).unwrap();
    let fire_id = store
        .upsert_fire(reminder_id, start, now - chrono::Duration::minutes(5))
        .unwrap();
    drop(db);
    (path, event_id, fire_id)
}

#[test]
fn reminder_banner_dismiss_silences_row() {
    let (path, _event_id, _fire_id) = banner_fixture("banner-dismiss");
    let mut h = boot(&path);
    open_calendar(&mut h);
    assert!(h.query_all_by_label_contains("Banner meeting").count() >= 1);
    assert_eq!(h.query_all_by_label_contains("Dismiss").count(), 1);

    h.get_by_label("Dismiss").click();
    h.run_steps(3);

    let db = Db::open(&path).unwrap();
    let store = CalendarStore::new(&db);
    assert!(store
        .banner_items(
            chrono::Utc::now() + chrono::Duration::days(1),
            chrono::Duration::hours(24)
        )
        .unwrap()
        .is_empty());
}

#[test]
fn reminder_banner_snooze_refires_at_target() {
    let (path, _event_id, fire_id) = banner_fixture("banner-snooze");
    let mut h = boot(&path);
    open_calendar(&mut h);
    assert_eq!(h.query_all_by_label_contains("Snooze").count(), 1);

    h.get_by_label("Snooze").click();
    h.run_steps(2);
    h.get_by_label("15 minutes").click();
    h.run_steps(3);

    // Banner clears; the ledger row carries the snooze target.
    let db = Db::open(&path).unwrap();
    let store = CalendarStore::new(&db);
    assert!(store
        .banner_items(chrono::Utc::now(), chrono::Duration::hours(24))
        .unwrap()
        .is_empty());
    let due_later = store
        .due_reminders(chrono::Utc::now() + chrono::Duration::minutes(20))
        .unwrap();
    assert_eq!(due_later.len(), 1);
    assert_eq!(due_later[0].fire_id, fire_id);
    assert!(due_later[0].snoozed_to_utc.is_some());

    // After the snooze target passes, the banner shows the row again and
    // Dismiss works on it.
    let db2 = Db::open(&path).unwrap();
    let store2 = CalendarStore::new(&db2);
    let fire = store2
        .due_reminders(chrono::Utc::now() + chrono::Duration::minutes(20))
        .unwrap()[0]
        .clone();
    store2
        .snooze_fire(
            fire.fire_id,
            chrono::Utc::now() - chrono::Duration::minutes(1),
        )
        .unwrap();
    drop(db);
    drop(db2);
    let mut h = boot(&path);
    open_calendar(&mut h);
    assert_eq!(h.query_all_by_label_contains("Dismiss").count(), 1);
    h.get_by_label("Dismiss").click();
    h.run_steps(3);
    let db = Db::open(&path).unwrap();
    let store = CalendarStore::new(&db);
    assert!(store
        .banner_items(
            chrono::Utc::now() + chrono::Duration::days(1),
            chrono::Duration::hours(24)
        )
        .unwrap()
        .is_empty());
}

#[test]
fn todo_link_picker_gains_event_entries() {
    use adjutant::core::entity::{EntityRef, EntityType};
    use adjutant::core::link::LinkStore;
    use adjutant::todo::TodoStore;

    let path = temp_db_path("link-picker");
    let db = Db::open(&path).unwrap();
    let todos = TodoStore::new(&db);
    let group = todos.create_group("Personal").unwrap();
    todos.create(group.id, None, "Prep slides").unwrap();
    let store = CalendarStore::new(&db);
    let start = chrono::Utc::now() + chrono::Duration::days(1);
    let event_id = store
        .create_event(&EventInput {
            start_utc: Some(start),
            end_utc: Some(start + chrono::Duration::hours(1)),
            ..base_input("Team sync")
        })
        .unwrap();
    drop(db);

    let mut h = boot(&path);
    open_calendar(&mut h);
    // Todo module: unfold the card, open the link picker, search "sync".
    h.get_by_label("Todo").click();
    h.run_steps(3);
    {
        let mut nodes: Vec<_> = h.query_all_by_label("Prep slides").collect();
        nodes.sort_by(|a, b| a.rect().width().partial_cmp(&b.rect().width()).unwrap());
        nodes.last().unwrap().click();
    }
    h.run_steps(3);
    h.get_by_label("Blocked by…").click();
    h.run_steps(2);
    {
        let inputs: Vec<_> = h
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .collect();
        inputs.last().unwrap().focus();
    }
    h.run_steps(1);
    {
        let inputs: Vec<_> = h
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .collect();
        inputs.last().unwrap().type_text("sync");
    }
    h.run_steps(2);
    // The calendar entry appears in the picker results and links on click.
    h.get_by_label_contains("Event — Team sync").click();
    h.run_steps(2);

    let db = Db::open(&path).unwrap();
    let links = LinkStore::new(&db)
        .links_to(&EntityRef::new(EntityType::Event, event_id))
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].source.kind, EntityType::Todo);
}
