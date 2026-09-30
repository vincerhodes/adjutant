//! Todo module UI: card-based list (M1.5 redesign).
//!
//! The old right-hand detail pane is gone — every todo is a full-width card;
//! clicking the card body unfolds notes, status/priority rows, due field and
//! actions in place. Children render as nested cards.
//!
//! Zero SQL in this file — all data access goes through `TodoStore` /
//! `LinkStore`. Rendering only; every color comes from the theme palette.

use std::collections::HashSet;

use chrono::{Local, NaiveDate};
use egui::{Align, Color32, Context, Key, Layout, RichText, Sense, Stroke, Ui};
use uuid::Uuid;

use crate::core::entity::{EntityRef, EntityType};
use crate::core::link::{LinkStore, Relation};
use crate::db::Db;
use crate::todo::{Priority, Status, Todo, TodoError, TodoGroup, TodoNode, TodoStore};
use crate::ui::icons::Icon;
use crate::ui::{self, accent_of, fonts, icons, theme};

/// Nested-card indent per level.
const CHILD_INDENT_PX: f32 = 24.0;
/// Visual indent caps at this depth; structure stays unbounded.
const MAX_VISUAL_DEPTH: usize = 8;
/// Gap between cards.
const CARD_GAP_PX: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Tree,
    Trash,
}

/// In-flight editor state. Keyed independently of the tree so background
/// reloads never clobber an edit.
#[derive(Debug, Clone)]
struct Editor {
    target: EditorTarget,
    buffer: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorTarget {
    /// Inline title edit of an existing todo.
    Title(Uuid),
    /// Title of a todo not yet created (group, optional parent).
    NewTodo { group: Uuid, parent: Option<Uuid> },
    /// Name of a group not yet created.
    NewGroup,
    /// Group rename.
    GroupRename(Uuid),
}

#[derive(Debug, Clone)]
enum Confirm {
    Trash {
        id: Uuid,
        title: String,
        subtree: usize,
    },
    Purge {
        id: Uuid,
        title: String,
        subtree: usize,
    },
}

pub struct TodoUi {
    view: View,
    groups: Vec<TodoGroup>,
    current_group: Option<Uuid>,
    tree: Vec<TodoNode>,
    trash: Vec<Todo>,
    trash_count: usize,
    selected: Option<Uuid>,
    /// Cards expanded in place (default folded).
    unfolded: HashSet<Uuid>,
    /// Cards the pointer hovered last frame (drives card_hover fill).
    hovered_cards: HashSet<Uuid>,
    /// Todos targeted by a live `blocks` link (loaded once per frame).
    blocked: HashSet<Uuid>,
    /// Card whose "Blocked by…" picker is open.
    picker_for: Option<Uuid>,
    /// Flattened list of visible todo ids in display order (arrow-key nav).
    visible_order: Vec<Uuid>,
    filter: String,
    filter_active: bool,
    editor: Option<Editor>,
    confirm: Option<Confirm>,
    blocked_search: String,
    due_buffer: String,
    due_for: Option<Uuid>,
    notes_buffer: Option<(Uuid, String)>,
    dirty: bool,
    inline_error: Option<String>,
}

impl TodoUi {
    pub fn new(db: &Db) -> TodoUi {
        let mut state = TodoUi {
            view: View::Tree,
            groups: Vec::new(),
            current_group: None,
            tree: Vec::new(),
            trash: Vec::new(),
            trash_count: 0,
            selected: None,
            unfolded: HashSet::new(),
            hovered_cards: HashSet::new(),
            blocked: HashSet::new(),
            picker_for: None,
            visible_order: Vec::new(),
            filter: String::new(),
            filter_active: false,
            editor: None,
            confirm: None,
            blocked_search: String::new(),
            due_buffer: String::new(),
            due_for: None,
            notes_buffer: None,
            dirty: true,
            inline_error: None,
        };
        state.reload(db, None);
        state
    }

    pub fn current_group(&self) -> Option<Uuid> {
        self.current_group
    }

    pub fn set_current_group(&mut self, db: &Db, group: Option<Uuid>) {
        if group != self.current_group {
            self.current_group = group;
            self.selected = None;
            self.editor = None;
            self.dirty = true;
            self.reload(db, None);
        }
    }

    pub fn trash_count(&self) -> usize {
        self.trash_count
    }

    /// True when an editor, the filter, or the delete-confirm modal is
    /// active — chrome-level Esc handling (e.g. exiting focus mode) must
    /// yield to these.
    pub fn is_editing(&self) -> bool {
        self.editor.is_some() || self.filter_active || self.confirm.is_some()
    }

