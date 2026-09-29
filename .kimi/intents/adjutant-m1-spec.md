# Adjutant — Technical Spec (M1: Foundation + Todo)

Source: `.kimi/intents/adjutant-m1-intent.md`. Greenfield — empty repo at `/home/jimmy/Projects/adjutant`.

## 1. Toolchain & prerequisites

- Rust stable via **rustup** at `~/.cargo/bin` (1.98.1, installed 2026-09-29). NOTE: plan originally said mise — mise's rust plugin downloader fails behind the GFW proxy; rustup is the toolchain manager for this repo. All cargo invocations need `PATH="$HOME/.cargo/bin:$PATH"` and `https_proxy=http://127.0.0.1:7897` (crates.io via Clash).
- System libs (already present, verified 2026-09-29): `libsecret-1`, `sqlite3` (pkg-config).
- `rustfmt` + `clippy` components (installed).

## 2. Crate layout

Single binary crate `adjutant` (name verified free on crates.io, 2026-09-29). Module tree designed so M2+ modules slot in as siblings without touching core:

```
adjutant/
├── Cargo.toml
├── .mise.toml
├── migrations/
│   └── 0001_init.sql          # full M1 schema
├── src/
│   ├── main.rs                # eframe entry, app bootstrap
│   ├── app.rs                 # AdjutantApp: eframe::App impl, module nav, routing
│   ├── db/
│   │   ├── mod.rs             # Db handle (rusqlite::Connection wrapper), open/init
│   │   ├── migrate.rs         # versioned migration runner (user_version pragma)
│   │   └── settings.rs        # key-value app settings CRUD
│   ├── core/
│   │   ├── entity.rs          # EntityType enum, EntityRef { type, id }
│   │   └── link.rs            # link layer: create/remove/query/traverse
│   ├── todo/
│   │   ├── mod.rs             # TodoStore: CRUD, tree queries, reorder
│   │   ├── model.rs           # Todo, TodoGroup, Priority, Status
│   │   └── ui.rs              # egui tree view, inline edit, detail pane
│   └── ui/
│       ├── mod.rs             # shared widgets
│       ├── theme.rs           # Omarchy colors.toml → egui Visuals, live reload
│       ├── fonts.rs           # system font resolution + type scale
│       ├── help.rs            # F1 shortcut overlay
│       └── placeholder.rs     # disabled nav items (Email/Calendar/Notes)
└── tests/
    ├── link_layer.rs          # in-memory DB integration tests
    ├── todo_store.rs
    └── theme.rs               # palette parse/map tests
```

Governance rules:
- `core/` and `db/` never depend on `todo/` or `ui/` — dependency direction is UI → module → core/db.
- All SQL lives in `db/` + module `mod.rs` files; UI files contain zero SQL.
- Later modules (email/calendar/notes) add `src/<module>/` + migrations, nothing else.

## 3. Dependencies (pinned majors; exact at scaffold time)

| Crate | Version | Purpose |
|---|---|---|
| eframe | 0.36 | GUI framework (winit+wgpu default; egui 0.36 API) |
| rusqlite | 0.40 | SQLite; features: `bundled` (static binary, no system lib dep) |
| serde + serde_json | 1 / 1 | settings values, future MCP payloads |
| thiserror | 2 | library-style errors (db, core, todo) |
| anyhow | 1 | app-level error aggregation in main/app |
| chrono | 0.4 | timestamps; feature `serde` |
| directories | 6 | XDG paths (`~/.local/share/adjutant/`) |
| uuid | 1 | entity IDs; features `v4`, `serde` |
| secret-service | 5 | keyring — **dependency declared now, used from M2**; M1 code must not touch it |

No tokio in M1 (no async work yet). `env_logger`/`log` only if debugging demands — default: no logging framework in M1.

## 4. SQLite schema (migrations/0001_init.sql)

IDs: `TEXT` UUIDv4. Timestamps: `TEXT` ISO-8601 UTC (chrono). Bools: `INTEGER 0/1`.

