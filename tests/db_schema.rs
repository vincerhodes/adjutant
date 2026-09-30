//! Schema + migration runner smoke tests (Step 2).
#![allow(clippy::unwrap_used)]

use adjutant::db::Db;

fn mem_db() -> Db {
    Db::open(std::path::Path::new(":memory:")).expect("in-memory db")
}

#[test]
fn fresh_db_has_all_tables_and_user_versions() {
    let db = mem_db();
    let version: i64 = db
        .conn()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 4);

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
        assert!(
            tables.iter().any(|t| t == expected),
            "missing table {expected}"
        );
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
    assert_eq!(version, 4);
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

#[cfg(unix)]
#[test]
fn file_and_dir_permissions_are_restrictive() {
    use std::os::unix::fs::PermissionsExt;

    let root = std::env::temp_dir().join(format!(
        "adjutant-perm-test-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let db_path = root.join("nested").join("adjutant.db");
    let db = Db::open(&db_path).expect("open");

    let dir_mode = std::fs::metadata(root.join("nested"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let file_mode = std::fs::metadata(&db_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700, "dir mode");
    assert_eq!(file_mode, 0o600, "file mode");

    // Sanity: migrations ran on the on-disk DB too.
    let version: i64 = db
        .conn()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 4);

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
}