    /// Sidebar content: group list + Trash entry. Rendered inside the app's
    /// left panel.
    pub fn sidebar(&mut self, ui: &mut Ui, db: &Db) {
        ui.add_space(8.0);
        // "Groups" label with the new-group button immediately beside it.
        ui.horizontal(|ui| {
            ui.label(RichText::new("Groups").size(fonts::SIZE_SMALL).strong());
            if ui::ghost_button(ui, "+")
                .on_hover_text("New group (Ctrl+Shift+N)")
                .clicked()
            {
                self.editor = Some(Editor {
                    target: EditorTarget::NewGroup,
                    buffer: String::new(),
                });
            }
        });
        ui.add_space(4.0);

        if self.groups.is_empty() && self.editor_new_group_active().is_none() {
            ui.label(
                RichText::new("No groups yet")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        }

        let mut activate: Option<(Uuid, View)> = None;
        for group in self.groups.clone() {
            let selected = self.view == View::Tree && self.current_group == Some(group.id);
            let response = ui.selectable_label(selected, group.name.clone());
            if selected {
                // Same selection language as todo rows: fill + 3px accent bar.
                let strip = response.rect.expand2(egui::vec2(10.0, 3.0));
                ui.painter().rect_filled(
                    strip,
                    egui::CornerRadius::same(3),
                    with_alpha(accent_of(ui), 0.18),
                );
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(strip.min, egui::vec2(3.0, strip.height())),
                    0.0,
                    accent_of(ui),
                );
            }
            if response.clicked() {
                activate = Some((group.id, View::Tree));
            }
            // Rename via double click.
            if response.double_clicked() {
                self.editor = Some(Editor {
                    target: EditorTarget::GroupRename(group.id),
                    buffer: group.name.clone(),
                });
            }
        }
        if let Some((group, view)) = activate {
            self.view = view;
            self.editor = None; // editors reference todos of the old group
            self.set_current_group(db, Some(group));
        }

        // Inline new-group editor (Ctrl+Shift+N).
        if let Some(Editor { buffer, .. }) = self.editor_new_group_active() {
            let mut buffer = buffer.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut buffer)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .hint_text("Group name — Enter to create"),
            );
            ui::focused_underline(ui, &response);
            let mut done = false;
            if response.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter)) {
                done = true;
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                self.editor = None;
            } else if done {
                self.commit_group(db, buffer);
            } else if let Some(e) = &mut self.editor {
                e.buffer = buffer;
            }
        } else if ui.input(|i| i.modifiers.ctrl && i.key_pressed(Key::N) && i.modifiers.shift) {
            self.editor = Some(Editor {
                target: EditorTarget::NewGroup,
                buffer: String::new(),
            });
        }

        ui.add_space(12.0);
        ui.separator();
        let trash_label = format!("Trash ({})", self.trash_count);
        if ui
            .selectable_label(self.view == View::Trash, trash_label)
            .clicked()
        {
            self.view = View::Trash;
            self.editor = None; // tree editors don't render over the trash list
            self.dirty = true;
        }
    }

    fn editor_new_group_active(&self) -> Option<&Editor> {
        self.editor.as_ref().filter(|e| {
            matches!(
                e.target,
                EditorTarget::NewGroup | EditorTarget::GroupRename(_)
            )
        })
    }

    fn commit_group(&mut self, db: &Db, name: String) {
        let name = name.trim();
        if name.is_empty() {
            self.editor = None;
            return;
        }
        let store = TodoStore::new(db);
        match self.editor.take() {
            Some(Editor {
                target: EditorTarget::GroupRename(id),
                ..
            }) => {
                if let Err(e) = store.rename_group(id, name) {
                    self.inline_error = Some(e.to_string());
                }
            }
            _ => match store.create_group(name) {
                Ok(g) => {
                    self.current_group = Some(g.id);
                    self.view = View::Tree;
                }
                Err(e) => self.inline_error = Some(e.to_string()),
            },
        }
        self.dirty = true;
        self.reload(db, None);
    }

    /// Reload store data if dirty. `preserve` keeps in-flight editor state
    /// (the default); pass an editor only when re-seeding buffers.
    pub fn reload(&mut self, db: &Db, _preserve: Option<()>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let store = TodoStore::new(db);
        self.groups = store.list_groups().unwrap_or_default();
        if self.current_group.is_none() {
            self.current_group = self.groups.first().map(|g| g.id);
        }
        if let Some(g) = self.current_group {
            self.tree = store.tree(g).unwrap_or_default();
        } else {
            self.tree = Vec::new();
        }
        self.trash = store.list_trash().unwrap_or_default();
        self.trash_count = self.trash.len();
        // Selection may have vanished (e.g. trashed elsewhere).
        if let Some(sel) = self.selected {
            if !self.tree_contains(sel) {
                self.selected = None;
            }
        }
    }

    fn tree_contains(&self, id: Uuid) -> bool {
        self.find_node(id).is_some()
    }

    fn find_node(&self, id: Uuid) -> Option<&TodoNode> {
        fn walk(nodes: &[TodoNode], id: Uuid) -> Option<&TodoNode> {
            for n in nodes {
                if n.todo.id == id {
                    return Some(n);
                }
                if let Some(hit) = walk(&n.children, id) {
                    return Some(hit);
                }
            }
            None
        }
        walk(&self.tree, id)
    }

    /// Parent of `id` within the current tree, if any.
    fn parent_of(&self, id: Uuid) -> Option<Uuid> {
        fn walk(nodes: &[TodoNode], id: Uuid) -> Option<Uuid> {
            for n in nodes {
                if n.children.iter().any(|c| c.todo.id == id) {
                    return Some(n.todo.id);
                }
                if let Some(hit) = walk(&n.children, id) {
                    return Some(hit);
                }
            }
            None
        }
        walk(&self.tree, id)
    }

    /// Ensure every ancestor of `id` is unfolded so the card is reachable.
    fn unfold_ancestors_of(&mut self, id: Uuid) {
        fn find_path(nodes: &[TodoNode], id: Uuid, path: &mut Vec<Uuid>) -> bool {
            for n in nodes {
                if n.todo.id == id {
                    return true;
                }
                path.push(n.todo.id);
                if find_path(&n.children, id, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut path = Vec::new();
        if find_path(&self.tree, id, &mut path) {
            for ancestor in path {
                self.unfolded.insert(ancestor);
            }
        }
    }

    /// Full todo UI: keyboard shortcuts, card list, modal. Takes the root
    /// `ui` plus the ctx for shortcut input and the modal window.
    /// `focus_mode` is set when the header's Focus button is clicked.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        ctx: &Context,
        db: &Db,
        toasts: &mut Vec<String>,
        focus_mode: &mut bool,
    ) {
        self.handle_keys(ctx, db, toasts);
        self.reload(db, None);
        self.blocked = LinkStore::new(db).blocked_todo_ids().unwrap_or_default();

        egui::CentralPanel::default().show(ui, |ui| match self.view {
            View::Tree => self.tree_view(ui, db, toasts, focus_mode),
            View::Trash => self.trash_view(ui, db, toasts),
        });

        self.show_confirm_modal(ctx, db, toasts);
    }

    // ── keyboard ──────────────────────────────────────────────────────────

    fn handle_keys(&mut self, ctx: &Context, db: &Db, toasts: &mut Vec<String>) {
        if self.editor.is_some() {
            // Escape must always cancel, even when the editor's widget isn't
            // rendered this frame — an invisible editor would otherwise wedge
            // every key until restart.
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.editor = None;
            }
            return;
        }
        ctx.input(|i| {
            if i.modifiers.ctrl && i.key_pressed(Key::F) {
                self.filter_active = true;
            }
            if i.key_pressed(Key::Escape) && self.filter_active {
                self.filter_active = false;
                self.filter = String::new();
            }
            if i.key_pressed(Key::F1) {
                // Handled by the app; ignored here.
            }
            if i.modifiers.ctrl && i.key_pressed(Key::N) && !i.modifiers.shift {
                // Only meaningful in the tree view; the editor widget
                // doesn't render over the trash list.
                if self.view == View::Tree {
                    self.start_new_todo(toasts);
                }
            }
            if i.key_pressed(Key::Space) {
                if let Some(id) = self.selected {
                    if let Err(e) = self.toggle_done(db, id) {
                        toasts.push(e);
                    }
                }
            }
            if i.modifiers.alt && i.key_pressed(Key::ArrowUp) {
                if let Some(id) = self.selected {
                    if let Err(e) = TodoStore::new(db).reorder(id, true) {
                        toasts.push(e.to_string());
                    } else {
                        self.dirty = true;
                    }
                }
            }
            if i.modifiers.alt && i.key_pressed(Key::ArrowDown) {
                if let Some(id) = self.selected {
                    if let Err(e) = TodoStore::new(db).reorder(id, false) {
                        toasts.push(e.to_string());
                    } else {
                        self.dirty = true;
                    }
                }
            }
            if i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::ArrowUp) {
                let down = i.key_pressed(Key::ArrowDown);
                self.move_selection(down);
            }
            // ←/→ fold/unfold (standard tree-view convention). Skipped while
            // the filter field is active — its text editing owns the keys.
            if !self.filter_active && i.key_pressed(Key::ArrowRight) {
                if let Some(id) = self.selected {
                    if !self.unfolded.contains(&id) {
                        self.unfolded.insert(id);
                    } else {
                        let first_child = self
                            .find_node(id)
                            .and_then(|n| n.children.first().map(|c| c.todo.id));
                        if let Some(child) = first_child {
                            self.selected = Some(child);
                        }
                    }
                }
            }
            if !self.filter_active && i.key_pressed(Key::ArrowLeft) {
                if let Some(id) = self.selected {
                    if self.unfolded.contains(&id) {
                        self.unfolded.remove(&id);
                    } else if let Some(parent) = self.parent_of(id) {
                        self.selected = Some(parent);
                    }
                }
            }
            if i.key_pressed(Key::Enter) {
                if let Some(id) = self.selected {
                    if let Ok(todo) = TodoStore::new(db).get(id) {
                        self.unfold_ancestors_of(id);
                        self.editor = Some(Editor {
                            target: EditorTarget::Title(id),
                            buffer: todo.title,
                        });
                    }
                }
            }
        });
    }

    /// Ctrl+N: open the new-todo editor nested under the current selection,
    /// if any (documented in the F1 overlay).
    fn start_new_todo(&mut self, toasts: &mut Vec<String>) {
        let parent = self.selected;
        self.open_new_todo_editor(parent, toasts);
    }

    /// Open the new-todo editor in the current group. `parent = None`
    /// creates a top-level todo (header button); `Some(id)` nests.
    fn open_new_todo_editor(&mut self, parent: Option<Uuid>, toasts: &mut Vec<String>) {
        if let Some(group) = self.current_group {
            self.editor = Some(Editor {
                target: EditorTarget::NewTodo { group, parent },
                buffer: String::new(),
            });
            if let Some(p) = parent {
                self.unfold_ancestors_of(p);
            }
        } else {
            toasts.push("Create a group first (Ctrl+Shift+N)".to_string());
        }
    }

    fn move_selection(&mut self, down: bool) {
        if self.visible_order.is_empty() {
            return;
        }
        let current = self
            .selected
            .and_then(|id| self.visible_order.iter().position(|v| *v == id));
        let next = match (current, down) {
            (None, true) => 0,
            (None, false) => self.visible_order.len().saturating_sub(1),
            (Some(idx), true) => (idx + 1).min(self.visible_order.len() - 1),
            (Some(idx), false) => idx.saturating_sub(1),
        };
        self.selected = Some(self.visible_order[next]);
    }

    fn toggle_done(&mut self, db: &Db, id: Uuid) -> Result<(), String> {
        let store = TodoStore::new(db);
        let todo = store.get(id).map_err(|e| e.to_string())?;
        let new_status = if todo.status == Status::Done {
            Status::Open
        } else {
            Status::Done
        };
        match store.set_status(id, new_status) {
            Ok(_) => {
                self.dirty = true;
                Ok(())
            }
            Err(e) => {
                self.dirty = true;
                Err(e.to_string())
            }
        }
    }

    // ── tree view (center) ────────────────────────────────────────────────

    fn tree_view(&mut self, ui: &mut Ui, db: &Db, toasts: &mut Vec<String>, focus_mode: &mut bool) {
        let Some(group) = self.current_group else {
            ui::empty_state(
                ui,
                "No group selected",
                "Ctrl+Shift+N to create your first group",
            );
            return;
        };

        // List header: group name with the actions immediately beside it
        // (ghost buttons — not right-aligned across the pane).
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let name = self
                .groups
                .iter()
                .find(|g| g.id == group)
                .map(|g| g.name.clone())
                .unwrap_or_default();
            ui.label(RichText::new(name).size(fonts::SIZE_HEADING).strong());
            if ui::ghost_button(ui, "+ New todo")
                .on_hover_text("New top-level todo (Ctrl+N nests under the selection)")
                .clicked()
            {
                // Deliberately top-level: child-of-selection from a button
                // is undiscoverable. Ctrl+N keeps the nesting behavior.
                self.open_new_todo_editor(None, toasts);
            }
            if ui::ghost_button(ui, "Filter…")
                .on_hover_text("Filter todos (Ctrl+F)")
                .clicked()
            {
                self.filter_active = true;
            }
            if self.filter_active
                && ui::ghost_button(ui, "✕")
                    .on_hover_text("Clear filter (Esc)")
                    .clicked()
            {
                self.filter_active = false;
                self.filter.clear();
            }
            if ui::ghost_button(ui, "Focus")
                .on_hover_text("Focus mode — full-window list (Ctrl+.)")
                .clicked()
            {
                *focus_mode = true;
            }
        });
        ui.add_space(4.0);

        // Filter field (Ctrl+F).
        if self.filter_active {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let mut filter = self.filter.clone();
                let response = ui.add(
                    egui::TextEdit::singleline(&mut filter)
                        .frame(egui::Frame::NONE)
                        .desired_width(240.0)
                        .hint_text("Filter todos… (Esc to clear)"),
                );
                ui::focused_underline(ui, &response);
                if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    self.filter_active = false;
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.filter_active = false;
                    filter.clear();
                }
                self.filter = filter;
                let matches = count_matches(&self.tree, &self.filter);
                if !self.filter.is_empty() {
                    ui.label(
                        RichText::new(format!("{matches} matching"))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                }
            });
        }
        ui.add_space(8.0);

        if self.tree.is_empty() {
            // The new-todo editor must render even with an empty tree — its
            // Esc/Enter handling lives in the widget, and an unrendered
            // editor wedges every key.
            self.new_todo_row(ui, db, toasts, None);
            if self.editor.is_none() {
                ui::empty_state(ui, "Nothing here yet", "Ctrl+N to create your first todo");
            }
            return;
        }

        self.visible_order.clear();
        let nodes = self.tree.clone();
        // Suspend filtering while an editor is active: the edited card
        // would otherwise vanish from the list the moment its buffer stops
        // matching, stranding the editor.
        let filter = if self.editor.is_some() {
            String::new()
        } else {
            self.filter.clone()
        };
        let scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
        scroll.show(ui, |ui| {
            for node in &nodes {
                self.card(ui, node, 0, &filter, db, toasts);
            }
            // Top-level "new todo" editor at the end of the list.
            self.new_todo_row(ui, db, toasts, None);
        });
    }

    // ── cards ─────────────────────────────────────────────────────────────

    fn card_frame(&self, ui: &Ui, hovered: bool, unfolded: bool) -> egui::Frame {
        let p = theme::palette(ui);
        let mut frame = egui::Frame::new()
            .fill(if hovered { p.card_hover } else { p.card_fill })
            .corner_radius(egui::CornerRadius::same(if unfolded { 12 } else { 10 }))
            .inner_margin(egui::Margin::symmetric(16, 10));
        if let Some(shadow) = theme::card_shadow(&p, hovered) {
            frame = frame.shadow(shadow);
        }
        if theme::shows_card_border(&p) {
            frame = frame.stroke(Stroke::new(1.0, p.border));
        }
        frame
    }

    fn card(
        &mut self,
        ui: &mut Ui,
        node: &TodoNode,
        depth: usize,
        filter: &str,
        db: &Db,
        toasts: &mut Vec<String>,
    ) {
        let p = theme::palette(ui);
        let todo = &node.todo;
        let id = todo.id;

        // Filtering: keep matches and ancestors of matches.
        let child_match = !filter.is_empty() && contains_match(node, filter);
        if !filter.is_empty()
            && !todo.title.to_lowercase().contains(&filter.to_lowercase())
            && !child_match
        {
            return;
        }

        self.visible_order.push(id);
        let selected = self.selected == Some(id);
        let unfolded = self.unfolded.contains(&id);
        let hovered = self.hovered_cards.contains(&id);

        ui.add_space(CARD_GAP_PX);
        // Register the card's click target BEFORE painting content, using
        // last frame's rect: egui routes clicks to the topmost widget, so
        // inner widgets (checkbox, title, buttons) must register after the
        // card and win. The card only sees clicks on non-interactive areas.
        let card_id = ui.id().with(("todo_card", id));
        let prev_rect = ui.ctx().data_mut(|d| d.get_temp::<egui::Rect>(card_id));
        let card_response = prev_rect.map(|rect| ui.interact(rect, card_id, egui::Sense::click()));

        let frame = self.card_frame(ui, hovered, unfolded);
        let frame_response = frame.show(ui, |ui| {
            if !unfolded {
                ui.set_min_height(56.0 - 20.0);
            }
            let mut child_clicked = false;
            let mut badge_rects: Vec<egui::Rect> = Vec::new();
            ui.horizontal(|ui| {
                // Checkbox (custom-painted: visible outline, accent when checked).
                let checked = todo.status == Status::Done;
                if ui::todo_checkbox(ui, checked).clicked() {
                    match self.toggle_done(db, id) {
                        Ok(()) => {}
                        Err(e) => toasts.push(e),
                    }
                    child_clicked = true;
                }

                // Fold/unfold arrow for parents.
                if !node.children.is_empty() {
                    if ui::collapse_arrow(ui, !unfolded).clicked() {
                        if unfolded {
                            self.unfolded.remove(&id);
                        } else {
                            self.unfolded.insert(id);
                        }
                        child_clicked = true;
                    }
                } else {
                    ui.add_space(14.0);
                }

                // Title / inline editor. Title click selects (single); the
                // card body click (elsewhere) unfolds. Double-click edits.
                let editing_title = matches!(
                    self.editor,
                    Some(Editor { target: EditorTarget::Title(t), .. }) if t == id
                );
                if editing_title {
                    self.title_editor(ui, id, db, toasts);
                    child_clicked = true;
                } else {
                    let terminal = todo.status.is_terminal();
                    let mut text = RichText::new(&todo.title);
                    if terminal {
                        text = text.color(ui.visuals().weak_text_color()).strikethrough();
                    }
                    let response = ui.add(egui::Label::new(text).sense(Sense::click()));
                    if response.clicked() {
                        self.selected = Some(id);
                        child_clicked = true;
                    }
                    if response.double_clicked() {
                        self.editor = Some(Editor {
                            target: EditorTarget::Title(id),
                            buffer: todo.title.clone(),
                        });
                        child_clicked = true;
                    }
                }

                // Symbol badges: icon chips, right-aligned (spring via
                // right-to-left layout). Words live in tooltips/AccessKit.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // Blocked: circle-slash (danger).
                    if self.blocked.contains(&id) {
                        badge_rects
                            .push(icons::chip(ui, Icon::CircleSlash, p.danger, "blocked").rect);
                    }
                    // Sub-count: branch icon + open descendant count.
                    if !node.children.is_empty() {
                        let open = open_descendants_excluding_self(node);
                        let (_, rect) = icons::chip_with_text(
                            ui,
                            Icon::Branch,
                            p.muted,
                            &open.to_string(),
                            &format!("{open} sub-todos"),
                        );
                        badge_rects.push(rect);
                    }
                    // Due: clock + short date; hidden for terminal todos.
                    // Danger when overdue, gold (attention) when due today.
                    if let Some(due) = todo.due_date {
                        if !todo.status.is_terminal() {
                            let today = Local::now().date_naive();
                            let (icon_color, date) = if due < today {
                                (p.danger, due.format("%b %-d").to_string())
                            } else if due == today {
                                (p.gold, due.format("%b %-d").to_string())
                            } else {
                                (p.muted, due.format("%b %-d").to_string())
                            };
                            let (_, rect) = icons::chip_with_text(
                                ui,
                                Icon::Clock,
                                icon_color,
                                &date,
                                &format!("due {date}"),
                            );
                            badge_rects.push(rect);
                        }
                    }
                    // Priority: flag only for High/Urgent.
                    if todo.priority.value() >= 2 {
                        let (icon_color, label) = if todo.priority.value() == 3 {
                            (p.danger, "urgent priority")
                        } else {
                            (p.warn, "high priority")
                        };
                        badge_rects.push(icons::chip(ui, Icon::Flag, icon_color, label).rect);
                    }
                    // Status: icon only when NOT open (calm default).
                    // Done sits on the mint positive-action pill (§8).
                    match todo.status {
                        Status::Open => {}
                        Status::InProgress => {
                            badge_rects.push(
                                icons::chip(ui, Icon::CircleHalf, p.warn, "in progress").rect,
                            );
                        }
                        Status::Done => {
                            badge_rects.push(icons::chip(ui, Icon::Check, p.success, "done").rect);
                        }
                        Status::Cancelled => {
                            badge_rects
                                .push(icons::chip(ui, Icon::Cross, p.muted, "cancelled").rect);
                        }
                    }
                });
            });

            if unfolded {
                ui.add_space(8.0);
                self.expanded_card(ui, node, db, toasts, &mut child_clicked);
            }
            (child_clicked, badge_rects)
        });
        let (child_clicked, badge_rects) = frame_response.inner;

        // Card-body click (not checkbox/badges/title or any inner widget)
        // toggles unfold. Hover is pointer-position based (no z-order).
        let rect = frame_response.response.rect;
        ui.ctx().data_mut(|d| d.insert_temp(card_id, rect));
        if let Some(card_response) = card_response {
            card_response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, todo.title.clone())
            });
            let hovered_now = ui
                .input(|i| i.pointer.latest_pos())
                .is_some_and(|pos| rect.contains(pos));
            if hovered_now {
                self.hovered_cards.insert(id);
            } else {
                self.hovered_cards.remove(&id);
            }
            if card_response.clicked() && !child_clicked {
                let pointer = ui.input(|i| i.pointer.latest_pos());
                let on_badge = badge_rects
                    .iter()
                    .any(|r| pointer.is_some_and(|pos| r.contains(pos)));
                if !on_badge {
                    if unfolded {
                        self.unfolded.remove(&id);
                    } else {
                        self.unfolded.insert(id);
                    }
                    self.selected = Some(id);
                }
            }
        }

        // Selected (keyboard nav): 2px accent outline.
        if selected {
            ui.painter().rect_stroke(
                rect,
                egui::CornerRadius::same(6),
                Stroke::new(2.0, accent_of(ui)),
                egui::StrokeKind::Outside,
            );
        }

        // Children as nested cards (24px indent), when unfolded — or when a
        // filter/active editor needs them visible.
        let editor_here = matches!(
            self.editor,
            Some(Editor {
                target: EditorTarget::NewTodo { parent: Some(p), .. },
                ..
            }) if p == id
        );
        let show_children = (!node.children.is_empty() && (unfolded || child_match)) || editor_here;
        if show_children {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if depth < MAX_VISUAL_DEPTH {
                    ui.add_space(CHILD_INDENT_PX);
                }
                ui.vertical(|ui| {
                    for child in &node.children {
                        self.card(ui, child, depth + 1, filter, db, toasts);
                    }
                    if editor_here {
                        self.new_todo_row(ui, db, toasts, Some(id));
                    }
                });
            });
        }
    }

    /// Expanded card section: notes, status/priority rows, due field,
    /// actions — everything the old detail pane used to hold.
    fn expanded_card(
        &mut self,
        ui: &mut Ui,
        node: &TodoNode,
        db: &Db,
        toasts: &mut Vec<String>,
        inner_clicked: &mut bool,
    ) {
        let todo = &node.todo;

        // Notes (multiline: blur or Ctrl+S saves, no save button).
        ui.label(
            RichText::new("Notes")
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        let notes = match &self.notes_buffer {
            Some((buf_id, buf)) if *buf_id == todo.id => buf.clone(),
            _ => todo.notes.clone(),
        };
        let mut notes_edit = notes.clone();
        let response = ui.add(
            egui::TextEdit::multiline(&mut notes_edit)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .desired_rows(4)
                .hint_text("Notes… (saved on blur or Ctrl+S)"),
        );
        ui::focused_underline(ui, &response);
        if response.clicked() || response.changed() {
            // Clicking into the notes editor must not toggle the card fold.
            *inner_clicked = true;
        }
        let save = response.lost_focus()
            || (response.has_focus() && ui.input(|i| i.modifiers.ctrl && i.key_pressed(Key::S)));
        if save {
            if notes_edit != todo.notes {
                if let Err(e) =
                    TodoStore::new(db).update(todo.id, None, Some(&notes_edit), None, None)
                {
                    toasts.push(e.to_string());
                } else {
                    self.dirty = true;
                }
            }
            self.notes_buffer = None;
        } else {
            self.notes_buffer = Some((todo.id, notes_edit));
        }
        ui.add_space(8.0);

        // Status segmented row.
        ui.horizontal_wrapped(|ui| {
            for status in [
                Status::Open,
                Status::InProgress,
                Status::Done,
                Status::Cancelled,
            ] {
                let color = status_color(ui, status);
                let text = RichText::new(status.label()).color(color);
                let response = ui.selectable_label(todo.status == status, text);
                if response.clicked() {
                    *inner_clicked = true;
                    match TodoStore::new(db).set_status(todo.id, status) {
                        Ok(_) => self.dirty = true,
                        Err(TodoError::OpenDescendants(n)) => {
                            toasts.push(format!("Cannot complete: {n} open descendant(s) remain"));
                        }
                        Err(e) => toasts.push(e.to_string()),
                    }
                }
            }
        });
        ui.add_space(6.0);

        // Priority segmented row.
        ui.horizontal_wrapped(|ui| {
            for value in 0..=3u8 {
                let pr = Priority::new(value).unwrap_or(Priority::NORMAL);
                let color = priority_text_color(ui, pr);
                let text = RichText::new(pr.label()).color(color);
                let response = ui
                    .selectable_label(todo.priority == pr, text)
                    .on_hover_text(pr.label());
                if response.clicked() {
                    *inner_clicked = true;
                    if let Err(e) = TodoStore::new(db).update(todo.id, None, None, Some(pr), None) {
                        toasts.push(e.to_string());
                    } else {
                        self.dirty = true;
                    }
                }
            }
        });
        ui.add_space(8.0);

        // Due date (validated text, no calendar widget).
        ui.horizontal(|ui| {
            ui.label(RichText::new("Due").small());
            if self.due_for != Some(todo.id) {
                self.due_buffer = todo.due_date.map(|d| d.to_string()).unwrap_or_default();
                self.due_for = Some(todo.id);
            }
            let mut buffer = self.due_buffer.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut buffer)
                    .frame(egui::Frame::NONE)
                    .desired_width(110.0)
                    .hint_text("YYYY-MM-DD"),
            );
            ui::focused_underline(ui, &response);
            ui::hovered_underline(ui, &response);
            if response.clicked() || response.changed() {
                *inner_clicked = true;
            }
            if response.lost_focus() || ui.input(|i| i.modifiers.ctrl && i.key_pressed(Key::S)) {
                self.commit_due(db, todo.id, &buffer, toasts);
            }
            if todo.due_date.is_some() && ui::ghost_button(ui, "Clear").clicked() {
                *inner_clicked = true;
                self.due_buffer.clear();
                self.due_for = None;
                if let Err(e) = TodoStore::new(db).update(todo.id, None, None, None, Some(None)) {
                    toasts.push(e.to_string());
                }
                self.dirty = true;
            }
        });
        ui.add_space(8.0);

        // Actions row.
        ui.label(
            RichText::new("Actions")
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        ui.horizontal_wrapped(|ui| {
            if ui::ghost_button(ui, "Add sub-todo")
                .on_hover_text("New sub-todo of this item")
                .clicked()
            {
                *inner_clicked = true;
                self.editor = Some(Editor {
                    target: EditorTarget::NewTodo {
                        group: todo.group_id,
                        parent: Some(todo.id),
                    },
                    buffer: String::new(),
                });
                self.unfolded.insert(todo.id);
                self.view = View::Tree;
            }
            let (index, count) = self.sibling_index(todo.id).unwrap_or((0, 1));
            let accent = accent_of(ui);
            let move_up = egui::Button::new(RichText::new("Move up").color(accent)).frame(false);
            if ui
                .add_enabled(index > 0, move_up)
                .on_hover_text("Move up among siblings (Alt+↑)")
                .clicked()
            {
                *inner_clicked = true;
                if let Err(e) = TodoStore::new(db).reorder(todo.id, true) {
                    toasts.push(e.to_string());
                } else {
                    self.dirty = true;
                }
            }
            let move_down =
                egui::Button::new(RichText::new("Move down").color(accent)).frame(false);
            if ui
                .add_enabled(index + 1 < count, move_down)
                .on_hover_text("Move down among siblings (Alt+↓)")
                .clicked()
            {
                *inner_clicked = true;
                if let Err(e) = TodoStore::new(db).reorder(todo.id, false) {
                    toasts.push(e.to_string());
                } else {
                    self.dirty = true;
                }
            }
            let picker_open = self.picker_for == Some(todo.id);
            if ui::ghost_button(ui, "Blocked by…")
                .on_hover_text("Pick a todo that blocks this one")
                .clicked()
            {
                *inner_clicked = true;
                self.picker_for = if picker_open { None } else { Some(todo.id) };
                self.blocked_search.clear();
            }
            if ui::ghost_button(ui, "Move to trash…").clicked() {
                *inner_clicked = true;
                let subtree = TodoStore::new(db).live_subtree_count(todo.id).unwrap_or(1);
                self.confirm = Some(Confirm::Trash {
                    id: todo.id,
                    title: todo.title.clone(),
                    subtree,
                });
            }
        });

        // Blocked-by: current blockers + picker.
        self.blocked_by_section(ui, db, todo, toasts, inner_clicked);
    }

    fn title_editor(&mut self, ui: &mut Ui, id: Uuid, db: &Db, _toasts: &mut Vec<String>) {
        let Some(buffer) = self
            .editor
            .as_ref()
            .filter(|e| matches!(e.target, EditorTarget::Title(t) if t == id))
            .map(|e| e.buffer.clone())
        else {
            return;
        };
        let mut buffer = buffer;
        let response = ui.add(
            egui::TextEdit::singleline(&mut buffer)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY),
        );
        ui::focused_underline(ui, &response);
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            self.editor = None;
        } else if response.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter)) {
            let title = buffer.trim();
            if !title.is_empty() {
                if let Err(e) = TodoStore::new(db).update(id, Some(title), None, None, None) {
                    self.inline_error = Some(e.to_string());
                }
            }
            self.editor = None;
            self.dirty = true;
        } else if let Some(e) = &mut self.editor {
            e.buffer = buffer;
        }
    }

    fn new_todo_row(
        &mut self,
        ui: &mut Ui,
        db: &Db,
        toasts: &mut Vec<String>,
        expect_parent: Option<Uuid>,
    ) {
        let Some(Editor {
            target: EditorTarget::NewTodo { group, parent },
            buffer,
        }) = self
            .editor
            .clone()
            .filter(|e| matches!(e.target, EditorTarget::NewTodo { .. }))
        else {
            return;
        };
        // The editor renders either at list level (top-level) or inside its
        // parent's children block — never both.
        if parent != expect_parent {
            return;
        }
        ui.horizontal(|ui| {
            ui.add_space(26.0);
            let mut buffer = buffer.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut buffer)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .hint_text("New todo — Enter to create"),
            );
            ui::focused_underline(ui, &response);
            let mut done = false;
            if response.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter)) {
                done = true;
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                self.editor = None;
            } else if done {
                self.commit_new_todo(db, group, parent, buffer, toasts);
            } else if let Some(e) = &mut self.editor {
                e.buffer = buffer;
            }
        });
    }

    fn commit_new_todo(
        &mut self,
        db: &Db,
        group: Uuid,
        parent: Option<Uuid>,
        buffer: String,
        _toasts: &mut Vec<String>,
    ) {
        let title = buffer.trim();
        if title.is_empty() {
            self.editor = None;
            return;
        }
        match TodoStore::new(db).create(group, parent, title) {
            Ok(todo) => {
                self.selected = Some(todo.id);
                if let Some(p) = parent {
                    self.unfolded.insert(p);
                }
                self.unfold_ancestors_of(todo.id);
                self.editor = Some(Editor {
                    target: EditorTarget::Title(todo.id),
                    buffer: todo.title,
                });
            }
            Err(e) => self.inline_error = Some(e.to_string()),
        }
        self.dirty = true;
    }

    /// Position of `id` among its siblings: (index, sibling count). Drives
    /// the enabled/disabled state of the Move up/down buttons.
    fn sibling_index(&self, id: Uuid) -> Option<(usize, usize)> {
        fn find(nodes: &[TodoNode], id: Uuid) -> Option<(usize, usize)> {
            for (i, n) in nodes.iter().enumerate() {
                if n.todo.id == id {
                    return Some((i, nodes.len()));
                }
                if let Some(r) = find(&n.children, id) {
                    return Some(r);
                }
            }
            None
        }
        find(&self.tree, id)
    }

    fn commit_due(&mut self, db: &Db, id: Uuid, buffer: &str, toasts: &mut Vec<String>) {
        let trimmed = buffer.trim();
        let parsed: Option<NaiveDate> = if trimmed.is_empty() {
            None
        } else {
            match NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
                Ok(d) => Some(d),
                Err(_) => {
                    toasts.push(format!("Invalid date {trimmed:?} — use YYYY-MM-DD"));
                    self.due_buffer.clear();
                    return;
                }
            }
        };
        match TodoStore::new(db).update(id, None, None, None, Some(parsed)) {
            Ok(_) => self.dirty = true,
            Err(e) => toasts.push(e.to_string()),
        }
        self.due_buffer.clear();
        self.due_for = None;
    }

    fn blocked_by_section(
        &mut self,
        ui: &mut Ui,
        db: &Db,
        todo: &Todo,
        toasts: &mut Vec<String>,
        inner_clicked: &mut bool,
    ) {
        let links = LinkStore::new(db);
        let target = EntityRef::new(EntityType::Todo, todo.id);
        let incoming = links.links_to(&target).unwrap_or_default();
        let blocked_by: Vec<Uuid> = incoming
            .iter()
            .filter(|l| l.relation.is_blocks() && l.source.kind == EntityType::Todo)
            .map(|l| l.source.id)
            .collect();
        if !blocked_by.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new("Blocked by")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            let store = TodoStore::new(db);
            for blocker in &blocked_by {
                let label = store
                    .get(*blocker)
                    .map(|t| t.title)
                    .unwrap_or_else(|_| "Unavailable".to_string());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label).small());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui::ghost_button(ui, "Remove").clicked() {
                            *inner_clicked = true;
                            let source = EntityRef::new(EntityType::Todo, *blocker);
                            if let Err(e) = links.unlink(&source, &target, &Relation::blocks()) {
                                toasts.push(e.to_string());
                            } else {
                                self.dirty = true;
                                self.blocked.clear();
                            }
                        }
                    });
                });
            }
        }

        // Picker (opened from the Actions row).
        if self.picker_for == Some(todo.id) {
            ui.add_space(4.0);
            let mut search = self.blocked_search.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut search)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .hint_text("Search todos to add a blocker…"),
            );
            ui::focused_underline(ui, &response);
            if response.clicked() || response.changed() {
                *inner_clicked = true;
            }
            if response.changed() {
                self.blocked_search = search.clone();
            }
            if !search.is_empty() {
                let store = TodoStore::new(db);
                let candidates = store.search(&search, Some(todo.id)).unwrap_or_default();
                for candidate in candidates.iter().filter(|c| !blocked_by.contains(&c.id)) {
                    let full_text = format!("{} — {}", candidate.title, candidate.status.label());
                    let response = ui.add(
                        egui::Label::new(RichText::new(full_text).small()).sense(Sense::click()),
                    );
                    if response.clicked() {
                        *inner_clicked = true;
                        let source = EntityRef::new(EntityType::Todo, candidate.id);
                        match links.link(&source, &target, &Relation::blocks()) {
                            Ok(()) => {
                                self.blocked_search.clear();
                                self.dirty = true;
                                self.blocked = links.blocked_todo_ids().unwrap_or_default();
                            }
                            Err(crate::core::link::LinkError::Cycle) => {
                                toasts.push(format!(
                                    "Cannot link: {} would create a dependency cycle",
                                    candidate.title
                                ));
                            }
                            Err(e) => toasts.push(e.to_string()),
                        }
                    }
                }
            }
        }
    }

    // ── trash view ────────────────────────────────────────────────────────

    fn trash_view(&mut self, ui: &mut Ui, db: &Db, toasts: &mut Vec<String>) {
        ui.add_space(8.0);
        ui.label(RichText::new("Trash").size(fonts::SIZE_HEADING).strong());
        ui.add_space(8.0);
        if self.trash.is_empty() {
            ui::empty_state(ui, "Trash is empty", "Deleted todos land here for 30 days");
            return;
        }
        let items = self.trash.clone();
        for item in &items {
            ui.add_space(CARD_GAP_PX);
            let frame = self.card_frame(ui, false, false);
            let inner = frame.show(ui, |ui| {
                ui.set_min_height(36.0 - 16.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&item.title).color(ui.visuals().weak_text_color()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let red = ui.visuals().error_fg_color;
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Delete permanently").color(red))
                                    .frame(false),
                            )
                            .clicked()
                        {
                            let subtree =
                                TodoStore::new(db).live_subtree_count(item.id).unwrap_or(1);
                            self.confirm = Some(Confirm::Purge {
                                id: item.id,
                                title: item.title.clone(),
                                subtree,
                            });
                        }
                        if ui::ghost_button(ui, "Restore").clicked() {
                            match TodoStore::new(db).restore(item.id) {
                                Ok(()) => self.dirty = true,
                                Err(e) => toasts.push(e.to_string()),
                            }
                        }
                    });
                });
            });
            let _ = inner;
        }
    }

    // ── delete confirm modal (the only modal) ─────────────────────────────

    fn show_confirm_modal(&mut self, ctx: &Context, db: &Db, toasts: &mut Vec<String>) {
        let Some(confirm) = self.confirm.clone() else {
            return;
        };
        let (title, body, is_purge) = match &confirm {
            Confirm::Trash { title, subtree, .. } => (
                "Move to trash?",
                format!("“{title}” and its {subtree} item(s) will be hidden for 30 days."),
                false,
            ),
            Confirm::Purge { title, subtree, .. } => (
                "Delete permanently?",
                format!(
                    "“{title}” and its {subtree} item(s) will be gone forever, including links."
                ),
                true,
            ),
        };
        let mut open = true;
        egui::Window::new(title)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(body);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let action = if is_purge {
                        "Delete forever"
                    } else {
                        "Move to trash"
                    };
                    if ui::primary_button(ui, action).clicked() {
                        let store = TodoStore::new(db);
                        let id = match &confirm {
                            Confirm::Trash { id, .. } | Confirm::Purge { id, .. } => *id,
                        };
                        let result = if is_purge {
                            store.delete_permanent(id)
                        } else {
                            store.trash(id)
                        };
                        match result {
                            Ok(()) => {
                                if self.selected == Some(id) {
                                    self.selected = None;
                                }
                                self.unfolded.remove(&id);
                                self.dirty = true;
                            }
                            Err(e) => toasts.push(e.to_string()),
                        }
                        self.confirm = None;
                    }
                    if ui::ghost_button(ui, "Cancel").clicked() {
                        self.confirm = None;
                    }
                });
            });
        if !open {
            self.confirm = None;
        }
    }
}

