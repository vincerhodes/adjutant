//! Theme engine tests: real-format colors.toml parsing, fallback ordering,
#![allow(clippy::unwrap_used)]
//! light/dark visuals mapping. No GPU needed — pure functions.

use adjutant::ui::theme::{self, Mode, Palette};

const AETHER_LIKE: &str = r##"
mode = "dark"
background = "#1e1e2e"
dark_background = "#181825"
darker_background = "#11111b"
lighter_background = "#313244"
foreground = "#cdd6f4"
muted = "#6c7086"
accent = "#89b4fa"
selection = "#45475a"
red = "#f38ba8"
green = "#a6e3a1"
yellow = "#f9e2af"
"##;

const LIGHT_MODE: &str = r##"
mode = "light"
background = "#ffffff"
foreground = "#111111"
muted = "#888888"
accent = "#3366ff"
"##;

#[test]
fn parses_real_omarchy_format() {
    let p = Palette::parse(AETHER_LIKE).expect("parse");
    assert_eq!(p.mode, Mode::Dark);
    assert_eq!(p.background, theme::hex("#1e1e2e"));
    assert_eq!(p.accent, theme::hex("#89b4fa"));
    assert_eq!(p.red, theme::hex("#f38ba8"));
    assert_eq!(p.green, theme::hex("#a6e3a1"));
    assert_eq!(p.yellow, theme::hex("#f9e2af"));
}

#[test]
fn light_mode_respected() {
    let p = Palette::parse(LIGHT_MODE).expect("parse");
    assert_eq!(p.mode, Mode::Light);
    let v = theme::visuals(&p);
    assert!(!v.dark_mode);
    // Accent flows through to selection stroke (single accent principle).
    assert_eq!(v.selection.stroke.color, theme::hex("#3366ff"));
}

#[test]
fn dark_fallback_palette_is_complete() {
    let p = Palette::fallback();
    assert_eq!(p.mode, Mode::Dark);
    let v = theme::visuals(&p);
    assert!(v.dark_mode);
    assert_eq!(v.panel_fill, p.dark_background);
    assert_eq!(v.window_fill, p.background);
    assert_eq!(v.faint_bg_color, p.lighter_background);
    assert_eq!(v.extreme_bg_color, p.darker_background);
    assert_eq!(v.text_color(), p.foreground);
}

#[test]
fn missing_keys_fall_back_per_key() {
    let p = Palette::parse("accent = \"#ff0000\"\n").expect("parse");
    // Accent overridden…
    assert_eq!(p.accent, theme::hex("#ff0000"));
    // …everything else inherits the fallback.
    assert_eq!(p.background, Palette::fallback().background);
    assert_eq!(p.mode, Mode::Dark);
}

#[test]
fn visuals_map_semantic_colors() {
    let p = Palette::parse(AETHER_LIKE).expect("parse");
    let v = theme::visuals(&p);
    assert_eq!(v.error_fg_color, p.red);
    assert_eq!(v.warn_fg_color, p.yellow);
    // Inputs rest borderless (no visible border until focused, §7a).
    assert_eq!(v.widgets.inactive.bg_stroke.width, 0.0);
    assert_eq!(v.widgets.noninteractive.bg_stroke.width, 0.0);
}

#[test]
fn empty_content_is_not_a_palette() {
    assert!(Palette::parse("").is_none());
    assert!(Palette::parse("# just a comment").is_none());
}

#[test]
fn muted_text_has_a_contrast_floor() {
    fn luma(c: egui::Color32) -> f32 {
        (0.2126 * f32::from(c.r()) + 0.7152 * f32::from(c.g()) + 0.0722 * f32::from(c.b())) / 255.0
    }

    // Pathological palette: muted identical to the background.
    let p = Palette::parse(
        r##"
mode = "dark"
background = "#121212"
dark_background = "#121212"
foreground = "#e0e0e0"
muted = "#121212"
accent = "#3366ff"
"##,
    )
    .expect("parse");
    let v = theme::visuals(&p);
    let weak = v.weak_text_color();
    for surface in [v.window_fill, v.panel_fill] {
        assert!(
            (luma(weak) - luma(surface)).abs() >= 0.25,
            "weak text must keep luma distance ≥ 0.25 from every surface"
        );
    }
    // A healthy palette is left untouched.
    let healthy = Palette::parse(AETHER_LIKE).expect("parse");
    let healthy_weak = theme::visuals(&healthy).weak_text_color();
    assert_eq!(healthy_weak, healthy.muted);
}

#[test]
fn theme_watcher_falls_back_without_crashing() {
    // No Omarchy install (or one) — watcher must always yield a palette.
    let watcher = theme::ThemeWatcher::new();
    let _ = watcher.palette();
}
