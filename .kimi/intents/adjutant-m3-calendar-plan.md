# Adjutant — Execution Plan (M3: Calendar)

Normative: `.kimi/intents/adjutant-m3-calendar-spec.md` (read first, it wins conflicts). Repo `/home/jimmy/Projects/adjutant`, branch `m3-calendar` already cut from `main` tip `961dd67` (M1/M2 merged to main 2026-09-30 via PRs #1/#2). ENV: `export PATH="$HOME/.cargo/bin:$PATH" https_proxy=http://127.0.0.1:7897 http_proxy=http://127.0.0.1:7897`. Never launch GUI from agent sessions (Jimmy does the manual UI pass).

## Risk interrogation additions (beyond spec §8)

8. **chrono-tz version pairing** — chrono-tz 0.10 targets chrono 0.4.x; verify `cargo add chrono-tz@0.10` resolves against the locked chrono without bumping it. If it forces a chrono upgrade, pin the compatible pair in Cargo.toml and note it.
9. **notify-rust on Omarchy/Hyprland** — requires a session dbus + running notification daemon (Omarchy ships one). Failure mode must be in-app-only, never panic: wrap `notify_rust::Notification::show()` errors in a log line. Verify provider presence now: `busctl --user list | grep -i notification` (informational only — code must not depend on it).
10. **egui date/time picker widgets** — egui_extras has no calendar widget; datetime entry is hand-rolled (date = 3 drag-values or text fields, time = HH:MM text). Keep the form dumb and explicit; do NOT add a date-picker crate.
11. **Scope guard**: NO Google sync, NO invites/iTIP, NO free/busy, NO MCP tools, NO month view unless it drops in cleanly in Step 4. Recurrence expansion is the ONLY algorithmic surface — keep it in `rrule.rs` as pure functions.
12. **RRULE hand-roll risk** — the subset is small (spec §5) but BYDAY/BYMONTHDAY interaction needs a decision: when both present, intersect (per RFC 5545); document in rrule.rs. Test file `calendar_rrule.rs` is the safety net — write it before the store consumes expansion.
13. **Jimmy manual steps** — notify daemon presence + real desktop-notification smoke test are Jimmy's; agent asserts via mock only.

## Step 1 — branch + deps + spike

```bash
cd /home/jimmy/Projects/adjutant && git checkout m3-calendar
cargo add chrono-tz@0.10 notify-rust@4
cargo build  # resolve API surprises now (risk 8/9)
```

Write `src/calendar/notify.rs` (`Notifier` trait + notify-rust live impl compiling against the real API, error-swallowing per risk 9). No store yet. Commit `feat(calendar): notifier seam over notify-rust`.

**Acceptance:** `cargo build` green on the new deps; `Notifier` trait mockable without touching dbus.

## Step 2 — schema + model + rrule + store

1. `migrations/0003_calendar.sql` verbatim from spec §3; register `("0003_calendar", include_str!("../../migrations/0003_calendar.sql"))` in `src/db/migrate.rs` MIGRATIONS list (never touch 0001/0002; user_version becomes 3).
2. `src/calendar/model.rs` — Event, EventInput, Recurrence, Attendee, Reminder, DueReminder, enums (`ExceptionKind`, `Rsvp`), `OccurrenceChanges`. Serde JSON for `changes_json` / attendee-free fields only.
3. `src/calendar/rrule.rs` — spec §5: parser (typed error, never panic), `expand(event, window_from, window_to) -> Vec<Occurrence>` in TZ wall-clock → UTC, exceptions applied, iteration cap 10,000, UNTIL/COUNT, DST rule. Pure functions, `chrono-tz` only.
4. `src/calendar/mod.rs` — CalendarStore per spec §4 (SQL here only; zero SQL outside this file).
5. `tests/calendar_rrule.rs` per spec §10 (write FIRST, red before rrule impl exists — treat as TDD for the expansion core).
6. `tests/calendar_store.rs` per spec §10 (in-memory DB).

Commit `feat(calendar): schema, recurrence expansion, and store`.

**Acceptance:** `cargo test` green incl. both new suites; `cargo fmt --check && cargo clippy --all-targets -- -D warnings` clean; `sqlite3` on a scratch DB shows all calendar tables with `updated_at` triggers (shell-check: `cargo test calendar_` passes and `.schema calendar_events` in a temp DB includes `trg_calendar_events_updated`).

## Step 3 — reminders scheduler

1. `src/calendar/reminders.rs` — `ReminderScheduler` with injected `now: DateTime<Utc>` and `&mut dyn Notifier`: tick loop contract (`tick(now)` called from `app.rs` every ~15s + on launch), `catch_up(now, 24h)`, exactly-once ledger semantics, snooze/dismiss (spec §6).
2. Wire scheduler tick + catch-up into `src/app.rs` update path (calendar module only; toast on fire via existing toast system).
3. Unit tests: tick fires due reminders once; snooze re-fires at target; catch-up bounded to 24h and deduped; trashed events never schedule; recurring event fires per occurrence not per reminder row (spec §6). Mock `Notifier` asserts call log — no dbus, no network.

Commit `feat(calendar): reminder scheduler with catch-up and snooze`.

**Acceptance:** `cargo test` green; clippy/fmt clean.

## Step 4 — UI

1. `src/app.rs`: enable `Module::Calendar` (remove placeholder), construct `CalendarUi::new(&db)`, route.
2. `src/calendar/ui/mod.rs` + view switcher (Dashboard / Week / Upcoming, segmented control, palette semantics).
3. `dashboard.rs` per spec §7: upcoming-5 strip (compact cards) over week grid; single scroll column when window short. `week.rs` grid with all-day band + now-line + past-dimmed. `upcoming.rs` list.
4. `event_form.rs` per spec §7 incl. recurrence section, TZ picker (IANA dropdown, system default), attendees, reminders, color tag; validation per spec §8.3; recurring-edit chooser (this / future / series) per spec §5.
5. `reminder_banner.rs` — due banner with Dismiss / Snooze(5/15/60).
6. F1 overlay additions; empty states; link pickers in todo/email detail gain event/reminder entries (read-only link creation via existing `LinkStore`).
7. `tests/calendar_ui.rs` per spec §10 (kittest).

Commits split as `feat(calendar): dashboard and week views`, `feat(calendar): event form and recurrence editing`, `feat(calendar): reminder banner and links`.

**Acceptance:** all gates green incl. new suite; kittest exercises view switcher + banner + form validation.

## Step 5 — gates + docs

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo build --release`. README calendar section; AGENTS.md module-map update (`calendar/` added, Notifier test-seam rule, reminder-ledger rule). Commit `docs: calendar module`.

**Acceptance:** full gate set green; `grep -rn "unwrap\|expect" src/calendar --include=*.rs` returns nothing (lint gate is clippy `unwrap_used`, double-check).

## Acceptance (from intent, restated)

Automated: gates green incl. 3 new suites; DB schema check shows triggers on every calendar table. Manual (Jimmy): create timed event + all-day multi-day event → dashboard + week correct across a TZ/DST change; weekly recurrence correct across 2026-03-29 DST boundary; this-occurrence edit vs series edit behave per spec; reminder fires desktop notification + in-app banner; close app, reopen past due time → catch-up fires exactly once; Dismiss/Snooze hold; link reminder ↔ todo and ↔ email; trash + restore; restart → everything intact.

## Commit/branch discipline

Branch `m3-calendar` (cut from `main` after M1/M2 merges). One commit per step as named. Push after gates (auto-approved conditions). No PR without Jimmy's say. Agent never touches main. No Rust code or migration files were written as part of this triplet — implementation starts at Step 1.
