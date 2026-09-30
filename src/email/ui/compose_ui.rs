//! Compose editor: full-pane To/CC/BCC/subject/body with Stage/Discard
//! (fleshed out in the reading/compose commit).

use egui::Ui;

use crate::db::Db;
use crate::email::ui::ComposeState;
use crate::ui;

pub enum ComposeOutcome {
    Keep(ComposeState),
    Stage(ComposeState),
    Close,
}

pub fn show(
    ui: &mut Ui,
    state: &ComposeState,
    _db: &Db,
    _toasts: &mut Vec<String>,
) -> ComposeOutcome {
    ui::empty_state(ui, "Compose", "Coming online in the next commit");
    ComposeOutcome::Keep(state.clone())
}
