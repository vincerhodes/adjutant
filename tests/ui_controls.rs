//! Headless UI tests (egui_kittest) for the new-todo editor wedge and the
//! mouse-parity controls. No display needed — kittest drives egui with
//! synthesized input and queries the AccessKit tree.

#![allow(clippy::unwrap_used)]

use adjutant::app::AdjutantApp;
use adjutant::db::Db;
use adjutant::todo::ui::TodoUi;
use adjutant::todo::TodoStore;
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

struct TodoHarnessState {
    db: Db,
    todo: TodoUi,
    toasts: Vec<String>,
}

/// TodoUi in isolation: sidebar + main panes, one empty "Personal" group.
fn make_harness() -> Harness<'static, TodoHarnessState> {
    let db = Db::open(std::path::Path::new(":memory:")).unwrap();
    TodoStore::new(&db).create_group("Personal").unwrap();
    let todo = TodoUi::new(&db);
    Harness::builder()
        .with_size([1200.0, 800.0])
        .build_ui_state(
            |ui, app: &mut TodoHarnessState| {
                app.todo.sidebar(ui, &app.db);
                let ctx = ui.ctx().clone();
                app.todo.show(ui, &ctx, &app.db, &mut app.toasts);
            },
            TodoHarnessState {
                db,
                todo,
                toasts: Vec::new(),
            },
        )
}

/// Same, but with two existing todos "A" and "B" in the group.
fn make_harness_with_todos() -> Harness<'static, TodoHarnessState> {
    let db = Db::open(std::path::Path::new(":memory:")).unwrap();
    let store = TodoStore::new(&db);
    let group = store.create_group("Personal").unwrap();
    store.create(group.id, None, "A").unwrap();
    store.create(group.id, None, "B").unwrap();
    let todo = TodoUi::new(&db);
    Harness::builder()
        .with_size([1200.0, 800.0])
        .build_ui_state(
            |ui, app: &mut TodoHarnessState| {
                app.todo.sidebar(ui, &app.db);
                let ctx = ui.ctx().clone();
                app.todo.show(ui, &ctx, &app.db, &mut app.toasts);
            },
            TodoHarnessState {
                db,
                todo,
                toasts: Vec::new(),
            },
        )
}

fn text_input_count(h: &Harness<'_, TodoHarnessState>) -> usize {
    h.query_all_by_role(Role::TextInput).count()
}

fn group_tree_titles(h: &Harness<'_, TodoHarnessState>) -> Vec<String> {
    let app = h.state();
    let store = TodoStore::new(&app.db);
    let group = app.todo.current_group().unwrap();
    store
        .tree(group)
        .unwrap()
        .into_iter()
        .map(|n| n.todo.title)
        .collect()
}

// ── Item 1: the wedge ─────────────────────────────────────────────────────

/// The original bug: Ctrl+N on an empty tree set editor state whose widget
/// only rendered inside the non-empty-tree branch, wedging every key.
#[test]
fn ctrl_n_renders_editor_on_empty_tree_and_creates_todo() {
    let mut h = make_harness();
    h.run();
    assert_eq!(text_input_count(&h), 0);

    h.key_press_modifiers(Modifiers::CTRL, Key::N);
    h.run();
    assert_eq!(
        text_input_count(&h),
        1,
        "Ctrl+N editor must render even when the tree is empty"
    );

    // Focus, type, commit.
    h.get_by_role(Role::TextInput).focus();
    h.run();
    h.get_by_role(Role::TextInput).type_text("Buy milk");
    h.run();
    h.key_press(Key::Enter);
    h.run();

    assert_eq!(group_tree_titles(&h), vec!["Buy milk".to_string()]);
}

#[test]
fn escape_cancels_editor_and_keys_stay_live() {
    let mut h = make_harness();
    h.run();

    h.key_press_modifiers(Modifiers::CTRL, Key::N);
    h.run();
    assert_eq!(text_input_count(&h), 1);

    h.key_press(Key::Escape);
    h.run();
    assert_eq!(text_input_count(&h), 0, "Esc must cancel the editor");

    // Keys must not be wedged: Ctrl+F now opens the filter field.
    h.key_press_modifiers(Modifiers::CTRL, Key::F);
    h.run();
    assert_eq!(
        text_input_count(&h),
        1,
        "filter field should open after Esc — keys must not be wedged"
    );
}

#[test]
fn ctrl_n_does_not_fire_over_trash_view() {
    let mut h = make_harness_with_todos();
    h.run();
    // Select a todo, then switch to the trash view.
    h.key_press(Key::ArrowDown);
    h.run();
    h.key_press(Key::Space); // toggle done — allowed, no children
    h.run();
    h.get_by_label_contains("Trash").click();
    h.run();
    // Detail pane stays mounted: due date + blocked-by search.
    assert_eq!(text_input_count(&h), 2);

    // Ctrl+N over the trash list must not open a tree editor: back in the
    // tree there is still no editor widget.
    h.key_press_modifiers(Modifiers::CTRL, Key::N);
    h.run();
    h.get_by_label("Personal").click();
    h.run();
    assert_eq!(
        text_input_count(&h),
        2,
        "Ctrl+N over the trash view must not strand a tree editor"
    );

    // Liveness: Esc + Ctrl+F opens the filter field.
    h.key_press(Key::Escape);
    h.run();
    h.key_press_modifiers(Modifiers::CTRL, Key::F);
    h.run();
    assert_eq!(text_input_count(&h), 3);
}