// ── badge + color helpers ─────────────────────────────────────────────────

fn open_descendants_excluding_self(node: &TodoNode) -> usize {
    node.open_descendant_count() - usize::from(!node.todo.status.is_terminal())
}

fn contains_match(node: &TodoNode, filter: &str) -> bool {
    let f = filter.to_lowercase();
    node.todo.title.to_lowercase().contains(&f)
        || node.children.iter().any(|c| contains_match(c, filter))
}

fn count_matches(nodes: &[TodoNode], filter: &str) -> usize {
    if filter.is_empty() {
        return 0;
    }
    nodes
        .iter()
        .map(|n| {
            usize::from(n.todo.title.to_lowercase().contains(&filter.to_lowercase()))
                + count_matches(&n.children, filter)
        })
        .sum()
}

fn status_color(ui: &Ui, status: Status) -> Color32 {
    let p = theme::palette(ui);
    match status {
        Status::Open => ui.visuals().text_color(),
        Status::InProgress => p.warn,
        Status::Done => p.success,
        Status::Cancelled => ui.visuals().weak_text_color(),
    }
}

/// Priority label colors: urgent = red, high = yellow/orange,
/// normal = foreground, low = muted.
fn priority_text_color(ui: &Ui, priority: Priority) -> Color32 {
    let p = theme::palette(ui);
    match priority.value() {
        3 => p.danger,
        2 => p.warn,
        1 => ui.visuals().text_color(),
        _ => ui.visuals().weak_text_color(),
    }
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}
