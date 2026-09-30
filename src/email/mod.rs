//! Email module: store (SQL CRUD, queries, outbox ops, write-op queue),
//! sync engine, threading, compose, UI.
//!
//! Transport traits (`ImapTransport`/`SmtpTransport`) are the test seam —
//! live impls in `imap_client.rs`/`smtp.rs`, mocks in tests. No live
//! network in `cargo test`. Passwords live in the OS keyring only.
//!
//! All SQL for the email module lives in this file; UI files contain none.

pub mod compose;
pub mod imap_client;
pub mod model;
pub mod smtp;
pub mod sync;
pub mod thread;
pub mod ui;

use std::str::FromStr;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use thiserror::Error as ThisError;
use uuid::Uuid;

use crate::db::{Db, DbError};
use crate::email::imap_client::RemoteMail;
use crate::email::model::{
    Email, EmailAccount, EmailAttachment, EmailAttachmentMeta, EmailFolder, EmailThread,
    FolderRole, MailAddress, OutboxItem, OutboxState,
};
use crate::email::thread::{normalize_subject, thread_candidates};

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
    Db(#[from] DbError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("sync engine error: {0}")]
    Engine(String),
    #[error("no secret service provider available")]
    NoSecretService,
    #[error("not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, EmailError>;

/// Reduced, kind-only description of a transport error — never includes
/// command echoes (which can contain credentials).
pub fn sanitize_imap_error(e: &imap::Error) -> String {
    match e {
        imap::Error::Io(_) => "io".to_string(),
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

// ── EmailStore ────────────────────────────────────────────────────────────

pub struct EmailStore<'a> {
    db: &'a Db,
}

/// Input for ingest (what a header sync produces).
#[derive(Debug, Clone)]
pub struct IngestMail {
    pub account_id: Uuid,
    pub folder_id: Uuid,
    pub mail: RemoteMail,
}

impl<'a> EmailStore<'a> {
    pub fn new(db: &'a Db) -> Self {
        EmailStore { db }
    }

    fn conn(&self) -> &Connection {
        self.db.conn()
    }

    // ── accounts ──────────────────────────────────────────────────────────

    pub fn create_account(&self, input: &NewAccount) -> Result<EmailAccount> {
        let id = Uuid::new_v4();
        let now = chrono::Utc::now().to_rfc3339();
        self.conn().execute(
            "INSERT INTO email_accounts
                 (id, label, address, imap_host, imap_port, smtp_host, smtp_port,
                  username, color_idx, sync_interval_s, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11)",
            params![
                id.to_string(),
                input.label,
                input.address,
                input.imap_host,
                i64::from(input.imap_port),
                input.smtp_host,
                i64::from(input.smtp_port),
                input.username,
                input.color_idx,
                input.sync_interval_s,
                now
            ],
        )?;
        self.get_account(id)
    }

    pub fn list_accounts(&self) -> Result<Vec<EmailAccount>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, label, address, imap_host, imap_port, smtp_host, smtp_port,
                    username, color_idx, sync_interval_s, last_sync_at, created_at, updated_at
             FROM email_accounts ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], account_from_row)?;
        collect(rows)
    }

    pub fn get_account(&self, id: Uuid) -> Result<EmailAccount> {
        self.conn()
            .query_row(
                "SELECT id, label, address, imap_host, imap_port, smtp_host, smtp_port,
                        username, color_idx, sync_interval_s, last_sync_at, created_at, updated_at
                 FROM email_accounts WHERE id = ?1",
                params![id.to_string()],
                account_from_row,
            )
            .optional()?
            .ok_or_else(|| EmailError::NotFound(id.to_string()))
    }

    pub fn update_account(&self, account: &EmailAccount) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE email_accounts SET label=?2, address=?3, imap_host=?4, imap_port=?5,
                 smtp_host=?6, smtp_port=?7, username=?8, color_idx=?9, sync_interval_s=?10
             WHERE id=?1",
            params![
                account.id.to_string(),
                account.label,
                account.address,
                account.imap_host,
                i64::from(account.imap_port),
                account.smtp_host,
                i64::from(account.smtp_port),
                account.username,
                account.color_idx,
                account.sync_interval_s,
            ],
        )?;
        if n == 0 {
            return Err(EmailError::NotFound(account.id.to_string()));
        }
        Ok(())
    }

    pub fn delete_account(&self, id: Uuid) -> Result<()> {
        self.conn().execute(
            "DELETE FROM email_accounts WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    pub fn set_last_sync(&self, id: Uuid, at: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE email_accounts SET last_sync_at = ?2 WHERE id = ?1",
            params![id.to_string(), at],
        )?;
        Ok(())
    }

    // ── folders ───────────────────────────────────────────────────────────

    /// Upsert a folder by (account_id, name); preserves stored uid state.
    pub fn upsert_folder(
        &self,
        account_id: Uuid,
        name: &str,
        role: FolderRole,
    ) -> Result<EmailFolder> {
        let id = Uuid::new_v4();
        self.conn().execute(
            "INSERT INTO email_folders (id, account_id, name, role)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (account_id, name) DO UPDATE SET role = excluded.role",
            params![id.to_string(), account_id.to_string(), name, role.as_str()],
        )?;
        self.folder_by_name(account_id, name)?
            .ok_or_else(|| EmailError::NotFound(name.to_string()))
    }

    pub fn folder_by_name(&self, account_id: Uuid, name: &str) -> Result<Option<EmailFolder>> {
        self.conn()
            .query_row(
                "SELECT id, account_id, name, role, uidvalidity, last_uid
                 FROM email_folders WHERE account_id = ?1 AND name = ?2",
                params![account_id.to_string(), name],
                folder_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn folders_for(&self, account_id: Uuid) -> Result<Vec<EmailFolder>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, account_id, name, role, uidvalidity, last_uid
             FROM email_folders WHERE account_id = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map(params![account_id.to_string()], folder_from_row)?;
        collect(rows)
    }

    pub fn folder_by_id(&self, id: Uuid) -> Result<EmailFolder> {
        self.conn()
            .query_row(
                "SELECT id, account_id, name, role, uidvalidity, last_uid
                 FROM email_folders WHERE id = ?1",
                params![id.to_string()],
                folder_from_row,
            )
            .optional()?
            .ok_or_else(|| EmailError::NotFound(id.to_string()))
    }

    pub fn set_folder_uidvalidity(&self, folder_id: Uuid, uidvalidity: Option<i64>) -> Result<()> {
        self.conn().execute(
            "UPDATE email_folders SET uidvalidity = ?2 WHERE id = ?1",
            params![folder_id.to_string(), uidvalidity],
        )?;
        Ok(())
    }

    pub fn set_folder_last_uid(&self, folder_id: Uuid, last_uid: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE email_folders SET last_uid = ?2 WHERE id = ?1",
            params![folder_id.to_string(), last_uid],
        )?;
        Ok(())
    }

    /// Folder sync decision: full sync when UIDVALIDITY changed/missing
    /// state, incremental otherwise.
    pub fn folder_sync_state(
        &self,
        folder: &EmailFolder,
    ) -> Result<crate::email::model::SyncFolderState> {
        use crate::email::model::SyncFolderState;
        if folder.last_uid == 0 {
            return Ok(SyncFolderState::Full);
        }
        Ok(SyncFolderState::IncrementalFrom(
            u32::try_from(folder.last_uid).unwrap_or(0),
        ))
    }

    /// On UIDVALIDITY change: drop folder contents so the full resync
    /// re-ingests cleanly (thread_ids stay stable — §5 matches by
    /// message_id first, independent of ingest order).
    pub fn clear_folder_mail(&self, folder_id: Uuid) -> Result<()> {
        self.conn().execute(
            "DELETE FROM emails WHERE folder_id = ?1",
            params![folder_id.to_string()],
        )?;
        self.set_folder_last_uid(folder_id, 0)?;
        Ok(())
    }

    // ── ingest ────────────────────────────────────────────────────────────

    /// Idempotent ingest (UNIQUE(folder_id, uid)); computes thread_id per
    /// §5. Existing rows get flags refreshed. Returns the stored email.
    pub fn ingest(&self, input: &IngestMail) -> Result<Email> {
        let m = &input.mail;
        let thread_id = self.compute_thread_id(
            input.account_id,
            m.message_id.as_deref(),
            m.in_reply_to.as_deref(),
            &m.references,
            &m.subject,
        );
        let id = Uuid::new_v4();
        let now = chrono::Utc::now().to_rfc3339();
        let flags = m.flags.join(" ");
        let to_json = serde_json::to_string(
            &m.to
                .iter()
                .map(|(n, a)| json!({"name": n, "addr": a}))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string());
        let cc_json = serde_json::to_string(
            &m.cc
                .iter()
                .map(|(n, a)| json!({"name": n, "addr": a}))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string());
        self.conn().execute(
            "INSERT INTO emails
                 (id, account_id, folder_id, uid, message_id, thread_id,
                  from_name, from_addr, to_json, cc_json, subject, date, flags,
                  size, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?15)
             ON CONFLICT (folder_id, uid) DO UPDATE SET
                 message_id = excluded.message_id,
                 thread_id = excluded.thread_id,
                 from_name = excluded.from_name,
                 from_addr = excluded.from_addr,
                 to_json = excluded.to_json,
                 cc_json = excluded.cc_json,
                 subject = excluded.subject,
                 date = excluded.date,
                 flags = excluded.flags,
                 size = excluded.size",
            params![
                id.to_string(),
                input.account_id.to_string(),
                input.folder_id.to_string(),
                i64::from(m.uid),
                m.message_id,
                thread_id.to_string(),
                m.from_name,
                m.from_addr,
                to_json,
                cc_json,
                m.subject,
                m.date,
                flags,
                i64::try_from(m.size).unwrap_or(i64::MAX),
                now
            ],
        )?;
        let email = self.email_by_folder_uid(input.folder_id, m.uid)?;
        email.ok_or_else(|| EmailError::NotFound(format!("uid {}", m.uid)))
    }

    /// §5: match by threading candidates (message_id) → subject fallback →
    /// new uuid. Deterministic across resyncs (candidate match first).
    pub fn compute_thread_id(
        &self,
        account_id: Uuid,
        message_id: Option<&str>,
        in_reply_to: Option<&str>,
        references: &[String],
        subject: &str,
    ) -> Uuid {
        let candidates = thread_candidates(message_id, in_reply_to, references);
        for cand in &candidates {
            // No rows → error → skipped; thread_id is NOT NULL so a hit
            // always yields a parseable uuid.
            if let Ok(found) = self.conn().query_row(
                "SELECT thread_id FROM emails
                 WHERE account_id = ?1 AND message_id = ?2 LIMIT 1",
                params![account_id.to_string(), cand],
                |row| row.get::<_, String>(0),
            ) {
                if let Ok(uuid) = Uuid::parse_str(&found) {
                    return uuid;
                }
            }
        }
        let normalized = normalize_subject(subject);
        if !normalized.is_empty() {
            // Subject fallback (broken mail with no threading headers):
            // normalized comparison happens in Rust — SQL can't strip
            // Re:/Fwd: prefixes. Bounded to this account; the message-id
            // path above handles the common case first.
            let Ok(mut stmt) = self
                .conn()
                .prepare("SELECT thread_id, subject FROM emails WHERE account_id = ?1")
            else {
                return fallback_thread_id(account_id, message_id, subject);
            };
            let rows = stmt.query_map(params![account_id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            });
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    if normalize_subject(&row.1) == normalized {
                        if let Ok(uuid) = Uuid::parse_str(&row.0) {
                            return uuid;
                        }
                    }
                }
            }
        }
        fallback_thread_id(account_id, message_id, subject)
    }

    // ── queries ───────────────────────────────────────────────────────────

    pub fn email_by_folder_uid(&self, folder_id: Uuid, uid: u32) -> Result<Option<Email>> {
        self.conn()
            .query_row(
                "SELECT id, account_id, folder_id, uid, message_id, thread_id,
                        from_name, from_addr, to_json, cc_json, subject, snippet,
                        date, flags, has_attachments, body_text, body_html,
                        body_fetched_at, size, created_at, updated_at
                 FROM emails WHERE folder_id = ?1 AND uid = ?2",
                params![folder_id.to_string(), i64::from(uid)],
                email_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get_email(&self, id: Uuid) -> Result<Email> {
        self.conn()
            .query_row(
                "SELECT id, account_id, folder_id, uid, message_id, thread_id,
                        from_name, from_addr, to_json, cc_json, subject, snippet,
                        date, flags, has_attachments, body_text, body_html,
                        body_fetched_at, size, created_at, updated_at
                 FROM emails WHERE id = ?1",
                params![id.to_string()],
                email_from_row,
            )
            .optional()?
            .ok_or_else(|| EmailError::NotFound(id.to_string()))
    }

    /// Thread groups for the unified inbox or one account. Groups within
    /// (account_id, thread_id); latest activity first.
    pub fn threads(&self, account_id: Option<Uuid>, limit: usize) -> Result<Vec<EmailThread>> {
        let sql = "SELECT thread_id, account_id FROM emails
             WHERE (?1 IS NULL OR account_id = ?1)
             GROUP BY account_id, thread_id
             ORDER BY MAX(date) DESC
             LIMIT ?2";
        let mut stmt = self.conn().prepare(sql)?;
        let account = account_id.map(|a| a.to_string());
        let groups: Vec<(String, String)> = {
            let rows = stmt.query_map(
                params![account, i64::try_from(limit).unwrap_or(i64::MAX)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };
        let mut out = Vec::new();
        for (thread_id, acc_id) in groups {
            let messages = self.thread_messages(
                Uuid::parse_str(&acc_id).unwrap_or_default(),
                Uuid::parse_str(&thread_id).unwrap_or_default(),
            )?;
            if let Some(t) = thread_from_messages(messages) {
                out.push(t);
            }
        }
        Ok(out)
    }

    /// Messages of one thread, latest first.
    pub fn thread_messages(&self, account_id: Uuid, thread_id: Uuid) -> Result<Vec<Email>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, account_id, folder_id, uid, message_id, thread_id,
                    from_name, from_addr, to_json, cc_json, subject, snippet,
                    date, flags, has_attachments, body_text, body_html,
                    body_fetched_at, size, created_at, updated_at
             FROM emails
             WHERE account_id = ?1 AND thread_id = ?2
             ORDER BY date DESC",
        )?;
        let rows = stmt.query_map(
            params![account_id.to_string(), thread_id.to_string()],
            email_from_row,
        )?;
        collect(rows)
    }

    pub fn unread_count(&self, account_id: Uuid) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM emails
             WHERE account_id = ?1 AND folder_id IN
                 (SELECT id FROM email_folders WHERE role = 'inbox')
               AND flags NOT LIKE '%seen%'",
            params![account_id.to_string()],
            |row| row.get(0),
        )?)
    }

    // ── body + attachments ────────────────────────────────────────────────

    /// Store a fetched body: text (dumb-stripped when HTML-only), raw HTML
    /// (when present — never rendered in M2), refreshed snippet, attachment
    /// metadata (content stays NULL until saved).
    pub fn set_body(
        &self,
        email_id: Uuid,
        body_text: &str,
        body_html: Option<&str>,
        attachments: &[EmailAttachmentMeta],
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let snippet: String = body_text.chars().take(140).collect();
        let has_attachments = i64::from(!attachments.is_empty());
        self.conn().execute(
            "UPDATE emails SET body_text = ?2, body_html = ?3,
                 body_fetched_at = ?4, snippet = ?5, has_attachments = ?6
             WHERE id = ?1",
            params![
                email_id.to_string(),
                body_text,
                body_html,
                now,
                snippet,
                has_attachments
            ],
        )?;
        for att in attachments {
            self.conn().execute(
                "INSERT INTO email_attachments (id, email_id, filename, mime, size)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (email_id, filename) DO NOTHING",
                params![
                    Uuid::new_v4().to_string(),
                    email_id.to_string(),
                    att.filename,
                    att.mime,
                    att.size
                ],
            )?;
        }
        Ok(())
    }

    pub fn attachments_for(&self, email_id: Uuid) -> Result<Vec<EmailAttachment>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, email_id, filename, mime, size, content IS NOT NULL
             FROM email_attachments WHERE email_id = ?1 ORDER BY filename",
        )?;
        let rows = stmt.query_map(params![email_id.to_string()], |row| {
            Ok(EmailAttachment {
                id: parse_uuid(&row.get::<_, String>(0)?)?,
                email_id: parse_uuid(&row.get::<_, String>(1)?)?,
                filename: row.get(2)?,
                mime: row.get(3)?,
                size: row.get(4)?,
                has_content: row.get::<_, i64>(5)? == 1,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn set_attachment_content(&self, attachment_id: Uuid, content: &[u8]) -> Result<()> {
        self.conn().execute(
            "UPDATE email_attachments SET content = ?2 WHERE id = ?1",
            params![attachment_id.to_string(), content],
        )?;
        Ok(())
    }

    pub fn attachment_content(&self, attachment_id: Uuid) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT content FROM email_attachments WHERE id = ?1",
                params![attachment_id.to_string()],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Startup cache eviction: bodies fetched >90 days ago are dropped.
    pub fn evict_body_cache(&self) -> Result<usize> {
        let n = self.conn().execute(
            "UPDATE emails SET body_text = NULL, body_html = NULL, body_fetched_at = NULL
             WHERE body_fetched_at IS NOT NULL
               AND body_fetched_at < datetime('now', '-90 days')",
            [],
        )?;
        Ok(n)
    }

    // ── flags + write-op queue (optimistic local, engine reconciles) ──────

    /// Apply a flag change locally AND enqueue the server write op.
    pub fn mark_seen(&self, email_id: Uuid, seen: bool) -> Result<()> {
        let email = self.get_email(email_id)?;
        let mut flags: Vec<String> = email
            .flags
            .iter()
            .filter(|f| f.as_str() != "seen")
            .cloned()
            .collect();
        if seen {
            flags.push("seen".to_string());
        }
        self.conn().execute(
            "UPDATE emails SET flags = ?2 WHERE id = ?1",
            params![email_id.to_string(), flags.join(" ")],
        )?;
        self.enqueue_write_op(email_id, if seen { "set_seen" } else { "unset_seen" }, None)?;
        Ok(())
    }

    pub fn enqueue_write_op(&self, email_id: Uuid, op: &str, arg: Option<&str>) -> Result<()> {
        if !["set_seen", "unset_seen", "move_folder"].contains(&op) {
            return Err(EmailError::Engine(format!("unknown write op {op:?}")));
        }
        self.conn().execute(
            "INSERT INTO email_write_ops (id, email_id, op, arg, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                Uuid::new_v4().to_string(),
                email_id.to_string(),
                op,
                arg,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn pending_write_ops(&self) -> Result<Vec<WriteOp>> {
        let mut stmt = self.conn().prepare(
            "SELECT w.id, w.email_id, w.op, w.arg
             FROM email_write_ops w
             WHERE w.state = 'pending'
             ORDER BY w.created_at",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WriteOp {
                id: parse_uuid(&row.get::<_, String>(0)?)?,
                email_id: parse_uuid(&row.get::<_, String>(1)?)?,
                op: row.get(2)?,
                arg: row.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn mark_write_op(&self, id: Uuid, done: bool, error: Option<&str>) -> Result<()> {
        self.conn().execute(
            "UPDATE email_write_ops SET state = ?2, error = ?3 WHERE id = ?1",
            params![id.to_string(), if done { "done" } else { "failed" }, error],
        )?;
        Ok(())
    }

    // ── outbox ────────────────────────────────────────────────────────────

    pub fn stage_outbox(&self, input: &NewOutbox) -> Result<OutboxItem> {
        let id = Uuid::new_v4();
        let now = chrono::Utc::now().to_rfc3339();
        let to_json = serde_json::to_string(&input.to).unwrap_or_else(|_| "[]".to_string());
        let cc_json = serde_json::to_string(&input.cc).unwrap_or_else(|_| "[]".to_string());
        let bcc_json = serde_json::to_string(&input.bcc).unwrap_or_else(|_| "[]".to_string());
        self.conn().execute(
            "INSERT INTO email_outbox
                 (id, account_id, to_json, cc_json, bcc_json, subject, body,
                  in_reply_to, source_email_id, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",
            params![
                id.to_string(),
                input.account_id.to_string(),
                to_json,
                cc_json,
                bcc_json,
                input.subject,
                input.body,
                input.in_reply_to,
                input.source_email_id.map(|s| s.to_string()),
                now
            ],
        )?;
        self.get_outbox_item(id)
    }

    pub fn get_outbox_item(&self, id: Uuid) -> Result<OutboxItem> {
        self.conn()
            .query_row(
                "SELECT id, account_id, state, to_json, cc_json, bcc_json, subject,
                        body, in_reply_to, source_email_id, origin, error,
                        created_at, updated_at, sent_at
                 FROM email_outbox WHERE id = ?1",
                params![id.to_string()],
                outbox_from_row,
            )
            .optional()?
            .ok_or_else(|| EmailError::NotFound(id.to_string()))
    }

    pub fn list_outbox(&self, include_done: bool) -> Result<Vec<OutboxItem>> {
        let sql = if include_done {
            "SELECT id, account_id, state, to_json, cc_json, bcc_json, subject,
                    body, in_reply_to, source_email_id, origin, error,
                    created_at, updated_at, sent_at
             FROM email_outbox
             WHERE state != 'discarded'
             ORDER BY created_at DESC"
        } else {
            "SELECT id, account_id, state, to_json, cc_json, bcc_json, subject,
                    body, in_reply_to, source_email_id, origin, error,
                    created_at, updated_at, sent_at
             FROM email_outbox
             WHERE state IN ('staged','approved','sending','failed')
             ORDER BY created_at DESC"
        };
        let mut stmt = self.conn().prepare(sql)?;
        let rows = stmt.query_map([], outbox_from_row)?;
        collect(rows)
    }

    pub fn set_outbox_state(
        &self,
        id: Uuid,
        state: OutboxState,
        error: Option<&str>,
    ) -> Result<()> {
        let sent_at = if state == OutboxState::Sent {
            Some(chrono::Utc::now().to_rfc3339())
        } else {
            None
        };
        self.conn().execute(
            "UPDATE email_outbox SET state = ?2, error = ?3,
                 sent_at = COALESCE(?4, sent_at)
             WHERE id = ?1",
            params![id.to_string(), state.as_str(), error, sent_at],
        )?;
        Ok(())
    }

    pub fn update_outbox_draft(
        &self,
        id: Uuid,
        to: &[MailAddress],
        cc: &[MailAddress],
        bcc: &[MailAddress],
        subject: &str,
        body: &str,
    ) -> Result<()> {
        let to_json = serde_json::to_string(to).unwrap_or_else(|_| "[]".to_string());
        let cc_json = serde_json::to_string(cc).unwrap_or_else(|_| "[]".to_string());
        let bcc_json = serde_json::to_string(bcc).unwrap_or_else(|_| "[]".to_string());
        self.conn().execute(
            "UPDATE email_outbox SET to_json=?2, cc_json=?3, bcc_json=?4,
                 subject=?5, body=?6 WHERE id=?1",
            params![id.to_string(), to_json, cc_json, bcc_json, subject, body],
        )?;
        Ok(())
    }

    /// Outbox items ready for the engine to send.
    pub fn approved_outbox(&self) -> Result<Vec<OutboxItem>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, account_id, state, to_json, cc_json, bcc_json, subject,
                    body, in_reply_to, source_email_id, origin, error,
                    created_at, updated_at, sent_at
             FROM email_outbox WHERE state = 'approved' ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], outbox_from_row)?;
        collect(rows)
    }

    /// Move a message's local folder row (applied by the engine after a
    /// server-side move succeeds — the UI never moves rows optimistically).
    pub fn move_email_local(&self, email_id: Uuid, folder_id: Uuid) -> Result<()> {
        self.conn().execute(
            "UPDATE emails SET folder_id = ?2 WHERE id = ?1",
            params![email_id.to_string(), folder_id.to_string()],
        )?;
        Ok(())
    }

    /// Subject/from search for the todo picker's email entries
    /// (todo → email `mentions` links). Returns (id, subject, from_addr).
    pub fn search_emails(&self, needle: &str, limit: usize) -> Result<Vec<(Uuid, String, String)>> {
        let pattern = format!("%{}%", needle.to_lowercase());
        let mut stmt = self.conn().prepare(
            "SELECT id, subject, from_addr FROM emails
             WHERE lower(subject) LIKE ?1 OR lower(from_addr) LIKE ?1
             ORDER BY date DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![pattern, i64::try_from(limit).unwrap_or(10)],
            |row| {
                Ok((
                    parse_uuid(&row.get::<_, String>(0)?)?,
                    row.get(1)?,
                    row.get(2)?,
                ))
            },
        )?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    // ── sync log ──────────────────────────────────────────────────────────

    pub fn log_sync(&self, account_id: Uuid, event: &str, detail: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO email_sync_log (id, account_id, event, detail, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                Uuid::new_v4().to_string(),
                account_id.to_string(),
                event,
                detail,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }
}

/// A pending server write operation.
#[derive(Debug, Clone)]
pub struct WriteOp {
    pub id: Uuid,
    pub email_id: Uuid,
    pub op: String,
    pub arg: Option<String>,
}

// ── row mapping helpers ───────────────────────────────────────────────────

fn account_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EmailAccount> {
    Ok(EmailAccount {
        id: parse_uuid(&row.get::<_, String>(0)?)?,
        label: row.get(1)?,
        address: row.get(2)?,
        imap_host: row.get(3)?,
        imap_port: u16::try_from(row.get::<_, i64>(4)?).unwrap_or(993),
        smtp_host: row.get(5)?,
        smtp_port: u16::try_from(row.get::<_, i64>(6)?).unwrap_or(465),
        username: row.get(7)?,
        color_idx: row.get(8)?,
        sync_interval_s: row.get(9)?,
        last_sync_at: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn folder_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EmailFolder> {
    let role: String = row.get(3)?;
    Ok(EmailFolder {
        id: parse_uuid(&row.get::<_, String>(0)?)?,
        account_id: parse_uuid(&row.get::<_, String>(1)?)?,
        name: row.get(2)?,
        role: FolderRole::from_str(&role).unwrap_or(FolderRole::Other),
        uidvalidity: row.get(4)?,
        last_uid: row.get(5)?,
    })
}

fn email_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Email> {
    let to_json: String = row.get(8)?;
    let cc_json: String = row.get(9)?;
    let flags: String = row.get(13)?;
    Ok(Email {
        id: parse_uuid(&row.get::<_, String>(0)?)?,
        account_id: parse_uuid(&row.get::<_, String>(1)?)?,
        folder_id: parse_uuid(&row.get::<_, String>(2)?)?,
        uid: row.get(3)?,
        message_id: row.get(4)?,
        thread_id: parse_uuid(&row.get::<_, String>(5)?)?,
        from_name: row.get(6)?,
        from_addr: row.get(7)?,
        to: serde_json::from_str(&to_json).unwrap_or_default(),
        cc: serde_json::from_str(&cc_json).unwrap_or_default(),
        subject: row.get(10)?,
        snippet: row.get(11)?,
        date: row.get(12)?,
        flags: flags.split_whitespace().map(str::to_string).collect(),
        has_attachments: row.get::<_, i64>(14)? == 1,
        body_text: row.get(15)?,
        body_html: row.get(16)?,
        body_fetched_at: row.get(17)?,
        size: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
    })
}

fn outbox_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxItem> {
    let to_json: String = row.get(3)?;
    let cc_json: String = row.get(4)?;
    let bcc_json: String = row.get(5)?;
    let state: String = row.get(2)?;
    let src: Option<String> = row.get(9)?;
    Ok(OutboxItem {
        id: parse_uuid(&row.get::<_, String>(0)?)?,
        account_id: parse_uuid(&row.get::<_, String>(1)?)?,
        state: OutboxState::from_str(&state).unwrap_or(OutboxState::Failed),
        to: serde_json::from_str(&to_json).unwrap_or_default(),
        cc: serde_json::from_str(&cc_json).unwrap_or_default(),
        bcc: serde_json::from_str(&bcc_json).unwrap_or_default(),
        subject: row.get(6)?,
        body: row.get(7)?,
        in_reply_to: row.get(8)?,
        source_email_id: src.and_then(|s| Uuid::parse_str(&s).ok()),
        origin: row.get(10)?,
        error: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        sent_at: row.get(14)?,
    })
}

fn thread_from_messages(messages: Vec<Email>) -> Option<EmailThread> {
    let first = messages.first()?.clone();
    let unread_count = messages.iter().filter(|m| !m.is_seen()).count();
    let subject = first.subject.clone();
    let latest = messages.iter().map(|m| m.date.clone()).max()?;
    Some(EmailThread {
        thread_id: first.thread_id,
        account_id: first.account_id,
        subject,
        latest_date: latest,
        message_count: messages.len(),
        unread_count,
        has_attachments: messages.iter().any(|m| m.has_attachments),
        snippet: first.snippet.clone(),
        messages,
    })
}

/// Deterministic fresh-thread id: derived from (account, identity) so a
/// UIDVALIDITY resync re-ingests to the SAME thread_id (plan risk 10 —
/// "match by message_id first, deterministic"). Identity = message-id when
/// present, else the normalized subject.
fn fallback_thread_id(account_id: Uuid, message_id: Option<&str>, subject: &str) -> Uuid {
    let identity = message_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| normalize_subject(subject));
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("{account_id}:{identity}").as_bytes(),
    )
}

fn parse_uuid(s: &str) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn collect<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    let mut v = Vec::new();
    for r in rows {
        v.push(r?);
    }
    Ok(v)
}

/// Input for `EmailStore::create_account`.
#[derive(Debug, Clone)]
pub struct NewAccount<'a> {
    pub label: &'a str,
    pub address: &'a str,
    pub imap_host: &'a str,
    pub imap_port: u16,
    pub smtp_host: &'a str,
    pub smtp_port: u16,
    pub username: &'a str,
    pub color_idx: i64,
    pub sync_interval_s: i64,
}

/// Input for `EmailStore::stage_outbox`.
#[derive(Debug, Clone)]
pub struct NewOutbox {
    pub account_id: Uuid,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub bcc: Vec<MailAddress>,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub source_email_id: Option<Uuid>,
}
