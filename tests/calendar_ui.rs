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
        .build_eframe(move |cc| {
            let mut app = AdjutantApp::new_with_engine(db, cc, None);
            // Notifier is the test seam (AGENTS.md): never dbus in cargo test.
            app.set_reminder_notifier(Box::new(MockNotifier));
            app
        })
}

/// Records nothing, notifies nothing — exists only to keep the live
/// `DesktopNotifier` (and its dbus connection) out of the test process.
struct MockNotifier;

impl adjutant::calendar::notify::Notifier for MockNotifier {
    fn notify(&self, _title: &str, _body: &str) {}
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

    // Week view: the grid widget is live (headers/blocks are painter-drawn
    // and kittest-invisible by design).
    h.get_by_label("Week").click();
    h.run_steps(3);
    assert_eq!(h.query_all_by_label("Week grid").count(), 1);

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

// ── Phase 2: week grid interactions ────────────────────────────────────────

const GUTTER_W: f32 = 44.0;
const HOUR_H: f32 = 48.0;

fn empty_fixture(name: &str) -> PathBuf {
    let path = temp_db_path(name);
    let db = Db::open(&path).unwrap();
    drop(db);
    path
}

fn expected_week_label(anchor: chrono::DateTime<chrono::Utc>) -> String {
    use chrono::Datelike;
    let local = anchor.with_timezone(&chrono::Local);
    let date = local.date_naive();
    let monday = date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64);
    let sunday = monday + chrono::Duration::days(6);
    format!("{} – {}", monday.format("%d %b"), sunday.format("%d %b"))
}

#[test]
fn week_nav_steps_period_label_and_today_resets() {
    let path = empty_fixture("week-nav");
    let mut h = boot(&path);
    open_calendar(&mut h);

    let initial = expected_week_label(chrono::Utc::now());
    assert_eq!(h.query_all_by_label(&initial).count(), 1);

    h.get_by_label_contains("Next week (icon button)").click();
    h.run_steps(3);
    let next = expected_week_label(chrono::Utc::now() + chrono::Duration::weeks(1));
    assert_eq!(h.query_all_by_label(&next).count(), 1, "‹/› steps the week");

    h.get_by_label("Today").click();
    h.run_steps(3);
    assert_eq!(
        h.query_all_by_label(&initial).count(),
        1,
        "Today resets the anchor"
    );

    h.get_by_label_contains("Previous week (icon button)")
        .click();
    h.run_steps(3);
    let prev = expected_week_label(chrono::Utc::now() - chrono::Duration::weeks(1));
    assert_eq!(h.query_all_by_label(&prev).count(), 1);

    h.key_press(egui::Key::T);
    h.run_steps(3);
    assert_eq!(h.query_all_by_label(&initial).count(), 1, "T = today");
}

#[test]
fn click_at_time_prefills_form_slot() {
    let path = empty_fixture("click-slot");
    let mut h = boot(&path);
    open_calendar(&mut h);
    h.get_by_label("Week").click();
    h.run_steps(4);

    // Grid geometry from the app (content origin + column width).
    let origin = h.state().calendar().week_origin().expect("grid rendered");
    let col_w = h.state().calendar().week_col_w();
    // Click column 3 at 12:30 (grid-local y = 12.5 * HOUR_H).
    let pos = egui::pos2(origin.x + GUTTER_W + col_w * 2.5, origin.y + 12.5 * HOUR_H);
    h.drag_at(pos);
    h.run_steps(1);
    h.drop_at(pos);
    h.run_steps(2);

    // Form opens prefilled: start 12:30, end 13:30 (start + 60 min).
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    assert!(inputs.len() >= 5, "form open: {}", inputs.len());
    assert_eq!(inputs[3].value().as_deref(), Some("12:30"), "start slot");
    assert_eq!(
        inputs[4].value().as_deref(),
        Some("13:30"),
        "end = start+60"
    );
}

