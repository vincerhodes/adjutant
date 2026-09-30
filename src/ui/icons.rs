//! Painter-drawn mini-icons in soft-tinted circular chips — the HEY
//! avatar-circle echo. NOT font glyphs: the M1 tofu lesson is that font
//! coverage is unreliable, so every symbol is drawn with painter primitives
//! (crisp at any zoom, themeable, unit-testable headless).
//!
//! Each icon paints into a square rect (~14px at native size) in the given
//! color; `chip` seats it in a 22px circle tinted with the icon color at
//! 12% alpha. Tooltips (and the AccessKit label) carry the words.

use egui::{
    Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, WidgetInfo, WidgetType,
};

/// The mini-icon vocabulary (spec §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    CircleOutline,
    CircleHalf,
    Check,
    Cross,
    Flag,
    Clock,
    Branch,
    CircleSlash,
}

impl Icon {
    pub const ALL: [Icon; 8] = [
        Icon::CircleOutline,
        Icon::CircleHalf,
        Icon::Check,
        Icon::Cross,
        Icon::Flag,
        Icon::Clock,
        Icon::Branch,
        Icon::CircleSlash,
    ];

    /// Paint the icon centered in `rect` (square) with `color`.
    pub fn paint(self, painter: &Painter, rect: Rect, color: Color32) {
        let c = rect.center();
        let r = rect.width().min(rect.height()) / 2.0;
        let w = (r * 0.28).clamp(1.2, 2.2);
        let stroke = Stroke::new(w, color);
        match self {
            Icon::CircleOutline => {
                painter.circle_stroke(c, r - w / 2.0, stroke);
            }
            Icon::CircleHalf => {
                painter.circle_stroke(c, r - w / 2.0, stroke);
                // Filled right half as a fan of the left semicircle.
                let mut points = vec![c];
                for k in 0..=8 {
                    let a = std::f32::consts::FRAC_PI_2 + (std::f32::consts::PI * k as f32) / 8.0;
                    points.push(c + egui::vec2(a.cos(), a.sin()) * (r - w));
                }
                painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
            }
            Icon::Check => {
                painter.add(Shape::line(
                    vec![
                        pos(c, -0.62, 0.05, r),
                        pos(c, -0.15, 0.55, r),
                        pos(c, 0.7, -0.5, r),
                    ],
                    stroke,
                ));
            }
            Icon::Cross => {
                let d = r * 0.6;
                painter.line_segment([c + egui::vec2(-d, -d), c + egui::vec2(d, d)], stroke);
                painter.line_segment([c + egui::vec2(d, -d), c + egui::vec2(-d, d)], stroke);
            }
            Icon::Flag => {
                painter.line_segment([pos(c, -0.5, -0.75, r), pos(c, -0.5, 0.75, r)], stroke);
                painter.add(Shape::convex_polygon(
                    vec![
                        pos(c, -0.45, -0.7, r),
                        pos(c, 0.65, -0.45, r),
                        pos(c, -0.45, -0.1, r),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
            Icon::Clock => {
                painter.circle_stroke(c, r - w / 2.0, stroke);
                painter.line_segment([c, pos(c, 0.0, -0.5, r)], stroke);
                painter.line_segment([c, pos(c, 0.4, 0.15, r)], stroke);
            }
            Icon::Branch => {
                // Trunk with two branch lines and end dots — sub-task tree.
                painter.line_segment([pos(c, -0.55, -0.7, r), pos(c, -0.55, 0.45, r)], stroke);
                painter.line_segment([pos(c, -0.5, -0.35, r), pos(c, 0.45, -0.6, r)], stroke);
                painter.line_segment([pos(c, -0.5, 0.35, r), pos(c, 0.45, 0.6, r)], stroke);
                let dot = r * 0.22;
                painter.circle_filled(pos(c, 0.55, -0.65, r), dot, color);
                painter.circle_filled(pos(c, 0.55, 0.65, r), dot, color);
            }
            Icon::CircleSlash => {
                painter.circle_stroke(c, r - w / 2.0, stroke);
                let d = r * 0.62;
                painter.line_segment([c + egui::vec2(-d, d), c + egui::vec2(d, -d)], stroke);
            }
        }
    }
}

fn pos(c: Pos2, fx: f32, fy: f32, r: f32) -> Pos2 {
    c + egui::vec2(fx * r, fy * r)
}

/// A 22px soft-tinted circular chip carrying an icon. `label` is the
/// tooltip and the AccessKit name (kittest asserts on these).
pub fn chip(ui: &mut Ui, icon: Icon, icon_color: Color32, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter()
            .circle_filled(rect.center(), 11.0, tint(icon_color, 0.12));
        let icon_rect = Rect::from_center_size(rect.center(), egui::vec2(14.0, 14.0));
        icon.paint(ui.painter(), icon_rect, icon_color);
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, label.to_owned()));
    response.on_hover_text(label)
}

/// Chip followed by small text (due dates, sub-counts) — the text is
/// painter-drawn so the badge keeps a single AccessKit name. Returns the
/// full badge rect for card click-exclusion.
pub fn chip_with_text(
    ui: &mut Ui,
    icon: Icon,
    icon_color: Color32,
    text: &str,
    label: &str,
) -> (Response, egui::Rect) {
    let (chip_rect, response) = ui.allocate_exact_size(
        egui::vec2(22.0 + 4.0 + text_width(ui, text), 22.0),
        Sense::hover(),
    );
    if ui.is_rect_visible(chip_rect) {
        let circle = Rect::from_min_size(chip_rect.min, egui::vec2(22.0, 22.0));
        ui.painter()
            .circle_filled(circle.center(), 11.0, tint(icon_color, 0.12));
        let icon_rect = Rect::from_center_size(circle.center(), egui::vec2(14.0, 14.0));
        icon.paint(ui.painter(), icon_rect, icon_color);
        ui.painter().text(
            egui::pos2(circle.right() + 4.0, circle.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::new(12.0, egui::FontFamily::Proportional),
            icon_color,
        );
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, label.to_owned()));
    let response = response.on_hover_text(label);
    (response, chip_rect)
}

fn text_width(ui: &Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            text.to_owned(),
            egui::FontId::new(12.0, egui::FontFamily::Proportional),
            Color32::WHITE,
        )
        .rect
        .width()
}

fn tint(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}
