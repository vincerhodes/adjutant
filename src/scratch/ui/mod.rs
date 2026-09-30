//! Scratch module UI (spec §6): one screen — Pads list (pinned first,
//! then recency) + Trash toggle. Two-line cards (first line as title,
//! relative updated_at, color dot), hover-reveal Pin/Color/Trash icon
//! buttons, in-place unfold editor with debounced autosave.
//!
//! Zero SQL — all persistence goes through `ScratchStore`.

pub mod pad_card;

use std::collections::HashSet;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use egui::{Context, Key, RichText, Ui};
use uuid::Uuid;

use crate::db::Db;
use crate::scratch::{ScratchPad, ScratchPadInput, ScratchStore};
use crate::ui;

use pad_card::CardAction;

/// Idle time after the last keystroke before an autosave write (spec §5).
pub const AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Pads,
    Trash,
}

/// The single in-flight editor (spec risk 1: strictly single-instance).
/// `id: None` is the Ctrl+N draft — the row is created on first flush.
#[derive(Debug, Clone)]
pub struct Editor {
    id: Option<Uuid>,
    buffer: String,
    save_pending: bool,
    last_edit: Instant,
}

impl Editor {
    fn new(id: Option<Uuid>, buffer: String) -> Editor {
        Editor {
            id,
            buffer,
            save_pending: false,
            last_edit: Instant::now(),
        }
    }
}

pub struct ScratchUi {
    view: View,
    pads: Vec<ScratchPad>,
    trashed: Vec<ScratchPad>,
    editor: Option<Editor>,
    confirm_delete: Option<Uuid>,
    hovered: HashSet<Uuid>,
    dirty: bool,
}

impl ScratchUi {
    pub fn new(db: &Db) -> ScratchUi {
        let mut state = ScratchUi {
            view: View::Pads,
            pads: Vec::new(),
            trashed: Vec::new(),
            editor: None,
            confirm_delete: None,
            hovered: HashSet::new(),
            dirty: true,
        };
        state.reload(db);
        state
    }

    /// True while a modal-ish surface owns Esc (app.rs focus-mode guard).
    pub fn is_busy(&self) -> bool {
        self.editor.is_some() || self.confirm_delete.is_some()
    }

