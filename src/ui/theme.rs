//! Theme engine: five built-in palettes (Light default, Sepia, Slate, Dark,
//! Omarchy) + the Omarchy `colors.toml` watcher with 1s live reload.
//!
//! The whole palette is stashed in egui's data store on apply; widgets read
//! colors from there — zero hardcoded hex outside palette definitions.
//! Pure functions throughout; unit-tested in `tests/theme.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use egui::style::{Selection, WidgetVisuals, Widgets};
use egui::{Color32, CornerRadius, Stroke, Visuals};

/// Poll interval for the Omarchy theme file mtime.
pub const RELOAD_TICK: std::time::Duration = std::time::Duration::from_secs(1);

/// Luma distance below which a card hairline border is drawn (Light/Sepia).
const CARD_BORDER_LUMA_THRESHOLD: f32 = 0.08;

/// Semantic palette — every UI color comes from here (or is derived from
/// these fields at palette/visuals build time, never per-widget hex).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub background: Color32,
    pub panel: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub border: Color32,
    pub success: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub card_fill: Color32,
    pub card_hover: Color32,
    pub selection_fill: Color32,
}

/// The five switchable themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    Light,
    Sepia,
    Slate,
    Dark,
    Omarchy,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 5] = [
        ThemeChoice::Light,
        ThemeChoice::Sepia,
        ThemeChoice::Slate,
        ThemeChoice::Dark,
        ThemeChoice::Omarchy,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            ThemeChoice::Light => "light",
            ThemeChoice::Sepia => "sepia",
            ThemeChoice::Slate => "slate",
            ThemeChoice::Dark => "dark",
            ThemeChoice::Omarchy => "omarchy",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ThemeChoice::Light => "Light",
            ThemeChoice::Sepia => "Sepia",
            ThemeChoice::Slate => "Slate",
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Omarchy => "Omarchy",
        }
    }

    pub fn from_name(name: &str) -> Option<ThemeChoice> {
        ThemeChoice::ALL.iter().copied().find(|c| c.name() == name)
    }
}

impl Palette {
    pub fn light() -> Palette {
        Palette {
            dark: false,
            background: hex("#F7F6F2"),
            panel: hex("#FFFFFF"),
            text: hex("#1C1E21"),
            muted: hex("#6E7278"),
            accent: hex("#4C6EF5"),
            border: hex("#E0DED6"),
            success: hex("#2F9E44"),
            warn: hex("#B45309"),
            danger: hex("#DC2626"),
            card_fill: hex("#FFFFFF"),
            card_hover: hex("#EFEDE6"),
            selection_fill: hex("#E3E9FD"),
        }
    }

    pub fn sepia() -> Palette {
        Palette {
            dark: false,
            background: hex("#F1EADB"),
            panel: hex("#FAF5EA"),
            text: hex("#2E2A23"),
            muted: hex("#8A7E6A"),
            accent: hex("#A8732A"),
            border: hex("#DDD3BC"),
            success: hex("#4C7A34"),
            warn: hex("#96660F"),
            danger: hex("#B3452E"),
            card_fill: hex("#FAF5EA"),
            card_hover: hex("#EAE0CB"),
            selection_fill: hex("#EFE3CC"),
        }
    }

    pub fn slate() -> Palette {
        Palette {
            dark: true,
            background: hex("#262B33"),
            panel: hex("#2F3540"),
            text: hex("#D8DDE4"),
            muted: hex("#7C8590"),
            accent: hex("#7EA1FF"),
            border: hex("#3A4150"),
            success: hex("#63B77C"),
            warn: hex("#E5B94E"),
            danger: hex("#E5645F"),
            card_fill: hex("#3A4356"),
            card_hover: hex("#465064"),
            selection_fill: hex("#31405F"),
        }
    }

