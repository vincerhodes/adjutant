//! Event form (spec §7 + §8.3): create/edit modal — timed/all-day toggle,
//! hand-rolled date/time entry (plan risk 10: no date-picker crate), IANA
//! timezone picker defaulting to the system zone, recurrence section
//! (frequency/interval/by-day/until/count), attendees, reminders, color
//! tag, validation. Recurring events offer the this-occurrence /
//! this-and-future / whole-series choice (spec §5).
//!
//! Zero SQL — all persistence goes through `CalendarStore`.

use std::str::FromStr;

use chrono::{DateTime, NaiveDate, NaiveTime, Utc, Weekday};
use chrono_tz::Tz;
use egui::{Context, Key, RichText, Ui};
use uuid::Uuid;

use crate::calendar::model::{
    AttendeeInput, Event, EventInput, Freq, Occurrence, OccurrenceChanges, Rsvp,
};
use crate::calendar::rrule;
use crate::calendar::CalendarStore;
use crate::db::Db;
use crate::ui::{self, icons, theme};

use super::system_tz_name;

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

const WEEKDAYS: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

pub struct EventForm {
    editing_id: Option<Uuid>,
    /// The original series event (scope reset + Future RRULE source).
    base: Option<Event>,
    /// Original series instant of the clicked occurrence (identifies "this").
    occurrence_start: Option<DateTime<Utc>>,
    pub scope: Scope,
    // Fields.
    title: String,
    location: String,
    description: String,
    all_day: bool,
    start_date: String,
    end_date: String,
    start_time: String,
    end_time: String,
    tz: String,
    // Recurrence editor.
    recurring: bool,
    freq: Freq,
    interval: String,
    byday: Vec<Weekday>,
    bymonthday: String,
    until: String,
    count: String,
    color_idx: i64,
    attendees: Vec<(String, String, Rsvp)>,
    reminders: Vec<String>,
    // UI state.
    error: Option<String>,
    confirm_trash: bool,
    reminders_loaded: bool,
    close: bool,
}

impl EventForm {
    pub fn new(preset_date: Option<NaiveDate>, default_tz: Option<String>) -> EventForm {
        let today = chrono::Local::now().date_naive();
        let mut form = EventForm::skeleton();
        form.start_date = preset_date.unwrap_or(today).to_string();
        form.end_date = form.start_date.clone();
        form.tz = default_tz.unwrap_or_else(system_tz_name);
        form.reminders = vec!["10".to_string()];
        form
    }

    pub fn edit(event: &Event, occurrence: Option<(&Occurrence, DateTime<Utc>)>) -> EventForm {
        let mut form = EventForm::skeleton();
        form.editing_id = Some(event.id);
        form.base = Some(event.clone());
        form.scope = if event.rrule.is_some() {
            Scope::This
        } else {
            Scope::Series
        };
        form.recurring = event.rrule.is_some();
        if let Some(rrule_text) = &event.rrule {
            form.apply_rrule(rrule_text, &event.tz);
        }
        // Prefill from the clicked occurrence (exception overrides applied)
        // so "this occurrence" edits start from the occurrence's values.
        let source = occurrence.map(|(o, _)| o.clone()).or_else(|| {
            Some(Occurrence {
                event_id: event.id,
                title: event.title.clone(),
                description: event.description.clone(),
                location: event.location.clone(),
                all_day: event.all_day,
                start_utc: event.start_utc,
                end_utc: event.end_utc,
                start_date: event.start_date,
                end_date: event.end_date,
                tz: event.tz.clone(),
                color_idx: event.color_idx,
            })
        });
        if let Some(o) = source {
            form.apply_occurrence_times(&o);
            form.title = o.title;
            form.location = o.location;
            form.description = o.description;
            form.all_day = o.all_day;
            form.tz = o.tz;
            form.color_idx = o.color_idx;
        }
        form.attendees = event
            .attendees
            .iter()
            .map(|a| (a.name.clone(), a.email.clone(), a.rsvp))
            .collect();
        if let Some((_, start)) = occurrence {
            form.occurrence_start = Some(start);
        }
        form
    }

