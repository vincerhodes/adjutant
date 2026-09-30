//! Email store integration tests against an in-memory database (spec §9).

#![allow(clippy::unwrap_used)]

use adjutant::db::Db;
use adjutant::email::imap_client::RemoteMail;
use adjutant::email::model::{EmailAttachmentMeta, FolderRole, OutboxState};
use adjutant::email::{EmailStore, IngestMail, NewAccount, NewOutbox};
use uuid::Uuid;

fn mem_db() -> Db {
    Db::open(std::path::Path::new(":memory:")).expect("in-memory db")
}

fn add_account(store: &EmailStore, label: &str) -> adjutant::email::model::EmailAccount {
    store
        .create_account(&NewAccount {
            label,
            address: "jim@example.com",
            imap_host: "imap.example.com",
            imap_port: 993,
            smtp_host: "smtp.example.com",
            smtp_port: 465,
            username: "jim@example.com",
            color_idx: 0,
            sync_interval_s: 300,
        })
        .unwrap()
}

fn mail(uid: u32, subject: &str) -> RemoteMail {
    RemoteMail {
        uid,
        subject: subject.to_string(),
        from_addr: "sam@example.com".to_string(),
        from_name: Some("Sam".to_string()),
        date: format!("2026-10-{uid:02}T10:00:00+00:00"),
        ..Default::default()
    }
}

fn ingest(store: &EmailStore, account: Uuid, folder: Uuid, m: RemoteMail) {
    store
        .ingest(&IngestMail {
            account_id: account,
            folder_id: folder,
            mail: m,
        })
        .unwrap();
}

#[test]
fn account_crud() {
    let db = mem_db();
    let store = EmailStore::new(&db);
    let a = add_account(&store, "Purelymail");
    assert_eq!(a.label, "Purelymail");
    assert_eq!(a.imap_port, 993);
    assert_eq!(store.list_accounts().unwrap().len(), 1);

    let mut updated = a.clone();
    updated.label = "Work".to_string();
    updated.sync_interval_s = 60;
    store.update_account(&updated).unwrap();
    let fetched = store.get_account(a.id).unwrap();
    assert_eq!(fetched.label, "Work");
    assert_eq!(fetched.sync_interval_s, 60);

    store.delete_account(a.id).unwrap();
    assert!(store.list_accounts().unwrap().is_empty());
}

#[test]
fn folder_upsert_preserves_uid_state() {
    let db = mem_db();
    let store = EmailStore::new(&db);
    let a = add_account(&store, "A");
    let f = store
        .upsert_folder(a.id, "INBOX", FolderRole::Inbox)
        .unwrap();
    store.set_folder_uidvalidity(f.id, Some(42)).unwrap();
    store.set_folder_last_uid(f.id, 7).unwrap();

    // Re-upsert (sync discovered it again) must not clobber state.
    let f2 = store
        .upsert_folder(a.id, "INBOX", FolderRole::Inbox)
        .unwrap();
    assert_eq!(f2.id, f.id);
    assert_eq!(f2.uidvalidity, Some(42));
    assert_eq!(f2.last_uid, 7);
    assert_eq!(store.folders_for(a.id).unwrap().len(), 1);
}

#[test]
fn ingest_is_idempotent_by_folder_uid() {
    let db = mem_db();
    let store = EmailStore::new(&db);
    let a = add_account(&store, "A");
    let f = store
        .upsert_folder(a.id, "INBOX", FolderRole::Inbox)
        .unwrap();

    let mut m = mail(1, "Hello");
    m.flags = vec!["seen".to_string()];
    ingest(&store, a.id, f.id, m.clone());
    // Same UID again: refresh, not duplicate.
    m.flags = vec![];
    ingest(&store, a.id, f.id, m);
    let threads = store.threads(Some(a.id), 50).unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].message_count, 1);
    // Flags refreshed by the second ingest.
    assert!(threads[0].messages[0].flags.is_empty());
}

#[test]
fn threading_reply_chain_via_message_id() {
    let db = mem_db();
    let store = EmailStore::new(&db);
    let a = add_account(&store, "A");
    let inbox = store
        .upsert_folder(a.id, "INBOX", FolderRole::Inbox)
        .unwrap();

    let mut root = mail(1, "Project kickoff");
    root.message_id = Some("<root@x>".to_string());
    ingest(&store, a.id, inbox.id, root);

    // Reply: In-Reply-To matches the root's message-id → same thread.
    let mut reply = mail(2, "Re: Project kickoff");
    reply.in_reply_to = Some("<root@x>".to_string());
    reply.message_id = Some("<r1@x>".to_string());
    ingest(&store, a.id, inbox.id, reply);

    // Grandchild references the reply.
    let mut grandchild = mail(3, "Re: Project kickoff");
    grandchild.references = vec!["<root@x>".to_string(), "<r1@x>".to_string()];
    grandchild.message_id = Some("<r2@x>".to_string());
    ingest(&store, a.id, inbox.id, grandchild);

    let threads = store.threads(Some(a.id), 50).unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].message_count, 3);
}