    pub fn dark() -> Palette {
        Palette {
            dark: true,
            background: hex("#101216"),
            panel: hex("#171A1F"),
            text: hex("#E3E6EB"),
            muted: hex("#7A818C"),
            accent: hex("#7C9EFF"),
            border: hex("#262B33"),
            success: hex("#63B56F"),
            warn: hex("#E0B34C"),
            danger: hex("#E06C60"),
            card_fill: hex("#212830"),
            card_hover: hex("#2A323E"),
            selection_fill: hex("#22304A"),
        }
    }

    pub fn builtin(choice: ThemeChoice) -> Palette {
        match choice {
            ThemeChoice::Light => Palette::light(),
            ThemeChoice::Sepia => Palette::sepia(),
            ThemeChoice::Slate => Palette::slate(),
            ThemeChoice::Dark => Palette::dark(),
            // Omarchy's fallback when no colors.toml resolves.
            ThemeChoice::Omarchy => Palette::dark(),
        }
    }

    /// Parse a flat Omarchy `colors.toml` (`key = "#hex"` lines) into a
    /// palette. Missing keys inherit the built-in Dark palette so a partial
    /// theme still works.
    pub fn parse_omarchy(content: &str) -> Option<Palette> {
        let mut p = Palette::dark();
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
                "mode" => p.dark = value != "light",
                "background" => p.background = parse_hex(value)?,
                "dark_background" => p.panel = parse_hex(value)?,
                "foreground" => p.text = parse_hex(value)?,
                "muted" => p.muted = parse_hex(value)?,
                "accent" => p.accent = parse_hex(value)?,
                "selection" => p.selection_fill = parse_hex(value)?,
                "red" => p.danger = parse_hex(value)?,
                "green" => p.success = parse_hex(value)?,
                "yellow" => p.warn = parse_hex(value)?,
                _ => {}
            }
        }
        if !saw_any {
            return None;
        }
        // Derived Omarchy fields: cards sit one step above the background,
        // hover one further; borders are a translucent hairline of the text.
        p.card_fill = blend_toward(p.background, p.text, 0.10);
        p.card_hover = blend_toward(p.background, p.text, 0.16);
        p.border = with_alpha(p.text, 0.16);
        Some(p)
    }
}

/// Active palette for a theme choice. Omarchy follows the watcher (which
/// falls back to built-in Dark when no colors.toml resolves).
pub fn active_palette(choice: ThemeChoice, watcher: &ThemeWatcher) -> Palette {
    match choice {
        ThemeChoice::Omarchy => *watcher.palette(),
        _ => Palette::builtin(choice),
    }
}

/// Cards get a hairline border only where card fill and background are too
/// close to read (Light/Sepia); dark themes separate by value.
pub fn card_needs_border(p: &Palette) -> bool {
    (luma(p.card_fill) - luma(p.background)).abs() < CARD_BORDER_LUMA_THRESHOLD
}

