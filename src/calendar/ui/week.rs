//! Week grid (spec §7): 7 day columns, all-day band on top, timed blocks
//! stacked by hour, now-line, past occurrences dimmed. Clicking an event
//! opens it in the event form; clicking empty space starts a new event
//! that day.

use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use egui::{Align2, Pos2, RichText, Sense, Stroke, Ui};

use crate::ui::{self, icons, theme};

use super::{day_start_utc, CalendarUi};
use crate::calendar::rrule;

/// Height of the all-day band atop each column.
const ALLDAY_BAND_H: f32 = 28.0;
/// Column header height.
const HEADER_H: f32 = 26.0;

pub fn show(calendar: &mut CalendarUi, ui: &mut Ui, _db: &crate::db::Db) {
    let (week_start, week_end) = super::week_bounds(calendar.week_anchor);
    let now = calendar.now;

    // Expand cached base rows into this week's occurrences.
    let mut occs: Vec<crate::calendar::Occurrence> = Vec::new();
    for event in &calendar.events {
        match rrule::expand(event, week_start, week_end) {
            Ok(mut o) => occs.append(&mut o),
            Err(e) => eprintln!("adjutant: calendar view: {e}"),
        }
    }
    occs.sort_by_key(|o| o.start_utc.unwrap_or(DateTime::<Utc>::MIN_UTC));

    let available = ui.available_size();
    let column_w = (available.x / 7.0).max(90.0);
    let grid_h = (available.y - HEADER_H).max(200.0);

    ui.horizontal(|ui| {
        for day in 0..7 {
            let date = (week_start + Duration::days(day))
                .with_timezone(&Local)
                .date_naive();
            day_column(ui, calendar, date, column_w, grid_h, &occs, now);
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn day_column(
    ui: &mut Ui,
    calendar: &mut CalendarUi,
    date: NaiveDate,
    width: f32,
    grid_h: f32,
    occs: &[crate::calendar::Occurrence],
    now: DateTime<Utc>,
) {
    let day_start = day_start_utc(date);
    let day_end = day_start_utc(date + Duration::days(1));
    let is_today = now >= day_start && now < day_end;

    ui.vertical(|ui| {
        ui.set_width(width);
        // Header: weekday + day number, today accented.
        let p = theme::palette(ui);
        let header_text = if is_today {
            RichText::new(date.format("%a %d").to_string())
                .size(ui::fonts::SIZE_SMALL)
                .strong()
                .color(p.accent)
        } else {
            RichText::new(date.format("%a %d").to_string())
                .size(ui::fonts::SIZE_SMALL)
                .color(ui.visuals().weak_text_color())
        };
        ui.add(egui::Label::new(header_text).truncate());
        ui.add_space(2.0);

        let column_rect =
            ui.allocate_exact_size(egui::vec2(width, ALLDAY_BAND_H + grid_h), Sense::click());

        if ui.is_rect_visible(column_rect.0) {
            let painter = ui.painter_at(column_rect.0);
            let bounds = column_rect.0;
            let all_day_rect =
                egui::Rect::from_min_size(bounds.min, egui::vec2(bounds.width(), ALLDAY_BAND_H));
            let hours_rect = egui::Rect::from_min_size(
                bounds.min + egui::vec2(0.0, ALLDAY_BAND_H),
                egui::vec2(bounds.width(), bounds.height() - ALLDAY_BAND_H),
            );

            // Hour grid lines + labels every 3 hours.
            for hour in 0..=24 {
                if hours_rect.height() < 12.0 * 24.0 && hour % 3 != 0 && hour != 24 {
                    continue;
                }
                let y = hours_rect.top() + hours_rect.height() * (hour as f32 / 24.0);
                painter.hline(
                    hours_rect.left()..=hours_rect.right(),
                    y,
                    Stroke::new(
                        0.5,
                        if hour == 24 {
                            p.border
                        } else {
                            p.border.gamma_multiply(0.5)
                        },
                    ),
                );
                if hour % 6 == 0 && hour < 24 {
                    painter.text(
                        Pos2::new(hours_rect.left() + 2.0, y),
                        Align2::LEFT_TOP,
                        format!("{hour:02}:00"),
                        egui::FontId::new(9.0, egui::FontFamily::Proportional),
                        p.muted,
                    );
                }
            }

            // Now-line (today only): accent stroke + dot.
            if is_today && now >= day_start && now < day_end {
                let frac = (now - day_start).num_minutes() as f32 / (24.0 * 60.0);
                let y = hours_rect.top() + hours_rect.height() * frac;
                painter.hline(
                    hours_rect.left()..=hours_rect.right(),
                    y,
                    Stroke::new(1.5, p.accent),
                );
                painter.circle_filled(Pos2::new(hours_rect.left() + 3.0, y), 3.0, p.accent);
            }

            // All-day band: events whose date span covers this day.
            let all_day: Vec<&crate::calendar::Occurrence> = occs
                .iter()
                .filter(|o| {
                    o.all_day
                        && o.start_date.is_some_and(|s| s <= date)
                        && o.end_date.is_some_and(|e| e >= date)
                })
                .collect();
            let mut chip_x = all_day_rect.left() + 3.0;
            for occ in &all_day {
                let remaining = all_day_rect.right() - chip_x;
                if remaining < 30.0 {
                    break;
                }
                let w = (occ.title.len() as f32 * 6.5 + 16.0).min(remaining);
                let chip = egui::Rect::from_min_size(
                    Pos2::new(chip_x, all_day_rect.top() + 3.0),
                    egui::vec2(w, ALLDAY_BAND_H - 6.0),
                );
                paint_occurrence_block(ui, calendar, occ, chip, now, &painter, p);
                chip_x += w + 3.0;
            }

            // Timed blocks: stacked by start time into non-overlapping
            // lanes (two lanes max per column width).
            let block_w = (hours_rect.width() - 6.0) / 2.0;
            let mut lanes: Vec<(f32, f32)> = Vec::new(); // (bottom, x)
            for occ in occs.iter().filter(|o| !o.all_day) {
                let (Some(start), Some(end)) = (occ.start_utc, occ.end_utc) else {
                    continue;
                };
                if end <= day_start || start >= day_end {
                    continue;
                }
                let clipped_start = start.max(day_start);
                let clipped_end = end.min(day_end);
                let top_frac = (clipped_start - day_start).num_minutes() as f32 / (24.0 * 60.0);
                let bot_frac = (clipped_start - day_start).num_minutes() as f32 / (24.0 * 60.0)
                    + ((clipped_end - clipped_start).num_minutes().max(15) as f32 / (24.0 * 60.0));
                let top = (hours_rect.top() + hours_rect.height() * top_frac)
                    .min(hours_rect.bottom() - 14.0);
                let bottom =
                    (hours_rect.top() + hours_rect.height() * bot_frac).min(hours_rect.bottom());
                let height = (bottom - top).max(14.0).min(hours_rect.bottom() - top);

                let x = if let Some(lane) = lanes.iter_mut().find(|(bottom, _)| *bottom <= top) {
                    lane.0 = bottom;
                    lane.1
                } else {
                    let x = hours_rect.left() + 2.0 + lanes.len() as f32 * (block_w + 2.0);
                    lanes.push((bottom, x));
                    x
                };
                let x = x.min(hours_rect.right() - block_w - 1.0);
                let rect =
                    egui::Rect::from_min_size(Pos2::new(x, top), egui::vec2(block_w, height));
                paint_occurrence_block(ui, calendar, occ, rect, now, &painter, p);
            }
        }

        // Click empty column space → new event that day.
        if column_rect.1.clicked() {
            calendar.form = Some(super::event_form::EventForm::new(Some(date), None));
        }
    });
}

/// Paint one occurrence block (all-day chip or timed block) and register a
/// click target via egui's interaction layer on top of the painter work.
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

    // Click target: a transparent interactive widget over the block.
    let occ_id = occ.event_id;
    let occ_clone = occ.clone();
    let response = ui.interact(
        rect,
        ui.id().with(("occ", occ_id, (rect.top() * 10.0) as i32)),
        Sense::click(),
    );
    if response.clicked() {
        let event = calendar.events.iter().find(|e| e.id == occ_id).cloned();
        if let Some(event) = event {
            let start = occ_clone
                .start_utc
                .unwrap_or(week_start_fallback(&occ_clone));
            calendar.open_event(&event, Some((&occ_clone, start)));
        }
    }
    if response.hovered() {
        calendar.hovered.insert(occ_id);
    }
}

/// Fallback instant for identifying an all-day occurrence (its start date
/// at UTC midnight — the same anchor the reminder ledger uses).
fn week_start_fallback(occ: &crate::calendar::Occurrence) -> DateTime<Utc> {
    occ.start_date
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|n| n.and_utc())
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}
