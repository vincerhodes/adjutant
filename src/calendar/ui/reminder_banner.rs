//! Reminder banner (spec §6/§7): due and fired-today reminders in a strip
//! at the top of the calendar view — event title, relative fire time,
//! Dismiss (ghost) / Snooze (dropdown 5/15/60 min). Snooze re-fires in-app
//! only; Dismiss silences the row permanently. Zero SQL — store methods
//! only.

use chrono::{DateTime, Duration, Utc};
use egui::{RichText, Ui};

use crate::db::Db;
use crate::ui::{self, icons, theme};

use super::CalendarUi;

const SNOOZE_CHOICES: [(i64, &str); 3] = [(5, "5 minutes"), (15, "15 minutes"), (60, "1 hour")];

pub fn show(calendar: &mut CalendarUi, ui: &mut Ui, db: &Db, toasts: &mut Vec<String>) {
    if calendar.due.is_empty() {
        return;
    }
    let now = calendar.now;
    let items = calendar.due.clone();
    let frame = egui::Frame::new()
        .fill(theme::palette(ui).selection_fill)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(10, 6));
    frame.show(ui, |ui| {
        for item in &items {
            ui.horizontal(|ui| {
                icons::chip(
                    ui,
                    icons::Icon::Clock,
                    theme::palette(ui).accent,
                    "Reminder",
                );
                ui.label(RichText::new(item.title.clone()).size(ui::fonts::SIZE_BODY));
                ui.label(
                    RichText::new(relative_label(item, now))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui::ghost_button(ui, "Dismiss").clicked() {
                        match crate::calendar::CalendarStore::new(db).dismiss_fire(item.fire_id) {
                            Ok(()) => calendar.dirty = true,
                            Err(e) => toasts.push(e.to_string()),
                        }
                    }
                    let mut snooze_to = None;
                    egui::containers::menu::MenuButton::from_button(
                        egui::Button::new(RichText::new("Snooze").color(ui::accent_of(ui)))
                            .frame(false),
                    )
                    .ui(ui, |ui| {
                        for (minutes, label) in SNOOZE_CHOICES {
                            if ui.button(label).clicked() {
                                snooze_to = Some(now + Duration::minutes(minutes));
                                ui.close();
                            }
                        }
                    });
                    if let Some(target) = snooze_to {
                        match crate::calendar::CalendarStore::new(db)
                            .snooze_fire(item.fire_id, target)
                        {
                            Ok(()) => calendar.dirty = true,
                            Err(e) => toasts.push(e.to_string()),
                        }
                    }
                });
            });
            ui.add_space(2.0);
        }
    });
}

/// "due in 10 min" / "due 5 min ago" / "fired earlier today" style label.
fn relative_label(item: &crate::calendar::DueReminder, now: DateTime<Utc>) -> String {
    if let Some(fired) = item.fired_at {
        let ago = (now - fired).num_minutes().max(0);
        return format!("fired {ago} min ago");
    }
    if item.snoozed_to_utc.is_some() {
        let delta = (item.due_utc() - now).num_minutes();
        if delta > 0 {
            return format!("snoozed, fires in {delta} min");
        }
        return "snoozed — firing…".to_string();
    }
    let delta = (now - item.due_utc()).num_minutes();
    if delta >= 0 {
        format!("due {delta} min ago")
    } else {
        format!("due in {} min", -delta)
    }
}
