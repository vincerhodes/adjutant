# Adjutant — Intent (M3.5: Scratch Pad)

Captured: 2026-10-01 (agent-written — Jimmy asleep; decisions are provisional
until he confirms, recorded as "locked (agent, 2026-10-01)" where the spec/plan
depend on them). Builds on `adjutant-m1-intent.md`, `adjutant-m2-email-intent.md`,
and `adjutant-m3-calendar-intent.md`. Slots between M3 (calendar) and M4 —
numbering M3.5; the branch stays `m3-calendar`.

## Scope

Scratch pad module: the fastest possible local text capture. A single flat
list of plain-text pads — click, type, done. This is a quick-capture module,
NOT a notes app: no markdown rendering, no folders, no tagging beyond the
shared color dot, no sync, no sharing, no reminders.

## Locked decisions (2026-10-01)

1. **Plain text only.** `body` is a TEXT column, rendered as-is in a
   multiline editor. No markdown rendering in M3.5 (a renderer is a later
   milestone if ever — pads store raw text so one can be added without a
   migration).
2. **Reuse `EntityType::Note`** for the link graph — the variant already
   exists in `src/core/entity.rs` and the `entity_links` CHECK constraint in
   `migrations/0001_init.sql` already allows `'note'`. **No core change, no
   link-layer migration.** Scratch pads ARE notes as far as the link graph
   is concerned. If Jimmy later wants a distinct `scratch` type that renders
   differently from a future Notes module, that's a small migration then.
3. **Autosave: debounced (500ms idle), flushed on editor close.** Justified:
   the module exists to remove friction — a Save button or a
   remember-to-commit-on-blur model contradicts "click, type, done". A
   500ms idle debounce bounds DB writes; closing/unfolding-away flushes
   pending text immediately so nothing is lost. Pads are single-field, so
   partial-state risk is nil.
4. **One screen, no view switcher.** Pads list (pinned first, then
   `updated_at` desc) + a Trash section toggle (todo's `View::Tree`/`Trash`
   idiom). No dashboard/week-style switching — there is nothing to switch.
5. **Nav label stays "Scratchpad"** — the `Module::Scratchpad` variant and
   its label already exist in `src/app.rs`; renaming the label for fashion
   churns the kittest queries for zero user value. Module directory is
   `src/scratch/` per this intent.
6. **Color tag: the shared 6-color `GROUP_DOT_COLORS` cycle** (same
   `color_idx` 0..=5 rule as calendar events — one validation rule reused,
   zero new color literals).
7. **Local-only.** No sync, no MCP surface, no reminders, no notifications.
   Schema stays M5-agnostic on purpose: `updated_at` trigger maintained
   anyway (house rule — every table gets one).

## M3.5 feature list

- Pad CRUD: body text, pinned flag, color tag; create via "New pad" button
  or Ctrl+N (todo's existing pattern), edit in place by clicking the card
  (unfold idiom), autosave per locked decision 3.
- Ordering: pinned pads first (by `updated_at` desc within the pinned
  group), then unpinned by `updated_at` desc. New pads are pinned=false.
- Trash: soft delete via hover-reveal Trash icon, restore from the Trash
  view, 30-day purge at startup wired in `src/main.rs` (third purge call,
  after todos and calendar).
- Link graph: pads are linkable notes. Todo link picker gains pad entries
  (`mentions` relation, same as email entries); the email reading pane
  displays linked pads read-only (same as linked events). The calendar UI
  has no link picker in M3 — pads are linkable to events at the
  `LinkStore` API level; no new picker UI is built for M3.5.
- Empty state: designed `ui::empty_state` ("No pads yet" + hint).

## Non-goals (M3.5)

Markdown rendering (plain text capture; raw text survives any future
renderer). Folders/groups/nesting. Search (the filter pattern exists in
todo/email and can be grafted later — spec notes the seam, doesn't build
it). Sync/sharing. Reminders/notifications. Rich text/attachments.
Renaming the nav item. A distinct `EntityType::Scratch`.

## Open risks (spec-level answers)

1. **Debounced autosave vs egui's per-frame model** — spec pins the
   mechanism (time-gated write in `update`, not a timer thread) and the
   flush points (editor close, module switch is covered by close).
2. **kittest and debounce timing** — tests must not sleep 500ms; spec pins
   a test seam (public `flush()` on the UI state or immediate write when
   the harness sets a flag — decided in spec §6).
3. **Link pickers showing pads to todos AND pads linking out** — spec pins
   direction: todo/email link TO pads (incoming); pads themselves get no
   outgoing picker in M3.5 (read-only display only).
4. **Empty body pads** — spec decision: allow saving an empty body (a pad
   is a capture surface; an accidentally-empty pad is cheap to trash) but
   do not create pads via Ctrl+N until the user types (no blank-card spam).

## Acceptance (M3.5, shell-testable + manual)

- Full gates green: `cargo fmt --check`, `cargo clippy --all-targets --
  -D warnings`, `cargo test` (new `scratch_store` + `scratch_ui` suites),
  `cargo build --release`.
- `grep -rn "unwrap()\|expect(" src/scratch --include=*.rs` returns nothing.
- Zero SQL in `src/scratch/ui/` (`grep -rEl "SELECT |INSERT |UPDATE |
  DELETE " src/scratch/ui/` empty).
- `sqlite3` on a fresh DB shows `scratch_notes` with
  `trg_scratch_notes_updated` and `PRAGMA user_version` = 4.
- Manual (Jimmy): Ctrl+N → type → pad appears autosaved; pin holds it on
  top; trash + restore; link a pad to a todo from the todo picker; restart
  → pads intact.