#[test]
fn drag_create_prefills_range() {
    let path = empty_fixture("drag-range");
    let mut h = boot(&path);
    open_calendar(&mut h);
    h.get_by_label("Week").click();
    h.run_steps(4);

    let origin = h.state().calendar().week_origin().expect("grid rendered");
    let col_w = h.state().calendar().week_col_w();
    let x = origin.x + GUTTER_W + col_w * 3.5;
    // Drag 09:15 → 11:45 within one column: slots snap to 09:00–12:00.
    let start = egui::pos2(x, origin.y + 9.25 * HOUR_H);
    let end = egui::pos2(x, origin.y + 11.75 * HOUR_H);
    h.drag_at(start);
    h.run_steps(1);
    h.hover_at(end);
    h.run_steps(2);
    h.drop_at(end);
    h.run_steps(2);

    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    assert!(inputs.len() >= 5, "form open: {}", inputs.len());
    assert_eq!(
        inputs[3].value().as_deref(),
        Some("09:00"),
        "drag start slot"
    );
    assert_eq!(
        inputs[4].value().as_deref(),
        Some("12:00"),
        "drag end slot +30"
    );
}

#[test]
fn short_drag_falls_back_to_click_slot() {
    let path = empty_fixture("short-drag");
    let mut h = boot(&path);
    open_calendar(&mut h);
    h.get_by_label("Week").click();
    h.run_steps(4);

    let origin = h.state().calendar().week_origin().expect("grid rendered");
    let col_w = h.state().calendar().week_col_w();
    let x = origin.x + GUTTER_W + col_w * 1.5;
    // Tiny nudge (< 15 min) from 15:00 → treat as a click at 15:00.
    let start = egui::pos2(x, origin.y + 15.1 * HOUR_H);
    let end = egui::pos2(x, origin.y + 15.2 * HOUR_H);
    h.drag_at(start);
    h.run_steps(1);
    h.hover_at(end);
    h.run_steps(2);
    h.drop_at(end);
    h.run_steps(2);

    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    assert_eq!(
        inputs[3].value().as_deref(),
        Some("15:00"),
        "click fallback"
    );
    assert_eq!(
        inputs[4].value().as_deref(),
        Some("16:00"),
        "end = start+60"
    );
}

#[test]
fn all_day_band_click_presets_all_day_form() {
    let path = empty_fixture("allday-band");
    let mut h = boot(&path);
    open_calendar(&mut h);
    h.get_by_label("Week").click();
    h.run_steps(4);

    let band = h
        .query_all_by_label("All-day band")
        .next()
        .expect("all-day band widget");
    let rect = band.rect();
    let col_w = (rect.width() - GUTTER_W) / 7.0;
    let click = egui::pos2(rect.left() + GUTTER_W + col_w * 2.5, rect.center().y);
    h.drag_at(click);
    h.run_steps(1);
    h.drop_at(click);
    h.run_steps(2);

    // Save the preset form and assert the created event is all-day on the
    // clicked date (column 3 of the current week).
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    inputs[0].focus();
    h.run_steps(1);
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    inputs[0].type_text("Band day");
    h.run_steps(1);
    h.get_by_label("Save").click();
    h.run_steps(3);

    let db = Db::open(&path).unwrap();
    let store = CalendarStore::new(&db);
    let events = store
        .events_in_window(
            chrono::Utc::now() - chrono::Duration::days(1),
            chrono::Utc::now() + chrono::Duration::days(14),
        )
        .unwrap();
    assert_eq!(events.len(), 1);
    assert!(events[0].all_day, "band click creates an all-day event");
    assert_eq!(events[0].title, "Band day");
}

// ── Phase 3: month view + view persistence ────────────────────────────────

