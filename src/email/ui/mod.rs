//! Email module UI: sidebar Accounts section, unified/per-account inbox,
//! reading pane, compose, outbox review, account management.
//!
//! Zero SQL in this file tree — all data access goes through `EmailStore`;
//! the sync engine owns all network. Card language per M1.5/§8.

pub mod accounts;
pub mod compose_ui;
pub mod list;
pub mod outbox;
pub mod reading;

use std::collections::HashMap;

use egui::{Context, Ui};
use uuid::Uuid;

use crate::db::Db;
use crate::email::model::{EmailAccount, EmailThread, OutboxItem, OutboxState};
use crate::email::sync::{SyncEngine, SyncEvent};
use crate::email::EmailStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailView {
    Inbox,
    Outbox,
    Accounts,
}

/// Full-pane compose editor state.
#[derive(Debug, Clone)]
pub struct ComposeState {
    /// Some when editing an existing outbox draft.
    pub editing_outbox_id: Option<Uuid>,
    pub account_id: Uuid,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub source_email_id: Option<Uuid>,
}

/// Reading-pane state: which thread is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadingState {
    pub account_id: Uuid,
    pub thread_id: Uuid,
}

pub struct EmailUi {
    pub view: EmailView,
    /// None = Unified (all accounts).
    pub account_filter: Option<Uuid>,
    pub reading: Option<ReadingState>,
    pub compose: Option<ComposeState>,
    pub account_form: Option<accounts::AccountForm>,
    pub filter: String,
    pub filter_active: bool,
    accounts: Vec<EmailAccount>,
    threads: Vec<EmailThread>,
    outbox: Vec<OutboxItem>,
    unread: HashMap<Uuid, i64>,
    dirty: bool,
    /// Bodies the engine was asked for (spinner while pending).
    body_pending: std::collections::HashSet<Uuid>,
}

impl EmailUi {
    pub fn new() -> EmailUi {
        EmailUi {
            view: EmailView::Inbox,
            account_filter: None,
            reading: None,
            compose: None,
            account_form: None,
            filter: String::new(),
            filter_active: false,
            accounts: Vec::new(),
            threads: Vec::new(),
            outbox: Vec::new(),
            unread: HashMap::new(),
            dirty: true,
            body_pending: std::collections::HashSet::new(),
        }
    }

    /// Accounts section for the app sidebar: Unified + per-account dots and
    /// unread counts, Outbox, account management.
    pub fn sidebar(&mut self, ui: &mut Ui, _db: &Db) {
        use crate::ui::icons;
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Accounts")
                .size(crate::ui::fonts::SIZE_SMALL)
                .strong(),
        );
        ui.add_space(4.0);

        let unified_selected = self.view == EmailView::Inbox && self.account_filter.is_none();
        let unread_total: i64 = self.unread.values().sum();
        let unified_label = if unread_total > 0 {
            format!("Unified ({unread_total})")
        } else {
            "Unified".to_string()
        };
        if ui
            .selectable_label(unified_selected, unified_label)
            .clicked()
        {
            self.view = EmailView::Inbox;
            self.account_filter = None;
            self.reading = None;
            self.dirty = true;
        }

        let accounts = self.accounts.clone();
        for account in &accounts {
            let selected = self.view == EmailView::Inbox && self.account_filter == Some(account.id);
            let unread = self.unread.get(&account.id).copied().unwrap_or(0);
            ui.horizontal(|ui| {
                let color = icons::GROUP_DOT_COLORS
                    [(account.color_idx.rem_euclid(6)) as usize % icons::GROUP_DOT_COLORS.len()];
                icons::group_dot(ui, color, 10.0);
                let label = if unread > 0 {
                    format!("{} ({unread})", account.label)
                } else {
                    account.label.clone()
                };
                if ui.selectable_label(selected, label).clicked() {
                    self.view = EmailView::Inbox;
                    self.account_filter = Some(account.id);
                    self.reading = None;
                    self.dirty = true;
                }
            });
        }