    /// Write the pending editor buffer if one is due (debounce tick or
    /// explicit call). Public as the kittest seam — tests never sleep on
    /// the debounce (spec §5).
    pub fn flush_pending(&mut self, db: &Db) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if !editor.save_pending {
            return;
        }
        editor.save_pending = false;
        let store = ScratchStore::new(db);
        match editor.id {
            Some(id) => {
                if let Err(e) = store.update(id, &editor.buffer) {
                    eprintln!("adjutant: scratch autosave failed: {e:#}");
                }
            }
            None if !editor.buffer.trim().is_empty() => {
                let input = ScratchPadInput {
                    body: editor.buffer.clone(),
                    pinned: false,
                    color_idx: 0,
                };
                match store.create(&input) {
                    Ok(pad) => editor.id = Some(pad.id),
                    Err(e) => eprintln!("adjutant: scratch pad create failed: {e:#}"),
                }
            }
            None => {} // empty draft: nothing to persist
        }
        self.dirty = true;
    }

    /// Fold the editor, flushing first (spec §5 flush point).
    fn fold_editor(&mut self, db: &Db) {
        self.flush_pending(db);
        self.editor = None;
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        ctx: &Context,
        db: &Db,
        toasts: &mut Vec<String>,
        _focus_mode: &mut bool,
    ) {
        self.handle_keys(ctx, db);
        // Debounce tick: write after the idle window (spec §5).
        let due = self
            .editor
            .as_ref()
            .is_some_and(|e| e.save_pending && e.last_edit.elapsed() >= AUTOSAVE_DEBOUNCE);
        if due {
            self.flush_pending(db);
        }
        self.reload(db);

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Scratchpad")
                        .size(ui::fonts::SIZE_GROUP_HEADING)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui::primary_button(ui, "New pad").clicked() {
                        self.open_draft();
                    }
                    ui.add_space(8.0);
                    for view in [View::Pads, View::Trash] {
                        let label = match view {
                            View::Pads => "Pads",
                            View::Trash => "Trash",
                        };
                        if ui::selectable(ui, self.view == view, label).clicked() {
                            self.view = view;
                        }
                    }
                });
            });
            ui.add_space(8.0);

            match self.view {
                View::Pads => self.pads_view(ui, db, toasts),
                View::Trash => self.trash_view(ui, db, toasts),
            }
        });

        self.show_confirm_modal(ctx, db, toasts);
    }

    fn open_draft(&mut self) {
        if self.editor.is_none() {
            self.editor = Some(Editor::new(None, String::new()));
            self.view = View::Pads;
        }
    }

    fn handle_keys(&mut self, ctx: &Context, db: &Db) {
        if self.confirm_delete.is_some() {
            return;
        }
        ctx.input(|i| {
            if i.modifiers.ctrl && i.key_pressed(Key::N) {
                self.open_draft();
            }
        });
        if ctx.input(|i| i.key_pressed(Key::Escape)) && self.editor.is_some() {
            self.fold_editor(db);
        }
    }

    fn pads_view(&mut self, ui: &mut Ui, db: &Db, toasts: &mut Vec<String>) {
        if self.pads.is_empty() && self.editor.is_none() {
            ui::empty_state(
                ui,
                "No pads yet",
                "Press Ctrl+N or the New pad button — typing saves automatically.",
            );
            return;
        }
        let pads = self.pads.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            // Draft card sits at the top while composing.
            if let Some(editor) = self.editor.as_mut() {
                if editor.id.is_none() {
                    pad_card::draft_card(ui, editor);
                    ui.add_space(6.0);
                }
            }
            for pad in &pads {
                let is_editing = self.editor.as_ref().is_some_and(|e| e.id == Some(pad.id));
                let action =
                    pad_card::card(ui, pad, &mut self.hovered, is_editing, self.editor.as_mut());
                match action {
                    CardAction::None => {}
                    CardAction::OpenEditor => {
                        if !is_editing {
                            // Fold any other editor first (flushes it).
                            self.fold_editor(db);
                            self.editor = Some(Editor::new(Some(pad.id), pad.body.clone()));
                        }
                    }
                    CardAction::Pin => {
                        if let Err(e) = ScratchStore::new(db).set_pinned(pad.id, !pad.pinned) {
                            toasts.push(e.to_string());
                        }
                        self.dirty = true;
                    }
                    CardAction::CycleColor => {
                        let next = (pad.color_idx + 1) % 6;
                        if let Err(e) = ScratchStore::new(db).set_color(pad.id, next) {
                            toasts.push(e.to_string());
                        }
                        self.dirty = true;
                    }
                    CardAction::Trash => {
                        if let Err(e) = ScratchStore::new(db).trash(pad.id) {
                            toasts.push(e.to_string());
                        }
                        self.dirty = true;
                    }
                    // Trash-view actions don't reach the Pads view.
                    CardAction::Restore | CardAction::DeleteForever => {}
                }
                ui.add_space(6.0);
            }
        });
    }

    fn trash_view(&mut self, ui: &mut Ui, db: &Db, toasts: &mut Vec<String>) {
        if self.trashed.is_empty() {
            ui::empty_state(ui, "Trash is empty", "Trashed pads land here for 30 days.");
            return;
        }
        let trashed = self.trashed.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for pad in &trashed {
                let action = pad_card::trashed_card(ui, pad, &mut self.hovered);
                match action {
                    CardAction::Restore => {
                        if let Err(e) = ScratchStore::new(db).restore(pad.id) {
                            toasts.push(e.to_string());
                        }
                        self.dirty = true;
                    }
                    CardAction::DeleteForever => {
                        self.confirm_delete = Some(pad.id);
                    }
                    _ => {}
                }
                ui.add_space(6.0);
            }
        });
    }

    fn show_confirm_modal(&mut self, ctx: &Context, db: &Db, toasts: &mut Vec<String>) {
        let Some(id) = self.confirm_delete else {
            return;
        };
        let title = self
            .trashed
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.title().to_string())
            .unwrap_or_else(|| "this pad".to_string());
        let mut open = true;
        egui::Window::new("Delete permanently?")
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("“{title}” will be gone forever, including links."));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui::primary_button(ui, "Delete forever").clicked() {
                        if let Err(e) = ScratchStore::new(db).delete(id) {
                            toasts.push(e.to_string());
                        }
                        self.confirm_delete = None;
                        self.dirty = true;
                    }
                    if ui::ghost_button(ui, "Cancel").clicked() {
                        self.confirm_delete = None;
                    }
                });
            });
        if !open {
            self.confirm_delete = None;
        }
    }

    fn reload(&mut self, db: &Db) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let store = ScratchStore::new(db);
        self.pads = store.list().unwrap_or_default();
        self.trashed = store.list_trashed().unwrap_or_default();
    }
}

/// Relative recency label for card second lines: "just now", "5m ago",
/// "3h ago", "2d ago".
pub fn relative_label(updated_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let mins = (now - updated_at).num_minutes().max(0);
    if mins < 1 {
        "just now".to_string()
    } else if mins < 60 {
        format!("{mins}m ago")
    } else if mins < 60 * 24 {
        format!("{}h ago", mins / 60)
    } else {
        format!("{}d ago", mins / (60 * 24))
    }
}
