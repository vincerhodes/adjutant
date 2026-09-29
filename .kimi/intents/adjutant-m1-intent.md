# Adjutant — Intent (M1: Foundation + Todo)

Captured: 2026-09-29. Source: planning conversation (full transcript in session args) + discovery answers.

## Product vision

Adjutant — native Linux personal hub ("AI staff officer"). Email, calendar, todo,
scratchpad in one local-first app. Generic entity-link graph lets every entity
reference every other. App exposes itself as MCP server; any agent CLI (Kimi Code,
Claude Code) operates the hub through tools. **Agent proposes, human approves** —
no external action (send, sync-push) without explicit UI sign-off.

## Hard constraints

- Rust, single static binary. egui GUI. rusqlite storage. No web runtime, no heavy frameworks.
- Target env: Omarchy/Hyprland (Wayland).
- Secrets (IMAP/SMTP passwords, OAuth tokens) in OS keyring via `secret-service` — never in DB, logs, or MCP tool outputs.
- Local SQLite = source of truth for calendar (NOT a Google Calendar client). Google = synced mirror.
- Distribution: **personal only** (Jimmy's machine, his Google OAuth client creds). No multi-user/onboarding design.

## Data model (whole-app, foundational)

- Typed entities: `email`, `thread`, `event`, `todo`, `note`, `reminder`.
- Generic link table: `(source_type, source_id, target_type, target_id, relation)`.
- Relations (v1 vocabulary): `derived_from`, `blocks`, `scheduled_as`, `mentions`. Table must allow future relations without migration pain.
- Link graph is the core feature — email → todo → sub-todos → event, traversable both directions.

## Discovery answers (locked decisions)

1. **Calendar sync conflicts:** last-write-wins (newest `modified` timestamp wins).
2. **Approval model:** per-item review + "approve all" batch button per queue.
3. **Email sync scope:** all folder headers/metadata synced; bodies fetched on open, cached ~90 days.
4. **Distribution:** personal only.

## Milestone map (build order, from kickoff)

1. **M1 — Foundation + Todo** ← THIS PLAN'S SCOPE
2. M2 — Email: multi-account IMAP, unified inbox (SQL view), reading pane, staged outbox, SMTP via lettre
3. M3 — Calendar: local event store, two-way Google sync (official Calendar API), reminders linked to todos/emails
4. M4 — Agentic layer: MCP server, tools (list_unread_emails, draft_email, create_event, create_todo, create_reminder, link_entities, search_notes) + staged/dry-run variants, review queues
5. M5 — Google integration: Gmail IMAP/SMTP + OAuth2 (oauth2 crate, keyring, auto-refresh); Calendar sync
6. M6 — In-app agent chat: egui pane spawning agent CLI headless, pre-wired to Adjutant MCP, streaming, conversations in SQLite
7. M7 — Scratchpad: markdown notes, linkable to all entities

(Note: M2/M3 ordering here folds the kickoff's steps 2–4; MCP server lands before Google OAuth so agent tooling is testable against todo/email first. Exact later-milestone sequencing is advisory, not locked.)

## M1 scope (what this triplet plans)

- Cargo project scaffold, single binary crate, module layout that later milestones extend without rework.
- SQLite schema: entities above + link table + indexes, migration mechanism (versioned, runs at startup).
- Settings persistence (DB-backed app settings table; secrets excluded by design).
- Entity-link layer: Rust API over the link table (create/remove/query links, typed entity refs, graph traversal helpers).
- Todo module: nested sub-todos (arbitrary depth), groups, priority, statuses; full CRUD.
- Working egui UI: sidebar module nav (Todo live; Email/Calendar/Notes as disabled placeholders), todo tree view, inline edit, status/priority controls.
- App shell: window, theme fit for Omarchy/Hyprland, keyboard-first basics.

## M1 explicit non-goals

- No MCP server, no email/calendar/network code, no OAuth, no agent chat. Schema must not preclude them.
- No packaging/distribution (no AUR, no installer) — `cargo run`/`cargo build --release` only.
- No multi-window, no system tray.

## Acceptance (M1, shell-testable)

- `cargo build --release` clean on stable Rust, zero warnings (`clippy` clean).
- Fresh launch creates `~/.local/share/adjutant/adjutant.db` (XDG) with full schema; relaunch reuses it.
- Via UI: create todo group → todos → nested sub-todos (≥3 levels) → set priority/status → complete parent shows children state; all survives restart.
- Link layer exercised: todo can link to another todo (`blocks`); cycle prevented or handled explicitly.
- DB inspectable with `sqlite3` CLI — schema documented in repo.
