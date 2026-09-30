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
