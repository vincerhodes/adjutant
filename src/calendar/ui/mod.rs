//! Calendar module UI: view switcher (Dashboard / Week / Upcoming), event
//! form, reminder banner. Card language per M1.5/§8; zero SQL — all data
//! access goes through `CalendarStore`; occurrence expansion uses the pure
//! `rrule` functions over base rows loaded via the store.

pub mod dashboard;
pub mod event_form;
pub mod month;
pub mod reminder_banner;
pub mod upcoming;
pub mod week;

use std::collections::HashSet;

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Utc};
use egui::{Context, Key, Ui};

use crate::db::Db;
use crate::ui::{self, icons};

use super::model::{DueReminder, Event, Occurrence};
use super::CalendarStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarView {
    Dashboard,
    Week,
    Month,
    Upcoming,
}

impl CalendarView {
    const ALL: [CalendarView; 4] = [
        CalendarView::Dashboard,
        CalendarView::Week,
        CalendarView::Month,
        CalendarView::Upcoming,
    ];

    fn label(&self) -> &'static str {
        match self {
            CalendarView::Dashboard => "Dashboard",
            CalendarView::Week => "Week",
            CalendarView::Month => "Month",
            CalendarView::Upcoming => "Upcoming",
        }
    }

    fn name(&self) -> &'static str {
        match self {
            CalendarView::Dashboard => "dashboard",
            CalendarView::Week => "week",
            CalendarView::Month => "month",
            CalendarView::Upcoming => "upcoming",
        }
    }

    fn from_name(name: &str) -> Option<CalendarView> {
        match name {
            "dashboard" => Some(CalendarView::Dashboard),
            "week" => Some(CalendarView::Week),
            "month" => Some(CalendarView::Month),
            "upcoming" => Some(CalendarView::Upcoming),
            _ => None,
        }
    }
}

/// Settings key for the persisted calendar view (theme picker pattern).
const SETTINGS_CALENDAR_VIEW: &str = "calendar_view";

/// Shared view state: cached base rows + banner items, refreshed when
/// `dirty` (or every minute, so the now-line and upcoming strip advance).
pub struct CalendarUi {
    view: CalendarView,
    /// Any instant within the week the Week view displays (default: now).
    week_anchor: DateTime<Utc>,
    events: Vec<Event>,
    upcoming: Vec<Occurrence>,
    due: Vec<DueReminder>,
    now: DateTime<Utc>,
    loaded_minute: i64,
    pub form: Option<event_form::EventForm>,
    /// Event ids with the pointer over them (drives card_hover fill).
    hovered: HashSet<uuid::Uuid>,
    dirty: bool,
    /// Week grid: first-frame scroll-to-08:00 done.
    week_scrolled: bool,
    /// Week grid content origin in ctx coordinates (test seam).
    week_origin: Option<egui::Pos2>,
    /// Week grid column width (test seam, set when the grid renders).
    week_col_w: f32,
    /// Live drag-to-create state (grid-local).
    week_drag: Option<week::WeekDrag>,
    /// Month cell drill-down click, applied after rendering (needs db).
    month_drill: Option<NaiveDate>,
}

impl CalendarUi {
    pub fn new(db: &Db) -> CalendarUi {
        let persisted = db
            .get_setting::<String>(SETTINGS_CALENDAR_VIEW)
            .ok()
            .flatten()
            .and_then(|name| CalendarView::from_name(&name));
        let mut state = CalendarUi {
            view: persisted.unwrap_or(CalendarView::Dashboard),
            week_anchor: Utc::now(),
            events: Vec::new(),
            upcoming: Vec::new(),
            due: Vec::new(),
            now: Utc::now(),
            loaded_minute: -1,
            form: None,
            hovered: HashSet::new(),
            dirty: true,
            week_scrolled: false,
            week_origin: None,
            week_col_w: 0.0,
            week_drag: None,
            month_drill: None,
        };
        state.reload(db);
        state
    }

