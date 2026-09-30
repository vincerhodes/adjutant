//! Database handle: open/init, pragmas, path resolution.
//!
//! All SQL for the settings table lives in `settings.rs`; module SQL lives in
//! each module's `mod.rs`. UI files contain zero SQL.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub mod migrate;
pub mod settings;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("migration error: {0}")]
    Migration(String),
    #[error("settings error: {0}")]
    Settings(String),
}

pub type Result<T> = std::result::Result<T, DbError>;

pub struct Db {
    conn: Connection,
}

impl Db {
    /// Open (creating if needed) a database at `path`, set mandatory pragmas,
    /// and run pending migrations. Parent dirs are created with `0700`;
    /// the DB file is forced to `0600` post-create (covers WAL sidecars by
    /// inheritance of the data dir perms).
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
            set_dir_perms(parent)?;
        }
        let existed = path.exists();
        let conn = Connection::open(path)?;
        if !existed {
            // Best effort: file was just created by us.
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        migrate::run(&conn)?;
        Ok(Db { conn })
    }

    /// Open the database at the resolved default location.
    /// Resolution: `$ADJUTANT_DATA_DIR` → XDG data dir → `./adjutant.db`
    /// (last resort, loud warning).
    pub fn open_at_default() -> Result<Db> {
        let (path, warn) = default_path();
        if let Some(w) = warn {
            eprintln!("adjutant: WARNING: {w}");
        }
        Db::open(&path)
    }

    /// Default data dir (for the lock file, crash logs). `$ADJUTANT_DATA_DIR`
    /// if set, else the XDG data dir (created), else the current dir.
    pub fn data_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("ADJUTANT_DATA_DIR") {
            let p = PathBuf::from(dir);
            let _ = fs::create_dir_all(&p);
            let _ = set_dir_perms(&p);
            return p;
        }
        if let Some(dirs) = directories::ProjectDirs::from("net", "browzr", "adjutant") {
            let p = dirs.data_dir().to_path_buf();
            let _ = fs::create_dir_all(&p);
            let _ = set_dir_perms(&p);
            return p;
        }
        PathBuf::from(".")
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

/// Returns (path, optional warning).
fn default_path() -> (PathBuf, Option<String>) {
    if let Ok(dir) = std::env::var("ADJUTANT_DATA_DIR") {
        return (PathBuf::from(dir).join("adjutant.db"), None);
    }
    if let Some(dirs) = directories::ProjectDirs::from("net", "browzr", "adjutant") {
        return (dirs.data_dir().join("adjutant.db"), None);
    }
    (
        PathBuf::from("adjutant.db"),
        Some("XDG data dir unavailable; using ./adjutant.db".to_string()),
    )
}

#[cfg(unix)]
fn set_dir_perms(path: &Path) -> std::io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_dir_perms(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
