//! Sync engine: one worker thread owning its own DB connection (WAL), the
//! per-account sync cycle, on-demand body fetch, offline write-op push, and
//! outbox send. UI talks to it over mpsc commands; events come back on a
//! channel the UI drains each frame.
//!
//! Transport construction is injected as factories (the test seam): the
//! live factories read the keyring per connection; test factories return
//! scripted mocks. Panics are caught at the account boundary.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use uuid::Uuid;

use crate::db::Db;
use crate::email::compose;
use crate::email::imap_client::{ImapTransport, RemoteMail};
use crate::email::model::{EmailAccount, EmailAttachmentMeta, OutboxState, SyncFolderState};
use crate::email::smtp::SmtpTransport;
use crate::email::{EmailError, EmailStore, IngestMail};

/// Initial sync fetches at most the newest N messages per folder (spec §4).
const INITIAL_SYNC_CAP: usize = 500;
/// Engine tick: how often the loop wakes to check per-account dues.
const TICK: Duration = Duration::from_secs(1);

// ── commands + events ─────────────────────────────────────────────────────

#[derive(Debug)]
pub enum SyncCommand {
    /// Sync one account now (or all accounts when None).
    SyncNow(Option<Uuid>),
    /// Fetch + parse the body of one email.
    FetchBody(Uuid),
    /// Flush pending server write ops.
    PushWriteOps,
    /// Connect + NOOP an account; result comes back as AccountTested.
    TestAccount(Uuid),
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum SyncEvent {
    /// Background cycle started for an account.
    SyncStart {
        account_id: Uuid,
    },
    SyncOk {
        account_id: Uuid,
    },
    SyncErr {
        account_id: Uuid,
        error: String,
    },
    FolderSynced {
        account_id: Uuid,
        folder: String,
        new_messages: usize,
    },
    BodyReady {
        email_id: Uuid,
    },
    BodyErr {
        email_id: Uuid,
        error: String,
    },
    SendResult {
        outbox_id: Uuid,
        ok: bool,
        error: Option<String>,
    },
    WriteOpResult {
        op_id: Uuid,
        ok: bool,
    },
    AccountTested {
        account_id: Uuid,
        ok: bool,
        error: Option<String>,
    },
}

// ── transport factories (the live/test seam) ──────────────────────────────

pub type ImapFactory =
    Arc<dyn Fn(&EmailAccount) -> Result<Box<dyn ImapTransport>, EmailError> + Send + Sync>;
pub type SmtpFactory =
    Arc<dyn Fn(&EmailAccount) -> Result<Box<dyn SmtpTransport>, EmailError> + Send + Sync>;

/// Live factories: password from the keyring per connection, held in memory
/// only for the connection attempt (dropped with the closure locals).
pub fn live_factories() -> (ImapFactory, SmtpFactory) {
    let imap: ImapFactory = Arc::new(|account: &EmailAccount| {
        let password = crate::core::keyring::get_password(account.id)?;
        let transport = crate::email::imap_client::LiveImap::connect(
            &account.imap_host,
            account.imap_port,
            &account.username,
            &password,
        )?;
        Ok(Box::new(transport))
    });
    let smtp: SmtpFactory = Arc::new(|account: &EmailAccount| {
        let password = crate::core::keyring::get_password(account.id)?;
        let transport = crate::email::smtp::LiveSmtp::connect(
            &account.smtp_host,
            account.smtp_port,
            &account.username,
            &password,
        )?;
        Ok(Box::new(transport))
    });
    (imap, smtp)
}

// ── engine ────────────────────────────────────────────────────────────────

pub struct SyncEngine {
    cmd_tx: mpsc::Sender<SyncCommand>,
    event_rx: mpsc::Receiver<SyncEvent>,
    handle: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl SyncEngine {
    pub fn start(db_path: PathBuf, imap_factory: ImapFactory, smtp_factory: SmtpFactory) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<SyncCommand>();
        let (event_tx, event_rx) = mpsc::channel::<SyncEvent>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_flag = shutdown.clone();
        let handle = std::thread::Builder::new()
            .name("adjutant-email-sync".to_string())
            .spawn(move || {
                worker_loop(
                    db_path,
                    cmd_rx,
                    event_tx,
                    imap_factory,
                    smtp_factory,
                    shutdown_flag,
                );
            })
            .expect("spawn sync worker");
        SyncEngine {
            cmd_tx,
            event_rx,
            handle: Some(handle),
            shutdown,
        }
    }