```sql
-- migrations tracked via PRAGMA user_version
CREATE TABLE todos (
  id            TEXT PRIMARY KEY,
  group_id      TEXT NOT NULL REFERENCES todo_groups(id) ON DELETE CASCADE,
  parent_id     TEXT REFERENCES todos(id) ON DELETE CASCADE,   -- NULL = top-level in group
  title         TEXT NOT NULL,
  notes         TEXT NOT NULL DEFAULT '',
  status        TEXT NOT NULL DEFAULT 'open'
                CHECK (status IN ('open','in_progress','done','cancelled')),
  priority      INTEGER NOT NULL DEFAULT 2 CHECK (priority BETWEEN 0 AND 3), -- 0=low..3=urgent
  position      INTEGER NOT NULL DEFAULT 0,                     -- sibling ordering
  due_date      TEXT,                                           -- ISO-8601 date (YYYY-MM-DD), optional
  deleted_at    TEXT,                                           -- trash: NULL = live, set = trashed at timestamp
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  completed_at  TEXT
);
CREATE INDEX idx_todos_group ON todos(group_id, parent_id, position) ;
CREATE INDEX idx_todos_due ON todos(due_date) WHERE deleted_at IS NULL;

CREATE TABLE todo_groups (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  position   INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- updated_at maintained by triggers, NOT app code: M4's MCP server writes
-- directly to the DB and must not be able to skip timestamp bumps
-- (last-write-wins sync depends on accurate updated_at).
CREATE TRIGGER trg_todos_updated AFTER UPDATE ON todos
BEGIN UPDATE todos SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;
CREATE TRIGGER trg_todo_groups_updated AFTER UPDATE ON todo_groups
BEGIN UPDATE todo_groups SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;

-- Generic entity links — the core graph. Entity rows live in their own
-- tables per type; this table references them loosely (no FK, entities
-- from future modules don't exist yet). Integrity enforced in link layer code.
CREATE TABLE entity_links (
  id          TEXT PRIMARY KEY,
  source_type TEXT NOT NULL CHECK (source_type IN ('email','thread','event','todo','note','reminder')),
  source_id   TEXT NOT NULL,
  target_type TEXT NOT NULL CHECK (target_type IN ('email','thread','event','todo','note','reminder')),
  target_id   TEXT NOT NULL,
  relation    TEXT NOT NULL,   -- v1 vocab: derived_from|blocks|scheduled_as|mentions; open enum by design
  created_at  TEXT NOT NULL,
  UNIQUE (source_type, source_id, target_type, target_id, relation)
);
CREATE INDEX idx_links_source ON entity_links(source_type, source_id);
CREATE INDEX idx_links_target ON entity_links(target_type, target_id);

CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL          -- JSON-encoded
);
```

Design decisions:
- **`entity_links` has no FK constraints** — deliberate. Targets in tables that don't exist until M2+; FK would force schema churn. Link layer validates known types (todo in M1) and tolerates dangling refs (rendered as "unavailable" in UI later).
- `relation` is a CHECK-less TEXT — adding relations must not require migration. Vocabulary enforced in Rust (`Relation` newtype w/ known constants + `Other(String)`).
- Single `todos` table with self-FK for nesting (adjacency list) — arbitrary depth, simple recursive CTE queries.
- Status/priority as CHECK-constrained columns, not separate tables.

## 5. Link layer API (src/core/link.rs)

```rust
pub struct EntityRef { pub kind: EntityType, pub id: Uuid }
pub enum EntityType { Email, Thread, Event, Todo, Note, Reminder }  // as_str/FromStr

impl LinkStore<'_> {
    fn link(&mut self, src: &EntityRef, tgt: &EntityRef, rel: &Relation) -> Result<()>;
    fn unlink(&mut self, src: &EntityRef, tgt: &EntityRef, rel: &Relation) -> Result<()>;
    fn links_from(&self, src: &EntityRef) -> Result<Vec<Link>>;
    fn links_to(&self, tgt: &EntityRef) -> Result<Vec<Link>>;
    /// Cycle check for 'blocks': walk target's outgoing blocks-edges; reject if src reachable.
    fn would_cycle(&self, src: &EntityRef, tgt: &EntityRef) -> Result<bool>;
}
```

- `link()` on `relation == "blocks"` → run `would_cycle` first; reject with typed error. Other relations: cycles allowed (mentions is naturally cyclic).
- Cycle detection: recursive CTE over `entity_links` filtered `relation='blocks'`.
- Delete cascade: deleting a todo deletes its links (`DELETE FROM entity_links WHERE (source_type='todo' AND source_id=?) OR (target_type='todo' AND target_id=?)`) in the same transaction as the row delete.

## 6. Todo module behavior

