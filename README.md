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