    pub fn send(&self, cmd: SyncCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Drain pending events (UI calls each frame; repaint on non-empty).
    pub fn drain_events(&self) -> Vec<SyncEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.event_rx.try_recv() {
            out.push(ev);
        }
        if std::env::var("ADJ_SYNC_DEBUG").is_ok() && !out.is_empty() {
            eprintln!("SYNC DRAIN: {} events", out.len());
        }
        out
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = self.cmd_tx.send(SyncCommand::Shutdown);
    }
}

impl Drop for SyncEngine {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// ── worker ────────────────────────────────────────────────────────────────

/// Process one command. Returns true when the worker should shut down.
fn handle_command(
    cmd: SyncCommand,
    store: &EmailStore,
    imap_factory: &ImapFactory,
    event_tx: &mpsc::Sender<SyncEvent>,
    force: &mut Option<Option<Uuid>>,
) -> bool {
    match cmd {
        SyncCommand::SyncNow(account) => *force = Some(account),
        SyncCommand::FetchBody(email_id) => {
            let ev = fetch_body(store, imap_factory, email_id);
            let _ = event_tx.send(ev);
        }
        SyncCommand::PushWriteOps => {}
        SyncCommand::TestAccount(account_id) => {
            let ev = test_account(store, imap_factory, account_id);
            let _ = event_tx.send(ev);
        }
        SyncCommand::Shutdown => return true,
    }
    false
}

fn worker_loop(
    db_path: PathBuf,
    cmd_rx: mpsc::Receiver<SyncCommand>,
    event_tx: mpsc::Sender<SyncEvent>,
    imap_factory: ImapFactory,
    smtp_factory: SmtpFactory,
    shutdown: Arc<AtomicBool>,
) {
    // The engine owns its own connection (WAL; UI holds the primary one).
    let db = match Db::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("SYNC WORKER: db open failed: {e:#}");
            return;
        }
    };
    let store = EmailStore::new(&db);
    let mut synced_once: Vec<Uuid> = Vec::new();
    let mut force: Option<Option<Uuid>> = Some(None); // sync everything on start

    while !shutdown.load(Ordering::Relaxed) {
        // Drain queued commands.
        while let Ok(cmd) = cmd_rx.try_recv() {
            if handle_command(cmd, &store, &imap_factory, &event_tx, &mut force) {
                return;
            }
        }

        let accounts = store.list_accounts().unwrap_or_default();
        for account in &accounts {
            let due = force
                .map(|f| f.is_none() || f == Some(account.id))
                .unwrap_or_else(|| !synced_once.contains(&account.id));
            if !due {
                continue;
            }
            let _ = event_tx.send(SyncEvent::SyncStart {
                account_id: account.id,
            });
            // Panic caught at the account boundary: a poisoned parse or
            // transport bug must not kill the worker.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                sync_account(&store, &imap_factory, account)
            }));
            match result {
                Ok(Ok(new_by_folder)) => {
                    synced_once.push(account.id);
                    let _ = store.set_last_sync(account.id, &chrono::Utc::now().to_rfc3339());
                    for (folder, count) in new_by_folder {
                        let _ = event_tx.send(SyncEvent::FolderSynced {
                            account_id: account.id,
                            folder,
                            new_messages: count,
                        });
                    }
                    let _ = event_tx.send(SyncEvent::SyncOk {
                        account_id: account.id,
                    });
                }
                Ok(Err(e)) => {
                    let _ = store.log_sync(account.id, "sync_err", &e.to_string());
                    let _ = event_tx.send(SyncEvent::SyncErr {
                        account_id: account.id,
                        error: e.to_string(),
                    });
                }
                Err(_) => {
                    let _ = store.log_sync(account.id, "sync_err", "worker panic");
                    let _ = event_tx.send(SyncEvent::SyncErr {
                        account_id: account.id,
                        error: "internal error".to_string(),
                    });
                }
            }
        }
        force = None;

        // Approved outbox → SMTP → Sent APPEND (best-effort).
        if let Ok(items) = store.approved_outbox() {
            for item in items {
                let ev = send_outbox_item(&store, &imap_factory, &smtp_factory, item.id);
                let _ = event_tx.send(ev);
            }
        }

        // Pending write ops: cheap when empty; runs every tick so offline
        // changes go out as soon as a connection exists.
        for ev in push_write_ops(&store, &imap_factory) {
            let _ = event_tx.send(ev);
        }

        // Sleep in small slices so commands stay snappy. The command that
        // wakes us must be PROCESSED, not dropped.
        if let Ok(cmd) = cmd_rx.recv_timeout(TICK) {
            if handle_command(cmd, &store, &imap_factory, &event_tx, &mut force) {
                return;
            }
        }
    }
}

