//! Week grid (spec §7 + Phase 2 rework): fixed time gutter + 7 equal day
//! columns, one vertical scroll, hover ghost slots on 30-minute
//! increments, click-to-create at the hovered slot, drag-to-create a
//! range, all-day band with overflow collapse, full-width now-line.
//! Event blocks keep the existing painter/lane idiom in the new
//! coordinate space.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, Utc};
use egui::{Align2, Pos2, Sense, Stroke, Ui};

use crate::ui::{self, icons, theme};

use super::{day_start_utc, CalendarUi};
use crate::calendar::rrule;

/// Left time-gutter width (hour labels).
const GUTTER_W: f32 = 44.0;
/// Fixed height of one hour row.
const HOUR_H: f32 = 48.0;
/// All-day band height (fixed above the scroll area).
const ALLDAY_BAND_H: f32 = 28.0;
/// Day header row height (fixed, above the all-day band).
const HEADER_H: f32 = 22.0;
/// Grid-local snap unit for hover/click/drag slots.
const SLOT_MINUTES: i64 = 30;
/// Drags shorter than this are treated as plain clicks.
const DRAG_THRESHOLD_MINUTES: i64 = 15;

/// Live drag-to-create state (grid-local coordinates).
pub struct WeekDrag {
    day: usize,
    start_y: f32,
    cur_y: f32,
}

pub fn show(calendar: &mut CalendarUi, ui: &mut Ui, _db: &crate::db::Db) {
    let monday = monday_of(calendar.week_anchor);
    let now = calendar.now;
    let (week_start, week_end) = super::week_bounds(calendar.week_anchor);

    let mut occs: Vec<crate::calendar::Occurrence> = Vec::new();
    for event in &calendar.events {
        match rrule::expand(event, week_start, week_end) {
            Ok(mut o) => occs.append(&mut o),
            Err(e) => eprintln!("adjutant: calendar view: {e}"),
        }
    }
    occs.sort_by_key(|o| o.start_utc.unwrap_or(DateTime::<Utc>::MIN_UTC));

    let total_w = ui.available_width();
    let col_w = ((total_w - GUTTER_W) / 7.0).max(60.0);
    let grid_w = GUTTER_W + col_w * 7.0;
    calendar.week_col_w = col_w;

    // Fixed day-header row (gutter corner + weekday labels).
    let header = ui.allocate_exact_size(egui::vec2(grid_w, HEADER_H), Sense::hover());
    if ui.is_rect_visible(header.0) {
        let p = theme::palette(ui);
        for day in 0..7 {
            let date = monday + Duration::days(day);
            let day_start = day_start_utc(date);
            let day_end = day_start_utc(date + Duration::days(1));
            let is_today = now >= day_start && now < day_end;
            let center = Pos2::new(
                header.0.min.x + GUTTER_W + col_w * (day as f32 + 0.5),
                header.0.center().y,
            );
            let text = date.format("%a %d").to_string();
            ui.painter().text(
                center,
                Align2::CENTER_CENTER,
                text,
                egui::FontId::new(ui::fonts::SIZE_SMALL, egui::FontFamily::Proportional),
                if is_today {
                    p.accent
                } else {
                    ui.visuals().weak_text_color()
                },
            );
        }
    }

    // Fixed all-day band: gutter corner + per-day chips. Click → all-day
    // create for that date.
    let band = ui.allocate_exact_size(egui::vec2(grid_w, ALLDAY_BAND_H), Sense::click());
    let band_response = band.1.on_hover_cursor(egui::CursorIcon::PointingHand);
    band_response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "All-day band"));
    if ui.is_rect_visible(band.0) {
        paint_all_day_band(ui, calendar, &occs, band.0, monday, col_w);
    }
    if band_response.clicked() {
        if let Some(pos) = band_response.interact_pointer_pos() {
            let day = day_index(pos.x - band.0.min.x, col_w);
            if let Some(day) = day {
                let mut form = super::event_form::EventForm::new(
                    Some(monday + Duration::days(day as i64)),
                    None,
                    None,
                );
                form.preset_all_day(monday + Duration::days(day as i64));
                calendar.form = Some(form);
            }
        }
    }

    // Scrollable hour grid: gutter + 7 columns in ONE interaction rect.
    // Inside the closure the coordinates are already translated to ctx
    // space, so the grid rect IS the test seam (no scroll-state math).
    let mut origin = None;
    egui::ScrollArea::vertical()
        .id_salt("cal-week-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let grid_h = 24.0 * HOUR_H;
            let (grid_rect, grid_response) =
                ui.allocate_exact_size(egui::vec2(grid_w, grid_h), Sense::click_and_drag());
            let grid_response = grid_response.on_hover_cursor(egui::CursorIcon::PointingHand);
            grid_response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Week grid")
            });
            // First-frame scroll: put 08:00 near the top. The origin read
            // on this frame is pre-scroll (stale offset), so it is dropped;
            // from the next frame on the offset is stable.
            if !calendar.week_scrolled {
                calendar.week_scrolled = true;
                // Instant: an animated scroll leaves the origin read
                // mid-flight and tests would race the easing.
                ui.scroll_to_rect_animation(
                    egui::Rect::from_min_size(
                        Pos2::new(grid_rect.min.x, grid_rect.min.y + 8.0 * HOUR_H),
                        egui::vec2(1.0, 1.0),
                    ),
                    Some(egui::Align::TOP),
                    egui::style::ScrollAnimation::none(),
                );
            } else {
                origin = Some(grid_rect.min);
            }

            paint_grid_chrome(ui, &grid_response, grid_rect, monday, now, col_w);
            paint_hover_and_drag(ui, calendar, &grid_response, grid_rect, col_w);
            paint_day_blocks(ui, calendar, &occs, grid_rect, monday, now, col_w);

            handle_grid_input(calendar, &grid_response, grid_rect, monday, col_w);
        });
    calendar.week_origin = origin;
}

