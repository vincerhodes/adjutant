//! Sync engine tests: scripted mock transports against a temp-file DB
//! (the engine owns its own connection — WAL covers the two handles).
//! No live network (spec §2/§9).

#![allow(clippy::unwrap_used)]

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adjutant::db::Db;
use adjutant::email::imap_client::{FolderState, ImapTransport, RemoteFolder, RemoteMail};
use adjutant::email::model::{EmailAccount, OutboxState};
use adjutant::email::smtp::SmtpTransport;
use adjutant::email::sync::{ImapFactory, SmtpFactory, SyncCommand, SyncEngine, SyncEvent};
use adjutant::email::{EmailError, EmailStore, NewAccount, NewOutbox};

// ── mocks ─────────────────────────────────────────────────────────────────

#[derive(Default)]
struct MockMailbox {
    uidvalidity: u32,
    headers: BTreeMap<u32, RemoteMail>,
    bodies: HashMap<u32, RemoteMail>,
}

#[derive(Default)]
struct MockImapState {
    folders: Vec<RemoteFolder>,
    mailboxes: HashMap<String, MockMailbox>,
    selected: String,
    supports_move: bool,
    calls: Vec<String>,
}

#[derive(Clone)]
struct MockImap {
    state: Arc<Mutex<MockImapState>>,
}

impl MockImap {
    fn mailbox(&self) -> std::sync::MutexGuard<'_, MockImapState> {
        self.state.lock().unwrap()
    }
}

impl ImapTransport for MockImap {
    fn capabilities(&mut self) -> Result<Vec<String>, EmailError> {
        let caps = if self.mailbox().supports_move {
            vec!["IMAP4rev1".to_string(), "MOVE".to_string()]
        } else {
            vec!["IMAP4rev1".to_string()]
        };
        Ok(caps)
    }

    fn list_folders(&mut self) -> Result<Vec<RemoteFolder>, EmailError> {
        Ok(self.mailbox().folders.clone())
    }

    fn select(&mut self, folder: &str) -> Result<FolderState, EmailError> {
        let mut st = self.mailbox();
        st.calls.push(format!("select:{folder}"));
        if !st.mailboxes.contains_key(folder) {
            st.mailboxes
                .insert(folder.to_string(), MockMailbox::default());
        }
        st.selected = folder.to_string();
        let uv = st.mailboxes.get(folder).unwrap().uidvalidity;
        Ok(FolderState {
            uidvalidity: Some(uv),
        })
    }

    fn fetch_headers(
        &mut self,
        uid_start: u32,
        uid_end: Option<u32>,
    ) -> Result<Vec<RemoteMail>, EmailError> {
        let mut st = self.mailbox();
        st.calls
            .lock_push(format!("fetch_headers:{uid_start}:{:?}", uid_end));
        let selected = st.selected.clone();
        let mb = st.mailboxes.get(&selected).unwrap();
        Ok(mb
            .headers
            .range(uid_start..=uid_end.unwrap_or(u32::MAX))
            .map(|(_, m)| m.clone())
            .collect())
    }

    fn fetch_full(&mut self, uid: u32) -> Result<RemoteMail, EmailError> {
        let mut st = self.mailbox();
        st.calls.lock_push(format!("fetch_full:{uid}"));
        let selected = st.selected.clone();
        st.mailboxes
            .get(&selected)
            .and_then(|mb| mb.bodies.get(&uid).cloned())
            .ok_or_else(|| EmailError::Imap("no such uid".to_string()))
    }

    fn all_uids(&mut self) -> Result<Vec<u32>, EmailError> {
        let st = self.mailbox();
        Ok(st
            .mailboxes
            .get(&st.selected)
            .map(|mb| mb.headers.keys().copied().collect())
            .unwrap_or_default())
    }

    fn store_flags(&mut self, uid: u32, flags: &[String]) -> Result<(), EmailError> {
        let mut st = self.mailbox();
        st.calls.lock_push(format!("store_flags:{uid}:{flags:?}"));
        let selected = st.selected.clone();
        let mb = st.mailboxes.get_mut(&selected).unwrap();
        if let Some(m) = mb.headers.get_mut(&uid) {
            m.flags = flags.to_vec();
        }
        Ok(())
    }

