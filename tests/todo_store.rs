//! Todo store integration tests against an in-memory database.
#![allow(clippy::unwrap_used)]

use adjutant::core::entity::{EntityRef, EntityType};
use adjutant::core::link::{LinkStore, Relation};
use adjutant::db::Db;
use adjutant::todo::{Status, TodoError, TodoStore};
use chrono::{Duration, NaiveDate, Utc};

fn mem_db() -> Db {
    Db::open(std::path::Path::new(":memory:")).expect("in-memory db")
}

#[test]
fn groups_crud_and_ordering() {
    let db = mem_db();
    let store = TodoStore::new(&db);
    store.create_group("Personal").unwrap();
    let b = store.create_group("Browzr").unwrap();
    store.create_group("Hobby").unwrap();

    let names: Vec<String> = store
        .list_groups()
        .unwrap()
        .into_iter()
        .map(|g| g.name)
        .collect();
    assert_eq!(names, ["Personal", "Browzr", "Hobby"]);

    store.rename_group(b.id, "Browzr Ltd").unwrap();
    let names: Vec<String> = store
        .list_groups()
        .unwrap()
        .into_iter()
        .map(|g| g.name)
        .collect();
    assert_eq!(names, ["Personal", "Browzr Ltd", "Hobby"]);

    store.delete_group(b.id).unwrap();
    let names: Vec<String> = store
        .list_groups()
        .unwrap()
        .into_iter()
        .map(|g| g.name)
        .collect();
    assert_eq!(names, ["Personal", "Hobby"]);
}

#[test]
fn tree_build_and_nesting_four_deep() {
    let db = mem_db();
    let store = TodoStore::new(&db);
    let g = store.create_group("G").unwrap();
    let t1 = store.create(g.id, None, "level 1").unwrap();
    let t2 = store.create(g.id, Some(t1.id), "level 2").unwrap();
    let t3 = store.create(g.id, Some(t2.id), "level 3").unwrap();
    let t4 = store.create(g.id, Some(t3.id), "level 4").unwrap();
    let _ = t4;

    let tree = store.tree(g.id).unwrap();
    assert_eq!(tree.len(), 1);
    let n1 = &tree[0];
    assert_eq!(n1.todo.title, "level 1");
    assert_eq!(n1.children.len(), 1);
    let n2 = &n1.children[0];
    assert_eq!(n2.children.len(), 1);
    let n3 = &n2.children[0];
    assert_eq!(n3.children.len(), 1);
    assert_eq!(n3.children[0].todo.title, "level 4");
}

#[test]
fn completion_rule_blocks_parent_with_open_child() {
    let db = mem_db();
    let store = TodoStore::new(&db);
    let g = store.create_group("G").unwrap();
    let parent = store.create(g.id, None, "parent").unwrap();
    let child = store.create(g.id, Some(parent.id), "child").unwrap();
    let grandchild = store.create(g.id, Some(child.id), "grandchild").unwrap();

    // Parent cannot complete while any descendant is open.
    let err = store.set_status(parent.id, Status::Done).unwrap_err();
    match err {
        TodoError::OpenDescendants(n) => assert_eq!(n, 2),
        other => panic!("expected OpenDescendants, got {other:?}"),
    }

    // Cancelling a descendant counts as terminal.
    store.set_status(grandchild.id, Status::Cancelled).unwrap();
    let err = store.set_status(parent.id, Status::Done).unwrap_err();
    assert!(matches!(err, TodoError::OpenDescendants(1)));

    store.set_status(child.id, Status::Done).unwrap();
    let done = store.set_status(parent.id, Status::Done).unwrap();
    assert_eq!(done.status, Status::Done);
    assert!(done.completed_at.is_some());

    // Reopening a parent is allowed even with terminal descendants.
    let reopened = store.set_status(parent.id, Status::Open).unwrap();
    assert!(reopened.completed_at.is_none());
}

