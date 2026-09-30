# Adjutant — Execution Plan (M2: Email)

Normative: `.kimi/intents/adjutant-m2-email-spec.md` (read first, it wins conflicts). Repo `/home/jimmy/Projects/adjutant`, branch `m2-email` cut from `m1-foundation` tip (m1 not merged yet — stack branches; merge order decided by Jimmy later). ENV: `export PATH="$HOME/.cargo/bin:$PATH" https_proxy=http://127.0.0.1:7897 http_proxy=http://127.0.0.1:7897`. Never launch GUI from agent sessions (Jimmy does manual UI pass; exception: orchestrator may launch for screenshot QA while Jimmy is away, closing it after).

## Risk interrogation additions (beyond spec §8)

8. **lettre 0.11 sync API** — confirm `SmtpTransport::relay(...)...build()` sync sender exists at 0.11 with rustls features; if it's async-only now, use the `sync` feature or drop to `async-std`? NO async runtimes — if lettre 0.11 sync is gone, fall back to a minimal hand-rolled SMTP client over rustls (SMTP AUTH PLAIN + DATA is ~150 lines; document decision either way).
9. **secret-service blocking on Arch/Omarchy**: requires a running Secret Service provider (gnome-keyring or keepassxc). Verify presence at runtime; if absent → clear UI error "no secret service", not a panic. Check now: `busctl --user list | grep -i secret`.
10. **Thread-id churn**: ingest order affects subject-fallback grouping; re-ingesting on UIDVALIDITY resync must produce stable thread_ids (match by message_id first — deterministic).
11. **Scope guard**: NO MCP tools, NO Gmail, NO FTS, NO IDLE, NO HTML compose. Email links to todos = read-only display of existing links in reading pane + todo pickers gain email entries; do not build new link UI beyond that.
12. **Jimmy's real creds**: agent never asks for/reads his passwords. The live-server spike (risk §8.1) is JIMMY's manual step after UI ships.

## Step 1 — branch + deps + spike

```bash
cd /home/jimmy/Projects/adjutant && git checkout -b m2-email
cargo add imap@=3.0.0-alpha.15 mailparse@0.17
cargo add lettre@0.11 --no-default-features --features smtp-transport,rustls-tls,builder
cargo build  # resolve API surprises now (imap 3 alpha + lettre sync) per risks 1/8
```

Write `src/email/imap_client.rs` + `src/email/smtp.rs` thin wrappers (traits `ImapTransport`/`SmtpTransport` + live impls) compiling against the real crate APIs. No UI yet. Commit `feat(email): transport wrappers over imap and lettre`.

## Step 2 — schema + store

1. `migrations/0002_email.sql` verbatim from spec §3; register in `src/db/migrate.rs` MIGRATIONS list.
2. `src/email/model.rs` + `src/email/mod.rs` (EmailStore: accounts/folders CRUD, ingest (idempotent, computes thread_id), queries for list/thread/outbox/write-ops, body cache eviction fn).
3. `src/email/thread.rs` JWZ-lite per spec §5 (pure functions, unit-tested in-crate).
4. `tests/email_store.rs` per spec §9.

Commit `feat(email): schema, store, and jwz threading`.

## Step 3 — sync engine

1. `src/email/sync.rs` per spec §4: worker thread, command/event channels, own DB connection, per-account cycle (LIST → role detect → UIDVALIDITY check → incremental/initial-500 header sync), poll loop with recv_timeout, body fetch, write-op push (UID MOVE w/ COPY+DELETE fallback via capability check), outbox send (SMTP + Sent APPEND best-effort), keyring reads per connection, panic-caught-at-boundary, sanitized errors to email_sync_log.
2. Keyring helper `src/core/keyring.rs` (service `adjutant`, `account/<id>`; set/get/delete; clear error when no provider).
3. `tests/email_sync.rs` with scripted mock transports per spec §9.

Commit `feat(email): background sync engine with offline write queue`.

## Step 4 — UI

1. Accounts section in sidebar + account form (password → keyring only) + test-connection (spec §6).
2. Inbox: Unified (default) + per-account switcher, thread cards (folded card language: account dot, bold unseen, snippet, date, gold unread dot, paperclip, thread-count), client-side Ctrl+F filter, empty states.
3. Reading pane: back/prev/next (j/k), header block, body (fetch-on-open via engine event), attachments save-to-~/Downloads, linked-todos display (existing link layer).
4. Compose pane (new/reply/reply-all/forward, plain text, quote prefill) → Stage to outbox.
5. Outbox view: staged cards, Approve/Edit/Discard + Approve-all, state badges, failed retry.
6. F1 overlay additions. Todo detail "links" picker gains email search entries (read-only link creation todo→email).
7. `tests/email_ui.rs` per spec §9.

Commits split as `feat(email): accounts and inbox ui`, `feat(email): reading pane and compose`, `feat(email): outbox review flow`.

## Step 5 — gates + docs

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo build --release`. README email section; AGENTS.md module-map update (email/ added, transport-trait test seam rule, no-network-in-tests rule). Commit `docs: email module`.

## Acceptance (from intent, restated)

Automated: all gates green incl. new suites; `secret-tool` shows no password in DB (grep DB for known test password string → absent). Manual (Jimmy): add Purelymail account → sync → merged inbox → read (body cached; offline reopen works) → mark read w/ network down → reconnect → flag pushed → compose → outbox → approve → received → in Sent. Restart → incremental sync only.

## Commit/branch discipline

Branch `m2-email`. One commit per step as named. Push after gates (auto-approved conditions). No PR without Jimmy's say. Agent never touches main. Agent never handles Jimmy's real credentials.
