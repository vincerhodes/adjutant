//! Reading pane: full thread conversation with fetch-on-open bodies,
//! attachments (save to ~/Downloads), linked todos, prev/next navigation.

use egui::{Align, Context, Layout, RichText, Ui};
use uuid::Uuid;

use crate::core::entity::{EntityRef, EntityType};
use crate::core::link::LinkStore;
use crate::db::Db;
use crate::email::sync::{SyncCommand, SyncEngine};
use crate::email::ui::{list, EmailUi, ReadingState};
use crate::email::EmailStore;
use crate::todo::TodoStore;
use crate::ui::{self, icons, theme};

pub fn show(
    state: &mut EmailUi,
    ui: &mut Ui,
    ctx: &Context,
    db: &Db,
    engine: &SyncEngine,
    toasts: &mut Vec<String>,
    reading: ReadingState,
) {
    let store = EmailStore::new(db);
    let threads = state.threads.clone();
    let order = state.visible_threads();
    let Some(pos) = order
        .iter()
        .position(|(a, t)| *a == reading.account_id && *t == reading.thread_id)
    else {
        state.reading = None;
        return;
    };

    // Header row: back, subject, prev/next.
    ui.add_space(24.0);
    ui.horizontal(|ui| {
        if ui
            .add(egui::Label::new(
                RichText::new("‹ Back").color(theme::palette(ui).accent),
            ))
            .on_hover_text("Back to list (Esc)")
            .clicked()
        {
            state.reading = None;
            return;
        }
        let thread = threads
            .iter()
            .find(|t| t.account_id == reading.account_id && t.thread_id == reading.thread_id);
        let subject = thread.map(|t| t.subject.clone()).unwrap_or_default();
        ui.label(
            RichText::new(subject)
                .font(crate::ui::fonts::heading_font(ui.ctx()))
                .color(theme::palette(ui).text),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let has_next = pos + 1 < order.len();
            let has_prev = pos > 0;
            if ui
                .add_enabled(
                    has_next,
                    egui::Button::new(RichText::new("Next →").color(theme::palette(ui).accent))
                        .frame(false),
                )
                .on_hover_text("Next thread (j / →)")
                .clicked()
            {
                let (a, t) = order[pos + 1];
                state.reading = Some(ReadingState {
                    account_id: a,
                    thread_id: t,
                });
                return;
            }
            if ui
                .add_enabled(
                    has_prev,
                    egui::Button::new(RichText::new("← Prev").color(theme::palette(ui).accent))
                        .frame(false),
                )
                .on_hover_text("Previous thread (k / ←)")
                .clicked()
            {
                let (a, t) = order[pos - 1];
                state.reading = Some(ReadingState {
                    account_id: a,
                    thread_id: t,
                });
                return;
            }
            if !state.accounts.is_empty() && ui::ghost_button(ui, "Reply").clicked() {
                open_reply(state, db, reading.thread_id);
                state.reading = None;
            }
        });
    });
    ui.add_space(4.0);

    // Keyboard: j/k and arrows move between threads; Esc goes back.
    ctx.input(|i| {
        let next = i.key_pressed(egui::Key::J) || i.key_pressed(egui::Key::ArrowRight);
        let prev = i.key_pressed(egui::Key::K) || i.key_pressed(egui::Key::ArrowLeft);
        if next && pos + 1 < order.len() {
            let (a, t) = order[pos + 1];
            state.reading = Some(ReadingState {
                account_id: a,
                thread_id: t,
            });
        } else if prev && pos > 0 {
            let (a, t) = order[pos - 1];
            state.reading = Some(ReadingState {
                account_id: a,
                thread_id: t,
            });
        } else if i.key_pressed(egui::Key::Escape) {
            state.reading = None;
        }
    });
    if state.reading != Some(reading) {
        return; // navigation happened; render the new thread next frame
    }

    let Ok(messages) = store.thread_messages(reading.account_id, reading.thread_id) else {
        ui::empty_state(ui, "Thread unavailable", "It may have been deleted");
        return;
    };
    if messages.is_empty() {
        ui::empty_state(ui, "Thread unavailable", "It may have been deleted");
        return;
    }

    // Fetch bodies on open; mark unseen messages read (optimistic + queued).
    let mut marked = false;
    for message in &messages {
        if message.body_text.is_none() && !state.body_pending.contains(&message.id) {
            state.body_pending.insert(message.id);
            engine.send(SyncCommand::FetchBody(message.id));
        }
        if !message.is_seen() && store.mark_seen(message.id, true).is_ok() {
            marked = true;
        }
    }
    if marked {
        state.dirty = true;
        engine.send(SyncCommand::PushWriteOps);
    }

    let p = theme::palette(ui);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Oldest first, conversation style.
            for message in messages.iter().rev() {
                ui.add_space(10.0);
                let frame = crate::ui::card_frame(ui, false, true);
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let from = message
                            .from_name
                            .clone()
                            .unwrap_or_else(|| message.from_addr.clone());
                        ui.label(RichText::new(from).font(crate::ui::fonts::title_font(ui.ctx())));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(list::short_date(&message.date))
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );
                        });
                    });
                    ui.label(
                        RichText::new(format!("To: {}", addr_summary(&message.to)))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(6.0);
                    if state.body_pending.contains(&message.id) {
                        ui.label(
                            RichText::new("Loading body…").color(ui.visuals().weak_text_color()),
                        );
                    } else {
                        ui.label(
                            RichText::new(message.body_text.clone().unwrap_or_default())
                                .color(p.text),
                        );
                    }
                    attachments_section(state, ui, db, toasts, message.id);
                });
            }
            linked_todos(ui, db, messages[0].id);
        });
}

