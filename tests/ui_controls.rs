//! Headless UI tests (egui_kittest) for the new-todo editor wedge.
//! No display needed — kittest drives egui with synthesized input and
//! queries the AccessKit tree.

#![allow(clippy::unwrap_used)]

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