    fn skeleton() -> EventForm {
        EventForm {
            editing_id: None,
            base: None,
            occurrence_start: None,
            scope: Scope::Single,
            title: String::new(),
            location: String::new(),
            description: String::new(),
            all_day: false,
            start_date: String::new(),
            end_date: String::new(),
            start_time: "09:00".to_string(),
            end_time: "10:00".to_string(),
            tz: "UTC".to_string(),
            recurring: false,
            freq: Freq::Weekly,
            interval: "1".to_string(),
            byday: vec![Weekday::Mon],
            bymonthday: String::new(),
            until: String::new(),
            count: String::new(),
            color_idx: 0,
            attendees: Vec::new(),
            reminders: Vec::new(),
            error: None,
            confirm_trash: false,
            reminders_loaded: false,
            close: false,
        }
    }

    fn apply_rrule(&mut self, text: &str, tz: &str) {
        if let Ok(rule) = rrule::parse_rrule(text) {
            self.freq = rule.freq;
            self.interval = rule.interval.to_string();
            self.byday = rule.byday;
            self.bymonthday = rule
                .bymonthday
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join(",");
            if let Some(u) = rule.until {
                self.until = u
                    .with_timezone(&tz.parse::<Tz>().unwrap_or(chrono_tz::UTC))
                    .date_naive()
                    .to_string();
            }
            if let Some(c) = rule.count {
                self.count = c.to_string();
            }
        }
    }

    fn apply_occurrence_times(&mut self, o: &Occurrence) {
        let tz: Tz = self.tz.parse().unwrap_or(chrono_tz::UTC);
        if self.all_day {
            self.start_date = o.start_date.map(|d| d.to_string()).unwrap_or_default();
            self.end_date = o.end_date.map(|d| d.to_string()).unwrap_or_default();
        } else if let (Some(start), Some(end)) = (o.start_utc, o.end_utc) {
            let s = start.with_timezone(&tz);
            let e = end.with_timezone(&tz);
            self.start_date = s.date_naive().to_string();
            self.end_date = e.date_naive().to_string();
            self.start_time = s.format("%H:%M").to_string();
            self.end_time = e.format("%H:%M").to_string();
        }
    }

    /// Switching to "Entire series" resets the displayed fields to the
    /// series base values (occurrence values would bake an exception in).
    fn apply_scope(&mut self) {
        if self.scope != Scope::Series {
            return;
        }
        if let Some(base) = self.base.clone() {
            self.title = base.title.clone();
            self.location = base.location.clone();
            self.description = base.description.clone();
            self.all_day = base.all_day;
            self.tz = base.tz.clone();
            self.color_idx = base.color_idx;
            self.recurring = base.rrule.is_some();
            if let Some(text) = &base.rrule {
                self.apply_rrule(text, &base.tz);
            }
            let o = Occurrence {
                event_id: base.id,
                title: base.title,
                description: base.description,
                location: base.location,
                all_day: base.all_day,
                start_utc: base.start_utc,
                end_utc: base.end_utc,
                start_date: base.start_date,
                end_date: base.end_date,
                tz: base.tz,
                color_idx: base.color_idx,
            };
            self.apply_occurrence_times(&o);
        }
    }

