//! Email module: sync engine, store, threading, compose, UI.
//!
//! Transport traits (`ImapTransport`/`SmtpTransport`) are the test seam —
//! live impls in `imap_client.rs`/`smtp.rs`, mocks in tests. No live
//! network in `cargo test`. Passwords live in the OS keyring only.

pub mod compose;
pub mod imap_client;
pub mod model;
pub mod smtp;
pub mod sync;
pub mod thread;
pub mod ui;

use thiserror::Error as ThisError;

/// Errors are sanitized before they reach logs/UI: IMAP/SMTP error strings
/// may echo credentials, so only the error KIND crosses this boundary.
#[derive(Debug, ThisError)]
pub enum EmailError {
    #[error("imap error: {0}")]
    Imap(String),
    #[error("smtp error: {0}")]
    Smtp(String),
    #[error("keyring error: {0}")]
    Keyring(String),
    #[error("database error: {0}")]
    Db(#[from] crate::db::DbError),
    #[error("sync engine error: {0}")]
    Engine(String),
    #[error("no secret service provider available")]
    NoSecretService,
}

/// Reduced, kind-only description of a transport error — never includes
/// command echoes (which can contain credentials).
pub fn sanitize_imap_error(e: &imap::Error) -> String {
    match e {
        imap::Error::Io(_) => "io".to_string(),
        imap::Error::RustlsHandshake(_) => "tls-handshake".to_string(),
        imap::Error::Bad(_) => "bad".to_string(),
        imap::Error::No(_) => "no".to_string(),
        imap::Error::Bye(_) => "bye".to_string(),
        imap::Error::ConnectionLost => "connection-lost".to_string(),
        imap::Error::Parse(_) => "parse".to_string(),
        imap::Error::Validate(_) => "validate".to_string(),
        imap::Error::Append => "append".to_string(),
        _ => "other".to_string(),
    }
}

pub fn sanitize_smtp_error(e: &lettre::transport::smtp::Error) -> String {
    // Variant name only — server responses can echo connection details.
    let dbg = format!("{e:?}");
    dbg.split('(').next().unwrap_or(&dbg).trim().to_string()
}