// ── painting ───────────────────────────────────────────────────────────────

/// Hour lines across the full grid, gutter labels, column separators,
/// now-line gutter→right edge.
fn paint_grid_chrome(
    ui: &Ui,
    grid_response: &egui::Response,
    rect: egui::Rect,
    monday: NaiveDate,
    now: DateTime<Utc>,
    col_w: f32,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = theme::palette(ui);
    let painter = ui.painter_at(rect);

    for hour in 0..=24 {
        let y = rect.top() + hour as f32 * HOUR_H;
        painter.hline(rect.left()..=rect.right(), y, Stroke::new(0.5, p.border));
        if hour < 24 {
            painter.text(
                Pos2::new(rect.left() + 3.0, y + 2.0),
                Align2::LEFT_TOP,
                format!("{hour:02}:00"),
                egui::FontId::new(ui::fonts::SIZE_SMALL, egui::FontFamily::Proportional),
                p.muted,
            );
        }
    }
    // Column separators (0.5px hairlines).
    for day in 1..7 {
        let x = rect.left() + GUTTER_W + col_w * day as f32;
        painter.vline(x, rect.top()..=rect.bottom(), Stroke::new(0.5, p.border));
    }

    // Now-line spans the full grid when today is in view.
    let today = now.with_timezone(&Local).date_naive();
    if today >= monday && today < monday + Duration::days(7) {
        let day_start = day_start_utc(today);
        let frac = (now - day_start).num_minutes() as f32 / (24.0 * 60.0);
        let y = rect.top() + rect.height() * frac;
        painter.hline(rect.left()..=rect.right(), y, Stroke::new(1.5, p.accent));
        painter.circle_filled(Pos2::new(rect.left() + 4.0, y), 3.0, p.accent);
    }

    let _ = grid_response;
}