/// Full per-account cycle: LIST folders → role detect → per-folder
/// UIDVALIDITY check → incremental or capped-initial header sync → ingest.
fn sync_account(
    store: &EmailStore,
    factory: &ImapFactory,
    account: &EmailAccount,
) -> Result<Vec<(String, usize)>, EmailError> {
    let mut transport = factory(account)?;
    let remote_folders = transport.list_folders()?;
    for rf in &remote_folders {
        let role = crate::email::imap_client::detect_folder_role(&rf.name, &rf.attributes);
        store.upsert_folder(account.id, &rf.name, role)?;
    }

    let mut new_by_folder = Vec::new();
    for folder in store.folders_for(account.id)? {
        let state = transport.select(&folder.name)?;
        let uidvalidity = state.uidvalidity.map(i64::from);
        if uidvalidity != folder.uidvalidity {
            // New or changed UIDVALIDITY → full resync of the folder.
            if folder.uidvalidity.is_some() || folder.last_uid > 0 {
                store.clear_folder_mail(folder.id)?;
            } else if uidvalidity.is_some() {
                // First-ever sync: nothing to clear.
            }
            store.set_folder_uidvalidity(folder.id, uidvalidity)?;
        }
        let folder = store.folder_by_id(folder.id)?;
        let mails = match store.folder_sync_state(&folder)? {
            SyncFolderState::IncrementalFrom(last_uid) => {
                transport.fetch_headers(last_uid.saturating_add(1), None)?
            }
            SyncFolderState::Full => {
                // Initial sync: newest INITIAL_SYNC_CAP UIDs only.
                let all = transport.all_uids()?;
                let tail: Vec<u32> = all
                    .iter()
                    .copied()
                    .rev()
                    .take(INITIAL_SYNC_CAP)
                    .rev()
                    .collect();
                match (tail.first(), tail.last()) {
                    (Some(first), Some(last)) => transport.fetch_headers(*first, Some(*last))?,
                    _ => Vec::new(),
                }
            }
        };
        let mut new = 0usize;
        let mut max_uid = folder.last_uid;
        for mail in &mails {
            let existed = store.email_by_folder_uid(folder.id, mail.uid)?.is_some();
            store.ingest(&IngestMail {
                account_id: account.id,
                folder_id: folder.id,
                mail: mail.clone(),
            })?;
            if !existed {
                new += 1;
            }
            max_uid = max_uid.max(i64::from(mail.uid));
        }
        if max_uid != folder.last_uid {
            store.set_folder_last_uid(folder.id, max_uid)?;
        }
        if new > 0 || !mails.is_empty() {
            new_by_folder.push((folder.name.clone(), new));
        }
    }
    transport.logout()?;
    store.log_sync(account.id, "sync_ok", "")?;
    Ok(new_by_folder)
}

/// Fetch one body: text/plain preferred, HTML-only → dumb strip (+ raw HTML
/// stored for the future renderer), attachment metadata recorded.
fn fetch_body(store: &EmailStore, factory: &ImapFactory, email_id: Uuid) -> SyncEvent {
    let fail = |e: EmailError| SyncEvent::BodyErr {
        email_id,
        error: e.to_string(),
    };
    let email = match store.get_email(email_id) {
        Ok(e) => e,
        Err(e) => return fail(e),
    };
    let account = match store.get_account(email.account_id) {
        Ok(a) => a,
        Err(e) => return fail(e),
    };
    let folder = match store.folder_by_id(email.folder_id) {
        Ok(f) => f,
        Err(e) => return fail(e),
    };
    let mut transport = match factory(&account) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let result = (|| {
        transport.select(&folder.name)?;
        let mail: RemoteMail = transport.fetch_full(u32::try_from(email.uid).unwrap_or(0))?;
        let rfc822 = mail
            .rfc822
            .ok_or_else(|| EmailError::Imap("missing body".to_string()))?;
        let parsed = mailparse::parse_mail(&rfc822)
            .map_err(|e| EmailError::Engine(format!("mime parse: {e}")))?;
        let (text, html, attachments) = extract_body(&parsed);
        store.set_body(email_id, &text, html.as_deref(), &attachments)?;
        Ok(())
    })();
    let _ = transport.logout();
    match result {
        Ok(()) => SyncEvent::BodyReady { email_id },
        Err(e) => fail(e),
    }
}

