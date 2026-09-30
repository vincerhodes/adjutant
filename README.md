# Adjutant

A native Linux personal assistant — todos, email, calendar, notes — in a
single Rust binary with a local SQLite database. M1 ships the foundation:
the egui shell, Omarchy theme integration, and the todo module (groups,
arbitrary-depth nesting, priorities, due dates, trash, `blocks` links).

## Status

M1 (foundation + todo) — feature-complete, pending manual UI pass.

M2 (email) and M3 (calendar) — implemented; pending manual UI pass.

## Build & run

Rust via rustup at `~/.cargo/bin` (1.98.1). Behind the GFW, crate downloads
need the Clash proxy:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export https_proxy=http://127.0.0.1:7897 http_proxy=http://127.0.0.1:7897

cargo build --release
cargo run
```

## Email (M2)

IMAP (rustls) + SMTP (lettre/rustls) with a background sync engine: one
worker thread with its own WAL connection; per-account incremental sync
(UIDVALIDITY-guarded, initial sync capped at the newest 500 headers per
folder); bodies fetched on demand (plain text preferred, HTML-only →
dumb tag-strip with raw HTML kept for a future renderer); offline write
queue for mark-read/moves; JWZ-lite threading at ingest; staged outbox
— nothing sends without human approval in the Outbox view. Passwords
live in the OS keyring (secret-service) only — never in the DB.

The `ImapTransport`/`SmtpTransport` traits are the test seam: all
automated tests use scripted mocks, no live network in `cargo test`.
Connecting a real account is a manual step: Manage accounts → Add
account → Test connection.

## Calendar (M3)

Local-only calendar: timed + all-day events, weekly/monthly/yearly
recurrence (hand-rolled RRULE subset — expand-in-TZ-wall-clock, store UTC,
DST-correct), per-occurrence edits (this / this-and-future series split /
entire series), attendees, reminders, color tags, `scheduled_as` links to
todos. Views: Dashboard (upcoming strip over the week grid), Week
(all-day band, now-line, past-dimmed), Upcoming list. Reminders are
app-session, tick-driven (~15s): due reminders fire a desktop
notification (notify-rust over session dbus — a missing daemon is logged
and swallowed, never fatal) plus an in-app banner with Dismiss /
Snooze (5/15/60); catch-up on launch fires what was missed while the
app was closed, bounded to 24h, exactly once per occurrence (ledger
rows in `calendar_reminder_fires`).

The `Notifier` trait is the test seam: tests assert on a mock's call
log, no dbus in `cargo test`. Timezone handling uses `chrono-tz`;
the event form's TZ picker defaults to the system zone.

## Data

- Location: `$XDG_DATA_HOME/adjutant/` (usually `~/.local/share/adjutant/`).
  Override with `$ADJUTANT_DATA_DIR` (dev/test isolation).
- Permissions: data dir `0700`, DB file `0600`.
- Schema: `migrations/0001_init.sql` (`todos`, `todo_groups`,
  `entity_links`, `settings`), `0002_email.sql` (accounts, folders,
  messages, outbox), `0003_calendar.sql` (`calendar_events`,
  `calendar_attendees`, `calendar_event_exceptions`, `calendar_reminders`,
  `calendar_reminder_fires`); version tracked via `PRAGMA user_version`.
- **Backup:** use `VACUUM INTO 'backup.db'` or the sqlite backup API —
  never raw-copy a live DB (WAL sidecar files make copies inconsistent).

## Known shortcut conflicts

`Alt+↑/↓` (reorder) collides with some Hyprland bind configs. Shortcut
remapping arrives in a later milestone.

## Install the desktop entry

```bash
cp adjutant.desktop ~/.local/share/applications/
```
