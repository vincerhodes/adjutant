//! Email domain model.

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct EmailAccount {
    pub id: Uuid,
    pub label: String,
    pub address: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    pub color_idx: i64,
    pub sync_interval_s: i64,
    pub last_sync_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FolderRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Archive,
    Junk,
    Other,
}

impl FolderRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            FolderRole::Inbox => "inbox",
            FolderRole::Sent => "sent",
            FolderRole::Drafts => "drafts",
            FolderRole::Trash => "trash",
            FolderRole::Archive => "archive",
            FolderRole::Junk => "junk",
            FolderRole::Other => "other",
        }
    }
}

impl FromStr for FolderRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "inbox" => Ok(FolderRole::Inbox),
            "sent" => Ok(FolderRole::Sent),
            "drafts" => Ok(FolderRole::Drafts),
            "trash" => Ok(FolderRole::Trash),
            "archive" => Ok(FolderRole::Archive),
            "junk" => Ok(FolderRole::Junk),
            "other" => Ok(FolderRole::Other),
            _ => Err(format!("unknown folder role {s:?}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmailFolder {
    pub id: Uuid,
    pub account_id: Uuid,
    pub name: String,
    pub role: FolderRole,
    pub uidvalidity: Option<i64>,
    pub last_uid: i64,
}

/// A parsed address pair as stored in to_json/cc_json.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MailAddress {
    pub name: Option<String>,
    pub addr: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Email {
    pub id: Uuid,
    pub account_id: Uuid,
    pub folder_id: Uuid,
    pub uid: i64,
    pub message_id: Option<String>,
    pub thread_id: Uuid,
    pub from_name: Option<String>,
    pub from_addr: String,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub subject: String,
    pub snippet: String,
    pub date: String,
    pub flags: Vec<String>,
    pub has_attachments: bool,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub body_fetched_at: Option<String>,
    pub size: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl Email {
    pub fn is_seen(&self) -> bool {
        self.flags.iter().any(|f| f == "seen")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmailAttachment {
    pub id: Uuid,
    pub email_id: Uuid,
    pub filename: String,
    pub mime: String,
    pub size: i64,
    pub has_content: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxState {
    Staged,
    Approved,
    Sending,
    Sent,
    Failed,
    Discarded,
}

impl OutboxState {
    pub fn as_str(&self) -> &'static str {
        match self {
            OutboxState::Staged => "staged",
            OutboxState::Approved => "approved",
            OutboxState::Sending => "sending",
            OutboxState::Sent => "sent",
            OutboxState::Failed => "failed",
            OutboxState::Discarded => "discarded",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            OutboxState::Staged => "Staged",
            OutboxState::Approved => "Approved",
            OutboxState::Sending => "Sending",
            OutboxState::Sent => "Sent",
            OutboxState::Failed => "Failed",
            OutboxState::Discarded => "Discarded",
        }
    }
}

impl FromStr for OutboxState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "staged" => Ok(OutboxState::Staged),
            "approved" => Ok(OutboxState::Approved),
            "sending" => Ok(OutboxState::Sending),
            "sent" => Ok(OutboxState::Sent),
            "failed" => Ok(OutboxState::Failed),
            "discarded" => Ok(OutboxState::Discarded),
            _ => Err(format!("unknown outbox state {s:?}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutboxItem {
    pub id: Uuid,
    pub account_id: Uuid,
    pub state: OutboxState,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub bcc: Vec<MailAddress>,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub source_email_id: Option<Uuid>,
    pub origin: String,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub sent_at: Option<String>,
}

/// A display-level thread group (SQL GROUP BY thread_id — no threads table).
#[derive(Debug, Clone, PartialEq)]
pub struct EmailThread {
    pub thread_id: Uuid,
    pub account_id: Uuid,
    pub subject: String,
    pub latest_date: String,
    pub message_count: usize,
    pub unread_count: usize,
    pub has_attachments: bool,
    pub snippet: String,
    /// Latest message first.
    pub messages: Vec<Email>,
}

/// Per-folder sync high-water state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncFolderState {
    /// New folder or UIDVALIDITY changed — full (capped) sync needed.
    Full,
    /// Incremental from `last_uid`.
    IncrementalFrom(u32),
}

/// Attachment metadata captured at body-fetch time.
#[derive(Debug, Clone)]
pub struct EmailAttachmentMeta {
    pub filename: String,
    pub mime: String,
    pub size: i64,
}
