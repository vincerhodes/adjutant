//! Shared widgets implementing the §7a widget language:
//! one primary + one ghost button style, priority dots (no badges),
//! borderless-until-focus inputs, designed empty states.

pub mod fonts;
pub mod help;
pub mod placeholder;
pub mod theme;

use egui::{Color32, Response, RichText, Stroke, Ui};

use crate::todo::Priority;

/// Designed empty state (§7a principle 5): centered muted text + hint.
pub fn empty_state(ui: &mut Ui, message: &str, hint: &str) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(message).color(ui.visuals().weak_text_color()));
            if !hint.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(hint)
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            }
        });
    });
}

/// The one primary button style: accent fill, background-colored text.
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let accent = accent_of(ui);
    let text_color = ui.visuals().panel_fill;
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        v.widgets.inactive.bg_fill = accent;
        v.widgets.inactive.weak_bg_fill = accent;
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, text_color);
        v.widgets.hovered.bg_fill = accent;
        v.widgets.hovered.weak_bg_fill = accent;
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, text_color);
        v.widgets.active.bg_fill = accent;
        v.widgets.active.weak_bg_fill = accent;
        v.widgets.active.fg_stroke = Stroke::new(1.0, text_color);
        ui.add(egui::Button::new(RichText::new(text).color(text_color)))
    })
    .inner
}

/// The one ghost button style: no fill, accent text, hover fill.
pub fn ghost_button(ui: &mut Ui, text: &str) -> Response {
    let accent = accent_of(ui);
    let response = ui.add(egui::Button::new(RichText::new(text).color(accent)).frame(false));
    response
}

/// A 3px priority color dot (§7a: dot only, no badges).
/// urgent=red, high=orange, normal=muted foreground, low=muted.
pub fn priority_dot(ui: &mut Ui, priority: Priority) -> Response {
    let (color, tooltip) = match priority.value() {
        3 => (Color32::from_rgb(0xf7, 0x76, 0x8e), "Urgent"),
        2 => (Color32::from_rgb(0xe0, 0xaf, 0x68), "High"),
        1 => (ui.visuals().weak_text_color(), "Normal"),
        _ => (ui.visuals().weak_text_color().gamma_multiply(0.6), "Low"),
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(6.0, 6.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().circle_filled(rect.center(), 3.0, color);
    }
    response.on_hover_text(tooltip)
}

/// Accent color of the active theme.
pub fn accent_of(ui: &Ui) -> Color32 {
    ui.visuals().selection.stroke.color
}

/// Stroke an underline under a focused text field (accent, 1px).
pub fn focused_underline(ui: &Ui, response: &Response) {
    if response.has_focus() {
        let rect = response.rect;
        let y = rect.bottom() - 1.0;
        ui.painter().hline(
            rect.left()..=rect.right(),
            y,
            Stroke::new(1.5, accent_of(ui)),
        );
    }
}
