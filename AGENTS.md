# AGENTS.md — Adjutant

## Commands

Rust via rustup at `~/.cargo/bin` (not on the default PATH). Behind the GFW,
crates.io needs the proxy:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export https_proxy=http://127.0.0.1:7897 http_proxy=http://127.0.0.1:7897

cargo fmt --check
cargo clippy --all-targets -- -D warnings   # zero warnings gate
cargo test
cargo build --release
```

Never `cargo run` / launch the GUI from an agent session — no display access.
The human does the manual UI pass.

## Module layout & dependency direction

```
src/main.rs    bootstrap only: flock, panic hook, db open, run_native
src/app.rs     eframe::App impl (nav, routing, theme, toasts)
src/db/        Db handle, migration runner, settings — core persistence
src/core/      entity types + generic link graph (no todo/ui deps)
src/todo/      todo store (mod.rs, model.rs) + todo UI (ui.rs)
src/email/     email store (mod.rs, model.rs), transports (imap_client.rs,
               smtp.rs), sync engine (sync.rs), threading (thread.rs),
               compose (compose.rs), UI (ui/)
src/ui/        shared widgets: theme, fonts, icons, help overlay, placeholders
```

Dependency direction is strictly **UI → module → core/db**. `core/` and
`db/` never import `todo/` or `ui/`. Later modules (email/calendar/notes)
add `src/<module>/` + migrations as siblings — nothing else changes.

## Hard rules

- **Zero SQL in UI files.** All SQL lives in `src/db/` and module `mod.rs`
  files. UI calls store methods only.
- **Zero clippy warnings** under `-D warnings`; no `unwrap`/`expect` outside
  `main.rs` bootstrap and tests (`unwrap_used` lint is on).
- **Never edit an applied migration.** Schema changes = new
  `migrations/NNNN_name.sql` + extend the `MIGRATIONS` list in
  `src/db/migrate.rs` (embedded via `include_str!`; no runtime file reads).
- **No live network in `cargo test`.** `ImapTransport`/`SmtpTransport`
  are the test seam — tests inject scripted mocks; live impls only run
  from the real app's sync engine.
- **Email passwords: keyring only** (service "adjutant", key
  "account/<id>"). Never in the DB, logs, toasts, or sync log.

## Data

`$ADJUTANT_DATA_DIR` override → XDG data dir → `./adjutant.db` fallback.
Data dir `0700`, DB `0600`, WAL mode. Trashed rows purge after 30 days at
startup. Single instance enforced via `flock` on `adjutant.lock`.
