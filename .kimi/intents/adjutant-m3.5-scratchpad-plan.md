# Adjutant — Execution Plan (M3.5: Scratch Pad)

Normative: `.kimi/intents/adjutant-m3.5-scratchpad-spec.md` (read first, it
wins conflicts). Repo `/home/jimmy/Projects/adjutant`, branch `m3-calendar`
(already checked out — do NOT create or switch branches, never touch main;
M3.5 ships on the M3 branch as a fast-follow). ENV:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export https_proxy=http://127.0.0.1:7897 http_proxy=http://127.0.0.1:7897
```

Never launch the GUI from an agent session (Jimmy does the manual UI
pass). No push, no PR.

## Risk interrogation additions (beyond spec §11)

4. **db_schema test churn** — `tests/db_schema.rs` pins `user_version == 3`
in three places; 0004 bumps it to 4. Step 1 updates those assertions
(maintenance, same as M3's 2 → 3 — never edit the migration to fit a test).
5. **placeholder module removal** — after M3.5 every `Module` variant is
live; `src/ui/placeholder.rs` keeps `placeholder_screen` (still used for
theoretical future modules) but `disabled_nav_item` may go unused. If
clippy flags it, gate it with `#[allow(dead_code)]` and a note — do NOT
delete the shared widget.
6. **Scope guard**: NO markdown, NO search UI, NO pad-to-pad links UI, NO
sync, NO filter field. Resist gold-plating — this module is quick capture.

## Step 1 — schema + model + store + purge wiring

1. `migrations/0004_scratchpad.sql` verbatim from spec §3; register
   `("0004_scratchpad", include_str!("../../migrations/0004_scratchpad.sql"))`
   in `src/db/migrate.rs` MIGRATIONS (never touch 0001-0003; user_version
   becomes 4).
2. `src/scratch/model.rs` — `ScratchPad`, `ScratchPadInput` (spec §4).
3. `src/scratch/mod.rs` — `ScratchStore` per spec §4, typed
   `ScratchError` (mirror `CalendarError`'s shape: Db/Sqlite/NotFound/
   InvalidInput/Corrupt), `purge_expired` copying the calendar pattern
   (`PURGE_AGE = 30 days`, link purge in-tx).
4. `src/main.rs` — third startup purge after todos + calendar:
   `ScratchStore::new(&db).purge_expired()` with the same match/println
   shape; add the import.
5. Register `pub mod scratch;` in `src/lib.rs` (alphabetical, matching
   existing modules).
6. `tests/scratch_store.rs` per spec §10 (write FIRST for the ordering
   and purge cases, watch them fail, then implement — TDD only where it
   bites: ordering + purge + link purge).
7. `tests/db_schema.rs` — `user_version` 3 → 4 in the three assertions;
   extend the trigger test with `trg_scratch_notes_updated`.

Commit: `feat(scratch): schema and store`

**Acceptance (shell-testable):**
- `cargo test` green incl. new `scratch_store` suite; fmt + clippy clean.
- `cargo test scratch_` passes AND (scratch check) on a temp DB:
  `sqlite3 "$TMP/adjutant.db" "SELECT sql FROM sqlite_master WHERE name='trg_scratch_notes_updated'" | grep -q trg_scratch_notes_updated`
- `cargo test db_schema` shows user_version assertions at 4.

## Step 2 — UI + links + kittest

1. `src/scratch/ui/mod.rs` — `ScratchUi` (pattern: `src/todo/ui.rs` +
  `src/calendar/ui/mod.rs`): `View::Pads/Trash`, `pads` cache + `dirty`
  reload, `draft: Option<DraftPad>` single-instance Ctrl+N flow, autosave
  debounce per spec §5 (`AUTOSAVE_DEBOUNCE = 500ms`, `flush_pending` test
  seam), `is_busy()` for the app.rs Esc guard, `handle_keys` (Ctrl+N,
  Esc folds+flushes).
2. `src/scratch/ui/pad_card.rs` — card + unfolded editor per spec §6:
   two-line card (first-line title, relative `updated_at`, color dot),
   hover-reveal Pin/Color/Trash icon buttons, in-place unfold editor,
   pinned flag chip (`Icon::Flag`, painter). Zero SQL — store methods only.
3. `src/app.rs` — enable `Module::Scratchpad` (swap
   `disabled_nav_item` for the real `selectable` like the other modules),
   construct `ScratchUi`, route, extend the Esc guard with
   `!self.scratch.is_busy()`.
4. Todo picker (`src/todo/ui.rs`): scratch entries after calendar entries
   — `ScratchStore::search`, "Pad — {first line}" rows, link todo → pad
   `MENTIONS` (spec §7).
5. Email reading pane (`src/email/ui/reading.rs`): "Linked pads"
   read-only section (first-line labels).
6. `tests/scratch_ui.rs` per spec §10 (pattern: `tests/calendar_ui.rs` —
   temp-db fixture + `build_eframe` + `new_with_engine(db, cc, None)`).

Commit: `feat(scratch): scratch pad UI and links`

**Acceptance:**
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test` (incl. `scratch_ui` kittest), `cargo build --release` green.
- `grep -rn "unwrap()\|expect(" src/scratch --include=*.rs` → empty.
- `grep -rEl "SELECT |INSERT |UPDATE |DELETE " src/scratch/ui/` → empty.
- kittest drives: create+autosave round-trip (via `flush_pending`, no
  sleeps), pin reorder, trash/restore, todo-picker pad entry, empty state.

## Final gate set

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
grep -rn "unwrap()\|expect(" src/scratch --include=*.rs   # must be empty
```

Manual (Jimmy, from intent acceptance): Ctrl+N → type → autosaved; pin on
top; trash + restore; link a pad to a todo; restart → intact.

## Commit/branch discipline

Branch `m3-calendar`, one commit per step as named above. No push, no PR.
No Rust/SQL written as part of this triplet — implementation starts at
Step 1 after Jimmy (or the orchestrator) confirms.
