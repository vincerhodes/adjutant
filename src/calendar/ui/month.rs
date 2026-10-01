//! Month view (Phase 3): 6×7 grid starting Monday, day numbers as real
//! labels (kittest-visible), up to 3 event chips per cell with "+n more"
//! overflow, today accented, empty-cell click drills into that week.
//! Chips are painter+interact (kittest-invisible) like the week grid.

use chrono::{Datelike, Duration, Local, NaiveDate, Utc};
use egui::{Align2, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};

use crate::db::Db;
use crate::ui::{icons, theme};

use super::{day_start_utc, CalendarUi};
use crate::calendar::rrule;

/// Minimum cell height before the grid wraps in a vertical ScrollArea.
const MIN_CELL_H: f32 = 64.0;
/// Day-of-week header height.
const HEADER_H: f32 = 20.0;

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

pub fn show(calendar: &mut CalendarUi, ui: &mut Ui, db: &Db) {
    let (from_utc, grid_monday, month_first) = super::month_bounds(calendar.week_anchor);
    let to_utc = day_start_utc(grid_monday + Duration::days(42));

    let mut occs: Vec<crate::calendar::Occurrence> = Vec::new();
    for event in &calendar.events {
        match rrule::expand(event, from_utc, to_utc) {
            Ok(mut o) => occs.append(&mut o),
            Err(e) => eprintln!("adjutant: month view: {e}"),
        }
    }

    let avail = ui.available_size();
    let cell_w = (avail.x / 7.0).max(80.0);
    let cell_h = ((avail.y - HEADER_H) / 6.0).max(MIN_CELL_H);
    let geom = MonthGeom {
        cell_w,
        cell_h,
        grid_size: Vec2::new(cell_w * 7.0, HEADER_H + cell_h * 6.0),
    };

    if geom.grid_size.y > avail.y + 0.5 {
        egui::ScrollArea::vertical()
            .id_salt("cal-month-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                render(ui, calendar, &occs, grid_monday, month_first, &geom);
            });
    } else {
        render(ui, calendar, &occs, grid_monday, month_first, &geom);
    }

    // Drill-down clicks are applied here (view switch + persistence need db).
    if let Some(date) = calendar.month_drill.take() {
        calendar.drill_to_week(db, date);
    }
}

struct MonthGeom {
    cell_w: f32,
    cell_h: f32,
    grid_size: Vec2,
}

