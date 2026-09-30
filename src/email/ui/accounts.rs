//! Account management: add/edit form (password → keyring only) + list with
//! test-connection and delete.

use egui::{Align, Layout, RichText, Ui};
use uuid::Uuid;

use crate::db::Db;
use crate::email::sync::{SyncCommand, SyncEngine, SyncEvent};
use crate::email::ui::EmailUi;
use crate::email::{EmailStore, NewAccount};
use crate::ui::{self, icons, theme};

/// The account form's editable state. Password NEVER enters the DB.
#[derive(Debug, Clone, Default)]
pub struct AccountForm {
    pub editing_id: Option<Uuid>,
    pub label: String,
    pub address: String,
    pub imap_host: String,
    pub imap_port: String,
    pub smtp_host: String,
    pub smtp_port: String,
    pub username: String,
    pub password: String,
    pub sync_interval_s: String,
    pub error: Option<String>,
}

impl AccountForm {
    fn new() -> AccountForm {
        AccountForm {
            imap_port: "993".to_string(),
            smtp_port: "465".to_string(),
            sync_interval_s: "300".to_string(),
            ..Default::default()
        }
    }

    fn from_account(account: &crate::email::model::EmailAccount) -> AccountForm {
        AccountForm {
            editing_id: Some(account.id),
            label: account.label.clone(),
            address: account.address.clone(),
            imap_host: account.imap_host.clone(),
            imap_port: account.imap_port.to_string(),
            smtp_host: account.smtp_host.clone(),
            smtp_port: account.smtp_port.to_string(),
            username: account.username.clone(),
            password: String::new(), // never read back from the keyring
            sync_interval_s: account.sync_interval_s.to_string(),
            error: None,
        }
    }
}

pub fn show(
    state: &mut EmailUi,
    ui: &mut Ui,
    db: &Db,
    engine: &SyncEngine,
    toasts: &mut Vec<String>,
) {
    ui.add_space(24.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Accounts")
                .font(crate::ui::fonts::heading_font(ui.ctx()))
                .color(theme::palette(ui).text),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if state.account_form.is_none() && ui::ghost_button(ui, "+ Add account").clicked() {
                state.account_form = Some(AccountForm::new());
            }
        });
    });
    ui.add_space(8.0);

    if let Some(form) = state.account_form.clone() {
        form_card(state, ui, db, engine, toasts, &form);
        return;
    }

    let store = EmailStore::new(db);
    let accounts = store.list_accounts().unwrap_or_default();
    if accounts.is_empty() {
        ui::empty_state(
            ui,
            "No accounts",
            "\"+ Add account\" connects your first mailbox",
        );
        return;
    }
    let p = theme::palette(ui);
    for account in &accounts {
        ui.add_space(10.0);
        let frame = crate::ui::card_frame(ui, false, false);
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                let color = icons::GROUP_DOT_COLORS
                    [(account.color_idx.rem_euclid(6)) as usize % icons::GROUP_DOT_COLORS.len()];
                icons::group_dot(ui, color, 10.0);
                ui.label(
                    RichText::new(&account.label).font(crate::ui::fonts::title_font(ui.ctx())),
                );
                ui.label(
                    RichText::new(&account.address)
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let red = p.danger;
                    if ui
                        .add(egui::Button::new(RichText::new("Delete").color(red)).frame(false))
                        .on_hover_text("Remove account and all local mail")
                        .clicked()
                    {
                        match store.delete_account(account.id) {
                            Ok(()) => {
                                if let Err(e) = crate::core::keyring::delete_password(account.id) {
                                    toasts.push(e.to_string());
                                }
                                state.dirty = true;
                            }
                            Err(e) => toasts.push(e.to_string()),
                        }
                    }
                    if ui::ghost_button(ui, "Edit").clicked() {
                        state.account_form = Some(AccountForm::from_account(account));
                    }
                    if ui::ghost_button(ui, "Test connection")
                        .on_hover_text("IMAP login + NOOP via the sync engine")
                        .clicked()
                    {
                        engine.send(SyncCommand::TestAccount(account.id));
                        toasts.push("Testing connection…".to_string());
                    }
                });
            });
        });
    }
}

