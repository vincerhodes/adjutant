//! F1 shortcut overlay — the only popup besides delete confirmation.

use egui::{Align2, RichText};

use super::fonts;

const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+N", "New todo (child of current selection, if any)"),
    ("Ctrl+Shift+N", "New group"),
    ("Enter", "Edit title / commit edit"),
    (
        "Esc",
        "Cancel edit / clear filter / exit focus mode / close overlay",
    ),
    ("Space", "Toggle done (refused while descendants are open)"),
    ("Ctrl+F", "Filter list"),
    ("Alt+↑ / Alt+↓", "Move todo among siblings"),
    ("↑ / ↓", "Navigate cards"),
    ("→ / ←", "Unfold card / fold card (or first child / parent)"),
    ("Ctrl+.", "Focus mode — full-window list"),
    ("Ctrl+= / Ctrl+- / Ctrl+0", "Zoom in / out / reset"),
    ("F1", "This overlay"),
];

pub fn show(ctx: &egui::Context, open: &mut bool) {
    let mut visible = *open;
    egui::Window::new("Shortcuts")
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .open(&mut visible)
        .show(ctx, |ui| {
            ui.label(
                RichText::new("Keyboard shortcuts")
                    .size(fonts::SIZE_HEADING)
                    .strong(),
            );
            ui.add_space(8.0);
            egui::Grid::new("shortcut-grid")
                .num_columns(2)
                .spacing([24.0, 6.0])
                .show(ui, |ui| {
                    for (keys, what) in SHORTCUTS {
                        ui.label(egui::RichText::new(*keys).strong());
                        ui.label(*what);
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(
                    "Click a card to unfold it — notes, status, due date and actions live inside the card.",
                )
                .small()
                .color(ui.visuals().weak_text_color()),
            );
            ui.label(
                egui::RichText::new(
                    "The \"+ New todo\" button creates a top-level todo. The theme picker is the \"Theme:\" button at the sidebar bottom.",
                )
                .small()
                .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Esc or F1 to close")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        });
    *open = visible;
}