        ui.add_space(8.0);
        let pending_outbox = self
            .outbox
            .iter()
            .filter(|i| matches!(i.state, OutboxState::Staged | OutboxState::Failed))
            .count();
        let outbox_label = if pending_outbox > 0 {
            format!("Outbox ({pending_outbox})")
        } else {
            "Outbox".to_string()
        };
        if ui
            .selectable_label(self.view == EmailView::Outbox, outbox_label)
            .clicked()
        {
            self.view = EmailView::Outbox;
            self.reading = None;
            self.compose = None;
            self.dirty = true;
        }
        if ui
            .selectable_label(self.view == EmailView::Accounts, "Manage accounts")
            .clicked()
        {
            self.view = EmailView::Accounts;
            self.reading = None;
            self.compose = None;
            self.account_form = None;
            self.dirty = true;
        }
    }

    /// Apply engine events; returns true when a repaint is warranted.
    pub fn handle_events(&mut self, events: &[SyncEvent]) -> bool {
        let mut repaint = false;
        for ev in events {
            match ev {
                SyncEvent::FolderSynced { .. }
                | SyncEvent::SyncOk { .. }
                | SyncEvent::SendResult { .. }
                | SyncEvent::AccountTested { .. } => {
                    self.dirty = true;
                    repaint = true;
                }
                SyncEvent::BodyReady { email_id } => {
                    self.body_pending.remove(email_id);
                    self.dirty = true;
                    repaint = true;
                }
                SyncEvent::BodyErr { email_id, .. } => {
                    self.body_pending.remove(email_id);
                    repaint = true;
                }
                SyncEvent::SyncErr { .. } | SyncEvent::WriteOpResult { .. } => {
                    repaint = true;
                }
                SyncEvent::SyncStart { .. } => {}
            }
        }
        repaint
    }

    pub fn is_busy(&self) -> bool {
        self.compose.is_some()
            || self.account_form.is_some()
            || self.filter_active
            || !self.body_pending.is_empty()
    }

    fn reload(&mut self, db: &Db) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let store = EmailStore::new(db);
        self.accounts = store.list_accounts().unwrap_or_default();
        if self.accounts.is_empty() {
            self.threads = Vec::new();
            self.unread = HashMap::new();
        } else {
            self.threads = store.threads(self.account_filter, 500).unwrap_or_default();
            self.unread = self
                .accounts
                .iter()
                .map(|a| (a.id, store.unread_count(a.id).unwrap_or(0)))
                .collect();
        }
        self.outbox = store.list_outbox(false).unwrap_or_default();
    }

    /// Central area. The reading pane and compose replace the list.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        ctx: &Context,
        db: &Db,
        engine: &SyncEngine,
        toasts: &mut Vec<String>,
    ) {
        self.reload(db);
        if let Some(compose) = self.compose.clone() {
            let next = compose_ui::show(ui, &compose, db, toasts);
            match next {
                compose_ui::ComposeOutcome::Keep(state) => self.compose = Some(state),
                compose_ui::ComposeOutcome::Stage(state) => {
                    self.stage_draft(db, &state, toasts);
                }
                compose_ui::ComposeOutcome::Close => self.compose = None,
            }
            return;
        }
        if let Some(reading) = self.reading {
            reading::show(self, ui, ctx, db, engine, toasts, reading);
            return;
        }
        match self.view {
            EmailView::Inbox => list::show(self, ui, db, toasts),
            EmailView::Outbox => outbox::show(self, ui, db, engine, toasts),
            EmailView::Accounts => accounts::show(self, ui, db, engine, toasts),
        }
    }

    fn stage_draft(&mut self, db: &Db, state: &ComposeState, toasts: &mut Vec<String>) {
        let store = EmailStore::new(db);
        let parse_list = |raw: &str| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|addr| crate::email::model::MailAddress {
                    name: None,
                    addr: addr.to_string(),
                })
                .collect::<Vec<_>>()
        };
        let to = parse_list(&state.to);
        if to.is_empty() {
            toasts.push("Add at least one recipient".to_string());
            return;
        }
        if state.subject.trim().is_empty() {
            toasts.push("Add a subject before staging".to_string());
            return;
        }
        if let Some(id) = state.editing_outbox_id {
            let _ = store.update_outbox_draft(
                id,
                &to,
                &parse_list(&state.cc),
                &parse_list(&state.bcc),
                &state.subject,
                &state.body,
            );
            let _ = store.set_outbox_state(id, OutboxState::Staged, None);
        } else {
            match store.stage_outbox(&crate::email::NewOutbox {
                account_id: state.account_id,
                to,
                cc: parse_list(&state.cc),
                bcc: parse_list(&state.bcc),
                subject: state.subject.clone(),
                body: state.body.clone(),
                in_reply_to: state.in_reply_to.clone(),
                source_email_id: state.source_email_id,
            }) {
                Ok(_) => toasts.push("Staged to outbox".to_string()),
                Err(e) => toasts.push(e.to_string()),
            }
        }
        self.compose = None;
        self.dirty = true;
    }

    /// (account_id, thread_id) list in display order, filtered.
    pub fn visible_threads(&self) -> Vec<(Uuid, Uuid)> {
        self.threads
            .iter()
            .filter(|t| {
                if self.filter.is_empty() {
                    return true;
                }
                let f = self.filter.to_lowercase();
                t.subject.to_lowercase().contains(&f)
                    || t.messages.iter().any(|m| {
                        m.from_addr.to_lowercase().contains(&f)
                            || m.snippet.to_lowercase().contains(&f)
                    })
            })
            .map(|t| (t.account_id, t.thread_id))
            .collect()
    }
}

impl Default for EmailUi {
    fn default() -> Self {
        Self::new()
    }
}