    fn move_mail(&mut self, uid: u32, target: &str) -> Result<(), EmailError> {
        let mut st = self.mailbox();
        st.calls.lock_push(format!("move:{uid}->{target}"));
        let src_name = st.selected.clone();
        let mail = st
            .mailboxes
            .get(&src_name)
            .and_then(|mb| mb.headers.get(&uid).cloned());
        let Some(mail) = mail else {
            return Err(EmailError::Imap("no such uid".to_string()));
        };
        st.mailboxes
            .entry(target.to_string())
            .or_default()
            .headers
            .insert(uid, mail.clone());
        st.mailboxes
            .entry(target.to_string())
            .or_default()
            .bodies
            .insert(uid, mail);
        if let Some(mb) = st.mailboxes.get_mut(&src_name) {
            mb.headers.remove(&uid);
            mb.bodies.remove(&uid);
        }
        Ok(())
    }

    fn append(
        &mut self,
        folder: &str,
        _content: &[u8],
        flags: &[String],
    ) -> Result<(), EmailError> {
        let mut st = self.mailbox();
        st.calls.lock_push(format!("append:{folder}:{flags:?}"));
        Ok(())
    }

    fn noop(&mut self) -> Result<(), EmailError> {
        Ok(())
    }

    fn logout(&mut self) -> Result<(), EmailError> {
        Ok(())
    }
}

/// Helper so pushes don't hold the guard awkwardly in one-liners.
trait PushCall {
    fn lock_push(&mut self, call: String);
}
impl PushCall for Vec<String> {
    fn lock_push(&mut self, call: String) {
        self.push(call);
    }
}

#[derive(Clone, Default)]
struct MockSmtp {
    state: Arc<Mutex<Vec<String>>>,
    fail: Arc<Mutex<bool>>,
}

impl SmtpTransport for MockSmtp {
    fn send_rfc822(
        &mut self,
        from: &str,
        to: &[String],
        _cc: &[String],
        _bcc: &[String],
        rfc822: &[u8],
    ) -> Result<(), EmailError> {
        if *self.fail.lock().unwrap() {
            return Err(EmailError::Smtp("permanent".to_string()));
        }
        self.state
            .lock()
            .unwrap()
            .push(format!("send:{from}->{to:?}:{}b", rfc822.len()));
        Ok(())
    }
}

// ── fixtures ──────────────────────────────────────────────────────────────

fn temp_db_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "adjutant-email-sync-{}-{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("adjutant.db")
}

fn mail(uid: u32, subject: &str) -> RemoteMail {
    RemoteMail {
        uid,
        subject: subject.to_string(),
        from_addr: "sam@example.com".to_string(),
        from_name: Some("Sam".to_string()),
        date: format!("2026-10-{:02}T10:00:00+00:00", (uid % 28) + 1),
        ..Default::default()
    }
}

fn seed_account(store: &EmailStore) -> EmailAccount {
    store
        .create_account(&NewAccount {
            label: "Mock",
            address: "jim@example.com",
            imap_host: "imap.mock",
            imap_port: 993,
            smtp_host: "smtp.mock",
            smtp_port: 465,
            username: "jim@example.com",
            color_idx: 0,
            sync_interval_s: 60,
        })
        .unwrap()
}

/// Wait until `pred` sees a matching event (or time out).
fn wait_for(engine: &SyncEngine, pred: impl Fn(&SyncEvent) -> bool) -> Option<SyncEvent> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        for ev in engine.drain_events() {
            if pred(&ev) {
                return Some(ev);
            }
            seen.push(ev);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for event; saw {seen:?}");
}

fn inbox_fixture(uids: std::ops::Range<u32>, uidvalidity: u32) -> MockImapState {
    let mut mb = MockMailbox {
        uidvalidity,
        ..Default::default()
    };
    for uid in uids {
        mb.headers.insert(uid, mail(uid, &format!("Mail {uid}")));
        mb.bodies.insert(uid, RemoteMail::default());
    }
    MockImapState {
        folders: vec![RemoteFolder {
            name: "INBOX".to_string(),
            attributes: vec![],
        }],
        mailboxes: HashMap::from([("INBOX".to_string(), mb)]),
        ..Default::default()
    }
}

