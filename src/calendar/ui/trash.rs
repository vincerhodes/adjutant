//! Calendar Trash panel (M3.6): trashed events as cards with hover-reveal
//! Restore / Delete-forever, confirm modal on hard delete (scratch
//! pattern). Rendering only — store methods; zero SQL.

use chrono::{DateTime, Utc};
use egui::{Context, RichText, Ui};

use crate::calendar::CalendarStore;
use crate::db::Db;
use crate::ui::{self, icons, theme};

use super::CalendarUi;

pub fn show(
    calendar: &mut CalendarUi,
    ui: &mut Ui,
    ctx: &Context,
    db: &Db,
    toasts: &mut Vec<String>,
) {
    if calendar.trashed.is_empty() {
        ui::empty_state(
            ui,
            "Trash is empty",
            "Trashed events land here for 30 days, then purge at startup.",
        );
        show_confirm(calendar, ctx, db, toasts);
        return;
    }
    let trashed = calendar.trashed.clone();
    egui::ScrollArea::vertical().show(ui, |ui| {
        for event in &trashed {
            let hovered = calendar.hovered.contains(&event.id);
            let frame = ui::card_frame(ui, hovered, false);
            let inner = frame.show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    icons::group_dot(
                        ui,
                        icons::GROUP_DOT_COLORS[event.color_idx.clamp(0, 5) as usize],
                        8.0,
                    );
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(event.title.clone())
                                .size(ui::fonts::SIZE_CARD_TITLE)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} · trashed {}",
                                event_date_label(event),
                                relative_label(
                                    event.trashed_at.unwrap_or(event.updated_at),
                                    Utc::now()
                                ),
                            ))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                        );
                    });
                });
            });
            ui.horizontal(|ui| {
                if ui::icon_button(ui, icons::Icon::Back, "Restore", "restore-ev").clicked() {
                    if let Err(e) = CalendarStore::new(db).restore_event(event.id) {
                        toasts.push(e.to_string());
                    }
                    calendar.dirty = true;
                }
                let danger = theme::palette(ui).danger;
                if ui::ghost_button_with(ui, "Delete", danger).clicked() {
                    calendar.confirm_delete_event = Some(event.id);
                }
            });
            let _ = inner;
            ui.add_space(6.0);
        }
    });
    show_confirm(calendar, ctx, db, toasts);
}

fn show_confirm(calendar: &mut CalendarUi, ctx: &Context, db: &Db, toasts: &mut Vec<String>) {
    let Some(id) = calendar.confirm_delete_event else {
        return;
    };
    let title = calendar
        .trashed
        .iter()
        .find(|e| e.id == id)
        .map(|e| e.title.clone())
        .unwrap_or_else(|| "this event".to_string());
    let mut open = true;
    egui::Window::new("Delete permanently?")
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(format!("“{title}” will be gone forever, including links."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui::primary_button(ui, "Delete forever").clicked() {
                    if let Err(e) = CalendarStore::new(db).delete_event(id) {
                        toasts.push(e.to_string());
                    }
                    calendar.confirm_delete_event = None;
                    calendar.dirty = true;
                }
                if ui::ghost_button(ui, "Cancel").clicked() {
                    calendar.confirm_delete_event = None;
                }
            });
        });
    if !open {
        calendar.confirm_delete_event = None;
    }
}

/// Relative recency label (same idiom as the scratch module's).
fn relative_label(updated_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let mins = (now - updated_at).num_minutes().max(0);
    if mins < 1 {
        "just now".to_string()
    } else if mins < 60 {
        format!("{mins}m ago")
    } else if mins < 60 * 24 {
        format!("{}h ago", mins / 60)
    } else {
        format!("{}d ago", mins / (60 * 24))
    }
}

/// One-line when/when label for the card's second line.
fn event_date_label(event: &crate::calendar::Event) -> String {
    if event.all_day {
        match (event.start_date, event.end_date) {
            (Some(s), Some(e)) if s == e => s.format("%d %b").to_string(),
            (Some(s), Some(e)) => format!("{} → {}", s.format("%d %b"), e.format("%d %b")),
            _ => "All day".to_string(),
        }
    } else {
        match (event.start_utc, event.end_utc) {
            (Some(s), Some(e)) => format!(
                "{} {}–{}",
                s.format("%d %b"),
                s.format("%H:%M"),
                e.format("%H:%M")
            ),
            _ => String::new(),
        }
    }
}
