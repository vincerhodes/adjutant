//! Link layer integration tests against an in-memory database.
#![allow(clippy::unwrap_used)]

use adjutant::core::entity::{EntityRef, EntityType};
use adjutant::core::link::{LinkStore, Relation};
use adjutant::db::Db;
use uuid::Uuid;

fn mem_db() -> Db {
    Db::open(std::path::Path::new(":memory:")).expect("in-memory db")
}

fn todo_ref() -> EntityRef {
    EntityRef::new(EntityType::Todo, Uuid::new_v4())
}

#[test]
fn create_and_query_both_directions() {
    let db = mem_db();
    let links = LinkStore::new(&db);
    let a = todo_ref();
    let b = todo_ref();
    links.link(&a, &b, &Relation::blocks()).expect("link a→b");

    let from = links.links_from(&a).expect("links_from");
    assert_eq!(from.len(), 1);
    assert_eq!(from[0].target, b);
    assert_eq!(from[0].relation.as_str(), "blocks");

    let to = links.links_to(&b).expect("links_to");
    assert_eq!(to.len(), 1);
    assert_eq!(to[0].source, a);
}

#[test]
fn unlink_removes_link() {
    let db = mem_db();
    let links = LinkStore::new(&db);
    let a = todo_ref();
    let b = todo_ref();
    links.link(&a, &b, &Relation::blocks()).unwrap();
    links.unlink(&a, &b, &Relation::blocks()).expect("unlink");
    assert!(links.links_from(&a).unwrap().is_empty());
    // Unlinking a missing link errors.
    assert!(links.unlink(&a, &b, &Relation::blocks()).is_err());
}

#[test]
fn duplicate_link_rejected() {
    let db = mem_db();
    let links = LinkStore::new(&db);
    let a = todo_ref();
    let b = todo_ref();
    links.link(&a, &b, &Relation::blocks()).unwrap();
    let err = links.link(&a, &b, &Relation::blocks()).unwrap_err();
    assert!(matches!(err, adjutant::core::link::LinkError::Duplicate));
}

#[test]
fn blocks_cycle_rejected() {
    let db = mem_db();
    let links = LinkStore::new(&db);
    let a = todo_ref();
    let b = todo_ref();
    let c = todo_ref();
    links.link(&a, &b, &Relation::blocks()).unwrap(); // a blocks b
    links.link(&b, &c, &Relation::blocks()).unwrap(); // b blocks c
                                                      // c → a would close a→b→c→a.
    let err = links.link(&c, &a, &Relation::blocks()).unwrap_err();
    assert!(matches!(err, adjutant::core::link::LinkError::Cycle));

    // Transitive: x blocks a, so c blocks x would close c→x→a→b→c.
    let x = todo_ref();
    links.link(&x, &a, &Relation::blocks()).unwrap();
    let err = links.link(&c, &x, &Relation::blocks()).unwrap_err();
    assert!(matches!(err, adjutant::core::link::LinkError::Cycle));
}

#[test]
fn self_link_rejected() {
    let db = mem_db();
    let links = LinkStore::new(&db);
    let a = todo_ref();
    assert!(links.link(&a, &a, &Relation::blocks()).is_err());
}

#[test]
fn mentions_cycles_allowed() {
    let db = mem_db();
    let links = LinkStore::new(&db);
    let a = todo_ref();
    let b = todo_ref();
    links
        .link(&a, &b, &Relation::from(Relation::MENTIONS))
        .unwrap();
    // Cyclic mention is fine.
    links
        .link(&b, &a, &Relation::from(Relation::MENTIONS))
        .unwrap();
    assert_eq!(links.links_from(&a).unwrap().len(), 1);
    assert_eq!(links.links_from(&b).unwrap().len(), 1);
}

#[test]
fn dangling_target_tolerated() {
    let db = mem_db();
    let links = LinkStore::new(&db);
    let a = todo_ref();
    let ghost = todo_ref(); // no row in todos — links are loose by design
    links.link(&a, &ghost, &Relation::blocks()).unwrap();
    let from = links.links_from(&a).unwrap();
    assert_eq!(from.len(), 1);
    assert_eq!(from[0].target, ghost);
}

#[test]
fn custom_relation_roundtrip() {
    let db = mem_db();
    let links = LinkStore::new(&db);
    let a = todo_ref();
    let b = todo_ref();
    let rel = Relation::custom("related_to");
    links.link(&a, &b, &rel).unwrap();
    assert_eq!(
        links.links_from(&a).unwrap()[0].relation.as_str(),
        "related_to"
    );
}

#[test]
fn mixed_relations_same_pair_coexist() {
    let db = mem_db();
    let links = LinkStore::new(&db);
    let a = todo_ref();
    let b = todo_ref();
    links.link(&a, &b, &Relation::blocks()).unwrap();
    links
        .link(&a, &b, &Relation::from(Relation::MENTIONS))
        .unwrap();
    assert_eq!(links.links_from(&a).unwrap().len(), 2);
}
