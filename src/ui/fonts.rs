//! System font resolution + type scale. No font binaries shipped —
//! candidates are looked up under the system font dirs; if none resolve,
//! egui's default font stays (logged once, not fatal).
//!
//! Weight strategy (HEY pass §8): egui has no fake-bold, so headings and
//! card titles need real face files. Each weight resolves independently —
//! bold prefers *-Bold cuts, semibold prefers *-SemiBold then *-Medium —
//! and falls back to the regular family when absent (documented behavior:
//! on this machine Noto Sans provides Bold + Medium, no SemiBold).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Context, FontData, FontDefinitions, FontFamily, FontId, TextStyle};

static LOGGED_FALLBACK: AtomicBool = AtomicBool::new(false);

const FAMILY: &str = "AdjutantSans";
const FAMILY_SEMIBOLD: &str = "AdjutantSansSemiBold";
const FAMILY_BOLD: &str = "AdjutantSansBold";

/// UI font candidates in priority order (§7a).
const UI_FONT_CANDIDATES: &[&str] = &["Inter", "IBMPlexSans", "IBM-Plex-Sans", "NotoSans"];

const FONT_ROOTS: &[&str] = &["/usr/share/fonts", "/usr/local/share/fonts"];

/// Type scale (§7a): 15px body, 13px secondary. The group heading (26px
/// heavy) and card titles (16px semibold) use explicit FontIds — see
/// `heading_font` / `title_font`.
pub const SIZE_BODY: f32 = 15.0;
pub const SIZE_SMALL: f32 = 13.0;
pub const SIZE_HEADING: f32 = 17.0;

/// Group heading size (HEY pass §8).
pub const SIZE_GROUP_HEADING: f32 = 26.0;
/// Card title size (HEY pass §8).
pub const SIZE_CARD_TITLE: f32 = 16.0;

const SEMIBOLD_RESOLVED_ID: &str = "adjutant.fonts.semibold_resolved";
const BOLD_RESOLVED_ID: &str = "adjutant.fonts.bold_resolved";

/// Find the first matching font file for a candidate family name, trying
/// the given weight cuts before the plain regular file.
fn find_font_file(candidate: &str, weights: &[&str]) -> Option<PathBuf> {
    let mut patterns: Vec<String> = weights
        .iter()
        .flat_map(|w| {
            [
                format!("{candidate}-{w}.ttf"),
                format!("{candidate}-{w}.otf"),
            ]
        })
        .collect();
    patterns.extend([
        format!("{candidate}-Regular.ttf"),
        format!("{candidate}.ttf"),
        format!("{candidate}-regular.otf"),
        format!("{candidate}.otf"),
    ]);
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

/// Resolve the regular UI font path, if any candidate exists on the system.
pub fn resolve_ui_font() -> Option<PathBuf> {
    UI_FONT_CANDIDATES
        .iter()
        .find_map(|c| find_font_file(c, &[]))
}

fn resolve_weight(weights: &[&str]) -> Option<PathBuf> {
    UI_FONT_CANDIDATES
        .iter()
        .find_map(|c| find_font_file(c, weights))
}

/// Build font definitions: resolved system sans (regular + semibold + bold
/// cuts when present) registered as named families, egui's bundled fonts
/// kept as fallbacks (glyph coverage for symbols the system font lacks),
/// default mono unchanged. Returns `None` when no system font resolves.
///
/// Also reports whether the semibold/bold families resolved to real face
/// files — callers stash that per-context (egui panics on named families
/// that aren't bound in the context's font definitions).
pub fn build_font_definitions(font_path: Option<&Path>) -> Option<(FontDefinitions, bool, bool)> {
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
    let fallbacks = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    let mut chain = vec![FAMILY.to_owned()];
    defs.font_data.insert(
        FAMILY.to_owned(),
        std::sync::Arc::new(FontData::from_owned(bytes)),
    );
    chain.extend(fallbacks.iter().cloned());
    defs.families.insert(FontFamily::Proportional, chain);

    let register = |defs: &mut FontDefinitions, family: &str, path: Option<PathBuf>| -> bool {
        let Some(path) = path else { return false };
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        defs.font_data.insert(
            family.to_owned(),
            std::sync::Arc::new(FontData::from_owned(bytes)),
        );
        let mut chain = vec![family.to_owned()];
        chain.extend(fallbacks.iter().cloned());
        // Also reach the regular face for glyphs the weight cut lacks.
        chain.insert(0, FAMILY.to_owned());
        defs.families
            .insert(FontFamily::Name(family.to_owned().into()), chain);
        true
    };

    let semibold = register(
        &mut defs,
        FAMILY_SEMIBOLD,
        resolve_weight(&["SemiBold", "Medium"]),
    );
    let bold = register(&mut defs, FAMILY_BOLD, resolve_weight(&["Bold"]));
    Some((defs, semibold, bold))
}

fn weight_resolved(ctx: &Context, id: &str) -> bool {
    ctx.data_mut(|d| d.get_persisted(egui::Id::new(id)))
        .unwrap_or(false)
}

/// 26px heavy group heading — real bold face when one resolved in this
/// context, else the proportional family (documented fallback, §8).
pub fn heading_font(ctx: &Context) -> FontId {
    let family = if weight_resolved(ctx, BOLD_RESOLVED_ID) {
        FontFamily::Name(FAMILY_BOLD.to_owned().into())
    } else {
        FontFamily::Proportional
    };
    FontId::new(SIZE_GROUP_HEADING, family)
}

/// 16px semibold card title — real semibold/medium face when one resolved
/// in this context, else the proportional family.
pub fn title_font(ctx: &Context) -> FontId {
    let family = if weight_resolved(ctx, SEMIBOLD_RESOLVED_ID) {
        FontFamily::Name(FAMILY_SEMIBOLD.to_owned().into())
    } else {
        FontFamily::Proportional
    };
    FontId::new(SIZE_CARD_TITLE, family)
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
    if let Some((defs, semibold, bold)) = build_font_definitions(None) {
        ctx.set_fonts(defs);
        ctx.data_mut(|d| {
            d.insert_persisted(egui::Id::new(SEMIBOLD_RESOLVED_ID), semibold);
            d.insert_persisted(egui::Id::new(BOLD_RESOLVED_ID), bold);
        });
    } else if !LOGGED_FALLBACK.swap(true, Ordering::Relaxed) {
        eprintln!(
            "adjutant: no system UI font found (Inter/IBM Plex Sans/Noto Sans); using egui default"
        );
    }
    ctx.all_styles_mut(apply_type_scale);
}