/// Walk MIME parts: first text/plain wins; otherwise text/html → dumb
/// strip. Attachments = any part with a filename.
fn extract_body(
    parsed: &mailparse::ParsedMail<'_>,
) -> (String, Option<String>, Vec<EmailAttachmentMeta>) {
    let mut plain: Option<String> = None;
    let mut html: Option<String> = None;
    let mut attachments = Vec::new();

    fn walk(
        part: &mailparse::ParsedMail<'_>,
        plain: &mut Option<String>,
        html: &mut Option<String>,
        attachments: &mut Vec<EmailAttachmentMeta>,
    ) {
        let ctype = part.ctype.mimetype.to_lowercase();
        let disp = part.get_content_disposition();
        if let Some(filename) = disp
            .params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("filename"))
            .map(|(_, v)| v.clone())
        {
            let mime = part.ctype.mimetype.clone();
            let size = part.get_body().map(|b| b.len() as i64).unwrap_or(0);
            attachments.push(EmailAttachmentMeta {
                filename,
                mime,
                size,
            });
        }
        if part.subparts.is_empty() {
            if ctype.starts_with("text/plain") && plain.is_none() {
                plain.replace(part.get_body().unwrap_or_default());
            } else if ctype.starts_with("text/html") && html.is_none() {
                html.replace(part.get_body().unwrap_or_default());
            }
            return;
        }
        for sub in &part.subparts {
            walk(sub, plain, html, attachments);
        }
    }
    walk(parsed, &mut plain, &mut html, &mut attachments);

    let (text, html) = match (plain, html) {
        (Some(t), h) => (t, h),
        (None, Some(h)) => (compose::html_to_text(&h), Some(h)),
        (None, None) => (String::new(), None),
    };
    (text, html, attachments)
}

/// Send one approved outbox item; on success APPEND to Sent (best-effort).
fn send_outbox_item(
    store: &EmailStore,
    imap_factory: &ImapFactory,
    smtp_factory: &SmtpFactory,
    outbox_id: Uuid,
) -> SyncEvent {
    let fail = |error: String| {
        let _ = store.set_outbox_state(outbox_id, OutboxState::Failed, Some(&error));
        SyncEvent::SendResult {
            outbox_id,
            ok: false,
            error: Some(error),
        }
    };
    let item = match store.get_outbox_item(outbox_id) {
        Ok(i) => i,
        Err(e) => return fail(e.to_string()),
    };
    let account = match store.get_account(item.account_id) {
        Ok(a) => a,
        Err(e) => return fail(e.to_string()),
    };
    let _ = store.set_outbox_state(outbox_id, OutboxState::Sending, None);
    let rfc822 = compose::build_rfc822(&item, &account);
    let to: Vec<String> = item.to.iter().map(|a| a.addr.clone()).collect();
    let cc: Vec<String> = item.cc.iter().map(|a| a.addr.clone()).collect();
    let bcc: Vec<String> = item.bcc.iter().map(|a| a.addr.clone()).collect();

    let mut smtp = match smtp_factory(&account) {
        Ok(s) => s,
        Err(e) => return fail(e.to_string()),
    };
    if let Err(e) = smtp.send_rfc822(&account.address, &to, &cc, &bcc, &rfc822) {
        return fail(e.to_string());
    }
    drop(smtp); // password-bearing transport dropped ASAP.

    // Sent copy: best-effort; failure doesn't undo the send.
    if let Ok(mut imap) = imap_factory(&account) {
        let sent = store
            .folders_for(account.id)
            .ok()
            .and_then(|folders| folders.into_iter().find(|f| f.role.as_str() == "sent"));
        if let Some(sent) = sent {
            let _ = imap.select(&sent.name);
            let _ = imap.append(&sent.name, &rfc822, &["seen".to_string()]);
        }
        let _ = imap.logout();
    }

    let _ = store.set_outbox_state(outbox_id, OutboxState::Sent, None);
    let _ = store.log_sync(account.id, "send_ok", "");
    SyncEvent::SendResult {
        outbox_id,
        ok: true,
        error: None,
    }
}