#[test]
fn threading_subject_fallback_when_no_references() {
    let db = mem_db();
    let store = EmailStore::new(&db);
    let a = add_account(&store, "A");
    let inbox = store
        .upsert_folder(a.id, "INBOX", FolderRole::Inbox)
        .unwrap();

    // Broken mail with no message-id at all.
    ingest(&store, a.id, inbox.id, mail(1, "Weekly numbers"));
    // Another broken reply with no threading headers.
    ingest(&store, a.id, inbox.id, mail(2, "RE: Weekly  numbers"));

    let threads = store.threads(Some(a.id), 50).unwrap();
    assert_eq!(threads.len(), 1, "subject fallback groups broken mail");
}

#[test]
fn threading_cross_folder_merge_by_message_id() {
    let db = mem_db();
    let store = EmailStore::new(&db);
    let a = add_account(&store, "A");
    let inbox = store
        .upsert_folder(a.id, "INBOX", FolderRole::Inbox)
        .unwrap();
    let sent = store.upsert_folder(a.id, "Sent", FolderRole::Sent).unwrap();

    // Incoming question in INBOX.
    let mut incoming = mail(1, "Question");
    incoming.message_id = Some("<q@x>".to_string());
    ingest(&store, a.id, inbox.id, incoming);

    // My reply lands in Sent with In-Reply-To → merges into one thread.
    let mut reply = mail(5, "Re: Question");
    reply.in_reply_to = Some("<q@x>".to_string());
    reply.message_id = Some("<a@x>".to_string());
    reply.from_addr = "jim@example.com".to_string();
    ingest(&store, a.id, sent.id, reply);

    let threads = store.threads(Some(a.id), 50).unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].message_count, 2);
}

#[test]
fn threading_isolated_per_account() {
    let db = mem_db();
    let store = EmailStore::new(&db);
    let a = add_account(&store, "A");
    let b = add_account(&store, "B");
    let fa = store
        .upsert_folder(a.id, "INBOX", FolderRole::Inbox)
        .unwrap();
    let fb = store
        .upsert_folder(b.id, "INBOX", FolderRole::Inbox)
        .unwrap();

    let mut m = mail(1, "Same subject");
    m.message_id = Some("<shared@x>".to_string());
    ingest(&store, a.id, fa.id, m.clone());
    ingest(&store, b.id, fb.id, m);

    assert_eq!(store.threads(Some(a.id), 50).unwrap().len(), 1);
    assert_eq!(store.threads(Some(b.id), 50).unwrap().len(), 1);
    // Two accounts → two separate threads despite identical message-id.
    let unified = store.threads(None, 50).unwrap();
    assert_eq!(unified.len(), 2);
}

#[test]
fn unified_threads_and_unread_counts() {
    let db = mem_db();
    let store = EmailStore::new(&db);
    let a = add_account(&store, "A");
    let b = add_account(&store, "B");
    let fa = store
        .upsert_folder(a.id, "INBOX", FolderRole::Inbox)
        .unwrap();
    let fb = store
        .upsert_folder(b.id, "INBOX", FolderRole::Inbox)
        .unwrap();

    let mut unread = mail(1, "Unread");
    unread.from_addr = "a@x".to_string();
    ingest(&store, a.id, fa.id, unread);

    let mut seen = mail(1, "Read");
    seen.flags = vec!["seen".to_string()];
    seen.from_addr = "b@x".to_string();
    ingest(&store, b.id, fb.id, seen);

    assert_eq!(store.unread_count(a.id).unwrap(), 1);
    assert_eq!(store.unread_count(b.id).unwrap(), 0);
    let unified = store.threads(None, 50).unwrap();
    assert_eq!(unified.len(), 2);
    assert!(unified.iter().any(|t| t.unread_count == 1));
}