fn form_card(
    state: &mut EmailUi,
    ui: &mut Ui,
    db: &Db,
    engine: &SyncEngine,
    toasts: &mut Vec<String>,
    form: &AccountForm,
) {
    let mut form = form.clone();
    let _ = engine;
    let frame = crate::ui::card_frame(ui, false, true);
    frame.show(ui, |ui| {
        ui.label(
            RichText::new(if form.editing_id.is_some() {
                "Edit account"
            } else {
                "Add account"
            })
            .font(crate::ui::fonts::title_font(ui.ctx())),
        );
        ui.add_space(8.0);
        form_row(ui, "Label", &mut form.label, "Work");
        form_row(ui, "Email address", &mut form.address, "jim@example.com");
        form_row(ui, "IMAP host", &mut form.imap_host, "imap.example.com");
        form_row(ui, "IMAP port", &mut form.imap_port, "993");
        form_row(ui, "SMTP host", &mut form.smtp_host, "smtp.example.com");
        form_row(ui, "SMTP port", &mut form.smtp_port, "465");
        form_row(ui, "Username", &mut form.username, "");
        // Password: keyring-only, never persisted to the DB struct.
        ui.horizontal(|ui| {
            ui.label(RichText::new("Password").small());
            let response = ui.add(
                egui::TextEdit::singleline(&mut form.password)
                    .frame(egui::Frame::NONE)
                    .desired_width(240.0)
                    .password(true)
                    .hint_text("stored in the OS keyring only"),
            );
            ui::focused_underline(ui, &response);
            ui::hovered_underline(ui, &response);
        });
        form_row(ui, "Sync interval (s)", &mut form.sync_interval_s, "300");

        if let Some(err) = &form.error {
            ui.label(RichText::new(err).color(theme::palette(ui).danger));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui::primary_button(
                ui,
                if form.editing_id.is_some() {
                    "Save"
                } else {
                    "Add"
                },
            )
            .clicked()
            {
                match save_account(db, &form, toasts) {
                    Ok(id) => {
                        engine.send(SyncCommand::SyncNow(Some(id)));
                        state.account_form = None;
                        state.dirty = true;
                    }
                    Err(e) => form.error = Some(e),
                }
            }
            if ui::ghost_button(ui, "Cancel").clicked() {
                state.account_form = None;
            }
        });
    });
    if state.account_form.is_some() {
        state.account_form = Some(form);
    }
}

fn form_row(ui: &mut Ui, label: &str, value: &mut String, hint: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).small());
        let response = ui.add(
            egui::TextEdit::singleline(value)
                .frame(egui::Frame::NONE)
                .desired_width(240.0)
                .hint_text(hint),
        );
        ui::focused_underline(ui, &response);
        ui::hovered_underline(ui, &response);
    });
}

fn save_account(db: &Db, form: &AccountForm, toasts: &mut Vec<String>) -> Result<Uuid, String> {
    let store = EmailStore::new(db);
    let port = |raw: &str, default: u16| raw.trim().parse().unwrap_or(default);
    let interval: i64 = form.sync_interval_s.trim().parse().unwrap_or(300);
    if form.label.trim().is_empty() || form.address.trim().is_empty() {
        return Err("Label and address are required".to_string());
    }
    // Validation before anything touches the DB.
    let imap_host = crate::email::imap_client::normalize_host(&form.imap_host);
    let smtp_host = crate::email::imap_client::normalize_host(&form.smtp_host);
    let username = form.username.trim().to_string();
    if imap_host.is_empty() || smtp_host.is_empty() {
        return Err("IMAP and SMTP hosts are required".to_string());
    }
    if username.is_empty() {
        return Err("Username is required".to_string());
    }
    let id = match form.editing_id {
        Some(id) => {
            let mut account = store.get_account(id).map_err(|e| e.to_string())?;
            account.label = form.label.trim().to_string();
            account.address = form.address.trim().to_string();
            account.imap_host = imap_host;
            account.imap_port = port(&form.imap_port, 993);
            account.smtp_host = smtp_host;
            account.smtp_port = port(&form.smtp_port, 465);
            account.username = username;
            account.sync_interval_s = interval.max(60);
            store.update_account(&account).map_err(|e| e.to_string())?;
            id
        }
        None => {
            let color_idx =
                i64::try_from(store.list_accounts().unwrap_or_default().len()).unwrap_or(0);
            let account = store
                .create_account(&NewAccount {
                    label: form.label.trim(),
                    address: form.address.trim(),
                    imap_host: &imap_host,
                    imap_port: port(&form.imap_port, 993),
                    smtp_host: &smtp_host,
                    smtp_port: port(&form.smtp_port, 465),
                    username: &username,
                    color_idx,
                    sync_interval_s: interval.max(60),
                })
                .map_err(|e| e.to_string())?;
            account.id
        }
    };
    // Password → keyring only (and only when one was entered).
    if !form.password.is_empty() {
        crate::core::keyring::set_password(id, &form.password).map_err(|e| e.to_string())?;
    } else if form.editing_id.is_none() {
        return Err("Password is required for a new account".to_string());
    }
    toasts.push("Account saved".to_string());
    Ok(id)
}

/// Surface a test-connection event as a toast.
pub fn account_tested_toast(ev: &SyncEvent, toasts: &mut Vec<String>) {
    if let SyncEvent::AccountTested { ok, error, .. } = ev {
        if *ok {
            toasts.push("Connection OK".to_string());
        } else {
            toasts.push(format!(
                "Connection failed: {}",
                error.clone().unwrap_or_default()
            ));
        }
    }
}