fn open_reply(state: &mut EmailUi, db: &Db, thread_id: Uuid) {
    let _ = db;
    let Some(thread) = state
        .threads
        .iter()
        .find(|t| t.thread_id == thread_id)
        .cloned()
    else {
        return;
    };
    let Some(latest) = thread.messages.first() else {
        return;
    };
    let account_id = thread.account_id;
    let quote = crate::email::compose::quote_reply(latest);
    let subject = if latest.subject.to_lowercase().starts_with("re:") {
        latest.subject.clone()
    } else {
        format!("Re: {}", latest.subject)
    };
    let to = latest.from_addr.clone();
    state.compose = Some(crate::email::ui::ComposeState {
        editing_outbox_id: None,
        account_id,
        to,
        cc: String::new(),
        bcc: String::new(),
        subject,
        body: format!("\n\n{quote}"),
        in_reply_to: latest.message_id.clone(),
        source_email_id: Some(latest.id),
    });
}

fn addr_summary(list: &[crate::email::model::MailAddress]) -> String {
    list.iter()
        .map(|a| a.name.clone().unwrap_or_else(|| a.addr.clone()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn attachments_section(
    _state: &mut EmailUi,
    ui: &mut Ui,
    db: &Db,
    toasts: &mut Vec<String>,
    email_id: Uuid,
) {
    let store = EmailStore::new(db);
    let Ok(attachments) = store.attachments_for(email_id) else {
        return;
    };
    if attachments.is_empty() {
        return;
    }
    ui.add_space(8.0);
    for att in &attachments {
        ui.horizontal(|ui| {
            icons::chip(
                ui,
                icons::Icon::Paperclip,
                theme::palette(ui).muted,
                "attachment",
            );
            ui.label(RichText::new(&att.filename).small());
            ui.label(
                RichText::new(format_size(att.size))
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui::ghost_button(ui, "Save")
                    .on_hover_text("Save to ~/Downloads")
                    .clicked()
                {
                    match save_attachment(db, att.id) {
                        Ok(path) => toasts.push(format!("Saved to {}", path.display())),
                        Err(e) => toasts.push(e),
                    }
                }
            });
        });
    }
}

fn save_attachment(db: &Db, attachment_id: Uuid) -> Result<std::path::PathBuf, String> {
    let store = EmailStore::new(db);
    let content = store
        .attachment_content(attachment_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Attachment not downloaded (open the body first)".to_string())?;
    let filename = {
        let conn = db.conn();
        conn.query_row(
            "SELECT filename FROM email_attachments WHERE id = ?1",
            rusqlite::params![attachment_id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?
    };
    let safe: String = filename
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    let downloads = dirs_downloads();
    std::fs::create_dir_all(&downloads).map_err(|e| e.to_string())?;
    let path = downloads.join(safe);
    std::fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(path)
}

fn dirs_downloads() -> std::path::PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return std::path::PathBuf::from(home).join("Downloads");
    }
    std::path::PathBuf::from("Downloads")
}

fn format_size(size: i64) -> String {
    if size >= 1_048_576 {
        format!("{:.1} MB", size as f64 / 1_048_576.0)
    } else if size >= 1024 {
        format!("{:.0} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    }
}

/// Read-only display of todo links touching this email (spec: no new link
/// UI here — todo pickers link TO emails).
fn linked_todos(ui: &mut Ui, db: &Db, email_id: Uuid) {
    let entity = EntityRef::new(EntityType::Email, email_id);
    let Ok(links) = LinkStore::new(db).links_to(&entity) else {
        return;
    };
    let todo_links: Vec<Uuid> = links
        .iter()
        .filter(|l| l.source.kind == EntityType::Todo)
        .map(|l| l.source.id)
        .collect();
    if todo_links.is_empty() {
        return;
    }
    ui.add_space(10.0);
    ui.label(
        RichText::new("Linked todos")
            .small()
            .color(ui.visuals().weak_text_color()),
    );
    let store = TodoStore::new(db);
    for id in todo_links {
        let title = store
            .get(id)
            .map(|t| t.title)
            .unwrap_or_else(|_| "Unavailable".to_string());
        ui.label(RichText::new(format!("• {title}")).small());
    }
}