/// Hover ghost slot (30-min increments) + live drag selection.
fn paint_hover_and_drag(
    ui: &Ui,
    calendar: &mut CalendarUi,
    grid_response: &egui::Response,
    rect: egui::Rect,
    col_w: f32,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = theme::palette(ui);
    let painter = ui.painter_at(rect);
    let ghost = |day: usize, start_min: i64, end_min: i64, strong: bool| {
        let x = rect.left() + GUTTER_W + col_w * day as f32;
        let top = rect.top() + start_min as f32 / 60.0 * HOUR_H;
        let bottom = rect.top() + end_min as f32 / 60.0 * HOUR_H;
        let slot = egui::Rect::from_min_size(
            Pos2::new(x + 2.0, top),
            egui::vec2(col_w - 4.0, (bottom - top).max(HOUR_H / 2.0)),
        );
        let alpha = if strong { 0.20 } else { 0.12 };
        painter.rect_filled(
            slot,
            egui::CornerRadius::same(4),
            egui::Color32::from_rgba_unmultiplied(
                p.accent.r(),
                p.accent.g(),
                p.accent.b(),
                (alpha * 255.0) as u8,
            ),
        );
    };

    if let Some(drag) = &calendar.week_drag {
        let (a, b) = drag_range(drag.start_y, drag.cur_y);
        ghost(drag.day, a, b + SLOT_MINUTES, true);
    } else if let Some(pos) = grid_response.hover_pos() {
        let local = pos - rect.min.to_vec2();
        if let Some(day) = day_index(local.x, col_w) {
            let start = slot_start(local.y);
            ghost(day, start, start + SLOT_MINUTES, false);
            let x = rect.left() + GUTTER_W + col_w * day as f32;
            let top = rect.top() + start as f32 / 60.0 * HOUR_H;
            painter.text(
                Pos2::new(x + 4.0, top + 1.0),
                Align2::LEFT_TOP,
                format!("{:02}:{:02}", start / 60, start % 60),
                egui::FontId::new(10.5, egui::FontFamily::Proportional),
                p.muted,
            );
        }
    }
}

/// All-day chips per day; overflow collapses to a muted "+n".
fn paint_all_day_band(
    ui: &Ui,
    calendar: &mut CalendarUi,
    occs: &[crate::calendar::Occurrence],
    rect: egui::Rect,
    monday: NaiveDate,
    col_w: f32,
) {
    let p = theme::palette(ui);
    let painter = ui.painter_at(rect);
    for day in 0..7 {
        let date = monday + Duration::days(day);
        let chips: Vec<&crate::calendar::Occurrence> = occs
            .iter()
            .filter(|o| {
                o.all_day
                    && o.start_date.is_some_and(|s| s <= date)
                    && o.end_date.is_some_and(|e| e >= date)
            })
            .collect();
        let x = rect.left() + GUTTER_W + col_w * day as f32 + 3.0;
        let right = rect.left() + GUTTER_W + col_w * (day as f32 + 1.0) - 3.0;
        let mut chip_x = x;
        let mut hidden = 0usize;
        for occ in &chips {
            let remaining = right - chip_x;
            if remaining < 30.0 {
                hidden += 1;
                continue;
            }
            let w = (occ.title.len() as f32 * 6.0 + 14.0).min(remaining);
            let chip = egui::Rect::from_min_size(
                Pos2::new(chip_x, rect.top() + 3.0),
                egui::vec2(w, ALLDAY_BAND_H - 6.0),
            );
            paint_occurrence_chip(ui, calendar, occ, chip, &painter, p);
            chip_x += w + 3.0;
        }
        if hidden > 0 {
            painter.text(
                Pos2::new(chip_x + 2.0, rect.top() + ALLDAY_BAND_H / 2.0),
                Align2::LEFT_CENTER,
                format!("+{hidden}"),
                egui::FontId::new(ui::fonts::SIZE_SMALL, egui::FontFamily::Proportional),
                p.muted,
            );
        }
    }
}

