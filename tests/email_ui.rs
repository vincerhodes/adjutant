//! Email UI tests (kittest): unified vs per-account view, unread counts,
//! outbox approve-all, compose staging. Mock sync engine — no network.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adjutant::app::AdjutantApp;
use adjutant::db::Db;
use adjutant::email::imap_client::{FolderState, ImapTransport, RemoteFolder, RemoteMail};
use adjutant::email::model::{FolderRole, OutboxState};
use adjutant::email::smtp::SmtpTransport;
use adjutant::email::sync::{ImapFactory, SmtpFactory, SyncEngine};
use adjutant::email::{EmailError, EmailStore, NewAccount, NewOutbox};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

#[derive(Default)]
struct MockImapState {
    folders: Vec<RemoteFolder>,
}

#[derive(Clone, Default)]
struct MockImap {
    state: Arc<Mutex<MockImapState>>,
}

impl ImapTransport for MockImap {
    fn capabilities(&mut self) -> Result<Vec<String>, EmailError> {
        Ok(vec!["IMAP4rev1".to_string()])
    }
    fn list_folders(&mut self) -> Result<Vec<RemoteFolder>, EmailError> {
        Ok(self.state.lock().unwrap().folders.clone())
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
        Err(EmailError::Imap("no bodies".to_string()))
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
struct MockSmtp {
    log: Arc<Mutex<Vec<String>>>,
}

impl SmtpTransport for MockSmtp {
    fn send_rfc822(
        &mut self,
        from: &str,
        to: &[String],
        _cc: &[String],
        _bcc: &[String],
        _rfc822: &[u8],
    ) -> Result<(), EmailError> {
        self.log.lock().unwrap().push(format!("{from}->{to:?}"));
        Ok(())
    }
}

fn temp_db_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("adjutant-email-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("adjutant.db")
}

fn mail(uid: u32, subject: &str, from: &str, seen: bool) -> RemoteMail {
    RemoteMail {
        uid,
        subject: subject.to_string(),
        from_addr: from.to_string(),
        from_name: None,
        date: format!("2026-10-{uid:02}T10:00:00+00:00"),
        flags: if seen {
            vec!["seen".to_string()]
        } else {
            vec![]
        },
        ..Default::default()
    }
}

struct Fixture {
    db: Db,
    account_a: adjutant::email::model::EmailAccount,
    #[allow(dead_code)]
    account_b: adjutant::email::model::EmailAccount,
    path: PathBuf,
    smtp: MockSmtp,
}

fn fixture(name: &str) -> Fixture {
    let path = temp_db_path(name);
    let db = Db::open(&path).unwrap();
    let store = EmailStore::new(&db);
    let account_a = store
        .create_account(&NewAccount {
            label: "Purelymail",
            address: "jim@browzr.net",
            imap_host: "imap.mock",
            imap_port: 993,
            smtp_host: "smtp.mock",
            smtp_port: 465,
            username: "jim@browzr.net",
            color_idx: 0,
            sync_interval_s: 60,
        })
        .unwrap();
    let account_b = store
        .create_account(&NewAccount {
            label: "Side",
            address: "side@example.com",
            imap_host: "imap.mock",
            imap_port: 993,
            smtp_host: "smtp.mock",
            smtp_port: 465,
            username: "side@example.com",
            color_idx: 1,
            sync_interval_s: 60,
        })
        .unwrap();
    for (account, host) in [(account_a.id, "INBOX"), (account_b.id, "INBOX")] {
        store
            .upsert_folder(account, host, FolderRole::Inbox)
            .unwrap();
    }
    let fa = store
        .folder_by_name(account_a.id, "INBOX")
        .unwrap()
        .unwrap();
    let fb = store
        .folder_by_name(account_b.id, "INBOX")
        .unwrap()
        .unwrap();
    // A: two unread, one read. B: one unread.
    store
        .ingest(&adjutant::email::IngestMail {
            account_id: account_a.id,
            folder_id: fa.id,
            mail: mail(1, "Alpha uno", "ann@x.com", false),
        })
        .unwrap();
    store
        .ingest(&adjutant::email::IngestMail {
            account_id: account_a.id,
            folder_id: fa.id,
            mail: mail(2, "Alpha dos", "bob@x.com", false),
        })
        .unwrap();
    store
        .ingest(&adjutant::email::IngestMail {
            account_id: account_a.id,
            folder_id: fa.id,
            mail: mail(3, "Alpha read", "cas@x.com", true),
        })
        .unwrap();
    store
        .ingest(&adjutant::email::IngestMail {
            account_id: account_b.id,
            folder_id: fb.id,
            mail: mail(1, "Beta uno", "dee@x.com", false),
        })
        .unwrap();
    Fixture {
        db,
        account_a,
        account_b,
        path,
        smtp: MockSmtp::default(),
    }
}

fn boot(fx: &Fixture) -> Harness<'static, AdjutantApp> {
    let imap = MockImap::default();
    let smtp = fx.smtp.clone();
    let ifac: ImapFactory = Arc::new(move |_| Ok(Box::new(imap.clone())));
    let sfac: SmtpFactory = Arc::new(move |_| Ok(Box::new(smtp.clone())));
    let engine = SyncEngine::start(fx.path.clone(), ifac, sfac);
    let db2 = Db::open(&fx.path).unwrap();
    Harness::builder()
        .with_size([1400.0, 900.0])
        .build_eframe(move |cc| AdjutantApp::new_with_engine(db2, cc, Some(engine)))
}

fn wait_until(deadline: Duration, mut pred: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if pred() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("condition not met within {deadline:?}");
}

#[test]
fn unified_vs_per_account_and_unread_counts() {
    let fx = fixture("merged");
    let mut h = boot(&fx);
    h.run_steps(2);
    // Switch to the Email module.
    h.get_by_label("Email").click();
    h.run_steps(3);

    // Sidebar: Unified with the combined unread count + per-account counts.
    assert_eq!(h.query_all_by_label_contains("Unified (3)").count(), 1);
    assert_eq!(h.query_all_by_label_contains("Purelymail (2)").count(), 1);
    assert_eq!(h.query_all_by_label_contains("Side (1)").count(), 1);

    // Unified view shows both accounts' threads.
    assert_eq!(h.query_all_by_label("Alpha uno").count(), 1);
    assert_eq!(h.query_all_by_label("Beta uno").count(), 1);

    // Switch to account B: only its thread remains.
    h.get_by_label_contains("Side (1)").click();
    h.run_steps(3);
    assert_eq!(h.query_all_by_label("Beta uno").count(), 1);
    assert_eq!(
        h.query_all_by_label("Alpha uno").count(),
        0,
        "per-account view hides other accounts' threads"
    );
    h.state().db(); // keep harness state referenced
}

#[test]
fn outbox_approve_all_sends_everything() {
    let fx = fixture("outbox");
    let store = EmailStore::new(&fx.db);
    for subject in ["First", "Second"] {
        store
            .stage_outbox(&NewOutbox {
                account_id: fx.account_a.id,
                to: vec![adjutant::email::model::MailAddress {
                    name: None,
                    addr: "sam@x.com".to_string(),
                }],
                cc: vec![],
                bcc: vec![],
                subject: subject.to_string(),
                body: "body".to_string(),
                in_reply_to: None,
                source_email_id: None,
            })
            .unwrap();
    }
    let mut h = boot(&fx);
    h.run_steps(2);
    h.get_by_label("Email").click();
    h.run_steps(3);

    h.get_by_label_contains("Outbox (2)").click();
    h.run_steps(3);
    // Both staged cards visible.
    assert_eq!(h.query_all_by_label("First").count(), 1);
    assert_eq!(h.query_all_by_label("Second").count(), 1);

    // Approve all → engine sends both via the mock SMTP.
    h.get_by_label_contains("Approve all").click();
    h.run_steps(2);
    wait_until(Duration::from_secs(10), || {
        let store = EmailStore::new(&fx.db);
        store
            .list_outbox(false)
            .unwrap()
            .iter()
            .all(|i| i.state == OutboxState::Sent)
    });
    let sent = fx.smtp.log.lock().unwrap().clone();
    assert_eq!(sent.len(), 2, "both messages sent: {sent:?}");
}

#[test]
fn compose_stages_to_outbox() {
    let fx = fixture("compose");
    let mut h = boot(&fx);
    h.run_steps(2);
    h.get_by_label("Email").click();
    h.run_steps(3);

    h.get_by_label("Compose").click();
    h.run_steps(2);
    // Inputs in order: To, Cc, Bcc, Subject (body is a multiline input).
    {
        let inputs: Vec<_> = h.query_all_by_role(Role::TextInput).collect();
        assert_eq!(inputs.len(), 4);
        inputs[0].focus();
    }
    h.run_steps(1);
    {
        let inputs: Vec<_> = h.query_all_by_role(Role::TextInput).collect();
        inputs[0].type_text("sam@x.com");
    }
    h.run_steps(1);
    {
        let inputs: Vec<_> = h.query_all_by_role(Role::TextInput).collect();
        inputs[3].focus();
    }
    h.run_steps(1);
    {
        let inputs: Vec<_> = h.query_all_by_role(Role::TextInput).collect();
        inputs[3].type_text("Hello there");
    }
    h.run_steps(1);
    {
        let body: Vec<_> = h.query_all_by_role(Role::MultilineTextInput).collect();
        assert_eq!(body.len(), 1);
        body[0].focus();
    }
    h.run_steps(1);
    {
        let body: Vec<_> = h.query_all_by_role(Role::MultilineTextInput).collect();
        body[0].type_text("Body text");
    }
    h.run_steps(1);

    h.get_by_label_contains("Stage to outbox").click();
    h.run_steps(3);

    let store = EmailStore::new(&fx.db);
    let outbox = store.list_outbox(false).unwrap();
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].subject, "Hello there");
    assert_eq!(outbox[0].body, "Body text");
    assert_eq!(outbox[0].state, OutboxState::Staged);
    assert_eq!(outbox[0].to[0].addr, "sam@x.com");
}

