//! Todo module UI: three-pane layout (group list → tree → detail),
//! trash view, filter, inline editing, delete-confirm modal.
//!
//! Zero SQL in this file — all data access goes through `TodoStore` /
//! `LinkStore`. Rendering only.

use std::collections::HashSet;

use chrono::{Local, NaiveDate};
use egui::{Align, Color32, Context, Key, Layout, RichText, Sense, Stroke, Ui};
use uuid::Uuid;

use crate::core::entity::{EntityRef, EntityType};
use crate::core::link::{LinkStore, Relation};
use crate::db::Db;
use crate::todo::{Priority, Status, Todo, TodoError, TodoGroup, TodoNode, TodoStore};
use crate::ui::{self, accent_of, fonts, theme};

const INDENT_PX: f32 = 20.0;
/// Visual indent caps at this depth (plan risk 5); structure stays unbounded.
const MAX_VISUAL_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Tree,
    Trash,
}

/// In-flight editor state. Keyed independently of the tree so background
/// reloads never clobber an edit (§6 edit semantics).
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
    collapsed: HashSet<Uuid>,
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
            collapsed: HashSet::new(),
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

    /// Sidebar content: group list + Trash entry. Rendered inside the app's
    /// left panel.
    pub fn sidebar(&mut self, ui: &mut Ui, db: &Db) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Groups").size(fonts::SIZE_SMALL).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
        fn walk(nodes: &[TodoNode], id: Uuid) -> bool {
            nodes
                .iter()
                .any(|n| n.todo.id == id || walk(&n.children, id))
        }
        walk(&self.tree, id)
    }

    /// Full todo UI: keyboard shortcuts, panes, modal. Takes the root `ui`
    /// (the detail panel is a right-side `egui::Panel` on it) plus the ctx
    /// for shortcut input and the modal window.
    pub fn show(&mut self, ui: &mut Ui, ctx: &Context, db: &Db, toasts: &mut Vec<String>) {
        self.handle_keys(ctx, db, toasts);
        self.reload(db, None);

        egui::Panel::right("todo_detail")
            .resizable(true)
            .default_size(300.0)
            .size_range(240.0..=420.0)
            .show_separator_line(false)
            .show(ui, |ui| {
                self.detail_pane(ui, db, toasts);
            });

        egui::CentralPanel::default().show(ui, |ui| match self.view {
            View::Tree => self.tree_view(ui, db, toasts),
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
                        toasts.push(e.to_string());
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
            if i.key_pressed(Key::Enter) {
                if let Some(id) = self.selected {
                    if let Ok(todo) = TodoStore::new(db).get(id) {
                        self.editor = Some(Editor {
                            target: EditorTarget::Title(id),
                            buffer: todo.title,
                        });
                    }
                }
            }
        });
    }

    /// Open the new-todo editor (Ctrl+N / "+ New todo" button). The new todo
    /// becomes a child of the current selection, if any.
    fn start_new_todo(&mut self, toasts: &mut Vec<String>) {
        if let Some(group) = self.current_group {
            let parent = self.selected;
            self.editor = Some(Editor {
                target: EditorTarget::NewTodo { group, parent },
                buffer: String::new(),
            });
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

    fn tree_view(&mut self, ui: &mut Ui, db: &Db, toasts: &mut Vec<String>) {
        let Some(group) = self.current_group else {
            ui::empty_state(
                ui,
                "No group selected",
                "Ctrl+Shift+N to create your first group",
            );
            return;
        };

        // Tree header: group name left, actions right (ghost buttons, §7a).
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let name = self
                .groups
                .iter()
                .find(|g| g.id == group)
                .map(|g| g.name.clone())
                .unwrap_or_default();
            ui.label(RichText::new(name).size(fonts::SIZE_HEADING).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.filter_active
                    && ui::ghost_button(ui, "✕")
                        .on_hover_text("Clear filter (Esc)")
                        .clicked()
                {
                    self.filter_active = false;
                    self.filter.clear();
                }
                if ui::ghost_button(ui, "Filter…")
                    .on_hover_text("Filter todos (Ctrl+F)")
                    .clicked()
                {
                    self.filter_active = true;
                }
                if ui::ghost_button(ui, "+ New todo")
                    .on_hover_text("New todo (Ctrl+N)")
                    .clicked()
                {
                    self.start_new_todo(toasts);
                }
            });
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
            self.new_todo_row(ui, db, toasts);
            if self.editor.is_none() {
                ui::empty_state(ui, "Nothing here yet", "Ctrl+N to create your first todo");
            }
            return;
        }

        self.visible_order.clear();
        let nodes = self.tree.clone();
        // Suspend filtering while a title is being edited: the edited row
        // would otherwise vanish from the tree the moment its buffer stops
        // matching, stranding the editor.
        let title_editing = matches!(
            self.editor,
            Some(Editor {
                target: EditorTarget::Title(_),
                ..
            })
        );
        let filter = if title_editing {
            String::new()
        } else {
            self.filter.clone()
        };
        let scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
        scroll.show(ui, |ui| {
            for node in &nodes {
                self.node_row(ui, node, 0, &filter, db, toasts);
            }
            // Inline "new todo" row right after its would-be siblings.
            self.new_todo_row(ui, db, toasts);
        });
    }

    fn new_todo_row(&mut self, ui: &mut Ui, db: &Db, toasts: &mut Vec<String>) {
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
        let depth = parent
            .map(|p| self.depth_of(p).min(MAX_VISUAL_DEPTH))
            .unwrap_or(0);
        ui.horizontal(|ui| {
            ui.add_space(INDENT_PX * depth as f32 + 24.0);
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
                    self.collapsed.remove(&p);
                }
                self.editor = Some(Editor {
                    target: EditorTarget::Title(todo.id),
                    buffer: todo.title,
                });
            }
            Err(e) => self.inline_error = Some(e.to_string()),
        }
        self.dirty = true;
    }

    fn depth_of(&self, id: Uuid) -> usize {
        fn find(nodes: &[TodoNode], id: Uuid, depth: usize) -> Option<usize> {
            for n in nodes {
                if n.todo.id == id {
                    return Some(depth);
                }
                if let Some(d) = find(&n.children, id, depth + 1) {
                    return Some(d);
                }
            }
            None
        }
        find(&self.tree, id, 0).unwrap_or(0)
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

    fn node_row(
        &mut self,
        ui: &mut Ui,
        node: &TodoNode,
        depth: usize,
        filter: &str,
        db: &Db,
        toasts: &mut Vec<String>,
    ) {
        // Filtering: keep matches and ancestors of matches (§6).
        let visible_depth = depth.min(MAX_VISUAL_DEPTH);
        if !filter.is_empty() {
            let self_match = node
                .todo
                .title
                .to_lowercase()
                .contains(&filter.to_lowercase());
            let child_match = contains_match(node, filter);
            if !self_match && !child_match {
                let hidden = node.open_descendant_count();
                if hidden > 0 && depth == 0 {
                    // Entire root hidden — nothing to draw.
                }
                return;
            }
        }

        self.visible_order.push(node.todo.id);
        let selected = self.selected == Some(node.todo.id);
        let row_bg = if selected {
            with_alpha(accent_of(ui), 0.10)
        } else {
            Color32::TRANSPARENT
        };
        let row = ui.horizontal(|ui| {
            ui.add_space(INDENT_PX * visible_depth as f32);
            // Collapse toggle for parents.
            if !node.children.is_empty() {
                let collapsed = self.collapsed.contains(&node.todo.id);
                if ui::collapse_arrow(ui, collapsed).clicked() {
                    if collapsed {
                        self.collapsed.remove(&node.todo.id);
                    } else {
                        self.collapsed.insert(node.todo.id);
                    }
                }
            } else {
                ui.add_space(14.0);
            }

            // Checkbox (custom-painted: visible outline, accent when checked).
            let checked = node.todo.status == Status::Done;
            if ui::todo_checkbox(ui, checked).clicked() {
                match self.toggle_done(db, node.todo.id) {
                    Ok(()) => {}
                    Err(e) => toasts.push(e),
                }
            }

            ui::priority_dot(ui, node.todo.priority);

            // Title / inline editor.
            let editing_title = matches!(
                self.editor,
                Some(Editor { target: EditorTarget::Title(id), .. }) if id == node.todo.id
            );
            if editing_title {
                self.title_editor(ui, node.todo.id, db, toasts);
            } else {
                let done = node.todo.status.is_terminal();
                let mut text = RichText::new(&node.todo.title);
                if done {
                    text = text.color(ui.visuals().weak_text_color()).strikethrough();
                }
                let response = ui.add(egui::Label::new(text).sense(Sense::click()));
                if response.clicked() {
                    self.selected = Some(node.todo.id);
                }
                if response.double_clicked() {
                    self.editor = Some(Editor {
                        target: EditorTarget::Title(node.todo.id),
                        buffer: node.todo.title.clone(),
                    });
                }
            }

            // Right-aligned due date (§7a: never for terminal items).
            if let Some(due) = node.todo.due_date {
                if !node.todo.status.is_terminal() {
                    let today = Local::now().date_naive();
                    let color = if due < today {
                        ui.visuals().error_fg_color
                    } else if due == today {
                        accent_of(ui)
                    } else {
                        ui.visuals().weak_text_color()
                    };
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(due.to_string()).small().color(color));
                    });
                }
            }
        });
        let row_rect = row.response.rect;
        if ui.is_rect_visible(row_rect) {
            if selected {
                ui.painter().rect_filled(row_rect, 0.0, row_bg);
                ui.painter().line_segment(
                    [row_rect.left_top(), row_rect.left_bottom()],
                    Stroke::new(3.0, accent_of(ui)),
                );
            } else if row.response.hovered() {
                ui.painter()
                    .rect_filled(row_rect, 0.0, ui.visuals().faint_bg_color);
            }
        }

        if !self.collapsed.contains(&node.todo.id) || self.editing_inside(node) {
            for child in &node.children {
                self.node_row(ui, child, depth + 1, filter, db, toasts);
            }
        }
    }

    /// True when a title editor is open somewhere inside this subtree — the
    /// subtree must stay expanded or the editor's widget would vanish.
    fn editing_inside(&self, node: &TodoNode) -> bool {
        let Some(Editor {
            target: EditorTarget::Title(id),
            ..
        }) = self.editor
        else {
            return false;
        };
        node.todo.id == id || node.children.iter().any(|c| self.editing_inside(c))
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
        egui::ScrollArea::vertical().show(ui, |ui| {
            for item in &items {
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
            }
        });
    }

    // ── detail pane (right) ───────────────────────────────────────────────

    fn detail_pane(&mut self, ui: &mut Ui, db: &Db, toasts: &mut Vec<String>) {
        ui.add_space(12.0);
        let Some(id) = self.selected else {
            ui::empty_state(ui, "Select a todo", "Details appear here");
            return;
        };
        let Ok(todo) = TodoStore::new(db).get(id) else {
            self.selected = None;
            return;
        };
        if todo.deleted_at.is_some() {
            ui::empty_state(ui, "Todo is in the trash", "Restore it from the Trash view");
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(4.0);
            // Title
            ui.label(
                RichText::new(&todo.title)
                    .size(fonts::SIZE_HEADING)
                    .strong(),
            );
            ui.add_space(8.0);

            // Status (text label, colored per mapping — no pills).
            ui.horizontal_wrapped(|ui| {
                for status in [
                    Status::Open,
                    Status::InProgress,
                    Status::Done,
                    Status::Cancelled,
                ] {
                    let color = status_color(ui, status);
                    let text = RichText::new(status.label()).color(color);
                    if ui.selectable_label(todo.status == status, text).clicked() {
                        match TodoStore::new(db).set_status(todo.id, status) {
                            Ok(_) => self.dirty = true,
                            Err(TodoError::OpenDescendants(n)) => {
                                toasts.push(format!(
                                    "Cannot complete: {n} open descendant(s) remain"
                                ));
                            }
                            Err(e) => toasts.push(e.to_string()),
                        }
                    }
                }
            });
            ui.add_space(8.0);

            // Priority dots (4 choices, dot language).
            ui.horizontal(|ui| {
                ui.label(RichText::new("Priority").small());
                for value in 0..=3u8 {
                    let p = Priority::new(value).unwrap_or(Priority::NORMAL);
                    let active = todo.priority == p;
                    let response = ui::priority_dot(ui, p);
                    if response.clicked() {
                        if let Err(e) =
                            TodoStore::new(db).update(todo.id, None, None, Some(p), None)
                        {
                            toasts.push(e.to_string());
                        } else {
                            self.dirty = true;
                        }
                    }
                    if active {
                        ui.painter().circle_stroke(
                            response.rect.center(),
                            4.5,
                            Stroke::new(1.0, accent_of(ui)),
                        );
                    }
                    response.on_hover_text(p.label());
                }
            });
            ui.add_space(8.0);

            // Due date (validated text, no calendar widget in M1).
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
                if response.lost_focus() || ui.input(|i| i.modifiers.ctrl && i.key_pressed(Key::S))
                {
                    self.commit_due(db, todo.id, &buffer, toasts);
                }
                if todo.due_date.is_some() && ui::ghost_button(ui, "Clear").clicked() {
                    self.due_buffer.clear();
                    self.due_for = None;
                    if let Err(e) = TodoStore::new(db).update(todo.id, None, None, None, Some(None))
                    {
                        toasts.push(e.to_string());
                    }
                    self.dirty = true;
                }
            });
            ui.add_space(8.0);

            // Notes (multiline: blur or Ctrl+S saves, no save button).
            ui.label(RichText::new("Notes").small());
            let notes = match &self.notes_buffer {
                Some((buf_id, buf)) if *buf_id == todo.id => buf.clone(),
                _ => todo.notes.clone(),
            };
            let mut notes_edit = notes.clone();
            let response = ui.add(
                egui::TextEdit::multiline(&mut notes_edit)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .desired_rows(6)
                    .hint_text("Notes… (saved on blur or Ctrl+S)"),
            );
            ui::focused_underline(ui, &response);
            let save = response.lost_focus()
                || (response.has_focus()
                    && ui.input(|i| i.modifiers.ctrl && i.key_pressed(Key::S)));
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

            // Mouse parity for keyboard shortcuts (§7a: ghost buttons).
            ui.horizontal(|ui| {
                if ui::ghost_button(ui, "Add sub-todo")
                    .on_hover_text("New sub-todo of this item")
                    .clicked()
                {
                    self.editor = Some(Editor {
                        target: EditorTarget::NewTodo {
                            group: todo.group_id,
                            parent: Some(todo.id),
                        },
                        buffer: String::new(),
                    });
                    self.view = View::Tree;
                    self.collapsed.remove(&todo.id);
                }
                let (index, count) = self.sibling_index(todo.id).unwrap_or((0, 1));
                let accent = accent_of(ui);
                let move_up =
                    egui::Button::new(RichText::new("Move up").color(accent)).frame(false);
                if ui
                    .add_enabled(index > 0, move_up)
                    .on_hover_text("Move up among siblings (Alt+↑)")
                    .clicked()
                {
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
                    if let Err(e) = TodoStore::new(db).reorder(todo.id, false) {
                        toasts.push(e.to_string());
                    } else {
                        self.dirty = true;
                    }
                }
            });
            ui.add_space(8.0);

            self.blocked_by_section(ui, db, &todo, toasts);
            ui.add_space(12.0);

            // Delete (destructive, behind explicit affordance — §7a principle 4).
            if ui::ghost_button(ui, "Move to trash…").clicked() {
                let subtree = TodoStore::new(db).live_subtree_count(todo.id).unwrap_or(1);
                self.confirm = Some(Confirm::Trash {
                    id: todo.id,
                    title: todo.title.clone(),
                    subtree,
                });
            }
        });
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

    fn blocked_by_section(&mut self, ui: &mut Ui, db: &Db, todo: &Todo, toasts: &mut Vec<String>) {
        ui.label(RichText::new("Blocked by").small());
        let links = LinkStore::new(db);
        let target = EntityRef::new(EntityType::Todo, todo.id);
        let incoming = links.links_to(&target).unwrap_or_default();
        let blocked_by: Vec<Uuid> = incoming
            .iter()
            .filter(|l| l.relation.is_blocks() && l.source.kind == EntityType::Todo)
            .map(|l| l.source.id)
            .collect();
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
                        let source = EntityRef::new(EntityType::Todo, *blocker);
                        if let Err(e) = links.unlink(&source, &target, &Relation::blocks()) {
                            toasts.push(e.to_string());
                        }
                    }
                });
            });
        }

        // Picker: search live todos by title; cycle rejected inline.
        let mut search = self.blocked_search.clone();
        let response = ui.add(
            egui::TextEdit::singleline(&mut search)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .hint_text("Search todos to add a blocker…"),
        );
        ui::focused_underline(ui, &response);
        if response.changed() {
            self.blocked_search = search.clone();
        }
        if !search.is_empty() {
            let candidates = store.search(&search, Some(todo.id)).unwrap_or_default();
            for candidate in candidates.iter().filter(|c| !blocked_by.contains(&c.id)) {
                let full_text = format!("{} — {}", candidate.title, candidate.status.label());
                if ui
                    .add(egui::Label::new(RichText::new(full_text).small()).sense(Sense::click()))
                    .clicked()
                {
                    let source = EntityRef::new(EntityType::Todo, candidate.id);
                    match links.link(&source, &target, &Relation::blocks()) {
                        Ok(()) => {
                            self.blocked_search.clear();
                            self.dirty = true;
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

// ── helpers ───────────────────────────────────────────────────────────────

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
    match status {
        Status::Open => ui.visuals().text_color(),
        // §7a: in_progress = theme yellow, done = theme green.
        Status::InProgress => ui.visuals().warn_fg_color,
        Status::Done => theme::success_color(ui),
        Status::Cancelled => ui.visuals().weak_text_color(),
    }
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}
