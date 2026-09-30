//! Account form regression: edit form must be fully seeded from the stored
//! account; saving without edits must not change anything.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::Arc;

use adjutant::app::AdjutantApp;
use adjutant::db::Db;
use adjutant::email::imap_client::{FolderState, ImapTransport, RemoteFolder, RemoteMail};
use adjutant::email::smtp::SmtpTransport;
use adjutant::email::sync::{ImapFactory, SmtpFactory, SyncEngine};
use adjutant::email::{EmailError, EmailStore, NewAccount};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

#[derive(Clone, Default)]
struct MockImap;

impl ImapTransport for MockImap {
    fn capabilities(&mut self) -> Result<Vec<String>, EmailError> {
        Ok(vec![])
    }
    fn list_folders(&mut self) -> Result<Vec<RemoteFolder>, EmailError> {
        Ok(vec![])
    }
    fn select(&mut self, _folder: &str) -> Result<FolderState, EmailError> {
        Ok(FolderState {
            uidvalidity: Some(1),
        })
    }
    fn fetch_headers(
        &mut self,
        _uid_start: u32,
        _uid_end: Option<u32>,
    ) -> Result<Vec<RemoteMail>, EmailError> {
        Ok(vec![])
    }
    fn fetch_full(&mut self, _uid: u32) -> Result<RemoteMail, EmailError> {
        Err(EmailError::Imap("none".to_string()))
    }
    fn all_uids(&mut self) -> Result<Vec<u32>, EmailError> {
        Ok(vec![])
    }
    fn store_flags(&mut self, _uid: u32, _flags: &[String]) -> Result<(), EmailError> {
        Ok(())
    }
    fn move_mail(&mut self, _uid: u32, _target: &str) -> Result<(), EmailError> {
        Ok(())
    }
    fn append(
        &mut self,
        _folder: &str,
        _content: &[u8],
        _flags: &[String],
    ) -> Result<(), EmailError> {
        Ok(())
    }
    fn noop(&mut self) -> Result<(), EmailError> {
        Ok(())
    }
    fn logout(&mut self) -> Result<(), EmailError> {
        Ok(())
    }
}

#[derive(Clone, Default)]
struct MockSmtp;

impl SmtpTransport for MockSmtp {
    fn send_rfc822(
        &mut self,
        _from: &str,
        _to: &[String],
        _cc: &[String],
        _bcc: &[String],
        _rfc822: &[u8],
    ) -> Result<(), EmailError> {
        Ok(())
    }
}

