# Adjutant — Intent (M2: Email)

Captured: 2026-09-30. Builds on `adjutant-m1-intent.md` (vision/constraints there apply). Discovery answers from Jimmy below.

## Scope

Email module: multiple IMAP accounts, unified inbox, reading pane, compose → staged outbox → approved SMTP send. Local-first: SQLite is the queryable store; servers are sync sources.

## Locked decisions (2026-09-30)

1. **Accounts: Purelymail only for M2** (ai@/expenses@/jimmy@browzr.net etc.). Gmail deferred to M5 OAuth milestone as planned. Account model must not assume Purelymail — generic IMAP/SMTP host+port+creds, Purelymail is just the first data.
2. **Credentials: OS keyring via secret-service** (dep already declared). Per-account app passwords entered once in the UI, stored under `adjutant/<account-id>` keys. Never in DB, never in logs, never in MCP output.
3. **Unified inbox: merged view is DEFAULT** (all accounts' INBOXes interleaved by date, account shown as colored chip — group-dot color language carries over). Switcher to view a single account. Unread counts per account + merged.
4. **Sending in scope for M2**: compose → staged outbox → per-item approve or approve-all (batch per M1 locked decision) → SMTP send → IMAP APPEND to Sent. An outbox that can't send is a todo list with worse UX.

## M2 feature list

- Account management: add/edit/remove accounts in UI (label, email address, IMAP host/port, SMTP host/port, username, password→keyring, accent color auto-assigned from the 6-color cycle). Connection test button.
- Sync engine: background thread (std::thread + channels — no tokio), manual refresh + periodic poll (default 5 min, configurable). All folders' headers synced; bodies fetched on open, cached ~90 days (per M1 locked decision). Initial sync bounded (e.g. newest 500 headers per folder, then incremental by UIDVALIDITY+UID).
- Flags: read/unread synced both ways (\Seen). Local actions: mark read/unread, archive (move to Archive), delete (move to Trash). Write ops queue locally and push to server on next sync — offline-tolerant.
- Threading: JWZ-style via Message-ID/In-Reply-To/References + normalized subject fallback. `thread_id` on emails at ingest; list groups by thread (default ON, toggleable to flat). No separate threads table in M2 (link layer tolerates dangling `thread` refs by design).
- Reading pane: headers (from/to/cc/date), text body (plain; HTML → text fallback via readable extraction), attachment list with save-to-disk on click (no auto-download).
- Compose: new mail + reply/reply-all/forward from reading pane. Plain text only in M2. To/CC/BCC, subject, body. Saves to outbox as staged draft; edit before approval.
- Outbox view: staged drafts (from UI now, from MCP agents later), per-item approve/edit/discard + "Approve all". On approve: SMTP send, then APPEND to Sent, mark outbox item sent. Failures stay staged with error, retryable.
- Email ↔ link graph: emails are linkable entities (link todo → email etc. from todo detail pickers once M2 exists; email reading pane shows linked todos, click navigates).
- Todo-module filter pattern carries over: client-side subject/sender filter (Ctrl+F). No FTS in M2.

## Non-goals (M2)

- Gmail/OAuth (M5). IMAP IDLE/push (M-later; poll is fine). FTS search index. HTML compose. PGP/signing. Contact management/autocomplete beyond same-account recents. Attachment previews/images. Multiple identities per account. MCP tools (M4 — but schema + outbox designed for them).

## Acceptance (M2, shell-testable + manual)

- `cargo build --release` clean; `cargo clippy --all-targets -- -D warnings` clean; `cargo test` green (store tests against in-memory DB + sync-engine tests against a fake IMAP server fixture or trait-mocked transport — no live server in tests).
- Add Purelymail account in UI → password in keyring (`secret-tool search service adjutant` shows entry; DB contains no password string) → initial sync populates folders+headers → merged inbox shows all accounts with chips → switcher filters to one account.
- Read email → body fetched + cached (second open offline works); \Seen pushed on next sync.
- Compose → staged in outbox → approve → sends via SMTP (verify receipt) → appears in Sent. Discard leaves server untouched.
- Offline write: mark-read while network down → queued → pushed when back.
- Restart: all state intact; sync resumes incrementally (no full re-fetch).