    /// Switch view and persist the choice (settings table).
    fn set_view(&mut self, db: &Db, view: CalendarView) {
        if self.view != view {
            self.view = view;
            self.dirty = true;
            let _ = db.set_setting(SETTINGS_CALENDAR_VIEW, &view.name());
        }
    }

    /// Month drill-down: switch to the week containing `date`.
    pub(crate) fn drill_to_week(&mut self, db: &Db, date: NaiveDate) {
        self.week_anchor = day_start_utc(date) + Duration::hours(12);
        self.set_view(db, CalendarView::Week);
    }

    /// True while a modal-ish surface owns Esc (mirrors email `is_busy`).
    pub fn is_busy(&self) -> bool {
        self.form.is_some()
    }

    pub fn open_new_event(&mut self) {
        self.form = Some(event_form::EventForm::new(None, None, None));
    }

    /// Week grid content origin in ctx coordinates (kittest seam for
    /// click-at-time assertions). Valid only right after the Week or
    /// Dashboard view rendered.
    pub fn week_origin(&self) -> Option<egui::Pos2> {
        self.week_origin
    }

    /// Week grid column width from the last render (kittest seam).
    pub fn week_col_w(&self) -> f32 {
        self.week_col_w
    }

    /// Open the form on an event. `occurrence` carries the clicked
    /// occurrence's (possibly exception-modified) values; `occ_start` is
    /// its original series instant, identifying "this occurrence".
    pub fn open_event(&mut self, event: &Event, occurrence: Option<(&Occurrence, DateTime<Utc>)>) {
        self.form = Some(event_form::EventForm::edit(event, occurrence));
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        ctx: &Context,
        db: &Db,
        toasts: &mut Vec<String>,
        _focus_mode: &mut bool,
    ) {
        self.handle_keys(ctx);
        self.reload(db);

        let mut form = self.form.take();
        if let Some(f) = form.as_mut() {
            f.handle_key(ctx);
            if !f.show(ctx, db, toasts) {
                form = None;
                self.dirty = true;
            }
        }
        self.form = form;

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Calendar")
                        .size(ui::fonts::SIZE_GROUP_HEADING)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui::primary_button(ui, "New event").clicked() {
                        self.open_new_event();
                    }
                    ui.add_space(12.0);
                    for view in CalendarView::ALL {
                        if ui::selectable(ui, self.view == view, view.label()).clicked() {
                            self.set_view(db, view);
                        }
                    }
                    ui.add_space(12.0);
                    // Period navigation — view-aware: Month steps whole
                    // months, Week/Dashboard step 7 days.
                    let month_mode = self.view == CalendarView::Month;
                    if ui::icon_button(
                        ui,
                        icons::Icon::ChevronRight,
                        if month_mode {
                            "Next month"
                        } else {
                            "Next week"
                        },
                        "next-week",
                    )
                    .clicked()
                    {
                        self.step_nav(1);
                    }
                    if ui::ghost_button(ui, "Today").clicked() {
                        self.week_anchor = Utc::now();
                        self.dirty = true;
                    }
                    if ui::icon_button(
                        ui,
                        icons::Icon::ChevronLeft,
                        if month_mode {
                            "Previous month"
                        } else {
                            "Previous week"
                        },
                        "prev-week",
                    )
                    .clicked()
                    {
                        self.step_nav(-1);
                    }
                    ui.label(
                        egui::RichText::new(period_label(self.view, self.week_anchor))
                            .strong()
                            .size(ui::fonts::SIZE_BODY),
                    );
                });
            });
            ui.add_space(4.0);
            reminder_banner::show(self, ui, db, toasts);
            ui.add_space(4.0);

            match self.view {
                CalendarView::Dashboard => dashboard::show(self, ui, db),
                CalendarView::Week => week::show(self, ui, db),
                CalendarView::Month => month::show(self, ui, db),
                CalendarView::Upcoming => upcoming::show(self, ui, db),
            }
        });
    }

    /// ‹/› stepping — view-aware: Month steps whole months (day-clamped),
    /// Week/Dashboard step 7 days.
    fn step_nav(&mut self, direction: i64) {
        self.step_nav_from_key(direction);
    }

    fn step_nav_from_key(&mut self, direction: i64) {
        if self.view == CalendarView::Month {
            let date = self.week_anchor.with_timezone(&Local).date_naive();
            let shifted = shift_months(date, direction);
            self.week_anchor = day_start_utc(shifted) + Duration::hours(12);
        } else {
            self.week_anchor += Duration::days(7 * direction);
        }
        self.dirty = true;
    }

    fn handle_keys(&mut self, ctx: &Context) {
        if self.form.is_some() {
            return;
        }
        ctx.input(|i| {
            if i.modifiers.ctrl && i.key_pressed(Key::N) {
                self.open_new_event();
            }
            if i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::ArrowRight) {
                let dir = if i.key_pressed(Key::ArrowLeft) { -1 } else { 1 };
                self.step_nav_from_key(dir);
            }
            if i.key_pressed(Key::T) {
                self.week_anchor = Utc::now();
                self.dirty = true;
            }
        });
    }

    fn reload(&mut self, db: &Db) {
        let now = Utc::now();
        let minute = now.timestamp() / 60;
        if !self.dirty && minute == self.loaded_minute {
            return;
        }
        self.dirty = false;
        self.loaded_minute = minute;
        self.now = now;
        let store = CalendarStore::new(db);
        let (week_start, week_end) = if self.view == CalendarView::Month {
            let (start, monday, _) = month_bounds(self.week_anchor);
            (start, day_start_utc(monday + Duration::days(42)))
        } else {
            week_bounds(self.week_anchor)
        };
        self.events = store
            .events_in_window(week_start, week_end)
            .unwrap_or_default();
        self.upcoming = store.upcoming_occurrences(now, 50).unwrap_or_default();
        self.due = store
            .banner_items(now, Duration::hours(24))
            .unwrap_or_default();
    }
}