#[allow(clippy::too_many_arguments)]
fn render(
    ui: &mut Ui,
    calendar: &mut CalendarUi,
    occs: &[crate::calendar::Occurrence],
    grid_monday: NaiveDate,
    month_first: NaiveDate,
    geom: &MonthGeom,
) {
    let (grid_rect, _) = ui.allocate_exact_size(geom.grid_size, Sense::hover());
    if !ui.is_rect_visible(grid_rect) {
        return;
    }
    let p = theme::palette(ui);
    let today = Local::now().date_naive();
    let painter = ui.painter_at(grid_rect);

    // Day-of-week headers (real labels — kittest asserts on these).
    for (i, wd) in WEEKDAYS.iter().enumerate() {
        ui.put(
            Rect::from_min_size(
                Pos2::new(grid_rect.min.x + geom.cell_w * i as f32, grid_rect.min.y),
                Vec2::new(geom.cell_w, HEADER_H),
            ),
            egui::Label::new(
                RichText::new(*wd)
                    .small()
                    .color(ui.visuals().weak_text_color()),
            ),
        );
    }
    painter.hline(
        grid_rect.left()..=grid_rect.right(),
        grid_rect.min.y + HEADER_H - 1.0,
        Stroke::new(0.5, p.border),
    );

    for idx in 0..42 {
        let date = grid_monday + Duration::days(idx);
        let col = idx % 7;
        let row = idx / 7;
        let cell = Rect::from_min_size(
            Pos2::new(
                grid_rect.min.x + geom.cell_w * col as f32,
                grid_rect.min.y + HEADER_H + geom.cell_h * row as f32,
            ),
            Vec2::new(geom.cell_w, geom.cell_h),
        );
        let in_month = date.year() == month_first.year() && date.month0() == month_first.month0();
        let cell_response = ui
            .interact(
                cell,
                ui.id().with(("month-cell", date.to_string())),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);

        // Column separators + today highlight.
        if col > 0 {
            painter.vline(
                cell.left(),
                cell.top()..=cell.bottom(),
                Stroke::new(0.5, p.border),
            );
        }
        if date == today {
            painter.rect_stroke(
                cell.shrink(1.0),
                egui::CornerRadius::ZERO,
                Stroke::new(1.0, p.accent),
                egui::StrokeKind::Inside,
            );
        }
        // Day number — a real, clickable widget: it IS the cell's
        // empty-area click target (kittest asserts on it and can click it).
        let number_color = if date == today {
            p.accent
        } else if in_month {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        let number = ui.put(
            Rect::from_min_size(cell.min + Vec2::new(4.0, 2.0), Vec2::new(28.0, 16.0)),
            egui::Label::new(RichText::new(date.day().to_string()).color(number_color))
                .sense(Sense::click()),
        );
        let number = number.on_hover_cursor(egui::CursorIcon::PointingHand);
        if number.clicked() {
            calendar.month_drill = Some(date);
        }

        // Chips: all-day first (by start date), then timed (by start).
        let mut items: Vec<&crate::calendar::Occurrence> = occs
            .iter()
            .filter(|o| {
                o.all_day
                    && o.start_date.is_some_and(|s| s <= date)
                    && o.end_date.is_some_and(|e| e >= date)
            })
            .collect();
        items.sort_by_key(|o| o.start_date.unwrap_or(date));
        let mut timed: Vec<&crate::calendar::Occurrence> = occs
            .iter()
            .filter(|o| {
                if o.all_day {
                    return false;
                }
                match o.start_utc {
                    Some(start) => {
                        start >= day_start_utc(date)
                            && start < day_start_utc(date + Duration::days(1))
                    }
                    None => false,
                }
            })
            .collect();
        timed.sort_by_key(|o| o.start_utc.unwrap_or(chrono::DateTime::<Utc>::MIN_UTC));
        items.extend(timed);

        let chip_w = geom.cell_w - 8.0;
        let max_chars = ((chip_w - 14.0) / 6.0).max(4.0) as usize;
        for (i, occ) in items.iter().take(3).enumerate() {
            let chip = Rect::from_min_size(
                Pos2::new(cell.min.x + 3.0, cell.min.y + 19.0 + i as f32 * 17.0),
                Vec2::new(chip_w, 15.0),
            );
            paint_chip(ui, calendar, occ, chip, max_chars, date, &painter, p);
        }
        if items.len() > 3 {
            painter.text(
                Pos2::new(cell.min.x + 5.0, cell.min.y + 19.0 + 3.0 * 17.0),
                Align2::LEFT_TOP,
                format!("+{} more", items.len() - 3),
                egui::FontId::new(10.5, egui::FontFamily::Proportional),
                p.muted,
            );
        }

        if cell_response.clicked() {
            calendar.month_drill = Some(date);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_chip(
    ui: &Ui,
    calendar: &mut CalendarUi,
    occ: &crate::calendar::Occurrence,
    rect: Rect,
    max_chars: usize,
    date: NaiveDate,
    painter: &egui::Painter,
    p: theme::Palette,
) {
    let dot = icons::GROUP_DOT_COLORS[(occ.color_idx.clamp(0, 5)) as usize];
    painter.rect_filled(rect, egui::CornerRadius::same(3), dot.gamma_multiply(0.16));
    let title: String = occ.title.chars().take(max_chars).collect();
    let title = if occ.title.chars().count() > max_chars {
        format!("{title}…")
    } else {
        title
    };
    let time_prefix = if occ.all_day {
        String::new()
    } else {
        match occ.start_utc {
            Some(s) => format!("{} ", s.format("%H:%M")),
            None => String::new(),
        }
    };
    painter.circle_filled(Pos2::new(rect.left() + 6.0, rect.center().y), 3.0, dot);
    painter.text(
        Pos2::new(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        format!("{time_prefix}{title}"),
        egui::FontId::new(10.0, egui::FontFamily::Proportional),
        p.text,
    );

    let occ_id = occ.event_id;
    let occ_clone = occ.clone();
    let response = ui
        .interact(
            rect,
            ui.id().with(("month-chip", occ_id, date.to_string())),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        let event = calendar.events.iter().find(|e| e.id == occ_id).cloned();
        if let Some(event) = event {
            let start = occ_clone.start_utc.unwrap_or_else(|| {
                occ_clone
                    .start_date
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|n| n.and_utc())
                    .unwrap_or(chrono::DateTime::<Utc>::MIN_UTC)
            });
            calendar.open_event(&event, Some((&occ_clone, start)));
        }
    }
}