fn factories(imap: &MockImap, smtp: &MockSmtp) -> (ImapFactory, SmtpFactory) {
    let imap = imap.clone();
    let smtp = smtp.clone();
    (
        Arc::new(move |_| Ok(Box::new(imap.clone()) as Box<dyn ImapTransport>)),
        Arc::new(move |_| Ok(Box::new(smtp.clone()) as Box<dyn SmtpTransport>)),
    )
}

// ── tests ─────────────────────────────────────────────────────────────────

#[test]
fn initial_sync_caps_at_500_newest() {
    let path = temp_db_path("cap");
    let db = Db::open(&path).unwrap();
    let store = EmailStore::new(&db);
    let account = seed_account(&store);

    // 600 messages on the "server".
    let imap = MockImap {
        state: Arc::new(Mutex::new(inbox_fixture(1..601, 1001))),
    };
    let smtp = MockSmtp::default();
    let (ifac, sfac) = factories(&imap, &smtp);
    let engine = SyncEngine::start(path.clone(), ifac, sfac);
    wait_for(&engine, |ev| matches!(ev, SyncEvent::SyncOk { .. }));

    let store = EmailStore::new(&db);
    let threads = store.threads(Some(account.id), 1000).unwrap();
    let total: usize = threads.iter().map(|t| t.message_count).sum();
    assert_eq!(total, 500, "initial sync keeps only the newest 500");
    let folder = store.folder_by_name(account.id, "INBOX").unwrap().unwrap();
    assert_eq!(folder.last_uid, 600);
    assert!(threads
        .iter()
        .all(|t| t.messages.iter().all(|m| m.uid > 100)));
    engine.shutdown();
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn incremental_sync_fetches_only_new_uids() {
    let path = temp_db_path("incremental");
    let db = Db::open(&path).unwrap();
    let store = EmailStore::new(&db);
    let account = seed_account(&store);

    let imap = MockImap {
        state: Arc::new(Mutex::new(inbox_fixture(1..10, 1001))),
    };
    let smtp = MockSmtp::default();
    let (ifac, sfac) = factories(&imap, &smtp);
    let engine = SyncEngine::start(path.clone(), ifac, sfac);
    wait_for(&engine, |ev| matches!(ev, SyncEvent::SyncOk { .. }));

    // New mail arrives; a forced sync picks up just the tail.
    imap.mailbox()
        .mailboxes
        .get_mut("INBOX")
        .unwrap()
        .headers
        .insert(10, mail(10, "New arrival"));
    engine.send(SyncCommand::SyncNow(Some(account.id)));
    wait_for(&engine, |ev| matches!(ev, SyncEvent::SyncOk { .. }));

    let store = EmailStore::new(&db);
    let threads = store.threads(Some(account.id), 1000).unwrap();
    let total: usize = threads.iter().map(|t| t.message_count).sum();
    assert_eq!(total, 10);
    assert!(threads.iter().any(|t| t.subject == "New arrival"));
    engine.shutdown();
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn uidvalidity_change_resyncs_and_thread_ids_stay_stable() {
    let path = temp_db_path("uidvalidity");
    let db = Db::open(&path).unwrap();
    let store = EmailStore::new(&db);
    let account = seed_account(&store);

    let imap = MockImap {
        state: Arc::new(Mutex::new(inbox_fixture(1..5, 1001))),
    };
    let smtp = MockSmtp::default();
    let (ifac, sfac) = factories(&imap, &smtp);
    let engine = SyncEngine::start(path.clone(), ifac, sfac);
    wait_for(&engine, |ev| matches!(ev, SyncEvent::SyncOk { .. }));

    let store = EmailStore::new(&db);
    let before = store.threads(Some(account.id), 100).unwrap();
    let thread_before = before[0].thread_id;

    // UIDVALIDITY bumps → folder clears and re-ingests.
    imap.mailbox()
        .mailboxes
        .get_mut("INBOX")
        .unwrap()
        .uidvalidity = 2002;
    engine.send(SyncCommand::SyncNow(Some(account.id)));
    wait_for(&engine, |ev| matches!(ev, SyncEvent::SyncOk { .. }));

    let store = EmailStore::new(&db);
    let after = store.threads(Some(account.id), 100).unwrap();
    let total: usize = after.iter().map(|t| t.message_count).sum();
    assert_eq!(total, 4, "folder re-ingested after UIDVALIDITY change");
    assert_eq!(
        after[0].thread_id, thread_before,
        "thread_id stable across resync"
    );
    engine.shutdown();
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn write_op_push_sets_seen_on_server() {
    let path = temp_db_path("writeops");
    let db = Db::open(&path).unwrap();
    let store = EmailStore::new(&db);
    let account = seed_account(&store);

    let imap = MockImap {
        state: Arc::new(Mutex::new(inbox_fixture(1..3, 1001))),
    };
    let smtp = MockSmtp::default();
    let (ifac, sfac) = factories(&imap, &smtp);
    let engine = SyncEngine::start(path.clone(), ifac, sfac);
    wait_for(&engine, |ev| matches!(ev, SyncEvent::SyncOk { .. }));

    // UI marks read locally → op queued; engine pushes it.
    let store = EmailStore::new(&db);
    let email = store.threads(Some(account.id), 10).unwrap()[0].messages[0].clone();
    store.mark_seen(email.id, true).unwrap();
    engine.send(SyncCommand::PushWriteOps);
    wait_for(&engine, |ev| {
        matches!(ev, SyncEvent::WriteOpResult { ok: true, .. })
    });

    let calls = imap.mailbox().calls.clone();
    assert!(
        calls
            .iter()
            .any(|c| c.starts_with("store_flags:") && c.contains("seen")),
        "server received the flag push: {calls:?}"
    );
    assert!(store.pending_write_ops().unwrap().is_empty());
    engine.shutdown();
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn send_success_marks_sent_and_appends_to_sent() {
    let path = temp_db_path("send-ok");
    let db = Db::open(&path).unwrap();
    let store = EmailStore::new(&db);
    let account = seed_account(&store);

    let mut fixture = inbox_fixture(1..2, 1001);
    fixture.folders.push(RemoteFolder {
        name: "Sent".to_string(),
        attributes: vec!["Sent".to_string()],
    });
    fixture
        .mailboxes
        .insert("Sent".to_string(), MockMailbox::default());
    let imap = MockImap {
        state: Arc::new(Mutex::new(fixture)),
    };
    let smtp = MockSmtp::default();
    let (ifac, sfac) = factories(&imap, &smtp);
    let engine = SyncEngine::start(path.clone(), ifac, sfac);
    wait_for(&engine, |ev| matches!(ev, SyncEvent::SyncOk { .. }));

    let store = EmailStore::new(&db);
    let item = store
        .stage_outbox(&NewOutbox {
            account_id: account.id,
            to: vec![adjutant::email::model::MailAddress {
                name: None,
                addr: "sam@example.com".to_string(),
            }],
            cc: vec![],
            bcc: vec![],
            subject: "Hello".to_string(),
            body: "Hi Sam".to_string(),
            in_reply_to: None,
            source_email_id: None,
        })
        .unwrap();
    store
        .set_outbox_state(item.id, OutboxState::Approved, None)
        .unwrap();

    let ev = wait_for(
        &engine,
        |ev| matches!(ev, SyncEvent::SendResult { outbox_id, .. } if *outbox_id == item.id),
    );
    assert!(matches!(ev, Some(SyncEvent::SendResult { ok: true, .. })));

    let store = EmailStore::new(&db);
    let sent = store.get_outbox_item(item.id).unwrap();
    assert_eq!(sent.state, OutboxState::Sent);
    assert!(sent.sent_at.is_some());

    let calls = imap.mailbox().calls.clone();
    assert!(
        calls.iter().any(|c| c.starts_with("append:Sent")),
        "Sent copy APPENDed: {calls:?}"
    );
    let smtp_log = smtp.state.lock().unwrap().clone();
    assert!(
        smtp_log.iter().any(|c| c.starts_with("send:")),
        "smtp sent: {smtp_log:?}"
    );
    engine.shutdown();
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn send_failure_marks_failed_and_retry_succeeds() {
    let path = temp_db_path("send-fail");
    let db = Db::open(&path).unwrap();
    let store = EmailStore::new(&db);
    let account = seed_account(&store);

    let imap = MockImap {
        state: Arc::new(Mutex::new(inbox_fixture(1..2, 1001))),
    };
    let smtp = MockSmtp {
        fail: Arc::new(Mutex::new(true)),
        ..Default::default()
    };
    let (ifac, sfac) = factories(&imap, &smtp);
    let engine = SyncEngine::start(path.clone(), ifac, sfac);
    wait_for(&engine, |ev| matches!(ev, SyncEvent::SyncOk { .. }));

    let store = EmailStore::new(&db);
    let item = store
        .stage_outbox(&NewOutbox {
            account_id: account.id,
            to: vec![adjutant::email::model::MailAddress {
                name: None,
                addr: "sam@example.com".to_string(),
            }],
            cc: vec![],
            bcc: vec![],
            subject: "Nope".to_string(),
            body: "x".to_string(),
            in_reply_to: None,
            source_email_id: None,
        })
        .unwrap();
    store
        .set_outbox_state(item.id, OutboxState::Approved, None)
        .unwrap();

    let ev = wait_for(
        &engine,
        |ev| matches!(ev, SyncEvent::SendResult { outbox_id, .. } if *outbox_id == item.id),
    );
    assert!(matches!(
        ev,
        Some(SyncEvent::SendResult {
            ok: false,
            error: Some(_),
            ..
        })
    ));
    let store = EmailStore::new(&db);
    let failed = store.get_outbox_item(item.id).unwrap();
    assert_eq!(failed.state, OutboxState::Failed);
    assert!(failed.error.is_some());

    // Fix the transport, re-approve, retry.
    *smtp.fail.lock().unwrap() = false;
    store
        .set_outbox_state(item.id, OutboxState::Approved, None)
        .unwrap();
    let ev = wait_for(
        &engine,
        |ev| matches!(ev, SyncEvent::SendResult { outbox_id, ok: true, .. } if *outbox_id == item.id),
    );
    assert!(ev.is_some());
    let store = EmailStore::new(&db);
    assert_eq!(
        store.get_outbox_item(item.id).unwrap().state,
        OutboxState::Sent
    );
    engine.shutdown();
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn body_fetch_prefers_plain_and_stores_html() {
    let path = temp_db_path("body");
    let db = Db::open(&path).unwrap();
    let store = EmailStore::new(&db);
    let account = seed_account(&store);

    let mut fixture = inbox_fixture(1..2, 1001);
    let rfc822 = b"From: sam@example.com\r\n\
Subject: MIME\r\n\
Content-Type: multipart/alternative; boundary=xyz\r\n\
\r\n\
--xyz\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Plain body here\r\n\
--xyz\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>HTML body</p>\r\n\
--xyz--\r\n";
    let full = RemoteMail {
        uid: 1,
        rfc822: Some(rfc822.to_vec()),
        ..Default::default()
    };
    fixture
        .mailboxes
        .get_mut("INBOX")
        .unwrap()
        .bodies
        .insert(1, full);
    let imap = MockImap {
        state: Arc::new(Mutex::new(fixture)),
    };
    let smtp = MockSmtp::default();
    let (ifac, sfac) = factories(&imap, &smtp);
    let engine = SyncEngine::start(path.clone(), ifac, sfac);
    wait_for(&engine, |ev| matches!(ev, SyncEvent::SyncOk { .. }));

    let store = EmailStore::new(&db);
    let email = store.threads(Some(account.id), 10).unwrap()[0].messages[0].clone();
    engine.send(SyncCommand::FetchBody(email.id));
    wait_for(&engine, |ev| matches!(ev, SyncEvent::BodyReady { .. }));

    let store = EmailStore::new(&db);
    let fetched = store.get_email(email.id).unwrap();
    assert_eq!(fetched.body_text.as_deref(), Some("Plain body here"));
    assert_eq!(fetched.body_html.as_deref(), Some("<p>HTML body</p>"));
    assert!(fetched.body_fetched_at.is_some());
    engine.shutdown();
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
