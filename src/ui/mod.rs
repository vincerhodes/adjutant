//! Shared widgets implementing the §7a widget language:
//! one primary + one ghost button style, priority dots (no badges),
//! borderless-until-focus inputs, designed empty states.

pub mod fonts;
pub mod help;
pub mod icons;
pub mod placeholder;
pub mod theme;

use egui::{Color32, Response, Rgba, RichText, Stroke, Ui};

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

/// The one primary button style: accent fill, panel-colored text.
/// Explicit paint — the previous visuals-mutation approach let the fill
/// drop out under some themes; painting the rect directly cannot.
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let accent = accent_of(ui);
    let text_color = ui.visuals().panel_fill;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text_width = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
        .rect
        .width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(text_width + 24.0, 26.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if response.is_pointer_button_down_on() {
            egui::lerp(Rgba::from(accent)..=Rgba::from(Color32::BLACK), 0.08).into()
        } else if response.hovered() {
            egui::lerp(Rgba::from(accent)..=Rgba::from(Color32::WHITE), 0.10).into()
        } else {
            accent
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(6), fill);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            font,
            text_color,
        );
    }
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text.to_owned()));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The one ghost button style: no fill, accent text, hover fill.
pub fn ghost_button(ui: &mut Ui, text: &str) -> Response {
    let accent = accent_of(ui);
    ghost_button_with(ui, text, accent)
}

/// Ghost button with an explicit text color (e.g. foreground for persistent
/// affordances like Help, red for destructive actions).
pub fn ghost_button_with(ui: &mut Ui, text: &str, color: Color32) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(color)).frame(false))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Pointing-hand cursor over any clickable — central cursor-consistency
/// rule (arrow elsewhere, I-beam only over editable text).
pub fn hand(response: Response) -> Response {
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// selectable_label + hand cursor (nav items, segmented rows, groups).
pub fn selectable(ui: &mut Ui, active: bool, text: impl Into<egui::WidgetText>) -> Response {
    ui.selectable_label(active, text)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An icon-only button whose LABEL slides in next to the icon on hover
/// (no tooltip). Allocation uses last frame's hover state, so the label
/// appears inline and the layout shifts — instant, by design.
pub fn icon_button(ui: &mut Ui, icon: icons::Icon, label: &str, id_salt: &str) -> Response {
    let id = ui.id().with(("icon_button", id_salt));
    let hovered_last = ui
        .ctx()
        .data_mut(|d| d.get_temp::<bool>(id))
        .unwrap_or(false);
    let label_width = if hovered_last {
        ui.painter()
            .layout_no_wrap(
                label.to_owned(),
                egui::FontId::new(13.0, egui::FontFamily::Proportional),
                Color32::WHITE,
            )
            .rect
            .width()
            + 6.0
    } else {
        0.0
    };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(22.0 + label_width, 22.0), egui::Sense::click());
    let hovered = response.hovered();
    if ui.is_rect_visible(rect) {
        let p = theme::palette(ui);
        let bg = if hovered {
            with_alpha(p.accent, 0.10)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(6), bg);
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 12.0, rect.center().y),
            egui::vec2(14.0, 14.0),
        );
        icon.paint(ui.painter(), icon_rect, p.accent);
        if hovered_last {
            ui.painter().text(
                egui::pos2(icon_rect.right() + 5.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::new(13.0, egui::FontFamily::Proportional),
                p.text,
            );
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, hovered));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{label} (icon button)"),
        )
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
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

/// The shared folded/unfolded card surface (todos, email threads, outbox…):
/// palette fill, 10/12 radius, conditional shadow + hairline border (§8).
pub fn card_frame(ui: &Ui, hovered: bool, unfolded: bool) -> egui::Frame {
    let p = theme::palette(ui);
    let mut frame = egui::Frame::new()
        .fill(if hovered { p.card_hover } else { p.card_fill })
        .corner_radius(egui::CornerRadius::same(if unfolded { 12 } else { 10 }))
        .inner_margin(egui::Margin::symmetric(12, 8));
    if let Some(shadow) = theme::card_shadow(&p, hovered) {
        frame = frame.shadow(shadow);
    }
    if theme::shows_card_border(&p) {
        frame = frame.stroke(Stroke::new(1.0, p.border));
    }
    frame
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

/// Muted hover underline for borderless inputs, so they read as editable
/// before focus (accent underline takes over when focused).
pub fn hovered_underline(ui: &Ui, response: &Response) {
    if response.hovered() && !response.has_focus() {
        let rect = response.rect;
        ui.painter().hline(
            rect.left()..=rect.right(),
            rect.bottom() - 1.0,
            Stroke::new(1.0, ui.visuals().weak_text_color()),
        );
    }
}

/// 18px todo checkbox, fully painter-drawn: the stock egui 0.36 checkbox
/// takes its outline color from widget-state visuals and cannot be styled
/// per-instance. Unchecked = visible muted outline; checked = accent fill
/// with a background-colored tick.
pub fn todo_checkbox(ui: &mut Ui, checked: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let accent = accent_of(ui);
        let box_rect = rect.shrink(1.0);
        let corner = egui::CornerRadius::same(3);
        if checked {
            ui.painter().rect_filled(box_rect, corner, accent);
            let tick = box_rect.shrink(4.0);
            ui.painter().add(egui::Shape::line(
                vec![
                    egui::pos2(tick.left(), tick.center().y),
                    egui::pos2(tick.center().x, tick.bottom()),
                    egui::pos2(tick.right(), tick.top()),
                ],
                Stroke::new(2.0, ui.visuals().window_fill),
            ));
        } else {
            let outline = if response.hovered() {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            ui.painter().rect_stroke(
                box_rect,
                corner,
                Stroke::new(1.5, outline),
                egui::StrokeKind::Inside,
            );
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Painter-drawn collapse triangle for tree rows. The ▸/▾ glyphs are not in
/// the loaded UI fonts (tofu) — drawing the triangle avoids any glyph
/// dependency and stays crisp at any zoom.
pub fn collapse_arrow(ui: &mut Ui, collapsed: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let color = if response.hovered() {
            accent_of(ui)
        } else {
            ui.visuals().weak_text_color()
        };
        let c = rect.center();
        let h = 4.0;
        let points = if collapsed {
            vec![
                egui::pos2(c.x - h * 0.7, c.y - h),
                egui::pos2(c.x - h * 0.7, c.y + h),
                egui::pos2(c.x + h, c.y),
            ]
        } else {
            vec![
                egui::pos2(c.x - h, c.y - h * 0.7),
                egui::pos2(c.x + h, c.y - h * 0.7),
                egui::pos2(c.x, c.y + h),
            ]
        };
        ui.painter()
            .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}
