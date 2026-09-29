//! Omarchy `colors.toml` theme engine: parse, resolve, live-reload, map to
//! egui `Visuals`. Pure functions throughout — parse + map need no GPU and
//! are unit-tested in `tests/theme.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use egui::style::{Selection, WidgetVisuals, Widgets};
use egui::{Color32, CornerRadius, Stroke, Visuals};

/// Poll interval for the theme file mtime.
pub const RELOAD_TICK: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

/// The subset of an Omarchy palette the UI consumes. Unknown keys ignored;
/// missing keys fall back per-key so a partial theme still works.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub mode: Mode,
    pub background: Color32,
    pub dark_background: Color32,
    pub darker_background: Color32,
    pub lighter_background: Color32,
    pub foreground: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub selection: Color32,
    pub red: Color32,
    pub green: Color32,
    pub yellow: Color32,
}

impl Palette {
    /// Built-in fallback (Tokyo Night-ish dark). Never a hard failure.
    pub fn fallback() -> Palette {
        Palette {
            mode: Mode::Dark,
            background: hex("#1a1b26"),
            dark_background: hex("#16161e"),
            darker_background: hex("#13131a"),
            lighter_background: hex("#24283b"),
            foreground: hex("#c0caf5"),
            muted: hex("#565f89"),
            accent: hex("#7aa2f7"),
            selection: hex("#283457"),
            red: hex("#f7768e"),
            green: hex("#9ece6a"),
            yellow: hex("#e0af68"),
        }
    }

    /// Parse a flat Omarchy `colors.toml` (`key = "#hex"` lines).
    /// Missing keys inherit the fallback palette; `mode = "light"` respected.
    pub fn parse(content: &str) -> Option<Palette> {
        let mut p = Palette::fallback();
        let mut saw_any = false;
        for line in content.lines() {
            let line = line.trim();
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            saw_any = true;
            match key {
                "mode" => {
                    p.mode = if value == "light" {
                        Mode::Light
                    } else {
                        Mode::Dark
                    };
                }
                "background" => p.background = parse_hex(value)?,
                "dark_background" => p.dark_background = parse_hex(value)?,
                "darker_background" => p.darker_background = parse_hex(value)?,
                "lighter_background" => p.lighter_background = parse_hex(value)?,
                "foreground" => p.foreground = parse_hex(value)?,
                "muted" => p.muted = parse_hex(value)?,
                "accent" => p.accent = parse_hex(value)?,
                "selection" => p.selection = parse_hex(value)?,
                "red" => p.red = parse_hex(value)?,
                "green" => p.green = parse_hex(value)?,
                "yellow" => p.yellow = parse_hex(value)?,
                _ => {}
            }
        }
        saw_any.then_some(p)
    }

    pub fn is_dark(&self) -> bool {
        self.mode == Mode::Dark
    }
}

/// Map a palette to egui visuals. Built from the palette directly — not a
/// `Visuals::dark()`/`light()` clone — so light `mode` is honored end-to-end.
pub fn visuals(p: &Palette) -> Visuals {
    let bg = p.background;
    let panel = p.dark_background;
    let step = p.lighter_background;
    let fg = p.foreground;
    let accent = p.accent;

    let mut v = Visuals {
        dark_mode: p.is_dark(),
        panel_fill: panel,
        window_fill: bg,
        extreme_bg_color: p.darker_background,
        faint_bg_color: step,
        selection: Selection {
            bg_fill: with_alpha(accent, 0.28),
            stroke: Stroke::new(1.0, accent),
        },
        hyperlink_color: accent,
        override_text_color: Some(fg),
        ..Visuals::default()
    };

    let widget = |bg_fill: Color32, fg_color: Color32, stroke: Stroke| WidgetVisuals {
        bg_fill,
        weak_bg_fill: bg_fill,
        bg_stroke: stroke,
        corner_radius: CornerRadius::same(4),
        fg_stroke: Stroke::new(1.0, fg_color),
        expansion: 0.0,
    };

    let transparent = Stroke::new(0.0, Color32::TRANSPARENT);
    v.widgets = Widgets {
        // Chrome (labels, separators): no fill, no stroke.
        noninteractive: widget(bg, fg, transparent),
        // Buttons/inputs at rest: no visible border until focused (§7a).
        inactive: widget(bg, fg, transparent),
        hovered: widget(step, fg, Stroke::new(1.0, with_alpha(fg, 0.25))),
        active: widget(with_alpha(accent, 0.25), fg, Stroke::new(1.0, accent)),
        open: widget(step, fg, Stroke::new(1.0, accent)),
    };

    v.window_stroke = Stroke::new(1.0, with_alpha(fg, 0.12));
    v.warn_fg_color = p.yellow;
    v.error_fg_color = p.red;
    v.text_edit_bg_color = Some(p.darker_background);
    v.button_frame = false;
    v
}

/// Resolve the active Omarchy theme file:
/// 1. `~/.config/omarchy/themes/aether/colors.toml` (live Aether symlink)
/// 2. a lone `colors.toml` directly under `~/.config/omarchy/themes/`
/// 3. `None` → built-in fallback
pub fn resolve_theme_path() -> Option<PathBuf> {
    let themes = theme_dir();
    let aether = themes.join("aether").join("colors.toml");
    if aether.is_file() {
        return Some(aether);
    }
    let mut singles = fs::read_dir(&themes)
        .ok()?
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.file_name().is_some_and(|n| n == "colors.toml"));
    let first = singles.next()?;
    // Only use the "exactly one exists" rule when there really is exactly one.
    if singles.next().is_none() {
        Some(first)
    } else {
        None
    }
}

fn theme_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("omarchy/themes");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config/omarchy/themes");
    }
    PathBuf::from(".config/omarchy/themes")
}

fn parse_hex(s: &str) -> Option<Color32> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}

/// `parse_hex` exposed for tests (invalid hex → None, caller keeps fallback).
pub fn hex(s: &str) -> Color32 {
    parse_hex(s).unwrap_or(Color32::MAGENTA)
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    let a = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

/// Watches the resolved theme file; polls mtime and re-parses on change.
pub struct ThemeWatcher {
    path: Option<PathBuf>,
    last_mtime: Option<SystemTime>,
    palette: Palette,
}

impl ThemeWatcher {
    pub fn new() -> ThemeWatcher {
        let path = resolve_theme_path();
        let (palette, last_mtime) = load(path.as_deref());
        ThemeWatcher {
            path,
            last_mtime,
            palette,
        }
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// Poll for changes; cheap no-op when mtime is unchanged.
    pub fn check(&mut self) -> bool {
        let Some(path) = &self.path else { return false };
        let mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
        if mtime == self.last_mtime {
            return false;
        }
        let (palette, last_mtime) = load(Some(path));
        self.palette = palette;
        self.last_mtime = last_mtime;
        true
    }
}

impl Default for ThemeWatcher {
    fn default() -> Self {
        Self::new()
    }
}

fn load(path: Option<&Path>) -> (Palette, Option<SystemTime>) {
    let Some(path) = path else {
        return (Palette::fallback(), None);
    };
    let Ok(content) = fs::read_to_string(path) else {
        return (Palette::fallback(), None);
    };
    let mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
    (
        Palette::parse(&content).unwrap_or_else(Palette::fallback),
        mtime,
    )
}