#[test]
fn due_date_set_clear_and_overdue() {
    let db = mem_db();
    let store = TodoStore::new(&db);
    let g = store.create_group("G").unwrap();
    let t = store.create(g.id, None, "task").unwrap();

    let yesterday = Utc::now().date_naive() - Duration::days(1);
    let updated = store
        .update(t.id, None, None, None, Some(Some(yesterday)))
        .unwrap();
    assert_eq!(updated.due_date, Some(yesterday));
    assert_eq!(store.overdue().unwrap().len(), 1);

    // Terminal items are not overdue.
    store.set_status(t.id, Status::Done).unwrap();
    assert!(store.overdue().unwrap().is_empty());

    // Clearing the due date drops it from overdue queries.
    let cleared = store.update(t.id, None, None, None, Some(None)).unwrap();
    assert_eq!(cleared.due_date, None);

    // A future date is not overdue.
    let tomorrow: NaiveDate = Utc::now().date_naive() + Duration::days(1);
    store
        .update(t.id, None, None, None, Some(Some(tomorrow)))
        .unwrap();
    assert!(store.overdue().unwrap().is_empty());
}

#[test]
fn trash_hides_from_tree_restore_keeps_everything() {
    let db = mem_db();
    let store = TodoStore::new(&db);
    let links = LinkStore::new(&db);
    let g = store.create_group("G").unwrap();
    let parent = store.create(g.id, None, "parent").unwrap();
    let child = store.create(g.id, Some(parent.id), "child").unwrap();
    let other = store.create(g.id, None, "other").unwrap();

    // Link child → other; trash must retain links.
    let child_ref = EntityRef::new(EntityType::Todo, child.id);
    let other_ref = EntityRef::new(EntityType::Todo, other.id);
    links
        .link(&child_ref, &other_ref, &Relation::blocks())
        .unwrap();

    store.trash(parent.id).unwrap();

    let tree = store.tree(g.id).unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].todo.id, other.id);

    let trash = store.list_trash().unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].id, parent.id);

    store.restore(parent.id).unwrap();
    let tree = store.tree(g.id).unwrap();
    assert_eq!(tree.len(), 2);
    let restored_parent = tree.iter().find(|n| n.todo.id == parent.id).unwrap();
    assert_eq!(restored_parent.children.len(), 1);
    // Links intact after the round-trip.
    assert_eq!(links.links_from(&child_ref).unwrap().len(), 1);
    assert_eq!(links.links_to(&other_ref).unwrap().len(), 1);
}

#[test]
fn permanent_delete_removes_rows_and_links() {
    let db = mem_db();
    let store = TodoStore::new(&db);
    let links = LinkStore::new(&db);
    let g = store.create_group("G").unwrap();
    let parent = store.create(g.id, None, "parent").unwrap();
    let child = store.create(g.id, Some(parent.id), "child").unwrap();

    let child_ref = EntityRef::new(EntityType::Todo, child.id);
    let parent_ref = EntityRef::new(EntityType::Todo, parent.id);
    links
        .link(&child_ref, &parent_ref, &Relation::blocks())
        .unwrap();

    store.trash(parent.id).unwrap();
    store.delete_permanent(parent.id).unwrap();

    assert!(store.get(parent.id).is_err());
    assert!(store.get(child.id).is_err());
    assert!(links.links_to(&parent_ref).unwrap().is_empty());
    assert!(links.links_from(&child_ref).unwrap().is_empty());
    assert!(store.list_trash().unwrap().is_empty());
}

