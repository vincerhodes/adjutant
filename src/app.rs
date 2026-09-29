//! AdjutantApp: eframe::App impl — module nav, routing, theme application,
//! zoom keys, help overlay, error toasts, settings persistence.
//!
//! eframe 0.36 splits the frame into `logic` (no UI, runs even when the
//! window is hidden) and `ui` (painting on the root `Ui`). The 1s theme tick
//! lives in `logic`; panels are `egui::Panel`s shown on the root ui.

use egui::{Context, Key, Modifiers, RichText, Ui, ViewportCommand};
use uuid::Uuid;

use crate::db::Db;
use crate::todo::ui::TodoUi;
use crate::ui::{fonts, help, placeholder, theme};

const SETTINGS_MAXIMIZED: &str = "window.maximized";
const SETTINGS_LAST_GROUP: &str = "todo.last_group";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    Todo,
    Email,
    Calendar,
    Scratchpad,
}

impl Module {
    fn label(&self) -> &'static str {
        match self {
            Module::Todo => "Todo",
            Module::Email => "Email",
            Module::Calendar => "Calendar",
            Module::Scratchpad => "Scratchpad",
        }
    }
}

pub struct AdjutantApp {
    db: Db,
    module: Module,
    todo: TodoUi,
    theme: theme::ThemeWatcher,
    theme_dirty: bool,
    help_open: bool,
    toasts: Vec<String>,
    maximized_on_startup: bool,
    first_frame: bool,
}

impl AdjutantApp {
    pub fn new(db: Db, cc: &eframe::CreationContext<'_>) -> AdjutantApp {
        fonts::setup(&cc.egui_ctx);
        let maximized: bool = db
            .get_setting(SETTINGS_MAXIMIZED)
            .ok()
            .flatten()
            .unwrap_or(false);
        let last_group: Option<String> = db.get_setting(SETTINGS_LAST_GROUP).ok().flatten();
        let mut todo = TodoUi::new(&db);
        if let Some(raw) = last_group {
            if let Ok(id) = Uuid::parse_str(&raw) {
                todo.set_current_group(&db, Some(id));
            }
        }
        if maximized {
            cc.egui_ctx
                .send_viewport_cmd(ViewportCommand::Maximized(true));
        }
        AdjutantApp {
            db,
            module: Module::Todo,
            todo,
            theme: theme::ThemeWatcher::new(),
            theme_dirty: true,
            help_open: false,
            toasts: Vec::new(),
            maximized_on_startup: maximized,
            first_frame: true,
        }
    }

    fn handle_global_keys(&mut self, ctx: &Context) {
        ctx.input(|i| {
            if i.key_pressed(Key::F1) {
                self.help_open = !self.help_open;
            }
            if i.modifiers == Modifiers::CTRL && i.key_pressed(Key::Equals) {
                let z = ctx.zoom_factor();
                ctx.set_zoom_factor((z * 1.1).min(3.0));
            }
            if i.modifiers == Modifiers::CTRL && i.key_pressed(Key::Minus) {
                let z = ctx.zoom_factor();
                ctx.set_zoom_factor((z / 1.1).max(0.5));
            }
            if i.modifiers == Modifiers::CTRL && i.key_pressed(Key::Num0) {
                ctx.set_zoom_factor(1.0);
            }
        });
    }

    fn save_settings(&mut self) {
        let _ = self
            .db
            .set_setting(SETTINGS_MAXIMIZED, &self.maximized_on_startup);
        if let Some(group) = self.todo.current_group() {
            let _ = self.db.set_setting(SETTINGS_LAST_GROUP, &group.to_string());
        }
    }
}

impl eframe::App for AdjutantApp {
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // 1s theme-watch tick: reactive repaint, no continuous loop.
        // Runs even while the window is hidden, so a theme switch under a
        // hidden window is picked up on the next shown frame.
        ctx.request_repaint_after(theme::RELOAD_TICK);
        if self.theme.check() {
            self.theme_dirty = true;
        }
        self.handle_global_keys(ctx);
        if let Some(maximized) = ctx.input(|i| i.viewport().maximized) {
            self.maximized_on_startup = maximized;
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.first_frame || self.theme_dirty {
            let visuals = theme::visuals(self.theme.palette());
            ctx.set_visuals(visuals);
            self.first_frame = false;
            self.theme_dirty = false;
        }

        if self.help_open {
            help::show(&ctx, &mut self.help_open);
        }

        // Left sidebar: module nav (+ todo group list when active).
        egui::Panel::left("sidebar")
            .resizable(false)
            .exact_size(180.0)
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.add_space(12.0);
                for module in [
                    Module::Todo,
                    Module::Email,
                    Module::Calendar,
                    Module::Scratchpad,
                ] {
                    if module == Module::Todo {
                        let active = self.module == Module::Todo;
                        if ui.selectable_label(active, module.label()).clicked() {
                            self.module = Module::Todo;
                        }
                    } else {
                        placeholder::disabled_nav_item(ui, module.label());
                    }
                }
                if self.module == Module::Todo {
                    ui.separator();
                    self.todo.sidebar(ui, &self.db);
                }
            });

        match self.module {
            Module::Todo => {
                let mut toasts = std::mem::take(&mut self.toasts);
                self.todo.show(ui, &ctx, &self.db, &mut toasts);
                self.toasts = toasts;
            }
            other => {
                let name = other.label();
                egui::CentralPanel::default().show(ui, |ui| {
                    placeholder::placeholder_screen(ui, name);
                });
            }
        }

        // Transient error/notice bar.
        if !self.toasts.is_empty() {
            egui::Panel::bottom("toasts")
                .resizable(false)
                .exact_size(28.0)
                .show_separator_line(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for toast in self.toasts.clone() {
                            ui.label(RichText::new(toast).color(ui.visuals().error_fg_color));
                        }
                    });
                });
            // Click anywhere dismisses.
            if ctx.input(|i| i.pointer.any_click()) {
                self.toasts.clear();
            }
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_settings();
    }

    fn on_exit(&mut self) {
        self.save_settings();
    }
}
