# Adjutant — Technical Spec (M3.5: Scratch Pad)

Source: `.kimi/intents/adjutant-m3.5-scratchpad-intent.md` (normative for
decisions). Builds on M1/M2/M3 layout + design language. Zero changes to
existing migrations (0001-0003). **`EntityType::Note` already exists in
`src/core/entity.rs` and the `entity_links` CHECK constraint already allows
`'note'` — the link layer needs no core change and no migration (locked
decision 2).** Repo `/home/jimmy/Projects/adjutant`, branch `m3-calendar`.

## 1. Dependencies (additions)

None. No new crates — chrono, serde, rusqlite, egui all present. This
module is deliberately boring.

## 2. Module layout (additions, dependency rules unchanged)

```
src/scratch/
├── mod.rs            # ScratchStore: SQL CRUD, pin ordering, trash, purge — ALL SQL here
├── model.rs          # ScratchPad, ScratchPadInput
└── ui/
    ├── mod.rs        # ScratchUi state + routing (Pads / Trash toggle), autosave debounce
    └── pad_card.rs   # two-line card + unfolded editor (kept separate so mod.rs stays readable)
migrations/0004_scratchpad.sql
tests/
├── scratch_store.rs  # in-memory DB: CRUD, pin ordering, trash/purge, link purge
└── scratch_ui.rs     # kittest: create/autosave, pin ordering, trash/restore, links
```

Dependency direction strictly **UI → module → core/db**. Zero SQL outside
`src/scratch/mod.rs` (and `src/db/`).

## 3. Schema — migrations/0004_scratchpad.sql

```sql
CREATE TABLE scratch_notes (
  id          TEXT PRIMARY KEY,            -- app uuid
  body        TEXT NOT NULL DEFAULT '',    -- plain text, no rendering
  pinned      INTEGER NOT NULL DEFAULT 0,
  color_idx   INTEGER NOT NULL DEFAULT 0,  -- 6-color cycle, matches group dots (0..=5)
  trashed_at  TEXT,                        -- soft delete (30-day purge reuses startup rule)
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);
CREATE TRIGGER trg_scratch_notes_updated AFTER UPDATE ON scratch_notes
BEGIN UPDATE scratch_notes SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;
```

Timestamps: ISO-8601 UTC text (`chrono::Utc::now().to_rfc3339()`), matching
M1/M2/M3. `updated_at` via trigger — M5 last-write-wins reads these;
`trashed_at` soft delete keeps link rows resolvable; hard delete purges
links via `LinkStore::delete_links_for` in the same transaction (todo and
calendar pattern).

## 4. ScratchStore API surface (src/scratch/mod.rs)

```rust
impl ScratchStore<'_> {
    fn create(&self, input: &ScratchPadInput) -> Result<ScratchPad>;  // tx: row only
    fn update(&self, id: Uuid, body: &str) -> Result<()>;             // text edits only (pin/color have own ops)
    fn set_pinned(&self, id: Uuid, pinned: bool) -> Result<()>;
    fn set_color(&self, id: Uuid, color_idx: i64) -> Result<()>;
    fn list(&self) -> Result<Vec<ScratchPad>>;                        // live: pinned first, then updated_at desc
    fn trash(&self, id: Uuid) -> Result<()>;                          // soft
    fn restore(&self, id: Uuid) -> Result<()>;
    fn delete(&self, id: Uuid) -> Result<()>;                         // hard + link purge, same tx
    fn purge_expired(&self) -> Result<usize>;                         // 30-day rule, startup (main.rs)
    fn search(&self, needle: &str, limit: usize) -> Result<Vec<(Uuid, String)>>; // id + body prefix (link picker)
    fn get(&self, id: Uuid) -> Result<ScratchPad>;
}
```

