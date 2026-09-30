# Adjutant — Intent (M3: Calendar)

Captured: 2026-09-30. Builds on `adjutant-m1-intent.md` (vision/constraints there apply) and `adjutant-m2-email-intent.md`. Discovery answers from Jimmy below.

## Scope

Calendar module: local-first event store in SQLite (source of truth — NOT a Google Calendar client; Google sync is M5). Timed events, all-day events, recurrence, attendees, desktop-notification reminders that also surface in-app and link into the existing entity graph (reminders ↔ todos/emails). Combined default view: next-5-upcoming strip over a week grid.

## Locked decisions (2026-09-30)

1. **All event features in scope**: timed events, all-day events (multi-day supported), recurrence (iCalendar RRULE-style subset), attendees (name/email/RSVP-status — informational only, no contact management, no invites).
2. **Reminders: desktop notifications via `notify-rust`** (Linux-native XDG notification; natural fit for Omarchy/Hyprland where a notification daemon is standard). Every desktop notification ALSO surfaces in-app (toast + due-reminder queue). Reminders are link-graph citizens: a reminder can be linked to todos/emails (`EntityType::Reminder` already exists in `src/core/entity.rs` — no schema addition needed).
3. **Default view**: list of next 5 upcoming events pinned at top, week view underneath. If the window is too short to fit both, the whole column scrolls (week included). View switcher: Upcoming+Week combined (default) / Week-only / Upcoming-only. Month view is an optional stretch — spec only, build only if it drops in cleanly.
4. **Local-first**: local SQLite event store is the single source of truth. NO Google/client sync in M3. Schema must be built M5-ready: `updated_at` maintained by SQL triggers on every table (M5 last-write-wins conflicts depend on this — locked 2026-09-29), UUID keys, no server-shaped columns that only make sense for Google.

## M3 feature list

- Event CRUD in UI: title, location, description, timed (start/end + timezone) or all-day (start/end dates), attendees, color tag (reuses 6-color group-dot cycle language), reminders attached per event.
- Recurrence: RRULE subset — FREQ in (DAILY, WEEKLY, MONTHLY, YEARLY), INTERVAL, BYDAY, BYMONTHDAY, UNTIL or COUNT, WKST. Edit recurring event: **this occurrence / this and all future / entire series**.
- Views: combined dashboard (upcoming-5 strip + week grid, default), week-only, upcoming-only; month spec'd as stretch. Today highlighted; past events dimmed, not hidden.
- Reminders: per-event reminder offsets (minutes before start). Delivery: desktop notification (notify-rust) + in-app toast + due-reminder banner with Dismiss / Snooze (5/15/60 min). Missed reminders (app closed) fire as catch-up on launch, bounded to the last 24h, each occurrence fires at most once.
- Link graph: events and reminders are linkable entities (link todo → event, reminder → email, etc.). Event detail shows linked entities; existing todo/email link pickers gain calendar entries.
- Trash: deleted events go to trash (soft), purge with the existing 30-day rule at startup (same mechanism as todos).

## Non-goals (M3)

- Google/any sync (M5 — schema is built for it, sync is not). Invitations/organizer semantics/iTIP (attendees are data fields only). Free/busy. Timezone editing UI beyond per-event IANA picker defaulting to system TZ. Calendar sharing. Task lists (that's the todo module). Month view unless the stretch lands cheap. MCP tools (M4 — but schema/store designed for them, same as email's outbox `origin` column was).

## Open risks (need spec-level answers)

1. **Timezone storage** — events need local-TZ semantics; chrono alone can't do IANA TZ. Decision needed at spec: add `chrono-tz` (recommended) and pin storage shape (UTC instants + IANA name per event).
2. **Recurrence implementation** — store RRULE text and expand on read for a query window, vs materializing occurrences. Spec must pick one (recommend: expand on read, no materialized occurrence table in M3) and justify.
3. **Reminder delivery while app is closed** — no daemon in M3; catch-up-on-launch is the accepted behavior (locked above), bounded and deduplicated per occurrence.
4. **notify-rust on Hyprland/Wayland** — depends on a running notification daemon (Omarchy ships one) and zbus/dbus session bus; failure must degrade to in-app-only, never panic.
5. **DST edge cases** — recurrence expansion across DST transitions is the classic bug farm; spec pins the rule (expand in the event's IANA TZ, not UTC arithmetic).

## Acceptance (M3, shell-testable + manual)

- `cargo build --release` clean; `cargo clippy --all-targets -- -D warnings` clean; `cargo test` green (store tests against in-memory DB; recurrence/expansion as pure-function unit tests; reminder scheduler against injected clock + mocked notifier — no live network, no real desktop notifications in tests).
- Create timed event with timezone → appears in week grid at correct local hour after a TZ offset change (verify: event created for Europe/London in summer displays at stored wall-clock local time); all-day multi-day event spans correct days.
- Weekly recurring event → correct occurrences in week view across a DST boundary; edit "this occurrence only" → one instance changed, series intact; edit "entire series" → all occurrences change.
- Set reminder 1 min before a past-due recurring occurrence → launch app → catch-up fires exactly once (desktop notification + in-app banner); Dismiss holds; Snooze re-fires in-app at the new time.
- Restart: all events/reminders/links intact; `sqlite3 adjutant.db ".schema calendar_events"` shows triggers on every calendar table.
- Manual (Jimmy): default dashboard shows upcoming-5 over week; view switcher works; link a reminder to a todo and an email; trash an event and restore it.
