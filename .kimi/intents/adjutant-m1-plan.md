# Adjutant — Execution Plan (M1: Foundation + Todo)

Execute top-to-bottom. Context: greenfield repo `/home/jimmy/Projects/adjutant` (currently empty except `.kimi/`). Read `.kimi/intents/adjutant-m1-spec.md` first — it is normative; this plan is its ordered implementation. Plan is self-contained: no external conversation needed.

## Risks & edge cases (interrogated from spec)

1. **No Rust toolchain installed.** Step 0 installs via mise. Verify before any cargo command.
2. **egui 0.36 API drift** — code examples in memory may be from older egui. Check docs for exact `eframe::run_native` signature at 0.36 if compile fails; do not blindly downgrade.
3. **rusqlite `bundled` build time** — first build compiles SQLite from C source (~1-2 min). Expected, not a hang.
4. **Self-FK cascade + cycle interplay:** deleting a todo deletes child todos AND their links — must be one transaction, order: collect descendant IDs → delete links → delete rows (or rely on FK cascade + manual link cleanup in same tx).
5. **Recursive CTE depth** — SQLite default recursion is fine for any realistic todo tree; no artificial depth limit imposed, but UI indent caps visual width (indent modulo at depth >8).
6. **`blocks` cycle check must be transactional** with the insert — check-then-insert race is theoretical in single-process M1 but write it correctly anyway (single tx).
7. **XDG dir creation** — `directories::ProjectDirs::from("net", "browzr", "adjutant")`; create data dir recursively, set DB file `0600` post-create. Fallback to `./adjutant.db` only with loud warning if XDG unavailable.
8. **Migration runner idempotency** — `PRAGMA user_version` gates each migration; runner must be safe on fresh DB (0→1) and on every subsequent launch (no-op).
9. **Parent completion rule** — enforce in `TodoStore::set_status`, not just UI, so M4 MCP tools inherit the invariant.
10. **Not in scope (do not build):** MCP server, email, calendar, OAuth, async runtime, packaging. Do not scaffold "for future" beyond the module tree + schema already specified.
11. **Trash/purge correctness:** `purge_expired` at startup must run in one tx (collect expired top-level → delete links → delete rows) and must not nuke items trashed <30d. Restore must restore exactly the trashed subtree, no more.
12. **WAL sidecar files:** `adjutant.db-wal`/`-shm` appear next to the DB — expected, not corruption. Data dir perms `0700` cover them; backup scripts (later) must use `VACUUM INTO` or sqlite backup API, never raw file copy of a live DB.

## Step 0 — toolchain (prerequisite, ~2 min)

```bash
cd /home/jimmy/Projects/adjutant
mise use rust@latest          # writes .mise.toml
rustup component add clippy rustfmt 2>/dev/null || true  # mise rust ships these; no-op if present
cargo --version               # must succeed
git init && git add -A && git commit -m "chore: planning artifacts"
# Remote repo: github.com/vincerhodes/adjutant (PUBLIC — Jimmy approved 2026-09-29)
gh repo create vincerhodes/adjutant --public --source . --remote origin
```

## Step 1 — scaffold

```bash
cd /home/jimmy/Projects/adjutant
cargo init --name adjutant --bin .
```

`Cargo.toml` dependencies (exact versions from crates.io, 2026-09-29 — use these or newer compatible):

```toml
[dependencies]
eframe = "0.36"
rusqlite = { version = "0.40", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
anyhow = "1"
chrono = { version = "0.4", features = ["serde"] }
directories = "6"
uuid = { version = "1", features = ["v4", "serde"] }
secret-service = "5"   # declared, unused until M2 — silences "dep not in manifest" later

[lints.clippy]
unwrap_used = "warn"
```

Verify: `cargo build` green. Then create dir tree per spec §2 (`src/db/`, `src/core/`, `src/todo/`, `src/ui/`, `migrations/`, `tests/`), each `mod.rs` stub. `.gitignore`: `/target` (cargo init default) plus `*.db`, `*.db-wal`, `*.db-shm`, `crash-*.log` — never commit a live database.

## Step 2 — schema + migration runner