#[test]
fn reading_pane_icon_buttons_open_compose() {
    let fx = fixture("reading-icons");
    let mut h = boot(&fx);
    h.run_steps(2);
    h.get_by_label("Email").click();
    h.run_steps(3);
    // Open the first thread by clicking its subject label.
    h.get_by_label("Alpha uno").click();
    h.run_steps(3);
    // Reading pane is up: icon buttons present by their AccessKit names.
    assert_eq!(
        h.query_all_by_label_contains("Reply (icon button)").count(),
        1,
        "reply icon button present"
    );
    h.get_by_label_contains("Reply (icon button)").click();
    h.run_steps(3);
    // Compose opened with reply prefill.
    {
        let inputs: Vec<_> = h.query_all_by_role(Role::TextInput).collect();
        // To, Cc, Bcc, Subject (body is multiline).
        assert_eq!(inputs.len(), 4);
        assert_eq!(
            inputs[0].value().as_deref(),
            Some("ann@x.com"),
            "reply to sender"
        );
        assert_eq!(
            inputs[3].value().as_deref(),
            Some("Re: Alpha uno"),
            "reply subject"
        );
    }
    {
        let body: Vec<_> = h.query_all_by_role(Role::MultilineTextInput).collect();
        assert_eq!(body.len(), 1);
        assert!(
            body[0].value().unwrap().contains("wrote:"),
            "reply quotes the original"
        );
    }
}