/// Monday 00:00 local → next Monday 00:00 local, as UTC instants.
pub fn week_bounds(anchor: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let anchor_local = anchor.with_timezone(&Local);
    let date = anchor_local.date_naive();
    let monday = date - Duration::days(date.weekday().num_days_from_monday() as i64);
    let sunday = monday + Duration::days(7);
    (day_start_utc(monday), day_start_utc(sunday))
}

/// Local midnight of `date` as a UTC instant (DST-safe: earliest
/// interpretation if the wall time is ambiguous/gapped).
pub fn day_start_utc(date: NaiveDate) -> DateTime<Utc> {
    let Some(ndt) = date.and_hms_opt(0, 0, 0) else {
        return DateTime::<Utc>::MIN_UTC;
    };
    let mapped = Local.from_local_datetime(&ndt);
    mapped
        .single()
        .or_else(|| mapped.earliest())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// Best-effort system IANA timezone name for the TZ picker default:
/// `$TZ` → /etc/timezone → /etc/localtime symlink target. Falls back to
/// "UTC" (the form validates against chrono-tz, so a bad guess is caught).
pub fn system_tz_name() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        let trimmed = tz.trim_start_matches(':').to_string();
        if trimmed.parse::<chrono_tz::Tz>().is_ok() {
            return trimmed;
        }
    }
    if let Ok(contents) = std::fs::read_to_string("/etc/timezone") {
        let name = contents.trim().to_string();
        if name.parse::<chrono_tz::Tz>().is_ok() {
            return name;
        }
    }
    if let Ok(link) = std::fs::read_link("/etc/localtime") {
        // Symlink target looks like .../zoneinfo/Europe/London.
        let text = link.to_string_lossy();
        if let Some(idx) = text.find("zoneinfo/") {
            let name = text[idx + "zoneinfo/".len()..].to_string();
            if name.parse::<chrono_tz::Tz>().is_ok() {
                return name;
            }
        }
    }
    "UTC".to_string()
}

