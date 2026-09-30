# Adjutant

A native Linux personal assistant — todos, email, calendar, notes — in a
single Rust binary with a local SQLite database. M1 ships the foundation:
the egui shell, Omarchy theme integration, and the todo module (groups,
arbitrary-depth nesting, priorities, due dates, trash, `blocks` links).

## Status

M1 (foundation + todo) — feature-complete, pending manual UI pass.

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

## Data

- Location: `$XDG_DATA_HOME/adjutant/` (usually `~/.local/share/adjutant/`).
  Override with `$ADJUTANT_DATA_DIR` (dev/test isolation).
- Permissions: data dir `0700`, DB file `0600`.
- Schema: `migrations/0001_init.sql` (`todos`, `todo_groups`,
  `entity_links`, `settings`; version tracked via `PRAGMA user_version`).
- **Backup:** use `VACUUM INTO 'backup.db'` or the sqlite backup API —
  never raw-copy a live DB (WAL sidecar files make copies inconsistent).

## Known shortcut conflicts

`Alt+↑/↓` (reorder) collides with some Hyprland bind configs. Shortcut
remapping arrives in a later milestone.

## Install the desktop entry

```bash
cp adjutant.desktop ~/.local/share/applications/
```