/// Flush pending write ops to the server; ops on unreachable accounts stay
/// pending (offline-tolerant queue).
fn push_write_ops(store: &EmailStore, factory: &ImapFactory) -> Vec<SyncEvent> {
    let mut events = Vec::new();
    let Ok(ops) = store.pending_write_ops() else {
        return events;
    };
    // Group ops by account to reuse one connection per account.
    let mut by_account: Vec<Uuid> = Vec::new();
    for op in &ops {
        let Ok(email) = store.get_email(op.email_id) else {
            continue;
        };
        if !by_account.contains(&email.account_id) {
            by_account.push(email.account_id);
        }
    }
    for account_id in by_account {
        let Ok(account) = store.get_account(account_id) else {
            continue;
        };
        let Ok(mut transport) = factory(&account) else {
            continue; // offline: ops stay pending
        };
        for op in ops.iter().filter(|op| {
            store
                .get_email(op.email_id)
                .is_ok_and(|e| e.account_id == account_id)
        }) {
            let ev = apply_write_op(store, &mut *transport, op.id);
            events.push(ev);
        }
        let _ = transport.logout();
    }
    events
}

fn apply_write_op(store: &EmailStore, transport: &mut dyn ImapTransport, op_id: Uuid) -> SyncEvent {
    let fail = |error: String| {
        let _ = store.mark_write_op(op_id, false, Some(&error));
        SyncEvent::WriteOpResult { op_id, ok: false }
    };
    let Ok(op) = store
        .pending_write_ops()
        .map(|ops| ops.into_iter().find(|o| o.id == op_id))
    else {
        return fail("op vanished".to_string());
    };
    let Some(op) = op else {
        return fail("op vanished".to_string());
    };
    let Ok(email) = store.get_email(op.email_id) else {
        return fail("email missing".to_string());
    };
    let Ok(folder) = store.folder_by_id(email.folder_id) else {
        return fail("folder missing".to_string());
    };
    if let Err(e) = transport.select(&folder.name) {
        return fail(e.to_string());
    }
    let uid = u32::try_from(email.uid).unwrap_or(0);
    let result = match op.op.as_str() {
        "set_seen" | "unset_seen" => {
            let seen = op.op == "set_seen";
            let mut flags: Vec<String> = email
                .flags
                .iter()
                .filter(|f| f.as_str() != "seen")
                .cloned()
                .collect();
            if seen {
                flags.push("seen".to_string());
            }
            transport.store_flags(uid, &flags)
        }
        "move_folder" => match op.arg.as_deref() {
            Some(target) => match store.folder_by_name(email.account_id, target) {
                Ok(Some(target_folder)) => transport.move_mail(uid, target).and_then(|_| {
                    store
                        .move_email_local(email.id, target_folder.id)
                        .map(|_| ())
                }),
                _ => Err(EmailError::Engine(format!("unknown folder {target:?}"))),
            },
            None => Err(EmailError::Engine("move op missing arg".to_string())),
        },
        other => Err(EmailError::Engine(format!("unknown op {other:?}"))),
    };
    match result {
        Ok(()) => {
            let _ = store.mark_write_op(op_id, true, None);
            SyncEvent::WriteOpResult { op_id, ok: true }
        }
        Err(e) => fail(e.to_string()),
    }
}

/// Connect + NOOP: the account form's "Test connection" path.
fn test_account(store: &EmailStore, factory: &ImapFactory, account_id: Uuid) -> SyncEvent {
    let fail = |error: String| SyncEvent::AccountTested {
        account_id,
        ok: false,
        error: Some(error),
    };
    let Ok(account) = store.get_account(account_id) else {
        return fail("account missing".to_string());
    };
    let mut transport = match factory(&account) {
        Ok(t) => t,
        Err(e) => return fail(e.to_string()),
    };
    let result = transport.noop();
    let _ = transport.logout();
    match result {
        Ok(()) => SyncEvent::AccountTested {
            account_id,
            ok: true,
            error: None,
        },
        Err(e) => fail(e.to_string()),
    }
}
