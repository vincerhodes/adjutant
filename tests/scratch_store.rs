//! Scratch store integration tests against an in-memory database (spec §10):
//! CRUD, pin ordering, trash/restore/purge, link purge, search, validation.
#![allow(clippy::unwrap_used)]

use adjutant::core::entity::{EntityRef, EntityType};
use adjutant::core::link::{LinkStore, Relation};
use adjutant::db::Db;
use adjutant::scratch::{ScratchPadInput, ScratchStore};
use adjutant::todo::TodoStore;

fn mem_db() -> Db {
    Db::open(std::path::Path::new(":memory:")).expect("in-memory db")
}

fn input(body: &str) -> ScratchPadInput {
    ScratchPadInput {
        body: body.to_string(),
        pinned: false,
        color_idx: 0,
    }
}

#[test]
fn crud_and_title_fallback() {
    let db = mem_db();
    let store = ScratchStore::new(&db);
    let pad = store.create(&input("First line\nsecond line")).unwrap();
    assert_eq!(pad.title(), "First line");
    assert!(!pad.pinned);
    assert_eq!(pad.color_idx, 0);

    store.update(pad.id, "Edited body\nmore").unwrap();
    assert_eq!(store.get(pad.id).unwrap().body, "Edited body\nmore");

    // All-whitespace body falls back to the "(empty pad)" title.
    store.update(pad.id, "\n   \n").unwrap();
    assert_eq!(store.get(pad.id).unwrap().title(), "(empty pad)");

    store.delete(pad.id).unwrap();
    assert!(store.get(pad.id).is_err());
}

#[test]
fn list_orders_pinned_first_then_updated_desc() {
    let db = mem_db();
    let store = ScratchStore::new(&db);
    let a = store.create(&input("alpha")).unwrap();
    let b = store.create(&input("bravo")).unwrap();
    let c = store.create(&input("charlie")).unwrap();
    // Touch a so its updated_at is newest; pin c.
    store.update(a.id, "alpha edited").unwrap();
    store.set_pinned(c.id, true).unwrap();

    let ids: Vec<uuid::Uuid> = store.list().unwrap().into_iter().map(|p| p.id).collect();
    assert_eq!(
        ids,
        vec![c.id, a.id, b.id],
        "pinned first, then updated desc"
    );

    // Unpinning returns c to the recency ordering.
    store.set_pinned(c.id, false).unwrap();
    let ids: Vec<uuid::Uuid> = store.list().unwrap().into_iter().map(|p| p.id).collect();
    assert_eq!(ids, vec![a.id, c.id, b.id]);
}

#[test]
fn color_validation() {
    let db = mem_db();
    let store = ScratchStore::new(&db);
    assert!(store
        .create(&ScratchPadInput {
            color_idx: 6,
            ..input("bad")
        })
        .is_err());
    assert!(store
        .create(&ScratchPadInput {
            color_idx: -1,
            ..input("bad")
        })
        .is_err());
    let pad = store.create(&input("ok")).unwrap();
    assert!(store.set_color(pad.id, 5).is_ok());
    assert!(store.set_color(pad.id, 6).is_err());
    assert_eq!(store.get(pad.id).unwrap().color_idx, 5);
}

#[test]
fn trash_restore_and_hard_delete_purges_links() {
    let db = mem_db();
    let store = ScratchStore::new(&db);
    let todos = TodoStore::new(&db);
    let links = LinkStore::new(&db);
    let group = todos.create_group("G").unwrap();
    let todo = todos.create(group.id, None, "related work").unwrap();
    let pad = store.create(&input("pad body")).unwrap();

    links
        .link(
            &EntityRef::new(EntityType::Todo, todo.id),
            &EntityRef::new(EntityType::Note, pad.id),
            &Relation::from(Relation::MENTIONS),
        )
        .unwrap();

    store.trash(pad.id).unwrap();
    assert!(store.list().unwrap().is_empty());
    assert_eq!(store.list_trashed().unwrap().len(), 1);
    // Soft delete keeps the link resolvable.
    assert_eq!(
        links
            .links_to(&EntityRef::new(EntityType::Note, pad.id))
            .unwrap()
            .len(),
        1
    );

    store.restore(pad.id).unwrap();
    assert_eq!(store.list().unwrap().len(), 1);

    store.delete(pad.id).unwrap();
    let remaining = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM entity_links
             WHERE (source_type = 'note' AND source_id = ?1)
                OR (target_type = 'note' AND target_id = ?1)",
            [pad.id.to_string()],
            |r| r.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(remaining, 0, "hard delete purges links both directions");
}

#[test]
fn purge_expired_hard_deletes_old_trashed_pads() {
    let db = mem_db();
    let store = ScratchStore::new(&db);
    let old = store.create(&input("old pad")).unwrap().id;
    let fresh = store.create(&input("fresh pad")).unwrap().id;
    store.trash(old).unwrap();
    store.trash(fresh).unwrap();
    db.conn()
        .execute(
            "UPDATE scratch_notes SET trashed_at = '2026-01-01T00:00:00Z' WHERE id = ?1",
            [old.to_string()],
        )
        .unwrap();

    let purged = store.purge_expired().unwrap();
    assert_eq!(purged, 1);
    assert!(store.get(old).is_err());
    assert!(store.get(fresh).unwrap().trashed_at.is_some());
}

#[test]
fn search_matches_body_and_skips_trashed() {
    let db = mem_db();
    let store = ScratchStore::new(&db);
    store.create(&input("buy milk tomorrow")).unwrap();
    let b = store.create(&input("MILK in caps")).unwrap();
    let c = store.create(&input("unrelated")).unwrap().id;
    store.trash(c).unwrap();

    let hits = store.search("milk", 10).unwrap();
    assert_eq!(hits.len(), 2, "case-insensitive LIKE");
    assert!(hits.iter().any(|(id, _)| *id == b.id));
    let none = store.search("unrelated", 10).unwrap();
    assert!(none.is_empty(), "trashed pads are excluded");
}