#[test]
fn month_view_renders_day_numbers_with_adjacent_days() {
    let path = empty_fixture("month-days");
    let mut h = boot(&path);
    open_calendar(&mut h);
    h.get_by_label("Month").click();
    h.run_steps(3);

    // Day-of-week headers + day numbers are real labels.
    for wd in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] {
        assert_eq!(h.query_all_by_label(wd).count(), 1, "header {wd}");
    }
    // The 1st of the anchor month and some high day number (28/30/31 —
    // every month has one) are visible.
    // The 1st of the anchor month (a trailing next-month "1" may also
    // appear in the 6-row grid — that IS the adjacent-day spillover).
    assert!(h.query_all_by_label("1").count() >= 1, "the 1st is visible");
    let high_days = ["28", "30", "31"]
        .iter()
        .map(|d| h.query_all_by_label(d).count())
        .sum::<usize>();
    assert!(high_days >= 1, "current-month high day numbers visible");
    // Adjacent-month days: the grid always shows leading days from the
    // previous month and/or trailing days from the next — count every
    // numeric label; a bare month (Mon=1st, 28 days, no spill) is
    // impossible in a Monday-first 6-row grid.
    let numeric_labels: usize = (1..=31)
        .map(|d| h.query_all_by_label(&d.to_string()).count())
        .sum();
    assert!(numeric_labels > 31, "adjacent-month days fill the 6x7 grid");
}

#[test]
fn month_nav_steps_period_label_by_month() {
    let path = empty_fixture("month-nav");
    let mut h = boot(&path);
    open_calendar(&mut h);
    h.get_by_label("Month").click();
    h.run_steps(3);

    let this_month = chrono::Local::now().format("%B %Y").to_string();
    assert_eq!(h.query_all_by_label(&this_month).count(), 1);

    h.get_by_label_contains("Next month (icon button)").click();
    h.run_steps(3);
    let next_month = (chrono::Local::now() + chrono::Duration::days(32))
        .format("%B %Y")
        .to_string();
    assert_eq!(
        h.query_all_by_label(&next_month).count(),
        1,
        "‹/› steps by a month"
    );

    h.get_by_label("Today").click();
    h.run_steps(3);
    assert_eq!(
        h.query_all_by_label(&this_month).count(),
        1,
        "Today returns to the current month"
    );
}

#[test]
fn month_cell_click_drills_into_that_week() {
    let path = empty_fixture("month-drill");
    let mut h = boot(&path);
    open_calendar(&mut h);
    h.get_by_label("Month").click();
    h.run_steps(3);

    // Click the "15" day-number node: the plain Label doesn't consume
    // clicks, so the cell interact underneath receives it.
    let fifteen = h
        .query_all_by_label("15")
        .next()
        .expect("day 15 in the current month");
    // Raw press/release at the label's position: the Label node itself
    // holds no pointer sense, so the cell interact underneath must take it.
    let pos = fifteen.rect().center();
    h.drag_at(pos);
    h.run_steps(1);
    h.drop_at(pos);
    h.run_steps(3);

    // Week view opens, anchored on the week containing the 15th.
    assert_eq!(h.query_all_by_label("Week grid").count(), 1);
    use chrono::Datelike;
    let today = chrono::Local::now().date_naive();
    let month_15 = today.with_day(15).unwrap_or(today);
    let monday =
        month_15 - chrono::Duration::days(month_15.weekday().num_days_from_monday() as i64);
    let sunday = monday + chrono::Duration::days(6);
    let expected = format!("{} – {}", monday.format("%d %b"), sunday.format("%d %b"));
    assert_eq!(h.query_all_by_label(&expected).count(), 1);
}

#[test]
fn calendar_view_persists_across_relaunch() {
    let path = empty_fixture("view-persist");
    let mut h = boot(&path);
    open_calendar(&mut h);
    h.get_by_label("Month").click();
    h.run_steps(3);

    // UI → settings table.
    let db = Db::open(&path).unwrap();
    let saved: Option<String> = db.get_setting("calendar_view").unwrap();
    assert_eq!(saved.as_deref(), Some("month"));
    drop(db);
    drop(h);

    // Fresh boot restores the persisted view.
    let mut h = boot(&path);
    open_calendar(&mut h);
    let this_month = chrono::Local::now().format("%B %Y").to_string();
    assert_eq!(
        h.query_all_by_label(&this_month).count(),
        1,
        "Month view restored from settings"
    );

    // Unknown values fall back to Dashboard.
    let db = Db::open(&path).unwrap();
    db.set_setting("calendar_view", &"bogus").unwrap();
    drop(db);
    drop(h);
    let mut h = boot(&path);
    open_calendar(&mut h);
    assert_eq!(
        h.query_all_by_label_contains("Nothing scheduled").count(),
        1,
        "unknown view value falls back to Dashboard"
    );
}