`model.rs`: `ScratchPad { id, body, pinned, color_idx, trashed_at,
created_at, updated_at }`; `ScratchPadInput { body, pinned, color_idx }`.
Validation (in `validate_input`): `color_idx` in 0..=5 (same rule as
calendar). Body may be empty (intent risk 4 — creation with empty body is
allowed by the STORE; the UI simply doesn't create until text exists).

**Link identity**: pads are `EntityRef::new(EntityType::Note, id)`. The
todo picker links todo → pad with `Relation::MENTIONS` (same relation and
direction as email entries — a pad "mentions"/is referenced by the todo,
read it either way; consistency with the existing picker code matters more
than relation philosophy). The email reading pane resolves incoming
`Note`-kind links and displays `body.lines().next()` as the label (first
line as title, todo-card two-line echo).

## 5. Autosave (spec § intent locked decision 3)

Mechanism, pinned down:

- The unfolded editor holds a local `String` buffer. Every keystroke marks
  `save_pending = true` and records `last_edit: Instant`.
- In `ScratchUi::show` (runs every frame), if `save_pending` and
  `last_edit.elapsed() >= 500ms` → `store.update(id, &buffer)`,
  `save_pending = false`.
- Flush points (immediate write, no debounce): the card folds (click
  elsewhere / Esc), the module switches away (`show()` detects the
  previously-open editor is gone), app exit is covered by fold-on-Esc in
  practice — there is no other writer.
- **Test seam**: `ScratchUi` exposes `pub fn flush_pending(&mut self,
  db: &Db)` performing the write if `save_pending`. kittest calls it
  instead of sleeping (intent risk 2). The debounce interval is a
  `const AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(500)`.

Rationale recap: capture-first UX, single field, bounded write rate, no
Save button anywhere in the module.

## 6. UI (design language per M1.5/§8)

- Nav: `Module::Scratchpad` enabled in `src/app.rs` like Email/Calendar
  (last disabled item — after M3.5 every nav item is live and
  `placeholder::disabled_nav_item` keeps only theoretical future modules).
  Label stays "Scratchpad" (locked decision 5).
- One screen, todo's view idiom: `View::Pads` (default) / `View::Trash`,
  segmented toggle top-right next to the primary "New pad" button.
- Pads view: vertical scroll of cards (`ui::card_frame`, two-line idiom:
  first line of body as title — `SIZE_CARD_TITLE` strong; second line =
  relative `updated_at` + color dot via `icons::group_dot`, painter-drawn,
  `GROUP_DOT_COLORS[color_idx]`). Pinned cards get a small `Icon::Flag`
  painter chip before the title (no font glyph).
- Hover-reveal icon buttons (existing `ui::icon_button` slide-in-label
  widget): Pin/Unpin (`Icon::Flag`), Color cycle (cycles 0..=5 on click —
  cheaper than a picker and matches the 6-dot cycle language), Trash
  (`Icon::Cross`).
- Click card body → unfold in place (todo idiom): the card grows a
  borderless multiline `TextEdit` (autosave per §5) and the icon row.
  Clicking another card or pressing Esc folds (flush).
- Ctrl+N → new pad: creates the row immediately (so the editor has an id)
  with a placeholder body convention — NO (intent risk 4: no blank-card
  spam). Instead Ctrl+N unfolds a draft card at the top with the editor
  focused; the row is created on first flush (first debounce tick after
  the first keystroke). If the draft is folded with an empty buffer, it
  silently discards. This is the one place the store's allow-empty rule is
  not exercised.
- Trash view: trashed cards (same rendering, dimmed) with Restore and
  Delete-forever icon buttons. Delete-forever uses the confirm-modal
  pattern from todo (`egui::Window` + "Delete permanently?" + primary
  danger).
- Empty states: `ui::empty_state("No pads yet", "Press Ctrl+N or the New
  pad button — typing saves automatically.")` for Pads; a trash-specific
  message for Trash.
- All colors from palette + the existing 6 group-dot constants; zero new
  color literals; painter icons only.

## 7. Link graph integration

- **Todo picker** (`src/todo/ui.rs`, the "Blocked by…" picker): after the
  calendar-event entries, add scratch entries — `search` hits
  `ScratchStore::search`, rows render "Pad — {first line}", click links
  todo → pad (`MENTIONS`). Same shape as the M3 calendar entries.
- **Email reading pane** (`src/email/ui/reading.rs`): `linked_todos` gains
  a linked-pads read-only section ("Linked pads", first-line labels) —
  same as the M3 "Linked events" section.
- **Calendar UI**: no link picker exists there in M3 — pads are linkable
  at the `LinkStore` API level (`EntityType::Note`) and M4+/later pickers
  pick them up for free. No M3.5 picker work.
- Hard delete purges links both directions (`LinkStore::delete_links_for`,
  same transaction — entity.rs Note variant already maps to `'note'`).

## 8. Edge cases

1. **First line as title**: body starting with blank lines → title falls
   back to the first non-empty line; all-empty body renders "(empty pad)".
2. **Huge bodies**: TEXT column, no limit; the multiline editor scrolls.
   No perf work in M3.5 (cards render first line only).
3. **Autosave conflict with pin/color clicks**: those use store ops that
   don't touch `body`; the pending buffer write is a plain `UPDATE body`
   — no lost-update path (single writer, single field).
4. **Trash with pending edits**: folding flushes BEFORE trash can run
   (trash is only reachable from the folded card's hover row) — ordering
   enforced by the UI structure, not by the store.
5. **Purge and links**: `purge_expired` purges links in the same tx per
   row, exactly like todos/calendar.
6. **Two-line idiom with `\n` in first line**: title is
   `body.lines().next()` truncated by the card width — no special-casing.

## 9. Security

- No secrets anywhere (no accounts, no network). notify-rust not involved.
- DB perms already 0600/0700. `quality_reports/` untouched, never
  committed.

## 10. Test strategy

- `tests/scratch_store.rs` — in-memory DB: CRUD; `list` ordering (pinned
  first, `updated_at` desc within groups); color_idx validation
  (0..=5, rejects 6 and -1); trash/restore; hard delete purges links
  (link a pad ↔ todo, delete pad, link row gone); `purge_expired` with a
  backdated `trashed_at` (mirrors the calendar purge test); `search`
  matches body substring.
- `tests/scratch_ui.rs` — kittest (pattern: `tests/calendar_ui.rs`):
  Ctrl+N (or button) → type → `flush_pending` → pad appears in list;
  reload harness → body persisted (autosave round-trip); pin click → pad
  jumps to top; trash → moves to Trash view → restore → back; todo picker
  shows pad entries and linking works (LinkStore assertion); empty state
  on fresh DB.
- `tests/db_schema.rs` — the three `user_version` assertions move 3 → 4
  (same maintenance as 0003 did); add `scratch_notes` + trigger coverage
  to the trigger test if it enumerates tables.
- Debounce timing is never slept on: `flush_pending` is the seam.

## 11. Risks

1. **Draft-then-create flow** (Ctrl+N without a row yet) is the only
   nontrivial UI state; mitigated by keeping the draft strictly
   single-instance (`draft: Option<DraftPad>`) and covered by kittest.
2. **`updated_at` ordering vs autosave**: every debounced write bumps
   `updated_at` via trigger → the pad reorders to top on edit. That IS
   the desired "recently touched floats up" behavior; pinned still wins.
3. **user_version bump breaks db_schema tests on purpose** — plan Step 1
   includes the 3 → 4 maintenance, exactly like M3's 2 → 3.
