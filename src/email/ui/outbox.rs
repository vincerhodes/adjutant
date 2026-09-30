//! Outbox review: staged/approved/sending/sent/failed cards with per-item
//! Approve / Edit / Discard and Approve-all. Nothing sends without the
//! human approval step.

use egui::{Align, Layout, RichText, Ui};

use crate::db::Db;
use crate::email::model::OutboxState;
use crate::email::sync::{SyncCommand, SyncEngine};
use crate::email::ui::{list, ComposeState, EmailUi};
use crate::email::EmailStore;
use crate::ui::{self, icons, theme};

pub fn show(
    state: &mut EmailUi,
    ui: &mut Ui,
    db: &Db,
    engine: &SyncEngine,
    toasts: &mut Vec<String>,
) {
    ui.add_space(24.0);
    let pending = state
        .outbox
        .iter()
        .filter(|i| matches!(i.state, OutboxState::Staged | OutboxState::Failed))
        .count();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Outbox")
                .font(crate::ui::fonts::heading_font(ui.ctx()))
                .color(theme::palette(ui).text),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if pending > 0
                && ui::primary_button(ui, &format!("Approve all ({pending})"))
                    .on_hover_text("Approve every staged/failed item — each is sent via SMTP")
                    .clicked()
            {
                approve_all(state, db, engine, toasts);
            }
        });
    });
    ui.add_space(4.0);

    if state.outbox.is_empty() {
        ui::empty_state(
            ui,
            "Outbox empty",
            "Compose a message and stage it — nothing sends without approval here",
        );
        return;
    }

    let items = state.outbox.clone();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for item in &items {
                outbox_card(state, ui, db, item, toasts);
            }
        });
}

fn approve_all(state: &mut EmailUi, db: &Db, engine: &SyncEngine, toasts: &mut Vec<String>) {
    let store = EmailStore::new(db);
    let items = state.outbox.clone();
    let mut approved = 0usize;
    for item in &items {
        if matches!(item.state, OutboxState::Staged | OutboxState::Failed)
            && store
                .set_outbox_state(item.id, OutboxState::Approved, None)
                .is_ok()
        {
            approved += 1;
        }
    }
    if approved > 0 {
        engine.send(SyncCommand::PushWriteOps);
        toasts.push(format!("Approved {approved} message(s)"));
        state.dirty = true;
    }
}

fn outbox_card(
    state: &mut EmailUi,
    ui: &mut Ui,
    db: &Db,
    item: &crate::email::model::OutboxItem,
    toasts: &mut Vec<String>,
) {
    let store = EmailStore::new(db);
    let p = theme::palette(ui);
    let to = item
        .to
        .iter()
        .map(|a| a.name.clone().unwrap_or_else(|| a.addr.clone()))
        .collect::<Vec<_>>()
        .join(", ");

    ui.add_space(10.0);
    let frame = crate::ui::card_frame(ui, false, false);
    frame.show(ui, |ui| {
        ui.set_min_height(56.0 - 20.0);
        ui.horizontal(|ui| {
            let color = list::account_color(state, item.account_id);
            icons::group_dot(ui, color, 10.0);
            ui.label(
                RichText::new(if to.is_empty() {
                    "(no recipients)"
                } else {
                    &to
                })
                .font(crate::ui::fonts::title_font(ui.ctx())),
            );
            ui.label(RichText::new(&item.subject));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                state_chip(ui, item, p);
                ui.label(
                    RichText::new(list::short_date(&item.created_at))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            });
        });
        ui.horizontal_wrapped(|ui| {
            match item.state {
                OutboxState::Staged | OutboxState::Failed => {
                    if ui::primary_button(ui, "Approve").clicked() {
                        match store.set_outbox_state(item.id, OutboxState::Approved, None) {
                            Ok(()) => {
                                toasts.push("Approved — sending…".to_string());
                                state.dirty = true;
                            }
                            Err(e) => toasts.push(e.to_string()),
                        }
                    }
                }
                OutboxState::Approved | OutboxState::Sending => {
                    ui.label(
                        RichText::new("Sending…")
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                }
                OutboxState::Sent | OutboxState::Discarded => {}
            }
            if matches!(item.state, OutboxState::Staged | OutboxState::Failed) {
                if ui::ghost_button(ui, "Edit").clicked() {
                    state.compose = Some(ComposeState {
                        editing_outbox_id: Some(item.id),
                        account_id: item.account_id,
                        to: item
                            .to
                            .iter()
                            .map(|a| a.addr.clone())
                            .collect::<Vec<_>>()
                            .join(", "),
                        cc: item
                            .cc
                            .iter()
                            .map(|a| a.addr.clone())
                            .collect::<Vec<_>>()
                            .join(", "),
                        bcc: item
                            .bcc
                            .iter()
                            .map(|a| a.addr.clone())
                            .collect::<Vec<_>>()
                            .join(", "),
                        subject: item.subject.clone(),
                        body: item.body.clone(),
                        in_reply_to: item.in_reply_to.clone(),
                        source_email_id: item.source_email_id,
                    });
                }
                let red = p.danger;
                if ui
                    .add(egui::Button::new(RichText::new("Discard").color(red)).frame(false))
                    .clicked()
                {
                    match store.set_outbox_state(item.id, OutboxState::Discarded, None) {
                        Ok(()) => state.dirty = true,
                        Err(e) => toasts.push(e.to_string()),
                    }
                }
            }
        });
    });
}

fn state_chip(ui: &mut Ui, item: &crate::email::model::OutboxItem, p: theme::Palette) {
    let (icon, color, label) = match item.state {
        OutboxState::Staged => (icons::Icon::CircleOutline, p.muted, "staged"),
        OutboxState::Approved => (icons::Icon::CircleHalf, p.accent, "approved"),
        OutboxState::Sending => (icons::Icon::CircleHalf, p.warn, "sending"),
        OutboxState::Sent => (icons::Icon::Check, p.success, "sent"),
        OutboxState::Failed => (icons::Icon::Cross, p.danger, "failed"),
        OutboxState::Discarded => (icons::Icon::Cross, p.muted, "discarded"),
    };
    let chip = icons::chip(ui, icon, color, label);
    if item.state == OutboxState::Failed {
        if let Some(err) = &item.error {
            chip.on_hover_text(format!("Failed: {err} — Approve to retry"));
        }
    }
}
