//! Compose editor: modal-less full pane, plain text only. Stage to outbox
//! (human approval before anything is sent) or discard.

use egui::{Align, Layout, RichText, Ui};

use crate::db::Db;
use crate::email::ui::ComposeState;
use crate::ui::{self, theme};

pub enum ComposeOutcome {
    Keep(ComposeState),
    Stage(ComposeState),
    Close,
}

pub fn show(
    ui: &mut Ui,
    state: &ComposeState,
    db: &Db,
    _toasts: &mut Vec<String>,
) -> ComposeOutcome {
    let _ = db;
    let mut next = state.clone();
    let mut outcome = ComposeOutcome::Keep(next.clone());

    ui.add_space(24.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if state.editing_outbox_id.is_some() {
                "Edit draft"
            } else if state.in_reply_to.is_some() {
                "Reply"
            } else {
                "New message"
            })
            .font(crate::ui::fonts::heading_font(ui.ctx()))
            .color(theme::palette(ui).text),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui::primary_button(ui, "Stage to outbox")
                .on_hover_text("Queues the message for your approval")
                .clicked()
            {
                outcome = ComposeOutcome::Stage(next.clone());
            }
            if ui::ghost_button(ui, "Discard")
                .on_hover_text("Drop this draft")
                .clicked()
            {
                outcome = ComposeOutcome::Close;
            }
        });
    });
    ui.add_space(8.0);

    // Esc closes (nothing is sent without staging).
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        return ComposeOutcome::Close;
    }

    let frame = crate::ui::card_frame(ui, false, true);
    frame.show(ui, |ui| {
        field(ui, "To", &mut next.to, "sam@example.com, jane@example.com");
        field(ui, "Cc", &mut next.cc, "");
        field(ui, "Bcc", &mut next.bcc, "");
        field(ui, "Subject", &mut next.subject, "");
        ui.label(RichText::new("Body").small());
        ui.add_space(2.0);
        let mut body = next.body.clone();
        let response = ui.add(
            egui::TextEdit::multiline(&mut body)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .desired_rows(16)
                .hint_text("Plain text only — replies quote below"),
        );
        ui::focused_underline(ui, &response);
        next.body = body;
    });

    match outcome {
        ComposeOutcome::Keep(_) => ComposeOutcome::Keep(next),
        other => other,
    }
}

fn field(ui: &mut Ui, label: &str, value: &mut String, hint: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).small());
        let response = ui.add(
            egui::TextEdit::singleline(value)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .hint_text(hint),
        );
        ui::focused_underline(ui, &response);
        ui::hovered_underline(ui, &response);
    });
    ui.add_space(2.0);
}