1. `migrations/0001_init.sql` — copy verbatim from spec §4 (includes `updated_at` triggers).
2. `src/db/migrate.rs` — `pub fn run(conn: &Connection) -> Result<()>`: read `PRAGMA user_version`; apply each pending `migrations/NNNN_*.sql` (embed via `include_str!`, not runtime file reads — static binary requirement); bump `user_version` per migration inside a tx.
3. `src/db/mod.rs` — `Db::open(path)` → create parent dirs, open, pragmas (mandatory, rusqlite defaults differ): `PRAGMA foreign_keys = ON;`, `PRAGMA journal_mode = WAL;`, `PRAGMA busy_timeout = 5000;` — WAL + busy_timeout now so M4's MCP server can share the DB with the GUI without retrofit. Run migrations. `Db::open_at_default()` resolution: `$ADJUTANT_DATA_DIR` env override (dev/test isolation) → XDG via `directories` → `./adjutant.db` fallback with loud warning. After create, `fs::set_permissions(0600)`.
4. `src/db/settings.rs` — `get<T: DeserializeOwned>(key)`, `set<T: Serialize>(key, val)` over the `settings` table.

Verify: `cargo build`; quick `sqlite3 ~/.local/share/adjutant/adjutant.db '.schema'` after a smoke run (defer to Step 6 if no binary path exists yet — write a `#[test]` in `tests/` instead: fresh in-memory DB has all 4 tables).

## Step 3 — link layer (src/core/)

1. `entity.rs` — `EntityType` enum ↔ `&str` (`as_str`, `FromStr` → `LinkError::UnknownEntityType`), `EntityRef`, `Relation` newtype with consts `DERIVED_FROM`, `BLOCKS`, `SCHEDULED_AS`, `MENTIONS` + `Relation::custom`.
2. `link.rs` — implement per spec §5 exactly: `link`, `unlink`, `links_from`, `links_to`, `would_cycle` (recursive CTE, `relation='blocks'` only), cycle rejection inside same tx as insert.
3. `tests/link_layer.rs` — in-memory DB: create/unlink/query both directions; `blocks` cycle A→B→C→A rejected; duplicate link rejected (UNIQUE); `mentions` cycle allowed; dangling target tolerated.

Verify: `cargo test link_layer` green.

## Step 4 — todo store (src/todo/model.rs + mod.rs)

1. `model.rs` — `Todo`, `TodoGroup`, `Status` enum ↔ str, `Priority` (0–3 newtype or enum).
2. `mod.rs` — `TodoStore<'a> { db: &'a Db }`:
   - groups: `create_group`, `list_groups` (ordered), `rename_group`, `delete_group`
   - todos: `create` (group, optional parent, title), `get`, `update` (title/notes/priority/due_date), `set_status` (enforce spec §6 completion rule: `done` requires all descendants terminal — query via recursive CTE, return `TodoError::OpenDescendants(n)`), `reorder` (position swap among siblings), `trash` / `restore` / `purge_expired(30 days)` / `delete_permanent` per spec §6 (each trash-state mutation covers descendants in one tx; live queries all filter `deleted_at IS NULL`), `tree(group_id)` → nested structure for UI (live only), `list_trash()` → top-level trashed items.
3. `tests/todo_store.rs` — tree build, nesting 4 deep, completion rule (parent blocked w/ open child, allowed after child done), due_date set/clear + overdue query, trash hides from tree → restore intact (links too) → permanent delete removes links, startup purge removes >30d rows only, reorder stability, `updated_at` trigger fires on UPDATE and not on INSERT-without-update.

Verify: `cargo test` green.

## Step 5 — app shell + UI

