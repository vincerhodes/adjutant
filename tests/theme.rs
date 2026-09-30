//! Theme engine tests: built-in palettes, Omarchy parsing, border rule,
//! contrast floors. No GPU needed — pure functions.

#![allow(clippy::unwrap_used)]

use adjutant::ui::theme::{self, Palette, ThemeChoice};

const AETHER_LIKE: &str = r##"
mode = "dark"
background = "#1e1e2e"
dark_background = "#181825"
foreground = "#cdd6f4"
muted = "#6c7086"
accent = "#89b4fa"
selection = "#45475a"
red = "#f38ba8"
green = "#a6e3a1"
yellow = "#f9e2af"
"##;

fn luma(c: egui::Color32) -> f32 {
    (0.2126 * f32::from(c.r()) + 0.7152 * f32::from(c.g()) + 0.0722 * f32::from(c.b())) / 255.0
}

#[test]
fn builtin_palettes_match_spec_table() {
    let light = Palette::light();
    assert_eq!(light.background, theme::hex("#F7F6F2"));
    assert_eq!(light.panel, theme::hex("#FFFFFF"));
    assert_eq!(light.text, theme::hex("#1C1E21"));
    assert_eq!(light.muted, theme::hex("#6E7278"));
    assert_eq!(light.accent, theme::hex("#4C6EF5"));
    assert!(!light.dark);

    let sepia = Palette::sepia();
    assert_eq!(sepia.background, theme::hex("#F1EADB"));
    assert_eq!(sepia.accent, theme::hex("#A8732A"));
    assert!(!sepia.dark);

    let slate = Palette::slate();
    assert_eq!(slate.background, theme::hex("#262B33"));
    assert_eq!(slate.accent, theme::hex("#7EA1FF"));
    assert!(slate.dark);

    let dark = Palette::dark();
    assert_eq!(dark.background, theme::hex("#101216"));
    assert_eq!(dark.accent, theme::hex("#7C9EFF"));
    assert!(dark.dark);
}

#[test]
fn parses_omarchy_colors_toml() {
    let p = Palette::parse_omarchy(AETHER_LIKE).expect("parse");
    assert!(p.dark);
    assert_eq!(p.background, theme::hex("#1e1e2e"));
    assert_eq!(p.panel, theme::hex("#181825"));
    assert_eq!(p.text, theme::hex("#cdd6f4"));
    assert_eq!(p.danger, theme::hex("#f38ba8"));
    assert_eq!(p.success, theme::hex("#a6e3a1"));
    assert_eq!(p.warn, theme::hex("#f9e2af"));
    // Derived card/border fields exist and are sane.
    assert!(luma(p.card_fill) > luma(p.background));
    assert!(luma(p.card_hover) > luma(p.card_fill));
}

#[test]
fn omarchy_light_mode_respected() {
    let p = Palette::parse_omarchy("mode = \"light\"\nbackground = \"#ffffff\"\n").expect("parse");
    assert!(!p.dark);
}

#[test]
fn theme_choice_roundtrips_through_settings_name() {
    for choice in ThemeChoice::ALL {
        assert_eq!(ThemeChoice::from_name(choice.name()), Some(choice));
    }
    assert_eq!(ThemeChoice::from_name("nonsense"), None);
}

#[test]
fn card_border_only_when_fill_close_to_background() {
    // Light and Sepia have near-white cards on off-white backgrounds.
    assert!(theme::card_needs_border(&Palette::light()));
    assert!(theme::card_needs_border(&Palette::sepia()));
    // Dark themes separate card from background by value.
    assert!(!theme::card_needs_border(&Palette::slate()));
    assert!(!theme::card_needs_border(&Palette::dark()));
}

#[test]
fn every_builtin_theme_passes_contrast_floor() {
    for choice in ThemeChoice::ALL {
        let p = match choice {
            ThemeChoice::Omarchy => continue, // system-dependent
            _ => Palette::builtin(choice),
        };
        let text_dist = (luma(p.text) - luma(p.background)).abs();
        let muted_dist = (luma(p.muted) - luma(p.background)).abs();
        assert!(
            text_dist >= 0.45,
            "{}: text↔bg luma {text_dist:.3} < 0.45",
            choice.label()
        );
        assert!(
            muted_dist >= 0.20,
            "{}: muted↔bg luma {muted_dist:.3} < 0.20",
            choice.label()
        );
    }
}

#[test]
fn muted_floor_lifts_pathological_palettes() {
    // Muted identical to the background must be blended toward text.
    let content = "background = \"#121212\"\ndark_background = \"#121212\"\nforeground = \"#e0e0e0\"\nmuted = \"#121212\"\naccent = \"#3366ff\"\n";
    let p = Palette::parse_omarchy(content).expect("parse");
    let v = theme::visuals(&p);
    let weak = v.weak_text_color();
    for surface in [v.window_fill, v.panel_fill] {
        assert!(
            (luma(weak) - luma(surface)).abs() >= 0.25,
            "weak text must keep luma distance ≥ 0.25 from every surface"
        );
    }
    // Healthy palettes keep their muted color.
    let healthy = Palette::parse_omarchy(AETHER_LIKE).unwrap();
    assert_eq!(theme::visuals(&healthy).weak_text_color(), healthy.muted);
}

#[test]
fn visuals_map_palette_fields() {
    let p = Palette::slate();
    let v = theme::visuals(&p);
    assert!(v.dark_mode);
    assert_eq!(v.panel_fill, p.panel);
    assert_eq!(v.window_fill, p.background);
    assert_eq!(v.warn_fg_color, p.warn);
    assert_eq!(v.error_fg_color, p.danger);
    // Inputs rest borderless.
    assert_eq!(v.widgets.inactive.bg_stroke.width, 0.0);
}

#[test]
fn theme_watcher_never_fails_hard() {
    let watcher = theme::ThemeWatcher::new();
    // Whatever the system state, Omarchy resolves to some palette and the
    // border rule is decidable.
    let p = theme::active_palette(ThemeChoice::Omarchy, &watcher);
    let _ = theme::card_needs_border(&p);
}
