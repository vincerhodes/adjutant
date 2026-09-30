# Handoff — Adjutant build state

> Living doc for resuming in a fresh session. Read this + AGENTS.md + the
> `.kimi/intents/` triplet for the phase you're entering. Last updated
> 2026-10-01, end of the overnight M3 (calendar) + M3.5 (scratch pad) session.

## Where we are

- **M1/M1.5 + M2 Email — MERGED to main** via PRs #1/#2 (2026-09-30; main
  tip `961dd67`). See the previous handoff commits for their scope; both
  field-tested by Jimmy (Purelymail account synced 2026-09-30).
- **M3 Calendar — DONE** on branch `m3-calendar` (7 commits: notifier seam
  `ccd33fe` → schema/rrule/store `6aa31e6` → scheduler `0bbc131` → 3 UI
  commits `bd245f3`/`1d03e1e`/`fdadcd9` → docs `127fbac`). Executed exactly
  per `.kimi/intents/adjutant-m3-calendar-plan.md` (5 steps). 146 tests at
  the M3 tip. Module: events (timed/all-day, chrono-tz), hand-rolled RRULE
  subset expansion, reminder scheduler (tick + 24h catch-up + snooze,
  notify-rust desktop notifications + in-app banner), Dashboard/Week/
  Upcoming views, event form with recurring-edit chooser, todo/email link
  integration.
- **M3.5 Scratch Pad — DONE** on the same branch (triplet `536a30e` +
  `f9e184b` store + `cf752b2` UI). 158 tests total at branch tip. Module:
  plain-text capture pads, 500ms-debounce autosave, pin/color/trash,
  todo-picker + email-pane link integration via the pre-existing
  `EntityType::Note` (no core change).
- **Branch NOT merged.** `m3-calendar` is stacked on main `961dd67`. PR
  needs Jimmy's explicit approval (see Open questions).

## Verify on entry

```sh
cd /home/jimmy/Projects/adjutant
export PATH="$HOME/.cargo/bin:$PATH"
export https_proxy=http://127.0.0.1:7897 http_proxy=http://127.0.0.1:7897
git checkout m3-calendar && git status --porcelain   # expect empty
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test        # expect 158 passed across the suites
cargo build --release
```

## Decisions taken (not otherwise recorded in code)

Carried forward from M1/M2 (still in force):

- **HTML mail = stripped plain text** (Jimmy, 2026-09-30). Raw HTML kept in
  `emails.body_html` for a future subset renderer. Never a webview/web
  runtime — architectural red line.
- **Gmail deferred to M5** (OAuth2); account model is generic IMAP/SMTP.
- **Calendar sync conflicts: last-write-wins** (locked 2026-09-29) —
  every calendar + scratch table has `updated_at` + SQL trigger.
- **Approval model: per-item + batch "Approve all"** — outbox `origin`
  column ('ui'/'agent') pre-built for M4 MCP drafts.
- **Email scope: headers all folders, bodies on open, 90-day cache**.
- **Distribution: personal-only.** Repo is PUBLIC (github.com/vincerhodes/
  adjutant). No secrets ever committed; DB and quality_reports/ gitignored.
- **Rust via rustup** (`~/.cargo/bin`), NOT mise — mise's rust plugin fails
  behind the GFW. cargo needs the Clash proxy env above.
- **Badges are allowed** (M1.5 §8); painter-drawn icons only, never font
  glyphs (tofu lesson).

M3/M3.5 additions:

- **Recurrence: expand-on-read, no materialized occurrence table** (spec
  §5) — store RRULE text, expand in Rust per query window, 10,000-eval
  cap → typed error, never a hang.
- **Timezone shape: UTC instants + IANA name per event; expansion in the
  event's TZ wall-clock, then converted to UTC** (DST rule). chrono-tz
  pinned at 0.10 against chrono 0.4 (no bump needed). chrono-tz resolves
  autumn overlaps to the LATER offset — `rrule::local_to_utc` walks back
  ≤3h to take the first occurrence; spring gaps shift forward.
- **Reminder ledger exactly-once**: `calendar_reminder_fires` is the only
  write path (`upsert_fire` idempotent, INSERT DO NOTHING); firing sets
  `fired_at`; snooze RE-OPENS the row (`fired_at = NULL` +
  `snoozed_to_utc`) and re-fires in-app only — the desktop notification
  fires once per occurrence, at first fire. Catch-up on launch bounded to
  24h. See the AGENTS.md hard rule.
- **notify-rust behind the `Notifier` trait** (test seam; mock + call-log
  asserts in tests). Daemon errors swallowed to a log line — never panic.
- **Timed events can't span midnight in the form** (end ≤ start →
  validation error) — known limitation, pending Jimmy's call.
- **Dashboard = upcoming-5 strip over week grid**; switcher
  Dashboard/Week/Upcoming; all-day band, now-line, past-dimmed.
- **Scratch pads: plain text, autosave 500ms idle debounce** (flushed on
  fold/Esc), Ctrl+N drafts and only creates the row on first keystroke;
  pinned-first + recency ordering; color = 6-dot cycle via one
  cycle-on-click button (no picker — revisit if Jimmy wants one).
- **`EntityType::Note` reused for scratch pads** — variant + links CHECK
  pre-existed; zero core change. A distinct `scratch` type deferred.
