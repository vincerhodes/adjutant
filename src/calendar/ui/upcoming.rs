//! Upcoming list (spec §7): every upcoming occurrence as a full-width card
//! in a single scroll column — date chip, time range or "All day", title,
//! location. Past-dimmed rows scroll away naturally (the store window
//! starts at `now`).

use egui::{RichText, Ui};

use crate::calendar::CalendarStore;
use crate::db::Db;
use crate::ui::{self, icons};

use super::{occurrence_day_label, occurrence_time_label, CalendarUi};

pub fn show(calendar: &mut CalendarUi, ui: &mut Ui, db: &Db) {
    let now = calendar.now;
    let occs: Vec<crate::calendar::Occurrence> = calendar.upcoming.clone();
    if occs.is_empty() {
        ui::empty_state(
            ui,
            "Nothing upcoming",
            "Press Ctrl+N or the New event button to schedule something.",
        );
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        for occ in &occs {
            let hovered = calendar.hovered.contains(&occ.event_id);
            let dot = icons::GROUP_DOT_COLORS[occ.color_idx.clamp(0, 5) as usize];
            let frame = ui::card_frame(ui, hovered, false);
            let response = frame
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        icons::group_dot(ui, dot, 8.0);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(occ.title.clone())
                                    .size(ui::fonts::SIZE_CARD_TITLE)
                                    .strong(),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{} · {}",
                                    occurrence_day_label(occ, now),
                                    occurrence_time_label(occ),
                                ))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                            );
                            if !occ.location.is_empty() {
                                ui.label(
                                    RichText::new(occ.location.clone())
                                        .small()
                                        .color(ui.visuals().weak_text_color()),
                                );
                            }
                        });
                    });
                })
                .response
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.hovered() {
                calendar.hovered.insert(occ.event_id);
            }
            if response.clicked() {
                open_occurrence(calendar, db, occ);
            }
            ui.add_space(6.0);
        }
    });
}

fn open_occurrence(calendar: &mut CalendarUi, db: &Db, occ: &crate::calendar::Occurrence) {
    let store = CalendarStore::new(db);
    let event = calendar
        .events
        .iter()
        .find(|e| e.id == occ.event_id)
        .cloned()
        .or_else(|| store.get_event(occ.event_id).ok());
    if let Some(event) = event.as_ref() {
        let start = occ
            .start_utc
            .or_else(|| {
                occ.start_date
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|n| n.and_utc())
            })
            .unwrap_or(calendar.now);
        calendar.open_event(event, Some((occ, start)));
    }
}