    /// Renders the modal window; returns false when the form should close.
    pub fn show(&mut self, ctx: &Context, db: &Db, toasts: &mut Vec<String>) -> bool {
        if self.editing_id.is_some() && !self.reminders_loaded {
            self.reminders_loaded = true;
            if let Some(id) = self.editing_id {
                let store = CalendarStore::new(db);
                self.reminders = store
                    .reminders_for_event(id)
                    .unwrap_or_default()
                    .iter()
                    .map(|r| r.offset_minutes.to_string())
                    .collect();
            }
        }
        let mut open = true;
        let title = if self.editing_id.is_some() {
            "Edit event"
        } else {
            "New event"
        };
        egui::Window::new(title)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(480.0);
                if self.confirm_trash {
                    self.show_trash_confirm(ui, db, toasts);
                } else {
                    self.show_fields(ui, db, toasts);
                }
            });
        open && !self.close
    }

    fn show_fields(&mut self, ui: &mut Ui, db: &Db, toasts: &mut Vec<String>) {
        // Recurring-edit chooser (spec §5).
        let recurring_edit =
            self.editing_id.is_some() && self.base.as_ref().is_some_and(|b| b.rrule.is_some());
        if recurring_edit {
            ui.horizontal(|ui| {
                for (scope, label) in [
                    (Scope::This, "This occurrence"),
                    (Scope::Future, "This and future"),
                    (Scope::Series, "Entire series"),
                ] {
                    if ui::selectable(ui, self.scope == scope, label).clicked()
                        && self.scope != scope
                    {
                        self.scope = scope;
                        self.apply_scope();
                    }
                }
            });
            ui.add_space(6.0);
        }

        let mut title = self.title.clone();
        let title_response = ui.add(
            egui::TextEdit::singleline(&mut title)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .hint_text("Title")
                .font(egui::FontId::new(
                    ui::fonts::SIZE_HEADING,
                    egui::FontFamily::Proportional,
                )),
        );
        ui::focused_underline(ui, &title_response);
        ui::hovered_underline(ui, &title_response);
        if title_response.changed() {
            self.title = title;
        }
        let mut location = self.location.clone();
        let loc_response = ui.add(
            egui::TextEdit::singleline(&mut location)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .hint_text("Location"),
        );
        ui::focused_underline(ui, &loc_response);
        ui::hovered_underline(ui, &loc_response);
        if loc_response.changed() {
            self.location = location;
        }

        ui.add_space(4.0);
        ui.checkbox(&mut self.all_day, "All day");
        if self.all_day {
            ui.horizontal(|ui| {
                self.date_field(ui, "Start date", "start-date-field");
                self.date_field(ui, "End date", "end-date-field");
            });
        } else {
            ui.horizontal(|ui| {
                self.date_field(ui, "Date", "start-date-field");
                self.time_field(ui, "Start", "start-time-field");
                self.time_field(ui, "End", "end-time-field");
            });
        }

        self.tz_picker(ui);

        // Recurrence section — hidden for this-occurrence edits (series data).
        if self.scope != Scope::This {
            ui.add_space(6.0);
            ui.checkbox(&mut self.recurring, "Repeats");
            if self.recurring {
                self.recurrence_fields(ui);
            }
        }

        // Color tag: the shared 6-color cycle.
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Color")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            for (i, color) in icons::GROUP_DOT_COLORS.iter().enumerate() {
                let selected = self.color_idx == i as i64;
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                if ui.is_rect_visible(rect) {
                    let center = rect.center();
                    if selected {
                        ui.painter().circle_stroke(
                            center,
                            9.0,
                            egui::Stroke::new(1.5, ui.visuals().text_color()),
                        );
                    }
                    ui.painter().circle_filled(center, 6.0, *color);
                }
                if response.clicked() {
                    self.color_idx = i as i64;
                }
                ui::hand(response);
            }
        });

        if self.scope != Scope::This {
            self.attendees_section(ui);
            self.reminders_section(ui);
        }

        ui.add_space(4.0);
        let description = self.description.clone();
        let mut desc = description;
        let response = ui.add(
            egui::TextEdit::multiline(&mut desc)
                .frame(egui::Frame::NONE)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text("Description"),
        );
        ui::focused_underline(ui, &response);
        ui::hovered_underline(ui, &response);
        self.description = desc;

        if let Some(err) = &self.error {
            ui.add_space(4.0);
            ui.label(
                RichText::new(err.clone())
                    .small()
                    .color(theme::palette(ui).danger),
            );
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if self.editing_id.is_some() {
                let danger = theme::palette(ui).danger;
                if ui::ghost_button_with(ui, "Trash", danger).clicked() {
                    self.confirm_trash = true;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui::primary_button(ui, "Save").clicked() {
                    self.save(db, toasts);
                }
                if ui::ghost_button(ui, "Cancel").clicked() {
                    self.close = true;
                }
            });
        });
    }

    fn recurrence_fields(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let mut freq_label = match self.freq {
                Freq::Daily => "Daily",
                Freq::Weekly => "Weekly",
                Freq::Monthly => "Monthly",
                Freq::Yearly => "Yearly",
            };
            egui::ComboBox::from_id_salt("freq")
                .selected_text(freq_label)
                .show_ui(ui, |ui| {
                    for (freq, label) in [
                        (Freq::Daily, "Daily"),
                        (Freq::Weekly, "Weekly"),
                        (Freq::Monthly, "Monthly"),
                        (Freq::Yearly, "Yearly"),
                    ] {
                        ui.selectable_value(&mut self.freq, freq, label);
                    }
                });
            freq_label = match self.freq {
                Freq::Daily => "Daily",
                Freq::Weekly => "Weekly",
                Freq::Monthly => "Monthly",
                Freq::Yearly => "Yearly",
            };
            let _ = freq_label;
            ui.label(
                RichText::new("every")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            let mut interval = self.interval.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut interval)
                    .frame(egui::Frame::NONE)
                    .desired_width(36.0),
            );
            ui::focused_underline(ui, &response);
            if response.changed() {
                self.interval = interval;
            }
        });
        // BYDAY checkboxes (Mon..Sun).
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("On")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            for wd in WEEKDAYS {
                let code = wd.to_string();
                let selected = self.byday.contains(&wd);
                if ui::selectable(ui, selected, code).clicked() {
                    if selected {
                        self.byday.retain(|d| *d != wd);
                    } else {
                        self.byday.push(wd);
                    }
                }
            }
        });
        // BYMONTHDAY (1,15) + UNTIL / COUNT.
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Day of month")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            let mut v = self.bymonthday.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut v)
                    .frame(egui::Frame::NONE)
                    .desired_width(70.0)
                    .hint_text("1,15"),
            );
            ui::focused_underline(ui, &response);
            if response.changed() {
                self.bymonthday = v;
            }
            ui.label(
                RichText::new("Ends")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            let mut u = self.until.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut u)
                    .frame(egui::Frame::NONE)
                    .desired_width(90.0)
                    .hint_text("YYYY-MM-DD"),
            );
            ui::focused_underline(ui, &response);
            if response.changed() {
                self.until = u;
            }
            ui.label(
                RichText::new("or after")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            let mut c = self.count.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut c)
                    .frame(egui::Frame::NONE)
                    .desired_width(50.0)
                    .hint_text("n"),
            );
            ui::focused_underline(ui, &response);
            if response.changed() {
                self.count = c;
            }
        });
        ui.label(
            RichText::new("BYDAY and day-of-month together intersect: a day must match both.")
                .small()
                .color(ui.visuals().weak_text_color()),
        );
    }

    fn attendees_section(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Attendees")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            if ui::ghost_button(ui, "Add attendee").clicked() {
                self.attendees
                    .push((String::new(), String::new(), Rsvp::NeedsAction));
            }
        });
        let mut remove = None;
        for (i, (name, email, rsvp)) in self.attendees.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let mut n = name.clone();
                let r1 = ui.add(
                    egui::TextEdit::singleline(&mut n)
                        .frame(egui::Frame::NONE)
                        .desired_width(120.0)
                        .hint_text("Name"),
                );
                ui::focused_underline(ui, &r1);
                *name = n;
                let mut e = email.clone();
                let r2 = ui.add(
                    egui::TextEdit::singleline(&mut e)
                        .frame(egui::Frame::NONE)
                        .desired_width(180.0)
                        .hint_text("email@address"),
                );
                ui::focused_underline(ui, &r2);
                *email = e;
                let mut r = *rsvp;
                egui::ComboBox::from_id_salt(ui.id().with(("rsvp", i)))
                    .selected_text(rsvp_label(r))
                    .show_ui(ui, |ui| {
                        for choice in [
                            Rsvp::NeedsAction,
                            Rsvp::Accepted,
                            Rsvp::Declined,
                            Rsvp::Tentative,
                        ] {
                            ui.selectable_value(&mut r, choice, rsvp_label(choice));
                        }
                    });
                *rsvp = r;
                if ui::icon_button(ui, icons::Icon::Cross, "Remove attendee", "rm-att").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.attendees.remove(i);
        }
    }

    fn reminders_section(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Reminders")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            if ui::ghost_button(ui, "Add reminder").clicked() {
                self.reminders.push("10".to_string());
            }
        });
        let mut remove = None;
        for (i, minutes) in self.reminders.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let mut m = minutes.clone();
                let response = ui.add(
                    egui::TextEdit::singleline(&mut m)
                        .frame(egui::Frame::NONE)
                        .desired_width(48.0),
                );
                ui::focused_underline(ui, &response);
                if response.changed() {
                    *minutes = m;
                }
                ui.label(
                    RichText::new("minutes before")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                if ui::icon_button(ui, icons::Icon::Cross, "Remove reminder", "rm-rem").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.reminders.remove(i);
        }
    }

    fn date_field(&mut self, ui: &mut Ui, hint: &str, id: &str) {
        let mut value = self.start_date.clone();
        if hint == "End date" {
            value = self.end_date.clone();
        }
        let response = ui.add(
            egui::TextEdit::singleline(&mut value)
                .id(egui::Id::new(id))
                .frame(egui::Frame::NONE)
                .desired_width(96.0)
                .hint_text(hint),
        );
        ui::focused_underline(ui, &response);
        ui::hovered_underline(ui, &response);
        if response.changed() {
            if hint == "End date" {
                self.end_date = value;
            } else {
                self.start_date = value;
            }
        }
    }

    fn time_field(&mut self, ui: &mut Ui, hint: &str, id: &str) {
        let mut value = if hint == "End" {
            self.end_time.clone()
        } else {
            self.start_time.clone()
        };
        let response = ui.add(
            egui::TextEdit::singleline(&mut value)
                .id(egui::Id::new(id))
                .frame(egui::Frame::NONE)
                .desired_width(54.0)
                .hint_text(hint),
        );
        ui::focused_underline(ui, &response);
        ui::hovered_underline(ui, &response);
        if response.changed() {
            if hint == "End" {
                self.end_time = value;
            } else {
                self.start_time = value;
            }
        }
    }

    fn tz_picker(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Timezone")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            egui::ComboBox::from_id_salt("event-tz")
                .selected_text(self.tz.clone())
                .show_ui(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for zone in chrono_tz::TZ_VARIANTS.iter() {
                            ui.selectable_value(&mut self.tz, zone.name().to_string(), zone.name());
                        }
                    });
                });
        });
    }

    fn show_trash_confirm(&mut self, ui: &mut Ui, db: &Db, toasts: &mut Vec<String>) {
        let title = self.title.clone();
        ui.label(format!(
            "“{title}” will be hidden for 30 days, links included. You can restore it from the trash."
        ));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui::primary_button(ui, "Move to trash").clicked() {
                if let Some(id) = self.editing_id {
                    match CalendarStore::new(db).trash_event(id) {
                        Ok(()) => self.close = true,
                        Err(e) => toasts.push(e.to_string()),
                    }
                }
            }
            if ui::ghost_button(ui, "Cancel").clicked() {
                self.confirm_trash = false;
            }
        });
    }

    // ── save ──────────────────────────────────────────────────────────────

    fn save(&mut self, db: &Db, toasts: &mut Vec<String>) {
        match self.save_inner(db) {
            Ok(()) => {
                self.close = true;
            }
            Err(message) => {
                self.error = Some(message);
            }
        }
        let _ = toasts;
    }

    fn save_inner(&mut self, db: &Db) -> std::result::Result<(), String> {
        let input = self.build_input()?;
        let store = CalendarStore::new(db);
        match self.editing_id {
            None => {
                let id = store.create_event(&input).map_err(|e| e.to_string())?;
                self.sync_reminders(&store, id)?;
            }
            Some(id) => match self.scope {
                Scope::This => {
                    let occurrence = self.occurrence_start.ok_or_else(|| {
                        "This-occurrence edit is missing its occurrence".to_string()
                    })?;
                    let changes = OccurrenceChanges {
                        title: Some(input.title),
                        start_utc: input.start_utc,
                        end_utc: input.end_utc,
                        tz: Some(input.tz),
                        location: Some(input.location),
                        description: Some(input.description),
                    };
                    store
                        .modify_occurrence(id, occurrence, &changes)
                        .map_err(|e| e.to_string())?;
                }
                Scope::Future => {
                    let occurrence = self
                        .occurrence_start
                        .ok_or_else(|| "Future edit is missing its occurrence".to_string())?;
                    let new_id = store
                        .edit_future(id, occurrence, &input)
                        .map_err(|e| e.to_string())?;
                    self.sync_reminders(&store, new_id)?;
                }
                Scope::Series | Scope::Single => {
                    store.update_event(id, &input).map_err(|e| e.to_string())?;
                    self.sync_reminders(&store, id)?;
                }
            },
        }
        Ok(())
    }

    /// Diff reminder offsets against the stored rows: add missing, remove
    /// extras. Matching by offset preserves ledger rows (exactly-once).
    fn sync_reminders(
        &self,
        store: &CalendarStore,
        event_id: Uuid,
    ) -> std::result::Result<(), String> {
        if self.scope == Scope::This {
            return Ok(());
        }
        let existing = store
            .reminders_for_event(event_id)
            .map_err(|e| e.to_string())?;
        let mut unmatched: Vec<i64> = existing.iter().map(|r| r.offset_minutes).collect();
        for raw in &self.reminders {
            let offset: i64 = raw
                .trim()
                .parse()
                .map_err(|_| format!("Reminder offset “{raw}” is not a whole number of minutes"))?;
            if offset < 0 {
                return Err("Reminder offsets must be zero or positive".to_string());
            }
            if let Some(pos) = unmatched.iter().position(|o| *o == offset) {
                unmatched.remove(pos);
            } else {
                store
                    .add_reminder(event_id, offset)
                    .map_err(|e| e.to_string())?;
            }
        }
        // Remove extras (their ledger rows cascade with them).
        for reminder in &existing {
            if unmatched.contains(&reminder.offset_minutes) {
                store
                    .remove_reminder(reminder.id)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Validate every field and assemble the `EventInput` (spec §8.3).
    fn build_input(&self) -> std::result::Result<EventInput, String> {
        if self.title.trim().is_empty() {
            return Err("Add a title".to_string());
        }
        let tz: Tz = self
            .tz
            .parse()
            .map_err(|_| format!("Unknown timezone “{}”", self.tz))?;
        let start_date = NaiveDate::from_str(self.start_date.trim())
            .map_err(|_| format!("Start date must be YYYY-MM-DD, got “{}”", self.start_date))?;
        let (start_utc, end_utc, start_day, end_day) = if self.all_day {
            let end_date = NaiveDate::from_str(self.end_date.trim())
                .map_err(|_| format!("End date must be YYYY-MM-DD, got “{}”", self.end_date))?;
            if end_date < start_date {
                return Err("End date is before the start date".to_string());
            }
            (None, None, start_date, end_date)
        } else {
            let start_time = parse_hhmm(&self.start_time)
                .ok_or_else(|| format!("Start time must be HH:MM, got “{}”", self.start_time))?;
            let end_time = parse_hhmm(&self.end_time)
                .ok_or_else(|| format!("End time must be HH:MM, got “{}”", self.end_time))?;
            let start_naive = start_date.and_time(start_time);
            let end_naive = start_date.and_time(end_time);
            if end_naive <= start_naive {
                return Err("End must be after the start".to_string());
            }
            let s = rrule::local_to_utc(&tz, start_naive).map_err(|e| e.to_string())?;
            let e = rrule::local_to_utc(&tz, end_naive).map_err(|e| e.to_string())?;
            (Some(s), Some(e), start_date, start_date)
        };
        let rrule_text = self.build_rrule(&tz)?;
        for (name, email, _) in &self.attendees {
            if !email.trim().is_empty()
                && !email.contains('@')
                && (!name.trim().is_empty() || !email.trim().is_empty())
            {
                return Err(format!("Attendee email “{email}” is missing an @"));
            }
        }
        let attendees = self
            .attendees
            .iter()
            .filter(|(_, email, _)| !email.trim().is_empty())
            .map(|(name, email, rsvp)| AttendeeInput {
                name: name.trim().to_string(),
                email: email.trim().to_string(),
                rsvp: *rsvp,
            })
            .collect();
        Ok(EventInput {
            title: self.title.trim().to_string(),
            description: self.description.trim().to_string(),
            location: self.location.trim().to_string(),
            all_day: self.all_day,
            start_utc,
            end_utc,
            start_date: self.all_day.then_some(start_day),
            end_date: self.all_day.then_some(end_day),
            tz: self.tz.trim().to_string(),
            rrule: rrule_text,
            color_idx: self.color_idx,
            attendees,
        })
    }

    /// Assemble + validate the RRULE subset text (empty = non-recurring).
    fn build_rrule(&self, tz: &Tz) -> std::result::Result<Option<String>, String> {
        if !self.recurring || self.scope == Scope::This {
            return Ok(None);
        }
        let freq = match self.freq {
            Freq::Daily => "DAILY",
            Freq::Weekly => "WEEKLY",
            Freq::Monthly => "MONTHLY",
            Freq::Yearly => "YEARLY",
        };
        let mut parts = vec![format!("FREQ={freq}")];
        let interval: u32 = self
            .interval
            .trim()
            .parse()
            .map_err(|_| format!("Interval “{}” is not a positive number", self.interval))?;
        if interval == 0 {
            return Err("Interval must be at least 1".to_string());
        }
        if interval > 1 {
            parts.push(format!("INTERVAL={interval}"));
        }
        if !self.byday.is_empty() {
            let mut days = self.byday.clone();
            days.sort_by_key(|d| d.num_days_from_monday());
            let codes: Vec<&str> = days
                .iter()
                .map(|d| match d {
                    Weekday::Mon => "MO",
                    Weekday::Tue => "TU",
                    Weekday::Wed => "WE",
                    Weekday::Thu => "TH",
                    Weekday::Fri => "FR",
                    Weekday::Sat => "SA",
                    Weekday::Sun => "SU",
                })
                .collect();
            parts.push(format!("BYDAY={}", codes.join(",")));
        }
        if !self.bymonthday.trim().is_empty() {
            for piece in self.bymonthday.split(',') {
                let n: i32 = piece
                    .trim()
                    .parse()
                    .map_err(|_| format!("Day-of-month “{}” is not a number", self.bymonthday))?;
                if !(1..=31).contains(&n) {
                    return Err(format!("Day-of-month {n} is out of range (1..=31)"));
                }
            }
            parts.push(format!("BYMONTHDAY={}", self.bymonthday.trim()));
        }
        let until_utc = if !self.until.trim().is_empty() {
            let date = NaiveDate::from_str(self.until.trim())
                .map_err(|_| format!("End date must be YYYY-MM-DD, got “{}”", self.until))?;
            // Inclusive reading: recur through the end of that local day.
            let naive = date
                .and_hms_opt(23, 59, 59)
                .ok_or_else(|| "Invalid end date".to_string())?;
            Some(rrule::local_to_utc(tz, naive).map_err(|e| e.to_string())?)
        } else {
            None
        };
        let count = if !self.count.trim().is_empty() {
            let n: u32 = self
                .count
                .trim()
                .parse()
                .map_err(|_| format!("Count “{}” is not a positive number", self.count))?;
            if n == 0 {
                return Err("Count must be at least 1".to_string());
            }
            Some(n)
        } else {
            None
        };
        if until_utc.is_some() && count.is_some() {
            return Err("Use either an end date or a count, not both".to_string());
        }
        if let Some(u) = until_utc {
            parts.push(format!("UNTIL={}", u.format("%Y%m%dT%H%M%SZ")));
        }
        if let Some(c) = count {
            parts.push(format!("COUNT={c}"));
        }
        let text = parts.join(";");
        rrule::parse_rrule(&text).map_err(|e| e.to_string())?;
        Ok(Some(text))
    }
}

fn rsvp_label(rsvp: Rsvp) -> &'static str {
    match rsvp {
        Rsvp::NeedsAction => "Needs action",
        Rsvp::Accepted => "Accepted",
        Rsvp::Declined => "Declined",
        Rsvp::Tentative => "Tentative",
    }
}

/// `HH:MM` → `NaiveTime`, strict two-part numeric parse.
fn parse_hhmm(raw: &str) -> Option<NaiveTime> {
    let (h, m) = raw.trim().split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    NaiveTime::from_hms_opt(h, m, 0)
}

// Key handling while the form is open (Esc closes — matches the overlay).
impl EventForm {
    pub fn handle_key(&mut self, ctx: &Context) {
        if ctx.input(|i| i.key_pressed(Key::Escape)) && !self.confirm_trash {
            self.close = true;
        }
    }
}
