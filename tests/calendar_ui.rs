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