/// Short time label for an occurrence: "09:00–09:30", "All day", or an
/// explicit date span for multi-day all-day events.
pub fn occurrence_time_label(occ: &Occurrence) -> String {
    if occ.all_day {
        match (occ.start_date, occ.end_date) {
            (Some(s), Some(e)) if s == e => "All day".to_string(),
            (Some(s), Some(e)) => format!("{} → {}", s.format("%b %d"), e.format("%b %d")),
            _ => "All day".to_string(),
        }
    } else {
        match (occ.start_utc, occ.end_utc) {
            (Some(s), Some(e)) => format!("{}–{}", s.format("%H:%M"), e.format("%H:%M")),
            _ => String::new(),
        }
    }
}

/// Weekday + day-of-month chip text for an occurrence's local day.
pub fn occurrence_day_label(occ: &Occurrence, now: DateTime<Utc>) -> String {
    let (date, is_multi) = if occ.all_day {
        (
            occ.start_date.unwrap_or(now.date_naive()),
            occ.start_date != occ.end_date,
        )
    } else {
        (occ.start_utc.unwrap_or(now).date_naive(), false)
    };
    let text = date.format("%a %d").to_string();
    if is_multi {
        format!("{text} →")
    } else {
        text
    }
}

/// Compact card for an upcoming occurrence (dashboard strip + list rows).
/// Returns the card response so callers can layer click handling.
pub fn occurrence_card(
    ui: &mut Ui,
    hovered: bool,
    dot: egui::Color32,
    day: &str,
    time: &str,
    title: &str,
    location: &str,
) -> egui::Response {
    let frame_response = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let frame = ui::card_frame(ui, hovered, false);
            frame.show(ui, |ui| {
                ui.set_min_width(140.0);
                ui.set_max_width(200.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        icons::group_dot(ui, dot, 8.0);
                        ui.label(
                            egui::RichText::new(day)
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    });
                    ui.label(
                        egui::RichText::new(title)
                            .size(ui::fonts::SIZE_CARD_TITLE)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(time)
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    if !location.is_empty() {
                        ui.label(
                            egui::RichText::new(location)
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    }
                });
            })
        })
        .response;
    // The frame's own response is hover-sense; layer a click interacter.
    ui.interact(
        frame_response.rect,
        ui.id().with(("occ-card", title, day)),
        egui::Sense::click(),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// "29 Sep – 5 Oct" for the week containing `anchor` (local dates).
fn period_label(view: CalendarView, anchor: DateTime<Utc>) -> String {
    if view == CalendarView::Month {
        return anchor.with_timezone(&Local).format("%B %Y").to_string();
    }
    let (start, end) = week_bounds(anchor);
    let start_local = (start + Duration::seconds(1))
        .with_timezone(&Local)
        .date_naive();
    let end_local = (end - Duration::seconds(1))
        .with_timezone(&Local)
        .date_naive();
    format!(
        "{} – {}",
        start_local.format("%d %b"),
        end_local.format("%d %b")
    )
}

/// Month grid window: the Monday on/before the 1st of the anchor's month,
/// plus 42 days. Returns (window_start_utc, grid_monday, month_first).
pub fn month_bounds(anchor: DateTime<Utc>) -> (DateTime<Utc>, NaiveDate, NaiveDate) {
    let local_date = anchor.with_timezone(&Local).date_naive();
    let first = local_date.with_day(1).unwrap_or(local_date);
    let monday = first - Duration::days(first.weekday().num_days_from_monday() as i64);
    (day_start_utc(monday), monday, first)
}

/// Calendar month arithmetic with day clamping: Jan 31 − 1 month → Dec 31.
fn shift_months(date: NaiveDate, months: i64) -> NaiveDate {
    let total = i64::from(date.year()) * 12 + i64::from(date.month0()) + months;
    let year = total.div_euclid(12).clamp(
        i64::from(NaiveDate::MIN.year()),
        i64::from(NaiveDate::MAX.year()),
    );
    let month0 = total.rem_euclid(12) as u32;
    let day = date.day().min(days_in_month(year, month0));
    NaiveDate::from_ymd_opt(year as i32, month0 + 1, day).unwrap_or(date)
}

fn days_in_month(year: i64, month0: u32) -> u32 {
    const COMMON: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    if month0 == 1 && leap {
        29
    } else {
        COMMON[month0 as usize]
    }
}
