//! System font resolution + type scale. No font binaries shipped —
//! candidates are looked up under the system font dirs; if none resolve,
//! egui's default font stays (logged once, not fatal).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{FontData, FontDefinitions, FontFamily, FontId, TextStyle};

static LOGGED_FALLBACK: AtomicBool = AtomicBool::new(false);

/// UI font candidates in priority order (§7a).
const UI_FONT_CANDIDATES: &[&str] = &["Inter", "IBMPlexSans", "IBM-Plex-Sans", "NotoSans"];

const FONT_ROOTS: &[&str] = &["/usr/share/fonts", "/usr/local/share/fonts"];

/// Type scale (§7a): 15px body, 13px secondary, 17px section headers.
pub const SIZE_BODY: f32 = 15.0;
pub const SIZE_SMALL: f32 = 13.0;
pub const SIZE_HEADING: f32 = 17.0;
pub const SIZE_TITLE: f32 = 22.0;

/// Find the first matching font file for a candidate family name.
fn find_font_file(candidate: &str) -> Option<PathBuf> {
    let patterns = [
        format!("{candidate}-Regular.ttf"),
        format!("{candidate}.ttf"),
        format!("{candidate}-regular.otf"),
        format!("{candidate}.otf"),
    ];
    for root in FONT_ROOTS {
        let root = Path::new(root);
        if !root.is_dir() {
            continue;
        }
        for pat in &patterns {
            if let Some(hit) = find_in(root, pat) {
                return Some(hit);
            }
        }
    }
    None
}

fn find_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(hit) = find_in(&path, name) {
                return Some(hit);
            }
        } else if path.file_name().is_some_and(|n| n == name) {
            return Some(path);
        }
    }
    None
}

/// Resolve the UI font path, if any candidate exists on the system.
pub fn resolve_ui_font() -> Option<PathBuf> {
    UI_FONT_CANDIDATES.iter().find_map(|c| find_font_file(c))
}

/// Build font definitions: resolved system sans as proportional, egui
/// default mono unchanged. Returns `None` when no system font resolves.
pub fn build_font_definitions(font_path: Option<&Path>) -> Option<FontDefinitions> {
    let owned: Option<PathBuf>;
    let path: &Path = match font_path {
        Some(p) => p,
        None => {
            owned = resolve_ui_font();
            owned.as_deref()?
        }
    };
    let bytes = std::fs::read(path).ok()?;
    let mut defs = FontDefinitions::default();
    let family = "AdjutantSans".to_owned();
    defs.font_data.insert(
        family.clone(),
        std::sync::Arc::new(FontData::from_owned(bytes)),
    );
    defs.families.insert(FontFamily::Proportional, vec![family]);
    Some(defs)
}

/// Text-style overrides implementing the type scale.
pub fn apply_type_scale(style: &mut egui::Style) {
    use std::collections::BTreeMap;
    let body = FontId::new(SIZE_BODY, FontFamily::Proportional);
    let overrides: BTreeMap<TextStyle, FontId> = [
        (TextStyle::Body, body.clone()),
        (TextStyle::Button, body.clone()),
        (
            TextStyle::Small,
            FontId::new(SIZE_SMALL, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(SIZE_HEADING, FontFamily::Proportional),
        ),
    ]
    .into_iter()
    .collect();
    style.text_styles = overrides;
}

/// Install fonts + type scale into a context. Logs once if no system font.
pub fn setup(ctx: &egui::Context) {
    if let Some(defs) = build_font_definitions(None) {
        ctx.set_fonts(defs);
    } else if !LOGGED_FALLBACK.swap(true, Ordering::Relaxed) {
        eprintln!(
            "adjutant: no system UI font found (Inter/IBM Plex Sans/Noto Sans); using egui default"
        );
    }
    ctx.all_styles_mut(apply_type_scale);
}
