# Handoff — Adjutant build state

> Living doc for resuming in a fresh session. Read this + AGENTS.md + the
> `.kimi/intents/` triplet for the phase you're entering. Last updated
> 2026-09-30, end of M2 (email).

## Where we are

- **M1 Foundation + Todo — DONE** (branch `m1-foundation`, tip `7db38d2`):
  scaffold, SQLite schema + migrations, entity-link graph, todo store
  (nesting, due dates, trash/restore/30d purge, completion rules), egui UI.
- **M1.5/M1.6 design system — DONE** (same branch): 5-theme system (Light
  HEY-inspired default `#F4F1EC`+white cards+violet accent, Sepia, Slate,
  Dark, Omarchy-live), theme picker persisted, collapsible sidebar, focus
  mode (`Ctrl+.`), todo cards w/ inline expansion + painter-icon badges,
  hover-reveal icon buttons, pointer-cursor contract. Specs:
  `.kimi/intents/todo-cards-redesign-spec.md` (§8 = HEY pass).
- **M2 Email — DONE, field-tested by Jimmy** (branch `m2-email`, tip
  `5e2b562`, pushed): Purelymail IMAP/SMTP, sync engine (incremental UID,
  UIDVALIDITY, offline write queue), JWZ threading, merged inbox +
  per-account switcher, two-line thread cards, reading pane, compose →
  staged outbox → Approve(-all) → SMTP → Sent. Keys in gnome-keyring only.
  Jimmy's account synced successfully 2026-09-30.
- **Branches NOT merged to main.** `m2-email` is stacked on `m1-foundation`.
  Merge/PR strategy is an open user decision (see Open questions).
- **Next phase: M3 Calendar** (per intent milestone map: local event store
  source-of-truth, Google two-way sync later in M5; reminders link to
  todos/emails; last-write-wins conflicts). No M3 triplet exists yet —
  Stage 1 intent capture is the first task.

## Verify on entry

```sh
cd /home/jimmy/Projects/adjutant
export PATH="$HOME/.cargo/bin:$PATH"
export https_proxy=http://127.0.0.1:7897 http_proxy=http://127.0.0.1:7897
git checkout m2-email && git status --porcelain   # expect empty
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test        # expect 91 passed across 13 suites
cargo build --release
```

## Decisions taken (not otherwise recorded in code)

- **HTML mail = stripped plain text** (Jimmy, 2026-09-30). Raw HTML kept in
  `emails.body_html` for a future subset renderer (M3.5 candidate). Never a
  webview/web runtime — architectural red line.
- **Gmail deferred to M5** (OAuth2); M2 is Purelymail-only but account model
  is generic IMAP/SMTP.
- **Calendar sync conflicts: last-write-wins** (locked 2026-09-29).
- **Approval model: per-item + batch "Approve all"** (locked 2026-09-29) —
  outbox `origin` column ('ui'/'agent') pre-built for M4 MCP drafts.
- **Email scope: headers all folders, bodies on open, 90-day cache** (locked).
- **Distribution: personal-only.** Repo is PUBLIC (github.com/vincerhodes/
  adjutant) — Jimmy approved 2026-09-29. No secrets ever committed; DB and
  quality_reports/ gitignored.
- **Rust via rustup** (`~/.cargo/bin`), NOT mise — mise's rust plugin fails
  behind the GFW. cargo needs the Clash proxy env above.
- **Badges are allowed** (M1.5 §8 supersedes the M1 no-badges rule);
  painter-drawn icons only, never font glyphs (tofu lesson).

## Repo map

- `planning/HANDOFF.md` — this file.
- `AGENTS.md` — build commands, module layout, hard rules (zero SQL in UI,
  clippy -D warnings, migrations immutable, no live network in tests,
  keyring-only passwords).
- `.kimi/intents/` — SDLC triplets per milestone: `adjutant-m1-*` (3 files),
  `todo-cards-redesign-spec.md` (M1.5/M1.6), `adjutant-m2-email-*` (3 files).
  Spec/plan are normative for implementation.
- `src/db/` — Db handle, `migrate.rs` (MIGRATIONS list, user_version=2),
  settings. `src/core/` — entity types, link graph, keyring helper.
- `src/todo/` — todo store + card UI. `src/email/` — store, imap_client.rs /
  smtp.rs (live transports behind `ImapTransport`/`SmtpTransport` traits =
  test seam), sync.rs (engine thread), thread.rs (JWZ-lite), compose.rs,
  ui/ (list, reading, compose_ui, outbox, accounts).
- `src/ui/` — theme.rs (Palette, 5 themes, Omarchy watcher), fonts.rs,
  icons.rs (painter icons + group-dot cycle), mod.rs (ghost/primary buttons,
  icon_button hover-reveal, card_frame, hand()/selectable() cursor helpers),
  help.rs (F1 overlay), placeholder.rs.
- `migrations/` — 0001_init.sql, 0002_email.sql (never edit; add new files).
- `tests/` — one suite per area; 91 tests total. Mocks for IMAP/SMTP live in
  tests; no live network.

## Key contracts for the next phase (M3 Calendar)

- New module = `src/calendar/{mod,model}.rs` + `src/calendar/ui/` +
  `migrations/0003_calendar.sql`. Dependency direction UI → module → core/db.
  Zero SQL in UI files. Migration registered in `src/db/migrate.rs`.
- Link layer: `src/core/link.rs` `LinkStore` — events/reminders are already
  valid `EntityType`s; links to todos/emails work today.
- Reminders must link to todos/emails per M1 intent; local SQLite event
  store is source of truth (NOT a Google client); Google sync = M5.
- Timestamps: ISO-8601 UTC text; `updated_at` via SQL triggers (sync
  conflict resolution depends on them — last-write-wins in M5).
- Theme/icons/card language: reuse `ui::card_frame`, `icons.rs`,
  palette semantics — no new color literals.
- Timezones: decide in M3 intent (chrono only today; events need local-tz
  handling — chrono-tz likely needed, decide at intent stage).

## M3 entry checklist

1. SDLC Stage 1: intent capture for calendar — ask Jimmy: event fields
   (all-day? recurrence? attendees?), reminder delivery (in-app vs desktop
   notification via notify-rust?), default view (week/list?), local-only
   until M5 confirmed.
2. Write `.kimi/intents/adjutant-m3-calendar-{intent,spec,plan}.md`.
3. Branch `m3-calendar` off `m2-email` tip (stacked) — or off main if
   merges happen first (Jimmy's call, see below).

## Open questions / not done

- **Merge strategy**: m1-foundation + m2-email unmerged; PRs need Jimmy's
  explicit approval. Ask at M3 kickoff whether to PR/merge first.
- **Visual polish backlog**: Jimmy parks further visual work ("I'll work on
  the rest of visual design later") — capture items as they arise; one
  consolidation pass before M4.
- HTML subset renderer (M3.5 candidate) — raw HTML stored, not rendered.
- IMAP IDLE/push, FTS search, contact autocomplete — deferred, no milestone.
- Secret zeroization (passwords held as String on sync thread) — hardening
  note, acceptable for personal v1.
- `quality_reports/` — local kimi hook artifact, gitignored; never commit.