/// Timed occurrence blocks with per-day lane stacking.
fn paint_day_blocks(
    ui: &Ui,
    calendar: &mut CalendarUi,
    occs: &[crate::calendar::Occurrence],
    rect: egui::Rect,
    monday: NaiveDate,
    now: DateTime<Utc>,
    col_w: f32,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = theme::palette(ui);
    let painter = ui.painter_at(rect);
    let block_w = (col_w - 6.0) / 2.0;
    for day in 0..7 {
        let date = monday + Duration::days(day);
        let day_start = day_start_utc(date);
        let day_end = day_start_utc(date + Duration::days(1));
        let col_left = rect.left() + GUTTER_W + col_w * day as f32;
        let mut lanes: Vec<(f32, f32)> = Vec::new();
        for occ in occs.iter().filter(|o| !o.all_day) {
            let (Some(start), Some(end)) = (occ.start_utc, occ.end_utc) else {
                continue;
            };
            if end <= day_start || start >= day_end {
                continue;
            }
            let clipped_start = start.max(day_start);
            let clipped_end = end.min(day_end);
            let top = rect.top() + (clipped_start - day_start).num_minutes() as f32 / 60.0 * HOUR_H;
            let height =
                ((clipped_end - clipped_start).num_minutes().max(15) as f32 / 60.0) * HOUR_H;
            let top = top.min(rect.bottom() - 14.0);
            let height = height.max(14.0).min(rect.bottom() - top);
            let x = if let Some(lane) = lanes.iter_mut().find(|(bottom, _)| *bottom <= top) {
                lane.0 = top + height;
                lane.1
            } else {
                let x = col_left + 2.0 + lanes.len() as f32 * (block_w + 2.0);
                lanes.push((top + height, x));
                x
            };
            let x = x.min(col_left + col_w - block_w - 1.0);
            let block = egui::Rect::from_min_size(Pos2::new(x, top), egui::vec2(block_w, height));
            paint_occurrence_block(ui, calendar, occ, block, now, &painter, p);
        }
    }
}

// ── event blocks (existing idiom, adapted coordinates) ────────────────────

fn paint_occurrence_chip(
    ui: &Ui,
    calendar: &mut CalendarUi,
    occ: &crate::calendar::Occurrence,
    rect: egui::Rect,
    painter: &egui::Painter,
    p: theme::Palette,
) {
    let dot = icons::GROUP_DOT_COLORS[(occ.color_idx.clamp(0, 5)) as usize];
    painter.rect_filled(rect, egui::CornerRadius::same(4), dot.gamma_multiply(0.16));
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(4),
        Stroke::new(1.0, dot.gamma_multiply(0.7)),
        egui::StrokeKind::Inside,
    );
    if !occ.title.is_empty() {
        painter.text(
            Pos2::new(rect.left() + 4.0, rect.top() + rect.height() / 2.0),
            Align2::LEFT_CENTER,
            occ.title.clone(),
            egui::FontId::new(10.5, egui::FontFamily::Proportional),
            p.text,
        );
    }
    let occ_id = occ.event_id;
    let occ_clone = occ.clone();
    let response = ui
        .interact(
            rect,
            ui.id()
                .with(("allday-chip", occ.event_id, rect.left() as i32)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        let event = calendar.events.iter().find(|e| e.id == occ_id).cloned();
        if let Some(event) = event {
            let start = occurrence_fallback_start(&occ_clone);
            calendar.open_event(&event, Some((&occ_clone, start)));
        }
    }
}

fn paint_occurrence_block(
    ui: &Ui,
    calendar: &mut CalendarUi,
    occ: &crate::calendar::Occurrence,
    rect: egui::Rect,
    now: DateTime<Utc>,
    painter: &egui::Painter,
    p: theme::Palette,
) {
    let past = occ
        .end_utc
        .or_else(|| {
            occ.end_date
                .and_then(|d| d.and_hms_opt(23, 59, 59))
                .map(|n| n.and_utc())
        })
        .is_some_and(|end| end < now);
    let dot = icons::GROUP_DOT_COLORS[(occ.color_idx.clamp(0, 5)) as usize];
    let fill = if past {
        p.card_fill.gamma_multiply(0.75)
    } else {
        dot.gamma_multiply(0.16)
    };
    painter.rect_filled(rect, egui::CornerRadius::same(4), fill);
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(4),
        Stroke::new(1.0, dot.gamma_multiply(if past { 0.4 } else { 0.7 })),
        egui::StrokeKind::Inside,
    );
    let text_color = if past { p.muted } else { p.text };
    if rect.height() >= 16.0 && !occ.title.is_empty() {
        let text = if rect.height() < 26.0 {
            occ.title.clone()
        } else {
            let time = super::occurrence_time_label(occ);
            format!("{} · {}", occ.title, time)
        };
        painter.text(
            Pos2::new(rect.left() + 4.0, rect.top() + 2.0),
            Align2::LEFT_TOP,
            text,
            egui::FontId::new(10.5, egui::FontFamily::Proportional),
            text_color,
        );
    }

    let occ_id = occ.event_id;
    let occ_clone = occ.clone();
    let response = ui
        .interact(
            rect,
            ui.id().with(("occ", occ_id, (rect.top() * 10.0) as i32)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        let event = calendar.events.iter().find(|e| e.id == occ_id).cloned();
        if let Some(event) = event {
            let start = occ_clone
                .start_utc
                .unwrap_or_else(|| occurrence_fallback_start(&occ_clone));
            calendar.open_event(&event, Some((&occ_clone, start)));
        }
    }
}