- **30-day purge covers todos + events + pads** (three startup calls in
  `src/main.rs`).

## Repo map

- `planning/HANDOFF.md` — this file.
- `AGENTS.md` — build commands, module layout (now incl. calendar +
  scratch), hard rules (zero SQL in UI, clippy -D warnings, migrations
  immutable, no live network in tests, keyring-only passwords, Notifier
  seam, reminder-ledger exactly-once).
- `.kimi/intents/` — SDLC triplets per milestone: `adjutant-m1-*` (3),
  `todo-cards-redesign-spec.md` (M1.5/M1.6), `adjutant-m2-email-*` (3),
  `adjutant-m3-calendar-*` (3), `adjutant-m3.5-scratchpad-*` (3).
  Spec/plan are normative for implementation.
- `src/db/` — Db handle, `migrate.rs` (MIGRATIONS list, user_version=4),
  settings. `src/core/` — entity types (incl. `Note`), link graph,
  keyring helper.
- `src/todo/`, `src/email/` — unchanged from M2 (see previous handoff).
- `src/calendar/` — mod.rs (CalendarStore, ALL SQL), model.rs, rrule.rs
  (pure expansion + local_to_utc + prev_in_wall_clock), reminders.rs
  (scheduler, injected now + `&mut dyn Notifier`), notify.rs (Notifier
  seam + DesktopNotifier), ui/ (mod/dashboard/week/upcoming/event_form/
  reminder_banner).
- `src/scratch/` — mod.rs (ScratchStore, ALL SQL), model.rs, ui/
  (mod.rs state + autosave debounce, pad_card.rs cards/editor).
- `src/ui/` — shared widgets (unchanged M1.5 set; help.rs F1 overlay now
  includes calendar shortcuts).
- `src/app.rs` — all four modules live (Todo/Email/Calendar/Scratchpad);
  reminder scheduler tick wired in `logic()` (~15s + catch-up on launch).
- `migrations/` — 0001_init.sql … 0004_scratchpad.sql (never edit; add
  new files; user_version = 4).
- `tests/` — one suite per area, **158 tests total**: calendar_rrule (24),
  calendar_store (16), calendar_reminders (6), calendar_ui (8),
  scratch_store (6), scratch_ui (6) + the pre-existing suites. Mocks for
  IMAP/SMTP/Notifier live in tests; no live network, no dbus.

## Key contracts for the next phase

Per the intent milestone map, the next module is one of:

- **M4 — MCP/agent drafts**: outbox `origin` column pre-built ('ui'/
  'agent'); approval model (per-item + Approve-all) is the gate. Store/
  schema need no calendar/scratch changes.
- **M5 — Google sync (calendar first)**: last-write-wins conflicts read
  the `updated_at` triggers (calendar + scratch tables all have them);
  recurrence is expand-on-read so only base rows + exception rows sync;
  Gmail is the OAuth2 piece.

Either way the sibling pattern is proven twice now: `src/<module>/`
(mod.rs + model.rs [+ submodules] + ui/) + `migrations/NNNN_<name>.sql` +
triplet in `.kimi/intents/` + store/UI test suites. Dependency direction
UI → module → core/db; zero SQL in UI; `EntityType` for links (add
variant + CHECK migration only if none fits).

## Open questions / not done

- **Merge strategy**: `m3-calendar` (M3 + M3.5, 9 commits) is unmerged,
  stacked on main `961dd67`. PR needs Jimmy's explicit approval. Ask at
  next kickoff.
- **Jimmy's manual UI pass — PENDING for both new modules** (automated
  gates are green; kittest is blind to painter-drawn week content):
  - Calendar (from M3 plan §Acceptance): create timed + all-day multi-day
    events → dashboard + week correct across a TZ/DST change; weekly
    recurrence across the 2026-03-29 DST boundary; this-occurrence vs
    series edits; **real desktop-notification smoke test with a running
    daemon** + in-app banner; close/reopen past due time → catch-up
    exactly once; Dismiss/Snooze hold; link reminder ↔ todo and ↔ email;
    trash + restore; restart → intact.
  - Also flagged by the implementer: week-grid density/legibility at
    normal window size; TZ dropdown feel (~600 IANA entries);
    Snooze-as-dropdown UX; banner placement above the switcher.
  - Scratch: autosave feel (½s pause, Esc folds+flushes); pin ordering;
    color-cycle click-through vs a picker; icon metaphors (Flag=pin,
    CircleHalf=color, Cross=trash, Back=restore); **mixed todo-picker
    result ordering** (todos + emails + events + pads in one list).
- **HTML subset renderer** — raw HTML stored, not rendered; parked.
- **Visual polish backlog** — Jimmy parks visual work; one consolidation
  pass before M4.
- IMAP IDLE/push, FTS search, contact autocomplete — deferred.
- Secret zeroization (passwords held as String on the sync thread) —
  hardening note, acceptable for personal v1.
- `quality_reports/` — local hook artifact, gitignored; never commit.

## Jimmy's decisions pending

1. **Merge/PR `m3-calendar`** (M3 + M3.5) — approve PR or hold.
2. **`scratch` EntityType**: keep reusing `Note`, or mint a distinct
   `scratch` type (small migration: entity.rs variant + links CHECK) when
   a real Notes module might collide?
3. **Cross-midnight timed events**: needed, or is the form's same-day
   restriction fine?
