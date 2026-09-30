//! Dashboard (spec §7): upcoming-5 strip of compact cards over the week
//! grid. One vertical scroll column — when the window is short the strip
//! scrolls away and the week stays reachable.

use egui::Ui;

use crate::calendar::CalendarStore;
use crate::db::Db;
use crate::ui::icons;

use super::{occurrence_card, occurrence_day_label, occurrence_time_label, CalendarUi};

pub fn show(calendar: &mut CalendarUi, ui: &mut Ui, db: &Db) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        upcoming_strip(calendar, ui, db);
        ui.add_space(10.0);
        super::week::show(calendar, ui, db);
    });
}

fn upcoming_strip(calendar: &mut CalendarUi, ui: &mut Ui, db: &Db) {
    let now = calendar.now;
    let strip: Vec<crate::calendar::Occurrence> =
        calendar.upcoming.iter().take(5).cloned().collect();
    if strip.is_empty() {
        ui.label(
            egui::RichText::new("Nothing scheduled")
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(6.0);
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for occ in &strip {
            let hovered = calendar.hovered.contains(&occ.event_id);
            let dot = icons::GROUP_DOT_COLORS[occ.color_idx.clamp(0, 5) as usize];
            let day = occurrence_day_label(occ, now);
            let time = occurrence_time_label(occ);
            let response =
                occurrence_card(ui, hovered, dot, &day, &time, &occ.title, &occ.location);
            if response.hovered() {
                calendar.hovered.insert(occ.event_id);
            }
            if response.clicked() {
                open_occurrence(calendar, db, occ);
            }
        }
    });
    ui.add_space(4.0);
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
    // A vanished event row (deleted mid-frame) is ignored — the strip
    // refreshes on the next reload.
}