/// Map a palette to egui visuals. Built from the palette directly — not a
/// `Visuals::dark()`/`light()` clone — so light themes are honored end-to-end.
pub fn visuals(p: &Palette) -> Visuals {
    let bg = p.background;
    let panel = p.panel;
    let fg = p.text;
    let accent = p.accent;

    // Inset surface (text-edit backgrounds) — scaled toward/away from text,
    // no per-widget hex.
    let inset = if p.dark {
        blend_toward(bg, fg, 0.05)
    } else {
        blend_toward(bg, fg, 0.03)
    };

    let mut v = Visuals {
        dark_mode: p.dark,
        panel_fill: panel,
        window_fill: bg,
        extreme_bg_color: inset,
        faint_bg_color: p.card_hover,
        selection: Selection {
            bg_fill: p.selection_fill,
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
        // Buttons/inputs at rest: no visible border until focused.
        inactive: widget(bg, fg, transparent),
        hovered: widget(p.card_hover, fg, Stroke::new(1.0, with_alpha(fg, 0.25))),
        active: widget(with_alpha(accent, 0.25), fg, Stroke::new(1.0, accent)),
        open: widget(p.card_hover, fg, Stroke::new(1.0, accent)),
    };

    v.window_stroke = Stroke::new(1.0, with_alpha(fg, 0.12));
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.danger;
    v.text_edit_bg_color = Some(inset);
    v.button_frame = false;
    // Legibility floor: weak/muted text must keep a minimum luma distance
    // from every surface it can sit on, else it blends toward foreground.
    v.weak_text_color = Some(ensure_contrast(
        p.muted,
        fg,
        &[bg, panel, p.card_fill],
        0.25,
    ));
    v
}

/// Blend `muted` toward `fg` until its luma distance from every surface in
/// `surfaces` is at least `min_dist`. Bounds the worst-case contrast of
/// placeholder/disabled/hint text on pathological palettes.
pub fn ensure_contrast(
    muted: Color32,
    fg: Color32,
    surfaces: &[Color32],
    min_dist: f32,
) -> Color32 {
    let mut m = muted;
    for _ in 0..16 {
        let worst = surfaces
            .iter()
            .map(|s| (luma(m) - luma(*s)).abs())
            .fold(0.0_f32, f32::max);
        if worst >= min_dist {
            return m;
        }
        m = blend_toward(m, fg, 0.2);
    }
    m
}

fn luma(c: Color32) -> f32 {
    (0.2126 * f32::from(c.r()) + 0.7152 * f32::from(c.g()) + 0.0722 * f32::from(c.b())) / 255.0
}

fn blend_toward(c: Color32, target: Color32, t: f32) -> Color32 {
    let ch =
        |a: u8, b: u8| -> u8 { (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8 };
    Color32::from_rgb(
        ch(c.r(), target.r()),
        ch(c.g(), target.g()),
        ch(c.b(), target.b()),
    )
}

const PALETTE_ID: &str = "adjutant.theme.palette";

/// Stash the whole active palette in egui's data store so any widget can
/// read semantic colors without threading state through the UI tree.
pub fn store_palette(ctx: &egui::Context, p: &Palette) {
    ctx.data_mut(|d| d.insert_persisted(egui::Id::new(PALETTE_ID), *p));
}

/// The active palette for this context (fall back to Light before the first
/// apply).
pub fn palette(ui: &egui::Ui) -> Palette {
    ui.ctx()
        .data_mut(|d| d.get_persisted(egui::Id::new(PALETTE_ID)))
        .unwrap_or_else(Palette::light)
}

/// Resolve the active Omarchy theme file:
/// 1. `~/.config/omarchy/themes/aether/colors.toml` (live Aether symlink)
/// 2. a lone `colors.toml` directly under `~/.config/omarchy/themes/`
/// 3. `None` → built-in Dark + log
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

/// `parse_hex` exposed for palette definitions and tests.
pub fn hex(s: &str) -> Color32 {
    parse_hex(s).unwrap_or(Color32::MAGENTA)
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    let a = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

/// Watches the resolved Omarchy theme file; polls mtime and re-parses on
/// change. Only consulted when the active theme choice is Omarchy.
pub struct ThemeWatcher {
    path: Option<PathBuf>,
    last_mtime: Option<SystemTime>,
    palette: Palette,
}

impl ThemeWatcher {
    pub fn new() -> ThemeWatcher {
        let path = resolve_theme_path();
        // Log once when falling back to built-in Dark (no colors.toml found).
        if path.is_none() {
            eprintln!("adjutant: no Omarchy colors.toml found; using built-in Dark palette");
        }
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
        let Some(path) = &self.path else {
            return false;
        };
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
        return (Palette::dark(), None);
    };
    let Ok(content) = fs::read_to_string(path) else {
        return (Palette::dark(), None);
    };
    let mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
    (
        Palette::parse_omarchy(&content).unwrap_or_else(Palette::dark),
        mtime,
    )
}
