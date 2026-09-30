//! Outbox review: staged cards with Approve/Edit/Discard + Approve-all
//! (fleshed out in the outbox commit).

use egui::Ui;

use crate::db::Db;
use crate::email::sync::SyncEngine;
use crate::email::ui::EmailUi;
use crate::ui;

pub fn show(
    state: &mut EmailUi,
    ui: &mut Ui,
    _db: &Db,
    _engine: &SyncEngine,
    _toasts: &mut Vec<String>,
) {
    if state.outbox.is_empty() {
        ui::empty_state(
            ui,
            "Outbox empty",
            "Staged mail waits here for your approval",
        );
    } else {
        ui::empty_state(ui, "Outbox", "Coming online in the outbox commit");
    }
}