- Groups: flat named lists (e.g. "Personal", "Browzr"), user-ordered.
- Todos: arbitrary-depth nesting within a group; `position` for manual sibling order.
- Statuses: `open`, `in_progress`, `done`, `cancelled`.
- Priority: 0–3 (low/normal/high/urgent), color-coded in UI.
- **Completing a parent with open children:** prompt-less policy — parent can be marked done only when all descendants are done/cancelled (UI disables checkbox otherwise, tooltip explains). Keeps tree integrity without modals.
- **Due dates:** optional date (no time) per todo, any depth. Displayed in tree row right-aligned (muted; red if overdue and status not terminal; accent if due today). Set/cleared from detail pane date field (`YYYY-MM-DD`, validated text input — no calendar widget in M1). M3 reminders will link onto this field.
- **Trash (soft delete):** "Delete" sets `deleted_at` on the todo and all descendants (same tx; descendants inherit parent's `deleted_at` timestamp). Trashed todos: excluded from every live query/tree/badge, links retained intact. Sidebar "Trash" section lists top-level trashed items → restore (clears `deleted_at` on item + descendants, original group/position/links intact) or delete permanently (hard delete, cascades rows + links per spec §5). Startup purge: rows with `deleted_at` older than 30 days are hard-deleted at app open (logged to stdout, count only).
- Reorder: `Alt+↑/↓` moves selected todo among siblings (position swap). No drag-drop in M1.
- **Edit commit semantics (applies to every inline-editable field):** `Enter` commits, `Esc` reverts, focus-loss commits. Multi-line notes editor in detail pane: saves on blur and on `Ctrl+S`; no explicit save button. A field being edited is never clobbered by background refresh — tree reloads preserve in-flight editor state.
- Filter: `Ctrl+F` opens inline filter field above tree; case-insensitive title substring, client-side over the loaded tree (no SQL); non-matching subtrees hidden unless they contain a match (ancestors of matches stay visible, collapsed count shown); `Esc` clears.
- Todo↔todo `blocks` linking from UI: detail pane "Blocked by" picker (search other todos by title); cycle rejected with inline error.

## 7. egui UI structure

- `AdjutantApp` state: `Db`, current `Module` enum, per-module state structs.
- Left sidebar: module nav — **Todo** active; Email, Calendar, Scratchpad rendered disabled with "M2+…" tooltips.
- Todo screen: group tabs/list (left) → tree of todos (center, indented, collapsible, checkbox + priority chip + title inline-edit, right-aligned due date, filter field above) → detail pane (right: notes, status, priority, due date, links, delete).
- Sidebar bottom section: Trash (count badge), opens trash list view (restore / delete permanently per item).
- Keyboard-first: `Ctrl+N` new todo, `Ctrl+Shift+N` new group, `Enter` edit, `Space` toggle done, `Ctrl+F` filter, `Alt+↑/↓` reorder, arrows navigate tree, `F1` help overlay (shortcut reference), `Ctrl+=`/`Ctrl+-`/`Ctrl+0` zoom in/out/reset (egui zoom factor — near-mandatory on HiDPI Hyprland).
- Rendering: reactive only — repaint on input events + the 1s theme-watch tick (`ctx.request_repaint_after`). No continuous render loop (battery). Known conflict: `Alt+↑/↓` collides with some Hyprland bind configs; shortcuts become configurable in a later milestone, note in README.
- Window 1200×800 default, title "Adjutant".
- Persistence: window size/position + last-open group in `settings` table.

## 7a. Visual design (normative — "beautiful, functional, simple, not cluttered")

Principles (apply to every screen, M1 and future):
1. **Content over chrome.** No visible box-drawing for its own sake: no nested frames, no bordered panels inside panels. Separation via spacing + subtle background steps, not strokes. Strokes only for focused/selected states.
2. **One accent color.** Theme `accent` is the only saturated UI color — used for: selected nav item, primary action button, active checkbox, focused-field underline. Everything else: background ramp + foreground ramp.
3. **Whitespace is the layout.** Generous padding (outer margins 16px, section gaps 12px, list item vertical padding 6px). When in doubt, add space, not a divider.
4. **Progressive disclosure.** Detail pane hidden until a todo is selected. Delete/destructive actions behind explicit affordances (button in detail pane, never inline in tree rows). No toolbars of icons.
5. **Empty states designed, not blank.** No groups yet → centered muted text + "Ctrl+Shift+N to create your first group". Empty tree → same pattern. Every screen has a designed zero state.

Theme engine (`src/ui/theme.rs`):
- Source: Omarchy `colors.toml` — flat `key = "#hex"` file (verified format 2026-09-29: `mode`, `accent`, `selection`, `muted`, `background`/`dark_background`/`darker_background`/`lighter_background`, `foreground` ramp, ANSI colors).
- Resolution order: `~/.config/omarchy/themes/aether/colors.toml` (symlink → live Aether theme) → if missing, single `colors.toml` under `~/.config/omarchy/themes/` when exactly one exists → built-in fallback palette (spec'd below, never a hard failure).
- Live reload: poll file mtime every 1s in `egui` update loop; on change, re-parse + apply. Cheap, no inotify dep.
- Mapping → egui `Visuals`: `background`→`panel_fill`, `darker_background`→sidebar fill, `lighter_background`→selection/hover bg, `foreground`→text, `muted`→disabled/placeholder text, `accent`→per principle 2, `red`→destructive buttons + errors, `green`→done status, `yellow`→in_progress status.
- Light `mode = "light"` respected via same mapping (egui visuals built from palette, not `Visuals::dark()` clone).
- Built-in fallback palette: Tokyo Night-ish dark (bg `#1a1b26`, accent `#7aa2f7`, fg `#c0caf5`, muted `#565f89`).

Typography:
- UI text: system sans — load `Inter` then fallback `IBM Plex Sans`, `Noto Sans`, via fontconfig path lookup (`/usr/share/fonts/`); ship no font binaries.
- Monospace: `JetBrains Mono` (fallback system mono) for: nothing in M1 UI chrome — reserved for future code snippets/email. M1 uses sans throughout, mono only in DB-visible IDs if ever displayed (it isn't — principle: no UUIDs in UI).
- Scale: 15px base body, 13px secondary/muted, 17px semi-bold section headers, 22px screen title. Tree indent 20px per level.
- If no system font resolves → egui default (logged once, not fatal).

Widget language:
- Nav: sidebar width 180px, text-only items, selected = accent text + 3px accent left bar, disabled = muted.
- Tree rows: single line — checkbox (accent when checked), priority shown as 3px left color dot (urgent=red, high=orange, normal=fg-muted, low=muted; dot only, no badges), title text. Hover = `lighter_background` row fill. Selected = persistent fill + accent left edge. Done = strikethrough + muted.
- Buttons: one primary style (accent fill, bg-colored text), one ghost style (no fill, accent text, hover fill). No third style.
- Inputs: no visible border until focused; focused = accent underline. Placeholder text in `muted`.
- Status: text label colored per mapping (no pills/badges).
- Due dates: right-aligned muted date text in tree rows; overdue (past date, non-terminal status) = `red`; due today = `accent`; never shown for done/cancelled items.
- Trash view: same list language as tree rows (single line, muted title, restore = ghost button right-aligned, permanent delete = `red` ghost button). Empty trash → designed empty state per principle 5.

Anti-patterns (explicitly banned in review):
- No emoji/icons-as-decoration in M1 UI (icon font comes with later modules only if needed).
- No modal dialogs except delete confirmation (principle 4 exception — destructive).
- No tab bars within panes; group switching lives in sidebar sub-list, not tabs.
- No animated transitions beyond egui defaults.

## 8. Error handling

- `db`/`core`/`todo` expose `thiserror` enums (`DbError`, `LinkError`, `TodoError`); `app.rs` maps to anyhow + UI error toast (egui `Window`/label bar).
- No `unwrap()`/`expect()` outside `main.rs` bootstrap and tests — enforced by clippy lint config (`unwrap_used` warn).

## 9. Security stance (M1)

- No network code, no secrets, no IPC. Data dir created `0700`, DB file `0600` (covers WAL sidecar files too).
- No user content leaves the process. Single-user.
- **Single-instance enforced:** `flock` on `<data_dir>/adjutant.lock` at startup; second instance exits non-zero with "Adjutant is already running" (no focus-steal IPC in M1). Two GUIs on one DB would silently last-write-wins each other.
- **Crash diagnostics:** panic hook writes `crash-<timestamp>.log` (message + backtrace) into the data dir — app launched from walker/rofi has no terminal, panics must not vanish. RUST_BACKTRACE honored.
- WAL + busy_timeout make future concurrent access (M4 MCP server) safe; single-instance lock covers the GUI-vs-GUI case only.

## 10. Testing strategy

- Integration tests in `tests/` against in-memory SQLite (`Connection::open_in_memory()` + run migrations): link CRUD, cycle rejection, todo tree ops, cascade delete, parent-completion rule.
- No UI tests in M1 (egui testing immature; manual verification per acceptance criteria).
- `cargo clippy -- -D warnings` and `cargo fmt --check` gate completion.