#[test]
fn purge_removes_only_rows_older_than_30_days() {
    let db = mem_db();
    let store = TodoStore::new(&db);
    let g = store.create_group("G").unwrap();

    // Fresh trash must survive the startup purge.
    let fresh = store.create(g.id, None, "fresh").unwrap();
    store.trash(fresh.id).unwrap();

    // Backdate a trashed row to 31 days ago (simulates age).
    let old = store.create(g.id, None, "old").unwrap();
    store.trash(old.id).unwrap();
    let stale = (Utc::now() - Duration::days(31)).to_rfc3339();
    db.conn()
        .execute(
            "UPDATE todos SET deleted_at = ?2 WHERE id = ?1",
            rusqlite::params![old.id.to_string(), stale],
        )
        .unwrap();

    let purged = store.purge_expired().unwrap();
    assert_eq!(purged, 1);
    assert!(store.get(old.id).is_err());
    assert!(store.get(fresh.id).is_ok());
    assert_eq!(store.list_trash().unwrap().len(), 1);
}

#[test]
fn reorder_swaps_sibling_positions() {
    let db = mem_db();
    let store = TodoStore::new(&db);
    let g = store.create_group("G").unwrap();
    let a = store.create(g.id, None, "a").unwrap();
    let b = store.create(g.id, None, "b").unwrap();
    let c = store.create(g.id, None, "c").unwrap();

    let titles = |store: &TodoStore| -> Vec<String> {
        store
            .tree(g.id)
            .unwrap()
            .into_iter()
            .map(|n| n.todo.title)
            .collect()
    };
    assert_eq!(titles(&store), ["a", "b", "c"]);

    store.reorder(c.id, true).unwrap();
    assert_eq!(titles(&store), ["a", "c", "b"]);

    // Edge: moving up at the top is a no-op.
    store.reorder(a.id, true).unwrap();
    assert_eq!(titles(&store), ["a", "c", "b"]);

    store.reorder(a.id, false).unwrap();
    assert_eq!(titles(&store), ["c", "a", "b"]);

    // Edge: moving down at the bottom is a no-op.
    store.reorder(b.id, false).unwrap();
    assert_eq!(titles(&store), ["c", "a", "b"]);

    // Reorder is scoped to siblings: another parent's child is unaffected.
    let p = store.create(g.id, None, "p").unwrap();
    let child = store.create(g.id, Some(p.id), "p-child").unwrap();
    store.reorder(child.id, true).unwrap(); // no-op: only child
    let tree = store.tree(g.id).unwrap();
    let pn = tree.iter().find(|n| n.todo.id == p.id).unwrap();
    assert_eq!(pn.children.len(), 1);
}

#[test]
fn updated_at_trigger_fires_on_update_not_insert() {
    let db = mem_db();
    let store = TodoStore::new(&db);
    let g = store.create_group("G").unwrap();
    let t = store.create(g.id, None, "task").unwrap();
    assert_eq!(t.created_at, t.updated_at);

    // Give the trigger a distinguishable later moment.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let updated = store
        .update(t.id, Some("renamed"), None, None, None)
        .unwrap();
    assert_ne!(updated.created_at, updated.updated_at);
    assert_eq!(updated.title, "renamed");
}

#[test]
fn trash_sets_same_timestamp_across_subtree() {
    let db = mem_db();
    let store = TodoStore::new(&db);
    let g = store.create_group("G").unwrap();
    let p = store.create(g.id, None, "p").unwrap();
    let c = store.create(g.id, Some(p.id), "c").unwrap();

    store.trash(p.id).unwrap();
    let p2 = store.get(p.id).unwrap();
    let c2 = store.get(c.id).unwrap();
    assert!(p2.deleted_at.is_some());
    assert_eq!(p2.deleted_at, c2.deleted_at);
}

#[test]
fn update_and_status_rejected_on_trashed() {
    let db = mem_db();
    let store = TodoStore::new(&db);
    let g = store.create_group("G").unwrap();
    let t = store.create(g.id, None, "t").unwrap();
    store.trash(t.id).unwrap();
    assert!(matches!(
        store.update(t.id, Some("x"), None, None, None),
        Err(TodoError::Trashed)
    ));
    assert!(matches!(
        store.set_status(t.id, Status::Done),
        Err(TodoError::Trashed)
    ));
}