fn occurrence_fallback_start(occ: &crate::calendar::Occurrence) -> DateTime<Utc> {
    occ.start_date
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|n| n.and_utc())
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

// ── input ──────────────────────────────────────────────────────────────────

fn handle_grid_input(
    calendar: &mut CalendarUi,
    response: &egui::Response,
    rect: egui::Rect,
    monday: NaiveDate,
    col_w: f32,
) {
    let Some(pointer) = response.interact_pointer_pos() else {
        if response.drag_stopped() {
            calendar.week_drag = None;
        }
        return;
    };
    let local = pointer - rect.min.to_vec2();

    if response.drag_started() {
        // drag_started fires on the threshold-crossing frame, when the
        // pointer is already displaced — the true anchor is the press
        // origin (persists on the pointer state for the whole gesture).
        let press = response
            .ctx
            .input(|i| i.pointer.press_origin())
            .map(|p| p - rect.min.to_vec2());
        if let (Some(day), Some(press)) = (day_index(local.x, col_w), press) {
            calendar.week_drag = Some(WeekDrag {
                day,
                start_y: press.y,
                cur_y: local.y,
            });
        }
    }
    if response.dragged() {
        if let Some(drag) = calendar.week_drag.as_mut() {
            drag.cur_y = local.y;
        }
    }
    if response.drag_stopped() {
        let drag = calendar.week_drag.take();
        if let Some(day) = day_index(local.x, col_w) {
            let date = monday + Duration::days(day as i64);
            if let Some(drag) = drag.filter(|d| d.day == day) {
                let (start_min, end_min) = drag_range(drag.start_y, drag.cur_y);
                if end_min - start_min >= DRAG_THRESHOLD_MINUTES {
                    calendar.form = Some(super::event_form::EventForm::new(
                        Some(date),
                        Some(time_at(start_min)),
                        Some(time_at(end_min + SLOT_MINUTES)),
                    ));
                    return;
                }
                // Too short: fall through to click-create at the press point.
                let slot = slot_start(drag.start_y);
                calendar.form = Some(super::event_form::EventForm::new(
                    Some(date),
                    Some(time_at(slot)),
                    None,
                ));
                return;
            }
        }
    }
    if response.clicked() {
        if let Some(day) = day_index(local.x, col_w) {
            let date = monday + Duration::days(day as i64);
            let slot = slot_start(local.y);
            calendar.form = Some(super::event_form::EventForm::new(
                Some(date),
                Some(time_at(slot)),
                None,
            ));
        }
    }
}

// ── helpers ────────────────────────────────────────────────────────────────

fn monday_of(anchor: DateTime<Utc>) -> NaiveDate {
    let date = anchor.with_timezone(&Local).date_naive();
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

/// Column index from a grid-local x, or None in the gutter.
fn day_index(local_x: f32, col_w: f32) -> Option<usize> {
    if local_x < GUTTER_W {
        return None;
    }
    let day = ((local_x - GUTTER_W) / col_w) as i64;
    (0..7).contains(&day).then_some(day as usize)
}

/// Floor a grid-local y to the start of its 30-minute slot, clamped.
fn slot_start(local_y: f32) -> i64 {
    let minutes = (local_y / HOUR_H * 60.0) as i64;
    (minutes / SLOT_MINUTES * SLOT_MINUTES).clamp(0, 24 * 60 - SLOT_MINUTES)
}

/// Ordered (start, end) slot range for a drag, both floored to 30 min.
fn drag_range(a: f32, b: f32) -> (i64, i64) {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    (slot_start(lo), slot_start(hi))
}

fn time_at(minutes: i64) -> NaiveTime {
    let minutes = minutes.clamp(0, 23 * 60 + 59);
    NaiveTime::from_hms_opt((minutes / 60) as u32, (minutes % 60) as u32, 0)
        .unwrap_or_else(|| DateTime::<Utc>::MIN_UTC.time())
}