// ── Item 2: mouse parity ──────────────────────────────────────────────────

#[test]
fn mouse_new_todo_button_matches_ctrl_n() {
    let mut h = make_harness();
    h.run();
    h.get_by_label("+ New todo").click();
    h.run();
    assert_eq!(
        text_input_count(&h),
        1,
        "\"+ New todo\" must open the editor on an empty tree"
    );

    h.get_by_role(Role::TextInput).focus();
    h.run();
    h.get_by_role(Role::TextInput).type_text("Via mouse");
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(group_tree_titles(&h), vec!["Via mouse".to_string()]);
}

#[test]
fn mouse_new_group_button_creates_group() {
    let mut h = make_harness();
    h.run();
    h.get_by_label("+").click();
    h.run();
    assert_eq!(text_input_count(&h), 1, "sidebar group editor should open");

    h.get_by_role(Role::TextInput).focus();
    h.run();
    h.get_by_role(Role::TextInput).type_text("Browzr");
    h.run();
    h.key_press(Key::Enter);
    h.run();

    let app = h.state();
    let names: Vec<String> = TodoStore::new(&app.db)
        .list_groups()
        .unwrap()
        .into_iter()
        .map(|g| g.name)
        .collect();
    assert_eq!(names, vec!["Personal".to_string(), "Browzr".to_string()]);
}

#[test]
fn mouse_add_sub_todo_from_detail_pane() {
    let mut h = make_harness();
    h.run();
    h.get_by_label("+ New todo").click();
    h.run();
    h.get_by_role(Role::TextInput).focus();
    h.run();
    h.get_by_role(Role::TextInput).type_text("Parent");
    h.run();
    h.key_press(Key::Enter);
    h.run();
    // Creating selects the todo and opens its title editor; close it.
    h.key_press(Key::Escape);
    h.run();

    h.get_by_label("Add sub-todo").click();
    h.run();
    // Three single-line inputs now: due date + blocked-by search (detail
    // pane) + new-sub-todo editor. The editor renders after the detail pane,
    // so it's the last in the accesskit tree.
    {
        let inputs: Vec<_> = h.query_all_by_role(Role::TextInput).collect();
        assert_eq!(
            inputs.len(),
            3,
            "due date + blocked-by search + new-sub-todo editor"
        );
        inputs.last().unwrap().focus();
    }
    h.run();
    {
        let inputs: Vec<_> = h.get_all_by_role(Role::TextInput).collect();
        inputs.last().unwrap().type_text("Child");
    }
    h.run();
    h.key_press(Key::Enter);
    h.run();

    let app = h.state();
    let store = TodoStore::new(&app.db);
    let group = app.todo.current_group().unwrap();
    let tree = store.tree(group).unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].todo.title, "Parent");
    assert_eq!(tree[0].children.len(), 1);
    assert_eq!(tree[0].children[0].todo.title, "Child");
}

#[test]
fn mouse_move_up_reorders_siblings() {
    let mut h = make_harness_with_todos();
    h.run();
    // Select B via arrow keys (avoids label ambiguity with the detail title).
    h.key_press(Key::ArrowDown);
    h.run();
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(
        group_tree_titles(&h),
        vec!["A".to_string(), "B".to_string()]
    );

    h.get_by_label("Move up").click();
    h.run();
    assert_eq!(
        group_tree_titles(&h),
        vec!["B".to_string(), "A".to_string()]
    );
}

#[test]
fn mouse_filter_open_and_clear() {
    let mut h = make_harness_with_todos();
    h.run();
    assert_eq!(text_input_count(&h), 0);

    h.get_by_label("Filter…").click();
    h.run();
    assert_eq!(text_input_count(&h), 1, "Filter… opens the filter field");

    h.get_by_label("✕").click();
    h.run();
    assert_eq!(text_input_count(&h), 0, "✕ clears the filter field");
}

#[test]
fn mouse_help_button_toggles_overlay() {
    let db = Db::open(std::path::Path::new(":memory:")).unwrap();
    let mut h = Harness::builder()
        .with_size([1200.0, 800.0])
        .build_eframe(move |cc| AdjutantApp::new(db, cc));
    // The app requests a repaint every second (theme tick), so step a fixed
    // number of frames instead of run().
    h.run_steps(2);
    assert_eq!(h.query_all_by_label_contains("Shortcuts").count(), 0);

    h.get_by_label("Help").click();
    h.run_steps(2);
    assert_eq!(
        h.query_all_by_label_contains("Shortcuts").count(),
        1,
        "Help button should open the shortcut overlay"
    );

    h.get_by_label("Help").click();
    h.run_steps(2);
    assert_eq!(h.query_all_by_label_contains("Shortcuts").count(), 0);
}
