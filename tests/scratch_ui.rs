//! Scratch pad UI tests (kittest, spec §10): create + autosave round-trip
//! (via the `flush_pending` seam — no debounce sleeps), pin reorder,
//! trash/restore, todo picker pad entries, designed empty state.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use adjutant::app::AdjutantApp;
use adjutant::db::Db;
use adjutant::scratch::{ScratchPadInput, ScratchStore};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

fn temp_db_path(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("adjutant-scratch-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("adjutant.db")
}

fn boot(path: &Path) -> Harness<'static, AdjutantApp> {
    let db = Db::open(path).unwrap();
    Harness::builder()
        .with_size([1200.0, 800.0])
        .build_eframe(move |cc| AdjutantApp::new_with_engine(db, cc, None))
}

fn open_scratch(h: &mut Harness<'static, AdjutantApp>) {
    h.run_steps(2);
    h.get_by_label("Scratchpad").click();
    h.run_steps(3);
}

#[test]
fn empty_state_on_fresh_db() {
    let path = temp_db_path("empty");
    let db = Db::open(&path).unwrap();
    drop(db);
    let mut h = boot(&path);
    open_scratch(&mut h);
    assert_eq!(h.query_all_by_label_contains("No pads yet").count(), 1);
}

#[test]
fn new_pad_types_and_autosaves() {
    let path = temp_db_path("autosave");
    let db = Db::open(&path).unwrap();
    drop(db);
    let mut h = boot(&path);
    open_scratch(&mut h);

    // Exact label: the empty-state hint also mentions "New pad".
    h.get_by_label("New pad").click();
    h.run_steps(2);
    // Draft editor: the multiline input.
    let inputs: Vec<_> = h.query_all_by_role(Role::MultilineTextInput).collect();
    assert_eq!(inputs.len(), 1, "draft editor visible");
    inputs[0].focus();
    h.run_steps(1);
    let inputs: Vec<_> = h.query_all_by_role(Role::MultilineTextInput).collect();
    inputs[0].type_text("Buy oat milk");
    h.run_steps(2);

    // Autosave seam: flush via the app's ScratchUi (no debounce sleep).
    {
        let flush_db = Db::open(&path).unwrap();
        h.state_mut().scratch().flush_pending(&flush_db);
    }
    h.run_steps(2);

    let db = Db::open(&path).unwrap();
    let store = ScratchStore::new(&db);
    let pads = store.list().unwrap();
    assert_eq!(pads.len(), 1);
    assert_eq!(pads[0].body, "Buy oat milk");

    // Fold (Esc) → flush path keeps working; pad stays in the list.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.query_all_by_label_contains("Buy oat milk").count(), 1);
}

#[test]
fn click_card_unfolds_and_edits_existing_pad() {
    let path = temp_db_path("edit-existing");
    let db = Db::open(&path).unwrap();
    let store = ScratchStore::new(&db);
    store
        .create(&ScratchPadInput {
            body: "Standup notes\nline two".to_string(),
            pinned: false,
            color_idx: 0,
        })
        .unwrap();
    drop(db);

    let mut h = boot(&path);
    open_scratch(&mut h);
    // Click the card body (widest node) to unfold.
    {
        let mut nodes: Vec<_> = h.query_all_by_label("Standup notes").collect();
        nodes.sort_by(|a, b| a.rect().width().partial_cmp(&b.rect().width()).unwrap());
        nodes.last().unwrap().click();
    }
    h.run_steps(3);
    let inputs: Vec<_> = h.query_all_by_role(Role::MultilineTextInput).collect();
    assert_eq!(inputs.len(), 1, "editor unfolded");
    inputs[0].focus();
    h.run_steps(1);
    let inputs: Vec<_> = h.query_all_by_role(Role::MultilineTextInput).collect();
    inputs[0].type_text("!!");
    h.run_steps(2);
    {
        let flush_db = Db::open(&path).unwrap();
        h.state_mut().scratch().flush_pending(&flush_db);
    }
    h.run_steps(2);

    let db = Db::open(&path).unwrap();
    let store = ScratchStore::new(&db);
    assert_eq!(store.list().unwrap()[0].body, "Standup notes\nline two!!");
}

