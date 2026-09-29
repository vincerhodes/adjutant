//! Schema + migration runner smoke tests (Step 2).

use adjutant::db::Db;

fn mem_db() -> Db {
    Db::open(std::path::Path::new(":memory:")).expect("in-memory db")
}

#[test]
fn fresh_db_has_all_tables_and_user_version_1() {
    let db = mem_db();
    let version: i64 = db
        .conn()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 1);

    let tables: Vec<String> = {
        let mut stmt = db
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let rows = stmt.query_map([], |row| row.get(0)).unwrap();
        let mut v = Vec::new();
        for r in rows {
            v.push(r.unwrap());
        }
        v
    };
    for expected in ["entity_links", "settings", "todo_groups", "todos"] {
        assert!(tables.iter().any(|t| t == expected), "missing table {expected}");
    }
}

#[test]
fn migrations_are_idempotent() {
    let db = mem_db();
    // Running migrations again (as every launch does) is a no-op.
    adjutant::db::migrate::run(db.conn()).expect("second migration run");
    let version: i64 = db
        .conn()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 1);
}

#[test]
fn updated_at_triggers_exist() {
    let db = mem_db();
    let triggers: Vec<String> = {
        let mut stmt = db
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger' ORDER BY name")
            .unwrap();
        let rows = stmt.query_map([], |row| row.get(0)).unwrap();
        let mut v = Vec::new();
        for r in rows {
            v.push(r.unwrap());
        }
        v
    };
    assert!(triggers.iter().any(|t| t == "trg_todos_updated"));
    assert!(triggers.iter().any(|t| t == "trg_todo_groups_updated"));
}

#[test]
fn settings_roundtrip() {
    let db = mem_db();
    assert!(db.get_setting::<String>("missing").unwrap().is_none());
    db.set_setting("window.maximized", &true).unwrap();
    db.set_setting("last_group", &"some-uuid").unwrap();
    assert!(db.get_setting::<bool>("window.maximized").unwrap().unwrap());
    assert_eq!(
        db.get_setting::<String>("last_group").unwrap().unwrap(),
        "some-uuid"
    );
    // Overwrite.
    db.set_setting("window.maximized", &false).unwrap();
    assert!(!db.get_setting::<bool>("window.maximized").unwrap().unwrap());
}
