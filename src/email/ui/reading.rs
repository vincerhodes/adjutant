//! Reading pane: full thread conversation (fleshed out in the
//! reading/compose commit).

use egui::{Context, Ui};

use crate::db::Db;
use crate::email::sync::SyncEngine;
use crate::email::ui::{EmailUi, ReadingState};
use crate::ui;

pub fn show(
    state: &mut EmailUi,
    ui: &mut Ui,
    _ctx: &Context,
    _db: &Db,
    _engine: &SyncEngine,
    _toasts: &mut Vec<String>,
    reading: ReadingState,
) {
    ui::empty_state(ui, "Reading pane", "Coming online in the next commit");
    state.reading = Some(reading);
}
