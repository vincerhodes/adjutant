//! Mini date picker (M3.6): a month-grid popup anchored under a
//! caller-owned date text field. The toggle is a painter calendar icon;
//! the popup writes `YYYY-MM-DD` into the bound `String` on day click.
//! Day numbers are click-sense Labels — plain hover-sense Labels swallow
//! clicks (the month-view lesson). Palette colors, hand cursors.

use std::str::FromStr;

use chrono::{Datelike, Duration, Local, NaiveDate};
use egui::{Pos2, Rect, RichText, Sense, Ui, Vec2};

use crate::ui::{self, icons, theme};

const CELL_W: f32 = 26.0;
const CELL_H: f32 = 20.0;
const WEEKDAY_H: f32 = 16.0;

#[derive(Clone, Copy, Default)]
struct PickerState {
    open: bool,
    /// First day of the month being viewed.
    view: Option<NaiveDate>,
}

/// Render the toggle button + (when open) the popup. The text field itself
/// stays the caller's; lay this next to it.
pub fn date_picker(ui: &mut Ui, value: &mut String, id_salt: &str) {
    let id = ui.id().with(("date-picker", id_salt));
    let mut state = ui
        .ctx()
        .data_mut(|d| d.get_temp::<PickerState>(id))
        .unwrap_or_default();

    let toggle = ui::icon_button(ui, icons::Icon::Calendar, "Pick date", id_salt);
    if toggle.clicked() {
        state.open = !state.open;
        if state.open && state.view.is_none() {
            let seed =
                NaiveDate::from_str(value.trim()).unwrap_or_else(|_| Local::now().date_naive());
            state.view = Some(first_of_month(seed));
        }
    }
    if state.open && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.open = false;
    }

    if state.open {
        let frame = ui::card_frame(ui, true, false);
        let popup = frame.show(ui, |ui| {
            render_popup(ui, &mut state, value);
        });
        // Click outside (toggle or popup) closes.
        if ui.input(|i| i.pointer.any_click()) {
            let on_popup = ui
                .input(|i| i.pointer.latest_pos())
                .is_some_and(|p| popup.response.rect.contains(p) || toggle.rect.contains(p));
            if !on_popup {
                state.open = false;
            }
        }
    }

    ui.ctx().data_mut(|d| d.insert_temp(id, state));
}

fn render_popup(ui: &mut Ui, state: &mut PickerState, value: &mut String) {
    let view = state
        .view
        .unwrap_or_else(|| first_of_month(Local::now().date_naive()));
    let selected = NaiveDate::from_str(value.trim()).ok();
    let today = Local::now().date_naive();
    let p = theme::palette(ui);
    let weak = ui.visuals().weak_text_color();

    // Month header: ‹  "%B %Y"  ›
    ui.horizontal(|ui| {
        if ui::icon_button(ui, icons::Icon::ChevronLeft, "Previous month", "dp-prev").clicked() {
            state.view = Some(shift_months(view, -1));
        }
        ui.label(
            RichText::new(view.format("%B %Y").to_string())
                .size(ui::fonts::SIZE_SMALL)
                .strong(),
        );
        if ui::icon_button(ui, icons::Icon::ChevronRight, "Next month", "dp-next").clicked() {
            state.view = Some(shift_months(view, 1));
        }
    });

    let grid_w = CELL_W * 7.0;
    let grid_h = WEEKDAY_H + CELL_H * 6.0;
    let (grid_rect, _) = ui.allocate_exact_size(Vec2::new(grid_w, grid_h), Sense::hover());
    for (i, wd) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        .iter()
        .enumerate()
    {
        ui.put(
            Rect::from_min_size(
                Pos2::new(grid_rect.min.x + CELL_W * i as f32, grid_rect.min.y),
                Vec2::new(CELL_W, WEEKDAY_H),
            ),
            egui::Label::new(RichText::new(*wd).small().color(weak)),
        );
    }
    let monday = view - Duration::days(view.weekday().num_days_from_monday() as i64);
    for idx in 0..42 {
        let date = monday + Duration::days(idx);
        let col = idx % 7;
        let row = idx / 7;
        let cell = Rect::from_min_size(
            Pos2::new(
                grid_rect.min.x + CELL_W * col as f32,
                grid_rect.min.y + WEEKDAY_H + CELL_H * row as f32,
            ),
            Vec2::new(CELL_W, CELL_H),
        );
        let in_month = date.month0() == view.month0() && date.year() == view.year();
        let is_selected = selected == Some(date);
        let is_today = date == today;
        if is_selected {
            ui.painter().rect_filled(
                cell.shrink(2.0),
                egui::CornerRadius::same(4),
                p.accent.gamma_multiply(0.18),
            );
        }
        let color = if is_selected || is_today {
            p.accent
        } else if in_month {
            ui.visuals().text_color()
        } else {
            weak
        };
        let day = ui
            .put(
                cell,
                egui::Label::new(
                    RichText::new(date.day().to_string())
                        .size(12.0)
                        .color(color),
                )
                .sense(Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if day.clicked() {
            *value = date.to_string();
            state.open = false;
        }
    }
}

fn first_of_month(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

/// Calendar month arithmetic with day clamping (1st stays 1st here, but
/// keep the clamp for symmetry with the calendar header nav).
fn shift_months(date: NaiveDate, months: i64) -> NaiveDate {
    let total = i64::from(date.year()) * 12 + i64::from(date.month0()) + months;
    let year = total.div_euclid(12).clamp(
        i64::from(NaiveDate::MIN.year()),
        i64::from(NaiveDate::MAX.year()),
    );
    let month0 = total.rem_euclid(12) as u32;
    NaiveDate::from_ymd_opt(
        year as i32,
        month0 + 1,
        date.day().min(days_in_month(year, month0)),
    )
    .unwrap_or(date)
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
