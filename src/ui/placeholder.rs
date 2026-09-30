//! Disabled module nav items for M2+ modules (Email / Calendar / Scratchpad)
//! and their placeholder screens.

use egui::{Response, RichText, Ui};

/// A disabled nav item: muted text, hover tooltip explaining availability.
///
/// Non-interactive on purpose: egui 0.36 labels with a click sense paint via
/// `interact().text_color()` (full foreground), which would swallow the
/// muted color.
pub fn disabled_nav_item(ui: &mut Ui, label: &str) -> Response {
    let text = RichText::new(label).color(ui.visuals().weak_text_color());
    ui.add(egui::Label::new(text))
        .on_hover_text(format!("{label} arrives in a later milestone (M2+)"))
}

/// Centered muted placeholder for a not-yet-built module screen.
pub fn placeholder_screen(ui: &mut Ui, name: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(
            RichText::new(format!("{name} is coming in a later milestone"))
                .color(ui.visuals().weak_text_color()),
        );
    });
}