#[test]
fn pin_moves_pad_to_top_and_color_cycles() {
    let path = temp_db_path("pin");
    let db = Db::open(&path).unwrap();
    let store = ScratchStore::new(&db);
    store
        .create(&ScratchPadInput {
            body: "Alpha pad".to_string(),
            pinned: false,
            color_idx: 0,
        })
        .unwrap();
    let b = store
        .create(&ScratchPadInput {
            body: "Bravo pad".to_string(),
            pinned: false,
            color_idx: 0,
        })
        .unwrap();
    drop(db);

    let mut h = boot(&path);
    open_scratch(&mut h);
    // Both pads are unpinned (one Pin button each). The list is recency-
    // ordered (Bravo created second, so it's on top) — pin the TOP card.
    {
        let mut pins: Vec<_> = h
            .query_all_by_label_contains("Pin pad (icon button)")
            .collect();
        pins.sort_by(|a, b| a.rect().top().partial_cmp(&b.rect().top()).unwrap());
        pins.first().unwrap().click();
    }
    h.run_steps(3);

    let db = Db::open(&path).unwrap();
    let store = ScratchStore::new(&db);
    let pads = store.list().unwrap();
    assert!(pads[0].pinned, "pinned pad leads the list");
    assert_eq!(pads[0].id, b.id);

    // Color cycle on the top (pinned) card: 0 → 1.
    {
        let mut buttons: Vec<_> = h
            .query_all_by_label_contains("Cycle color (icon button)")
            .collect();
        buttons.sort_by(|a, b| a.rect().top().partial_cmp(&b.rect().top()).unwrap());
        buttons.first().unwrap().click();
    }
    h.run_steps(2);
    let db = Db::open(&path).unwrap();
    let store = ScratchStore::new(&db);
    assert_eq!(store.list().unwrap()[0].color_idx, 1);
}

#[test]
fn trash_then_restore() {
    let path = temp_db_path("trash");
    let db = Db::open(&path).unwrap();
    let store = ScratchStore::new(&db);
    store
        .create(&ScratchPadInput {
            body: "Disposable pad".to_string(),
            pinned: false,
            color_idx: 0,
        })
        .unwrap();
    drop(db);

    let mut h = boot(&path);
    open_scratch(&mut h);
    h.get_by_label_contains("Trash pad (icon button)").click();
    h.run_steps(3);
    assert_eq!(h.query_all_by_label_contains("Disposable pad").count(), 0);

    h.get_by_label("Trash").click();
    h.run_steps(3);
    assert_eq!(h.query_all_by_label_contains("Disposable pad").count(), 1);
    h.get_by_label_contains("Restore pad (icon button)").click();
    h.run_steps(3);

    h.get_by_label("Pads").click();
    h.run_steps(3);
    assert_eq!(h.query_all_by_label_contains("Disposable pad").count(), 1);
}

#[test]
fn todo_picker_links_pad() {
    use adjutant::core::entity::{EntityRef, EntityType};
    use adjutant::core::link::LinkStore;
    use adjutant::todo::TodoStore;

    let path = temp_db_path("link-pad");
    let db = Db::open(&path).unwrap();
    let todos = TodoStore::new(&db);
    let group = todos.create_group("Personal").unwrap();
    todos.create(group.id, None, "Prep board").unwrap();
    let store = ScratchStore::new(&db);
    let pad = store
        .create(&ScratchPadInput {
            body: "Board talking points".to_string(),
            pinned: false,
            color_idx: 0,
        })
        .unwrap();
    drop(db);

    let mut h = boot(&path);
    open_scratch(&mut h);
    h.get_by_label("Todo").click();
    h.run_steps(3);
    {
        let mut nodes: Vec<_> = h.query_all_by_label("Prep board").collect();
        nodes.sort_by(|a, b| a.rect().width().partial_cmp(&b.rect().width()).unwrap());
        nodes.last().unwrap().click();
    }
    h.run_steps(3);
    h.get_by_label("Blocked by…").click();
    h.run_steps(2);
    {
        let inputs: Vec<_> = h.query_all_by_role(Role::TextInput).collect();
        inputs.last().unwrap().focus();
    }
    h.run_steps(1);
    {
        let inputs: Vec<_> = h.query_all_by_role(Role::TextInput).collect();
        inputs.last().unwrap().type_text("talking");
    }
    h.run_steps(2);
    h.get_by_label_contains("Pad — Board talking points")
        .click();
    h.run_steps(2);

    let db = Db::open(&path).unwrap();
    let links = LinkStore::new(&db)
        .links_to(&EntityRef::new(EntityType::Note, pad.id))
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].source.kind, EntityType::Todo);
}