1. `src/main.rs` — XDG-aware `Db::open_at_default()` (honors `$ADJUTANT_DATA_DIR`), single-instance `flock` on `<data_dir>/adjutant.lock` (second launch → eprintln "Adjutant is already running" + exit 1), panic hook writing `crash-<timestamp>.log` to data dir per spec §9, build `AdjutantApp`, `eframe::run_native` (1200×800, title "Adjutant", Wayland `app_id = "adjutant"` so Hyprland window rules can target it). anyhow error → eprintln + non-zero exit on bootstrap failure. Reactive repaint only (`request_repaint_after` for the theme tick, no continuous loop).
2. `src/ui/theme.rs` — Omarchy `colors.toml` parser + resolution order + 1s mtime live-reload + egui `Visuals` mapping + built-in fallback, all per spec §7a. Pure functions, unit-testable (parse + map need no GPU).
3. `src/ui/fonts.rs` — system font resolution per spec §7a (Inter → IBM Plex Sans → Noto Sans → egui default), type scale setup.
4. `src/app.rs` — `eframe::App` impl: apply theme each frame (cheap after first build), sidebar nav (`Module` enum; Todo active, Email/Calendar/Notes → `ui::placeholder::disabled_nav`), route to `todo::ui`, error toast bar, settings persist (window maximized flag, last group) on exit.
5. `src/todo/ui.rs` — three-pane layout per spec §7, styled per §7a widget language (dot priorities, no badges, single accent, borderless-until-focus inputs, designed empty states). Due date: validated text field in detail pane + right-aligned row label (red overdue / accent today / muted future). Filter field per spec §6. `Alt+↑/↓` reorder. Edit commit semantics per spec §6 (Enter/Esc/blur, notes save on blur + Ctrl+S, in-flight editor state survives tree reloads). Trash view: list + restore/permanent-delete per item. Delete confirm modal = only modal.
6. `src/ui/help.rs` — `F1` overlay listing all shortcuts, dismiss on `Esc`/`F1`. Zoom keys wired at app level (egui zoom factor).
7. Keyboard shortcuts per spec §7 via `ctx.input`.
8. `tests/theme.rs` — parse real-format colors.toml fixture, fallback ordering, light/dark mode mapping.

Visual review checkpoint before Step 6: run app, screenshot, compare against §7a principles + anti-patterns list. Any violation = fix before gates.

Verify: `cargo run` — full manual pass of acceptance criteria (§ below).

## Step 6 — gates + docs

1. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo build --release` — all green.
2. `README.md` (root): what Adjutant is (2-3 lines), M1 status, `mise trust && mise install && cargo run`, schema overview pointing at `migrations/`, data location + backup pointer (`VACUUM INTO`, never raw-copy live DB), known shortcut conflicts (Alt+arrows vs Hyprland binds).
3. `adjutant.desktop` (root, for manual install): `Exec=<target/release/adjutant path>`, `Name=Adjutant`, `Type=Application`, `Categories=Office;`, no icon in M1. README documents `cp adjutant.desktop ~/.local/share/applications/`.
4. `AGENTS.md` (root): build/test commands, module layout rules (dependency direction), "no SQL in UI files", migration conventions (never edit applied migration; add new file).

## Acceptance criteria (all must pass)

```bash
cd /home/jimmy/Projects/adjutant
cargo fmt --check && cargo clippy --all-targets -- -D warnings   # clean
cargo test                                                      # all green
cargo build --release                                           # single binary at target/release/adjutant
rm -rf ~/.local/share/adjutant && cargo run                     # fresh DB created, schema matches migrations/0001_init.sql
sqlite3 ~/.local/share/adjutant/adjutant.db "PRAGMA user_version;"  # → 1
stat -c '%a' ~/.local/share/adjutant/adjutant.db                # → 600
stat -c '%a' ~/.local/share/adjutant                             # → 700
cargo run & sleep 3; cargo run                                   # second instance → "already running", exit 1; kill %1
```

Manual UI pass: create 2 groups → todos → sub-todos 3 levels deep → set priorities/statuses/due dates → attempt parent-done with open child (must refuse w/ reason) → complete children → complete parent → link todo A `blocks` todo B, attempt reverse (must refuse, cycle) → `Ctrl+F` filter narrows tree, `Esc` clears → `Alt+↑/↓` reorders, persists across restart → trash a todo with children (confirm shows descendant count) → invisible from tree → restore from Trash (links intact) → trash again, delete permanently (gone from DB incl. links) → restart app → all state intact, last group reopened.

Visual pass (spec §7a): app matches current Omarchy theme colors → switch Omarchy theme → app re-themes within ~1s without restart → empty states show designed text → no badges/pills/nested frames anywhere → single accent color in use.

## Commit strategy

One commit per step (0/1 combined): `chore: scaffold adjutant crate with mise toolchain`, `feat(db): sqlite schema + migration runner`, `feat(core): entity link layer`, `feat(todo): todo store with nesting and completion rules`, `feat(ui): egui shell and todo views`, `docs: readme and agents.md`. Push per global git rules (branch `m1-foundation`, never main).