#[test]
fn outbox_state_machine() {
    let db = mem_db();
    let store = EmailStore::new(&db);
    let a = add_account(&store, "A");

    let item = store
        .stage_outbox(&NewOutbox {
            account_id: a.id,
            to: vec![adjutant::email::model::MailAddress {
                name: None,
                addr: "to@x.com".to_string(),
            }],
            cc: vec![],
            bcc: vec![],
            subject: "Hi".to_string(),
            body: "Body".to_string(),
            in_reply_to: Some("<src@x>".to_string()),
            source_email_id: None,
        })
        .unwrap();
    assert_eq!(item.state, OutboxState::Staged);
    assert_eq!(item.in_reply_to.as_deref(), Some("<src@x>"));

    store
        .set_outbox_state(item.id, OutboxState::Approved, None)
        .unwrap();
    assert_eq!(
        store.get_outbox_item(item.id).unwrap().state,
        OutboxState::Approved
    );
    assert!(store
        .approved_outbox()
        .unwrap()
        .iter()
        .any(|i| i.id == item.id));

    store
        .set_outbox_state(item.id, OutboxState::Sending, None)
        .unwrap();
    store
        .set_outbox_state(item.id, OutboxState::Sent, None)
        .unwrap();
    let sent = store.get_outbox_item(item.id).unwrap();
    assert_eq!(sent.state, OutboxState::Sent);
    assert!(sent.sent_at.is_some());

    // Retry path: failure keeps the error.
    let item2 = store
        .stage_outbox(&NewOutbox {
            account_id: a.id,
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "B".to_string(),
            body: "b".to_string(),
            in_reply_to: None,
            source_email_id: None,
        })
        .unwrap();
    store
        .set_outbox_state(item2.id, OutboxState::Failed, Some("smtp no"))
        .unwrap();
    let failed = store.get_outbox_item(item2.id).unwrap();
    assert_eq!(failed.state, OutboxState::Failed);
    assert_eq!(failed.error.as_deref(), Some("smtp no"));

    // Draft edit + discard.
    store
        .update_outbox_draft(
            item2.id,
            &[adjutant::email::model::MailAddress {
                name: None,
                addr: "c@x.com".to_string(),
            }],
            &[],
            &[],
            "B2",
            "b2",
        )
        .unwrap();
    store
        .set_outbox_state(item2.id, OutboxState::Discarded, None)
        .unwrap();
    // Discarded items drop out of the active list.
    assert!(!store
        .list_outbox(false)
        .unwrap()
        .iter()
        .any(|i| i.id == item2.id));
}

#[test]
fn mark_seen_enqueues_write_op() {
    let db = mem_db();
    let store = EmailStore::new(&db);
    let a = add_account(&store, "A");
    let f = store
        .upsert_folder(a.id, "INBOX", FolderRole::Inbox)
        .unwrap();
    ingest(&store, a.id, f.id, mail(1, "Hi"));
    let email = store.threads(Some(a.id), 1).unwrap()[0].messages[0].clone();

    // Optimistic local + server op queued.
    store.mark_seen(email.id, true).unwrap();
    assert!(store.get_email(email.id).unwrap().is_seen());
    let ops = store.pending_write_ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op, "set_seen");

    store.mark_seen(email.id, false).unwrap();
    let ops = store.pending_write_ops().unwrap();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[1].op, "unset_seen");

    store.mark_write_op(ops[0].id, true, None).unwrap();
    store
        .mark_write_op(ops[1].id, false, Some("offline"))
        .unwrap();
    assert!(store.pending_write_ops().unwrap().is_empty());
}

#[test]
fn body_storage_and_eviction() {
    let db = mem_db();
    let store = EmailStore::new(&db);
    let a = add_account(&store, "A");
    let f = store
        .upsert_folder(a.id, "INBOX", FolderRole::Inbox)
        .unwrap();
    ingest(&store, a.id, f.id, mail(1, "Hi"));
    let email = store.threads(Some(a.id), 1).unwrap()[0].messages[0].clone();

    store
        .set_body(
            email.id,
            "Hello body",
            Some("<p>Hello</p>"),
            &[EmailAttachmentMeta {
                filename: "a.pdf".to_string(),
                mime: "application/pdf".to_string(),
                size: 100,
            }],
        )
        .unwrap();
    let fetched = store.get_email(email.id).unwrap();
    assert_eq!(fetched.body_text.as_deref(), Some("Hello body"));
    assert_eq!(fetched.body_html.as_deref(), Some("<p>Hello</p>"));
    assert_eq!(fetched.snippet, "Hello body");
    assert!(fetched.has_attachments);

    let atts = store.attachments_for(email.id).unwrap();
    assert_eq!(atts.len(), 1);
    assert_eq!(atts[0].filename, "a.pdf");
    assert!(!atts[0].has_content);
    store
        .set_attachment_content(atts[0].id, b"pdf-bytes")
        .unwrap();
    assert_eq!(
        store.attachment_content(atts[0].id).unwrap().unwrap(),
        b"pdf-bytes"
    );

    // Backdate the fetch date past the 90-day eviction horizon.
    db.conn()
        .execute(
            "UPDATE emails SET body_fetched_at = '2020-01-01T00:00:00Z' WHERE id = ?1",
            rusqlite::params![email.id.to_string()],
        )
        .unwrap();
    let evicted = store.evict_body_cache().unwrap();
    assert_eq!(evicted, 1);
    let after = store.get_email(email.id).unwrap();
    assert!(after.body_text.is_none());
    assert!(after.body_html.is_none());
    assert!(after.body_fetched_at.is_none());
}

#[test]
fn folder_role_detection() {
    use adjutant::email::imap_client::detect_folder_role;
    use adjutant::email::model::FolderRole as R;
    assert_eq!(detect_folder_role("INBOX", &[]), R::Inbox);
    assert_eq!(detect_folder_role("Sent Items", &[]), R::Sent);
    assert_eq!(
        detect_folder_role("Anything", &["Sent".to_string()]),
        R::Sent
    );
    assert_eq!(detect_folder_role("Spam", &[]), R::Junk);
    assert_eq!(detect_folder_role("Deleted Messages", &[]), R::Trash);
    assert_eq!(detect_folder_role("Random", &[]), R::Other);
}