fn temp_db_path(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("adjutant-acct-form-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("adjutant.db")
}

#[test]
fn edit_form_seeds_all_fields_and_pristine_save_preserves_them() {
    let path = temp_db_path("seed");
    let db = Db::open(&path).unwrap();
    let store = EmailStore::new(&db);
    let account = store
        .create_account(&NewAccount {
            label: "Purelymail",
            address: "jim@browzr.net",
            imap_host: "imap.purelymail.com",
            imap_port: 993,
            smtp_host: "smtp.purelymail.com",
            smtp_port: 465,
            username: "jim@browzr.net",
            color_idx: 0,
            sync_interval_s: 300,
        })
        .unwrap();

    let ifac: ImapFactory = Arc::new(move |_| Ok(Box::new(MockImap)));
    let sfac: SmtpFactory = Arc::new(move |_| Ok(Box::new(MockSmtp)));
    let engine = SyncEngine::start(path.clone(), ifac, sfac);
    let db2 = Db::open(&path).unwrap();
    let mut h = Harness::builder()
        .with_size([1400.0, 900.0])
        .build_eframe(move |cc| AdjutantApp::new_with_engine(db2, cc, Some(engine)));
    h.run_steps(2);
    h.get_by_label("Email").click();
    h.run_steps(3);
    h.get_by_label_contains("Manage accounts").click();
    h.run_steps(3);
    h.get_by_label("Edit").click();
    h.run_steps(3);

    // Field order: Label, Email address, IMAP host, IMAP port, SMTP host,
    // SMTP port, Username, [Password is a PasswordInput], Sync interval.
    let inputs: Vec<_> = h.query_all_by_role(Role::TextInput).collect();
    assert_eq!(inputs.len(), 8, "non-password account form fields");
    assert_eq!(
        h.query_all_by_role(Role::PasswordInput).count(),
        1,
        "password field is a password input"
    );
    assert_eq!(
        inputs[0].value().as_deref(),
        Some("Purelymail"),
        "label seeded"
    );
    assert_eq!(
        inputs[1].value().as_deref(),
        Some("jim@browzr.net"),
        "address seeded"
    );
    assert_eq!(
        inputs[2].value().as_deref(),
        Some("imap.purelymail.com"),
        "imap host seeded"
    );
    assert_eq!(
        inputs[3].value().as_deref(),
        Some("993"),
        "imap port seeded"
    );
    assert_eq!(
        inputs[4].value().as_deref(),
        Some("smtp.purelymail.com"),
        "smtp host seeded"
    );
    assert_eq!(
        inputs[5].value().as_deref(),
        Some("465"),
        "smtp port seeded"
    );
    assert_eq!(
        inputs[6].value().as_deref(),
        Some("jim@browzr.net"),
        "username seeded — the live-reported bug"
    );
    assert_eq!(
        inputs[7].value().as_deref(),
        Some("300"),
        "sync interval seeded"
    );

    // Save without edits → nothing changes.
    h.get_by_label("Save").click();
    h.run_steps(3);
    let store = EmailStore::new(&db);
    let after = store.get_account(account.id).unwrap();
    assert_eq!(after.username, "jim@browzr.net");
    assert_eq!(after.imap_host, "imap.purelymail.com");
    assert_eq!(after.imap_port, 993);
    assert_eq!(after.smtp_host, "smtp.purelymail.com");
    assert_eq!(after.label, "Purelymail");
    assert_eq!(after.sync_interval_s, 300);
}

#[test]
fn save_validation_refuses_empty_username_and_host() {
    let path = temp_db_path("validation");
    let db = Db::open(&path).unwrap();
    let store = EmailStore::new(&db);
    store
        .create_account(&NewAccount {
            label: "Purelymail",
            address: "jim@browzr.net",
            imap_host: "imap.purelymail.com",
            imap_port: 993,
            smtp_host: "smtp.purelymail.com",
            smtp_port: 465,
            username: "jim@browzr.net",
            color_idx: 0,
            sync_interval_s: 300,
        })
        .unwrap();

    let ifac: ImapFactory = Arc::new(move |_| Ok(Box::new(MockImap)));
    let sfac: SmtpFactory = Arc::new(move |_| Ok(Box::new(MockSmtp)));
    let engine = SyncEngine::start(path.clone(), ifac, sfac);
    let db2 = Db::open(&path).unwrap();
    let mut h = Harness::builder()
        .with_size([1400.0, 900.0])
        .build_eframe(move |cc| AdjutantApp::new_with_engine(db2, cc, Some(engine)));
    h.run_steps(2);
    h.get_by_label("Email").click();
    h.run_steps(3);
    h.get_by_label_contains("Manage accounts").click();
    h.run_steps(3);
    h.get_by_label("Edit").click();
    h.run_steps(3);

    // Clear the username field, save → inline error, DB untouched.
    {
        let inputs: Vec<_> = h.query_all_by_role(Role::TextInput).collect();
        inputs[6].focus();
    }
    h.run_steps(1);
    for _ in 0..20 {
        h.key_press(egui::Key::Backspace);
        h.run_steps(1);
    }
    h.get_by_label("Save").click();
    h.run_steps(3);
    assert_eq!(
        h.query_all_by_label_contains("Username is required")
            .count(),
        1,
        "inline validation error shown"
    );
    let store = EmailStore::new(&db);
    let account = store.list_accounts().unwrap().pop().unwrap();
    assert_eq!(account.username, "jim@browzr.net", "DB unchanged");
}