#[test]
fn trash_panel_lists_restores_and_deletes_forever() {
    let path = temp_db_path("trash-panel");
    let db = Db::open(&path).unwrap();
    let store = CalendarStore::new(&db);
    let start = chrono::Utc::now() + chrono::Duration::days(2);
    let event_id = store
        .create_event(&EventInput {
            start_utc: Some(start),
            end_utc: Some(start + chrono::Duration::hours(1)),
            ..base_input("Trashed event")
        })
        .unwrap();
    store.trash_event(event_id).unwrap();
    drop(db);

    let mut h = boot(&path);
    open_calendar(&mut h);
    // Trash toggle in the header.
    h.get_by_label_contains("Trash (icon button)").click();
    h.run_steps(3);
    assert_eq!(h.query_all_by_label_contains("Trashed event").count(), 1);

    // Restore round-trips back to the live store.
    h.get_by_label_contains("Restore (icon button)").click();
    h.run_steps(3);
    let db = Db::open(&path).unwrap();
    let store = CalendarStore::new(&db);
    let event = store.get_event(event_id).unwrap();
    assert!(event.trashed_at.is_none());
    assert_eq!(
        store
            .events_in_window(
                chrono::Utc::now(),
                chrono::Utc::now() + chrono::Duration::days(7)
            )
            .unwrap()
            .len(),
        1
    );
    drop(db);

    // Trash again (via the panel: hover-reveal button on the card), then
    // delete forever through the confirm modal.
    store_retrash_and_purge_ui(&mut h, &path, event_id);
}

fn store_retrash_and_purge_ui(
    h: &mut Harness<'static, AdjutantApp>,
    path: &Path,
    event_id: uuid::Uuid,
) {
    let db = Db::open(path).unwrap();
    CalendarStore::new(&db).trash_event(event_id).unwrap();
    drop(db);
    // External trash doesn't dirty the UI — re-toggling the panel forces
    // the reload that picks it up.
    h.get_by_label_contains("Trash (icon button)").click();
    h.run_steps(2);
    h.get_by_label_contains("Trash (icon button)").click();
    h.run_steps(3);
    h.get_by_label_contains("Delete").click();
    h.run_steps(2);
    h.get_by_label("Delete forever").click();
    h.run_steps(3);
    let db = Db::open(path).unwrap();
    assert!(
        CalendarStore::new(&db).get_event(event_id).is_err(),
        "delete-forever removes the event"
    );
}

#[test]
fn event_form_date_picker_sets_start_date() {
    let path = empty_fixture("form-date-picker");
    let mut h = boot(&path);
    open_calendar(&mut h);
    h.get_by_label("New event").click();
    h.run_steps(3);

    // The form's date field has exactly one picker toggle (recurrence is
    // collapsed on a new event).
    h.get_by_label_contains("Pick date (icon button)").click();
    h.run_steps(2);
    let this_month = chrono::Local::now().format("%B %Y").to_string();
    assert_eq!(h.query_all_by_label(&this_month).count(), 1, "popup open");

    h.get_by_label("18").click();
    h.run_steps(2);

    // Field index 2 = Date (Title, Location, Date, Start, End, Reminder).
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .collect();
    let value = inputs[2].value().unwrap_or_default();
    assert!(
        value.ends_with("-18"),
        "picker wrote into the validated field, got {value:?}"
    );
    assert_eq!(
        h.query_all_by_label(&this_month).count(),
        0,
        "popup closed after selection"
    );
}
