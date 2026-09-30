//! Icon smoke tests: every painter-drawn icon must paint at several sizes
//! on a headless context without panicking (spec §8).

#![allow(clippy::unwrap_used)]

use adjutant::ui::icons::{chip, chip_with_text, Icon};
use egui::{Color32, RawInput};

#[test]
fn icons_paint_at_multiple_sizes() {
    let ctx = egui::Context::default();
    let ink = Color32::from_rgb(0x5B, 0x4B, 0xC4);
    for size in [12.0_f32, 14.0, 22.0, 44.0] {
        let input = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size * 12.0 + 80.0, size * 3.0 + 80.0),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| {
            for icon in Icon::ALL {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
                let icon_rect =
                    egui::Rect::from_center_size(rect.center(), egui::vec2(size * 0.6, size * 0.6));
                icon.paint(ui.painter(), icon_rect, ink);
            }
            // Chips at their native size, including text-bearing variants.
            let _ = chip(ui, Icon::Check, ink, "done");
            let _ = chip(ui, Icon::Flag, ink, "high priority");
            let _ = chip_with_text(ui, Icon::Clock, ink, "Oct 3", "due Oct 3");
            let _ = chip_with_text(ui, Icon::Branch, ink, "2", "2 sub-todos");
        });
        // We don't render — discard the font-texture deltas egui produced.
        let mut output = output;
        output.textures_delta.clear();
    }
}
