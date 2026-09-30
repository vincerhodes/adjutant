//! Inbox list: unified or per-account thread cards (folded-card language).

use chrono::Datelike;
use egui::{Align, Layout, RichText, Ui};
use uuid::Uuid;

use crate::db::Db;
use crate::email::ui::{EmailUi, ReadingState};
use crate::ui::{self, icons, theme};

pub fn show(state: &mut EmailUi, ui: &mut Ui, db: &Db, toasts: &mut Vec<String>) {
    crate::email::ui::with_list_margins(ui, |ui| show_inner(state, ui, db, toasts));
}

fn show_inner(state: &mut EmailUi, ui: &mut Ui, db: &Db, toasts: &mut Vec<String>) {
    let _ = (db, toasts);
    // Header: scope name + Compose + Filter….
    ui.add_space(24.0);
    ui.horizontal(|ui| {
        let scope = match state.account_filter {
            None => "Unified Inbox".to_string(),
            Some(id) => state
                .accounts
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.label.clone())
                .unwrap_or_else(|| "Inbox".to_string()),
        };
        let heading_color = theme::palette(ui).text;
        ui.label(
            RichText::new(scope)
                .font(crate::ui::fonts::heading_font(ui.ctx()))
                .color(heading_color),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui::ghost_button(ui, "Filter…")
                .on_hover_text("Filter threads (Ctrl+F)")
                .clicked()
            {
                state.filter_active = true;
            }
            if !state.accounts.is_empty()
                && ui::ghost_button(ui, "Compose")
                    .on_hover_text("New message")
                    .clicked()
            {
                state.compose = Some(crate::email::ui::ComposeState {
                    editing_outbox_id: None,
                    account_id: state.account_filter.unwrap_or_else(|| state.accounts[0].id),
                    to: String::new(),
                    cc: String::new(),
                    bcc: String::new(),
                    subject: String::new(),
                    body: String::new(),
                    in_reply_to: None,
                    source_email_id: None,
                });
            }
        });
    });
    ui.add_space(4.0);

    if state.filter_active {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let mut filter = state.filter.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut filter)
                    .frame(egui::Frame::NONE)
                    .desired_width(240.0)
                    .hint_text("Filter threads… (Esc to clear)"),
            );
            ui::focused_underline(ui, &response);
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                state.filter_active = false;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                state.filter_active = false;
                filter.clear();
            }
            state.filter = filter;
        });
        ui.add_space(4.0);
    }

    if state.accounts.is_empty() {
        ui::empty_state(
            ui,
            "No email accounts yet",
            "Open Manage accounts to add one",
        );
        return;
    }
    if state.threads.is_empty() {
        ui::empty_state(ui, "Inbox zero", "Synced mail lands here as threads");
        return;
    }

    let visible = state.visible_threads();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (account_id, thread_id) in &visible {
                let threads = state.threads.clone();
                let Some(thread) = threads
                    .iter()
                    .find(|t| t.account_id == *account_id && t.thread_id == *thread_id)
                else {
                    continue;
                };
                thread_card(ui, state, thread);
            }
        });
}

fn thread_card(ui: &mut Ui, state: &mut EmailUi, thread: &crate::email::model::EmailThread) {
    let p = theme::palette(ui);
    let latest = &thread.messages[0];
    let unread = thread.unread_count > 0;

    ui.add_space(10.0);
    let card_id = ui.id().with(("email_thread", thread.thread_id));
    // Register the click target BEFORE content (bottom z-order, last
    // frame's rect) so inner widgets win — same pattern as todo cards.
    let prev_rect = ui.ctx().data_mut(|d| d.get_temp::<egui::Rect>(card_id));
    let card_response = prev_rect.map(|rect| ui.interact(rect, card_id, egui::Sense::click()));

    let open = |state: &mut EmailUi| {
        state.reading = Some(ReadingState {
            account_id: thread.account_id,
            thread_id: thread.thread_id,
        });
    };

    let frame = crate::ui::card_frame(ui, false, false);
    let frame_response = frame.show(ui, |ui| {
        let mut child_clicked = false;
        let mut badge_rects: Vec<egui::Rect> = Vec::new();
        let color = account_color(state, thread.account_id);

        // Line 1: subject (semibold) … right: date, thread count, paperclip,
        // unread dot.
        ui.horizontal(|ui| {
            icons::group_dot(ui, color, 10.0);
            let subject = if thread.subject.trim().is_empty() {
                "(no subject)".to_string()
            } else {
                thread.subject.clone()
            };
            let mut subject_text =
                RichText::new(subject).font(crate::ui::fonts::title_font(ui.ctx()));
            if unread {
                subject_text = subject_text.strong();
            }
            let subject_resp =
                ui::hand(ui.add(egui::Label::new(subject_text).sense(egui::Sense::click())));
            if subject_resp.clicked() || subject_resp.double_clicked() {
                open(state);
                child_clicked = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if unread {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.0, p.gold);
                    badge_rects.push(rect);
                }
                if thread.has_attachments {
                    badge_rects.push(
                        icons::chip(ui, icons::Icon::Paperclip, p.muted, "has attachments").rect,
                    );
                }
                if thread.message_count > 1 {
                    let (_, rect) = icons::chip_with_text(
                        ui,
                        icons::Icon::Branch,
                        p.muted,
                        &thread.message_count.to_string(),
                        &format!("{} messages", thread.message_count),
                    );
                    badge_rects.push(rect);
                }
                ui.label(
                    RichText::new(short_date(&thread.latest_date))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            });
        });

        // Line 2: "sender — snippet", muted, single line, elided.
        ui.horizontal(|ui| {
            let from = latest
                .from_name
                .clone()
                .unwrap_or_else(|| latest.from_addr.clone());
            let from_resp =
                ui.add(egui::Label::new(RichText::new(from).small()).sense(egui::Sense::click()));
            if from_resp.clicked() {
                open(state);
                child_clicked = true;
            }
            if !thread.snippet.is_empty() {
                let line = format!("— {}", thread.snippet);
                let width = ui.available_width();
                ui.add_sized(
                    [width.max(40.0), 18.0],
                    egui::Label::new(
                        RichText::new(line)
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    )
                    .truncate(),
                );
            }
        });
        (child_clicked, badge_rects)
    });
    let (child_clicked, badge_rects) = frame_response.inner;

    let rect = frame_response.response.rect;
    ui.ctx().data_mut(|d| d.insert_temp(card_id, rect));
    let pointer = ui.input(|i| i.pointer.latest_pos());
    if pointer.is_some_and(|pos| rect.contains(pos)) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if let Some(response) = card_response {
        if response.clicked() && !child_clicked {
            let on_badge = badge_rects
                .iter()
                .any(|r| pointer.is_some_and(|pos| r.contains(pos)));
            if !on_badge {
                open(state);
            }
        }
    }
}

pub fn account_color(state: &EmailUi, account_id: Uuid) -> egui::Color32 {
    let idx = state
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .map(|a| a.color_idx)
        .unwrap_or(0);
    icons::GROUP_DOT_COLORS[(idx.rem_euclid(6)) as usize % icons::GROUP_DOT_COLORS.len()]
}

/// Short date: `Oct 3` (this year) or `2025-11-02`.
pub fn short_date(iso: &str) -> String {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso) {
        let now = chrono::Utc::now();
        if dt.year() == now.year() {
            dt.format("%b %-d").to_string()
        } else {
            dt.format("%Y-%m-%d").to_string()
        }
    } else {
        iso.chars().take(10).collect()
    }
}
