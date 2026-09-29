//! Adjutant bootstrap: single-instance lock, crash-log panic hook, database
//! open + startup purge, eframe launch. Keep this file thin — everything
//! else lives in the library crate.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;

use adjutant::app::AdjutantApp;
use adjutant::db::Db;
use adjutant::todo::TodoStore;

fn main() -> ExitCode {
    let data_dir = Db::data_dir();
    let lock_path = data_dir.join("adjutant.lock");

    let _lock = match acquire_lock(&lock_path) {
        Ok(lock) => lock,
        Err(LockError::AlreadyRunning) => {
            eprintln!("Adjutant is already running");
            return ExitCode::from(1);
        }
        Err(LockError::Io(e)) => {
            eprintln!(
                "adjutant: cannot open lock file {}: {e}",
                lock_path.display()
            );
            return ExitCode::from(1);
        }
    };

    install_panic_hook(data_dir.clone());

    let db = match Db::open_at_default() {
        Ok(db) => db,
        Err(e) => {
            eprintln!("adjutant: database error: {e:#}");
            return ExitCode::from(1);
        }
    };

    // Startup trash purge: hard-delete rows trashed >30 days ago.
    match TodoStore::new(&db).purge_expired() {
        Ok(0) => {}
        Ok(n) => println!("adjutant: purged {n} expired trashed todo(s)"),
        Err(e) => eprintln!("adjutant: trash purge failed: {e:#}"),
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Adjutant")
            .with_inner_size([1200.0, 800.0])
            .with_app_id("adjutant"),
        ..Default::default()
    };

    match eframe::run_native(
        "adjutant",
        options,
        Box::new(|cc| Ok(Box::new(AdjutantApp::new(db, cc)))),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("adjutant: {e}");
            ExitCode::from(1)
        }
    }
}

enum LockError {
    AlreadyRunning,
    Io(std::io::Error),
}

/// Advisory exclusive lock on `<data_dir>/adjutant.lock`. A second GUI
/// instance exits non-zero instead of last-write-wins corrupting the DB.
#[cfg(unix)]
fn acquire_lock(path: &std::path::Path) -> Result<Mutex<File>, LockError> {
    use std::os::unix::io::AsRawFd;

    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(LockError::Io)?;
    let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::WouldBlock {
            return Err(LockError::AlreadyRunning);
        }
        return Err(LockError::Io(err));
    }
    Ok(Mutex::new(file))
}

#[cfg(not(unix))]
fn acquire_lock(path: &std::path::Path) -> Result<Mutex<File>, LockError> {
    // Best effort off-Unix: O_EXCL create.
    match OpenOptions::new().create_new(true).write(true).open(path) {
        Ok(file) => Ok(Mutex::new(file)),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(LockError::AlreadyRunning),
        Err(e) => Err(LockError::Io(e)),
    }
}

/// Panics must not vanish when launched from walker/rofi (no terminal):
/// write message + backtrace to `<data_dir>/crash-<timestamp>.log`.
/// RUST_BACKTRACE is honored by `Backtrace::capture`.
fn install_panic_hook(data_dir: PathBuf) {
    std::panic::set_hook(Box::new(move |info| {
        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = data_dir.join(format!("crash-{timestamp}.log"));
        let backtrace = std::backtrace::Backtrace::force_capture();
        let payload = if let Some(s) = info.payload_as_str() {
            s.to_string()
        } else {
            "non-string panic payload".to_string()
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());
        if let Ok(mut file) = File::create(&path) {
            let _ = writeln!(
                file,
                "panic: {payload}\nlocation: {location}\n\n{backtrace}"
            );
        }
        // Still show something on a terminal if there is one.
        eprintln!(
            "adjutant: panic: {payload} ({location}) — see {}",
            path.display()
        );
    }));
}
