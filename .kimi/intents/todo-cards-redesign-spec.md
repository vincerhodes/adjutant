# Adjutant — Spec: Todo Cards Redesign + Theme System (M1.5)

Supersedes parts of `adjutant-m1-spec.md` §7/§7a (marked inline there). Jimmy's brief (2026-09-30): "polished app that will work cross-platform in the future… clean look… work on the todo area until we've nailed it." Intent: prove the design language on todos; email/calendar/notes inherit it later.

## 1. Theme system

Built-in themes, switchable in-app, persisted (`settings` key `ui.theme`), applied live (no restart). Five at launch:

| Theme | Flavour | Palette (bg / panel / text / muted / accent) |
|---|---|---|
| **Light** (DEFAULT) | off-white, clear legible text | `#F7F6F2` / `#FFFFFF` / `#1C1E21` / `#6E7278` / `#4C6EF5` |
| **Sepia** | warm mid | `#F1EADB` / `#FAF5EA` / `#2E2A23` / `#8A7E6A` / `#A8732A` |
| **Slate** | muted mid-dark | `#262B33` / `#2F3540` / `#D8DDE4` / `#7C8590` / `#7EA1FF` |
| **Dark** | near-black | `#101216` / `#171A1F` / `#E3E6EB` / `#7A818C` / `#7C9EFF` |
| **Omarchy** | follows system | existing `colors.toml` engine + 1s live reload, unchanged |

- Each palette also defines: `border` (hairline), `success` (green), `warn` (yellow/amber), `danger` (red), `card_fill`, `card_hover`, `selection_fill` — full struct in code, not computed per-widget.
- Palette struct gains semantic color fields; theme.rs already stashes green via data store (M1 pattern) — generalize: whole `Palette` into egui data store, widgets read from there.
- Theme picker: ghost button in sidebar bottom ("Theme: Light"), click → egui popup menu listing all five, checkmark on active. Choice persisted immediately.
- Omarchy theme keeps its fallback chain (file missing → built-in Dark + log).
- Light theme = default on fresh installs (settings unset). Current users (Jimmy's DB) will flip to Light once — fine, he asked for it.
- All widget colors must come from the palette/semantic layer — hardcoded hex in widgets is a review failure (exception: palette definitions themselves).

## 2. Layout chrome

- **Collapsible sidebar**: chevron-style ghost toggle ("«") at sidebar top-right collapses it to a 0-width hidden state; a slim reveal affordance ("»" floating top-left) brings it back. Sidebar state persisted (`ui.sidebar_collapsed`). Collapse/expand animates via egui's default panel width tweening if free, else instant (no custom animation work).
- **Focus mode**: collapses sidebar AND maximizes the todo list to the full window. Toggle: `Ctrl+.` and a ghost "Focus" button in the list header. In focus mode only the list header + cards remain. Esc exits focus mode (in addition to toggle). Persisted (`ui.focus_mode`) — restore on launch.
- Module nav (Todo/Email/Calendar/Scratchpad) stays in sidebar — unchanged behavior, muted-disabled for unbuilt modules.

## 3. Todo cards (the core change)

The right-hand **detail pane is deleted**. All viewing/editing happens in cards.

**Folded card (default state)** — one long horizontal card per todo:
- Full width of list area, fixed height (~46px), corner radius 6, fill `card_fill`, 8px gaps between cards (clear separation), hairline `border` only in Light/Sepia themes where fill/bg contrast is low (derive: if luma distance card_fill↔bg < 0.08, show border).
- Left to right: checkbox → title → spring → badges.
- **Badges** (small rounded pills, 12px text, 4px h-pad, semantic colors — badges are now explicitly allowed; §7a's no-badges rule is superseded for these):
  - Status pill: Open (muted outline), In progress (warn), Done (success), Cancelled (muted, plus strikethrough title).
  - Priority pill: only shown for High/Urgent (Low/Normal = no pill — calm default).
  - Due pill: date (`Oct 3`), danger if overdue, accent if today, muted otherwise. Hidden for terminal todos.
  - Sub-count pill: `N sub` when todo has children (shows open descendant count).
  - Blocked pill: `Blocked` when a live `blocks` link targets this todo (danger outline).
- Hover: `card_hover` fill. Selected (keyboard nav): 2px accent outline (replaces the old fill+bar — crisper cross-platform).

**Unfolded card** — click anywhere on the card body (not checkbox/badges) toggles:
- Card grows in place; children render as nested cards indented 24px beneath it (recursive — a child card unfolds the same way).
- Expanded section at card bottom (above children): 
  - Notes editor (multi-line, same commit semantics as before).
  - Status segmented row + Priority segmented row (existing widget language).
  - Due field (hover underline affordance).
  - Actions row: `Add sub-todo`, `Move up`, `Move down`, `Blocked by…` picker, `Move to trash…`.
  - Only ONE card expanded at a time per top-level subtree is NOT required — independent toggles, state kept per todo id (replaces the old `collapsed` set: invert semantics — default folded).
- Double-click title still enters inline title edit (folded or unfolded). Enter commits, Esc reverts.
- Done parent rule unchanged (checkbox disabled w/ tooltip when open descendants).

**Group header row** above list: group name + `+ New todo` (top-level) + `Filter…` + `Focus` — unchanged behavior from review pass, restyled to match.

## 4. Keyboard contract (unchanged, verify)

All existing shortcuts keep working: Ctrl+N (child of selection), Ctrl+Shift+N, Enter/Space/arrows, Alt+↑/↓, Ctrl+F, F1, zoom. New: `Ctrl+.` focus mode, Esc exits focus mode when no editor/filter/modal is active. Help overlay updated (theme picker note, focus mode, card click semantics).

## 5. What stays

Trash view (restyle rows as slim cards), group sidebar list, filter, all store/DB layers (zero schema changes), link layer, single-instance flock, crash logs, theme live-reload for Omarchy.

## 6. Testing

- Update all broken kittest tests (rows→cards, detail pane removal). New: card click unfolds (AccessKit), focus mode toggle, theme picker switches palette + persists, only-Light-shows-border logic unit test, badge presence rules (high priority only, blocked pill, sub-count).
- Palette unit tests: every built-in theme passes a contrast floor check (text↔bg luma ≥ 0.45, muted↔bg ≥ 0.20 — encode the M1 muted-floor rule as a test over all five palettes).
- Gates: fmt, clippy -D warnings, cargo test, release build.

## 7. Non-goals

No settings screen/dialog (theme picker is a popup), no custom user palettes, no per-module themes, no animation work beyond egui defaults, no changes to email/calendar/scratchpad placeholders beyond inheriting the palette.
