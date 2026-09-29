//! Disabled module nav items for M2+ modules (Email / Calendar / Scratchpad)
//! and their placeholder screens.

use egui::{Response, RichText, Sense, Ui};

/// A disabled nav item: muted text, hover tooltip explaining availability.
pub fn disabled_nav_item(ui: &mut Ui, label: &str) -> Response {
    let text = RichText::new(label).color(ui.visuals().weak_text_color());
    let response = ui
        .add(egui::Label::new(text).sense(Sense::click()))
        .on_hover_text(format!("{label} arrives in a later milestone (M2+)"));
    // Swallow clicks — the item is inert.
    let _ = response.clicked();
    response
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
