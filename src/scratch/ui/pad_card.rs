//! Scratch pad cards: the two-line folded card (first-line title, relative
//! updated_at, color dot), hover-reveal Pin/Color/Trash icon buttons, the
//! unfolded in-place editor, and the trash/restore variants. Rendering
//! only — all persistence is the caller's job (zero SQL).

use std::collections::HashSet;

use egui::{RichText, Sense, Ui};
use uuid::Uuid;

use crate::scratch::ScratchPad;
use crate::ui::{self, icons, theme};

use super::{relative_label, Editor};

/// What a card interaction wants the caller to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardAction {
    None,
    OpenEditor,
    Pin,
    CycleColor,
    Trash,
    Restore,
    DeleteForever,
}

/// A live pad card. `editor` is `Some` only for the one unfolded pad and
/// is mutated in place (buffer + save bookkeeping). `hovered` receives the
/// ids the pointer is over this frame (drives card_hover fill).
pub fn card(
    ui: &mut Ui,
    pad: &ScratchPad,
    hovered: &mut HashSet<Uuid>,
    is_editing: bool,
    editor: Option<&mut Editor>,
) -> CardAction {
    let mut action = CardAction::None;
    let is_hovered = hovered.contains(&pad.id);
    let frame = ui::card_frame(ui, is_hovered, is_editing);
    let inner = frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        if is_editing {
            if let Some(ed) = editor {
                editor_body(ui, ed);
            }
        } else {
            folded_body(ui, pad, false);
        }
    });

    // Icon row: hover-reveal, hidden while this card's editor is open
    // (spec §8.4 — trash is unreachable mid-edit by construction).
    if !is_editing {
        ui.horizontal(|ui| {
            if pad.pinned {
                if ui::icon_button(ui, icons::Icon::Flag, "Unpin pad", "unpin").clicked() {
                    action = CardAction::Pin;
                }
            } else if ui::icon_button(ui, icons::Icon::Flag, "Pin pad", "pin").clicked() {
                action = CardAction::Pin;
            }
            if ui::icon_button(ui, icons::Icon::CircleHalf, "Cycle color", "color").clicked() {
                action = CardAction::CycleColor;
            }
            if ui::icon_button(ui, icons::Icon::Cross, "Trash pad", "trash").clicked() {
                action = CardAction::Trash;
            }
        });
    }

    // Click target over the card body (frames are hover-sense only).
    let response = ui
        .interact(
            inner.response.rect,
            ui.id().with(("scratch-card", pad.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() {
        hovered.insert(pad.id);
    }
    if response.clicked() && !is_editing {
        action = CardAction::OpenEditor;
    }
    action
}

/// Draft card for the Ctrl+N flow: an editor with no row behind it yet.
pub fn draft_card(ui: &mut Ui, editor: &mut Editor) {
    let frame = ui::card_frame(ui, false, true);
    frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        editor_body(ui, editor);
    });
}

/// Trashed pad: dimmed card with Restore / Delete-forever.
pub fn trashed_card(ui: &mut Ui, pad: &ScratchPad, hovered: &mut HashSet<Uuid>) -> CardAction {
    let mut action = CardAction::None;
    let is_hovered = hovered.contains(&pad.id);
    let p = theme::palette(ui);
    let frame = ui::card_frame(ui, is_hovered, false).fill(p.card_fill.gamma_multiply(0.75));
    let inner = frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        folded_body(ui, pad, true);
    });
    ui.horizontal(|ui| {
        if ui::icon_button(ui, icons::Icon::Back, "Restore pad", "restore").clicked() {
            action = CardAction::Restore;
        }
        let danger = theme::palette(ui).danger;
        if ui::ghost_button_with(ui, "Delete forever", danger).clicked() {
            action = CardAction::DeleteForever;
        }
    });
    let response = ui
        .interact(
            inner.response.rect,
            ui.id().with(("scratch-trash-card", pad.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() {
        hovered.insert(pad.id);
    }
    action
}

fn folded_body(ui: &mut Ui, pad: &ScratchPad, dimmed: bool) {
    let now = chrono::Utc::now();
    let color = if dimmed {
        ui.visuals().weak_text_color()
    } else {
        ui.visuals().text_color()
    };
    ui.horizontal(|ui| {
        icons::group_dot(
            ui,
            icons::GROUP_DOT_COLORS[pad.color_idx.clamp(0, 5) as usize],
            8.0,
        );
        if pad.pinned {
            let p = theme::palette(ui);
            icons::chip(ui, icons::Icon::Flag, p.accent, "Pinned");
            ui.add_space(2.0);
        }
        ui.vertical(|ui| {
            ui.label(
                RichText::new(pad.title())
                    .size(ui::fonts::SIZE_CARD_TITLE)
                    .strong()
                    .color(color),
            );
            ui.label(
                RichText::new(relative_label(pad.updated_at, now))
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        });
    });
}

/// The unfolded editor: borderless multiline capture with the
/// borderless-until-focus underline idiom. Mutates the editor's buffer and
/// save bookkeeping; the caller's debounce performs the write.
fn editor_body(ui: &mut Ui, editor: &mut Editor) {
    let mut buffer = editor.buffer.clone();
    let response = ui.add(
        egui::TextEdit::multiline(&mut buffer)
            .frame(egui::Frame::NONE)
            .desired_rows(4)
            .desired_width(f32::INFINITY)
            .hint_text("Type — saves automatically."),
    );
    ui::focused_underline(ui, &response);
    ui::hovered_underline(ui, &response);
    if response.changed() {
        editor.buffer = buffer;
        editor.save_pending = true;
        editor.last_edit = std::time::Instant::now();
    }
    ui.label(
        RichText::new("autosaves after a short pause — Esc to fold")
            .small()
            .color(ui.visuals().weak_text_color()),
    );
}
