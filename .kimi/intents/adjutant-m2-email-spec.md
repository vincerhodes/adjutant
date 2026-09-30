# Adjutant — Technical Spec (M2: Email)

Source: `.kimi/intents/adjutant-m2-email-intent.md` (normative for decisions). Builds on M1 layout + M1.5 design language. Zero changes to existing migrations — all schema via `migrations/0002_email.sql`.

## 1. Dependencies (additions)

| Crate | Version | Purpose |
|---|---|---|
| imap | =3.0.0-alpha.15 (pinned exact) | IMAP client, sync API fits thread model. Fallback if trouble: 2.4.1 (note in Cargo.toml comment) |
| mailparse | 0.17 | MIME parse: headers, bodies, attachments, RFC822 |
| lettre | 0.11 | SMTP send; features: `smtp-transport`, `rustls-tls`, `builder` (no tokio — use `Transport::sync`… verify: lettre 0.11 sync transport is behind default `smtp-transport`; do NOT pull async) |
| rustls | 0.23 (indirect) | TLS for imap/lettre — prefer rustls over native-tls for static binary story |

No tokio. Sync engine = `std::thread` + `std::sync::mpsc` + its own rusqlite connection (WAL allows concurrent readers/writers; busy_timeout already set).

## 2. Module layout (additions, dependency rules unchanged)

```
src/email/
├── mod.rs            # EmailStore: SQL CRUD, queries, outbox ops, write-op queue
├── model.rs          # Account, Folder, Email, Thread refs, OutboxItem, SyncState
├── imap_client.rs    # thin wrapper over imap crate: connect/fetch/flags/move/append (mockable trait ImapTransport)
├── smtp.rs           # lettre send wrapper (mockable trait SmtpTransport)
├── sync.rs           # SyncEngine: worker thread, account sync, body fetch, write-op push
├── thread.rs         # JWZ threading: thread_id computation at ingest
├── compose.rs        # draft building, reply quoting (plain text)
└── ui/
    ├── mod.rs        # EmailUi state + routing
    ├── list.rs       # merged/per-account inbox, thread groups, card rows
    ├── reading.rs    # reading pane
    ├── compose_ui.rs # compose editor (staged)
    ├── outbox.rs     # staged outbox review view
    └── accounts.rs   # account add/edit/test UI
tests/
├── email_store.rs    # in-memory DB: ingest, threading, outbox, write-ops
├── email_sync.rs     # SyncEngine vs mock ImapTransport (no network)
└── email_ui.rs       # kittest: merged view, account switcher, outbox approve
```

`ImapTransport`/`SmtpTransport` traits = the test seam; live impls in imap_client.rs/smtp.rs, mocks in tests. No live network in `cargo test`.

## 3. Schema — migrations/0002_email.sql

```sql
CREATE TABLE email_accounts (
  id            TEXT PRIMARY KEY,
  label         TEXT NOT NULL,
  address       TEXT NOT NULL,
  imap_host     TEXT NOT NULL,
  imap_port     INTEGER NOT NULL DEFAULT 993,
  smtp_host     TEXT NOT NULL,
  smtp_port     INTEGER NOT NULL DEFAULT 465,
  username      TEXT NOT NULL,
  color_idx     INTEGER NOT NULL DEFAULT 0,   -- 6-color cycle, matches group dots
  sync_interval_s INTEGER NOT NULL DEFAULT 300,
  last_sync_at  TEXT,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);
CREATE TRIGGER trg_email_accounts_updated AFTER UPDATE ON email_accounts
BEGIN UPDATE email_accounts SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;
-- password: OS keyring under service "adjutant", key "account/<id>" — NEVER in DB

CREATE TABLE email_folders (
  id         TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES email_accounts(id) ON DELETE CASCADE,
  name       TEXT NOT NULL,              -- IMAP mailbox name, e.g. "INBOX", "Sent"
  role       TEXT NOT NULL DEFAULT 'other'
             CHECK (role IN ('inbox','sent','drafts','trash','archive','junk','other')),
  uidvalidity INTEGER,                   -- resync detector
  last_uid   INTEGER NOT NULL DEFAULT 0, -- incremental high-water mark
  UNIQUE (account_id, name)
);

CREATE TABLE emails (
  id          TEXT PRIMARY KEY,            -- app uuid
  account_id  TEXT NOT NULL REFERENCES email_accounts(id) ON DELETE CASCADE,
  folder_id   TEXT NOT NULL REFERENCES email_folders(id) ON DELETE CASCADE,
  uid         INTEGER NOT NULL,            -- IMAP UID within folder
  message_id  TEXT,                        -- RFC822 Message-ID (may be null on broken mail)
  thread_id   TEXT NOT NULL,               -- computed at ingest (§5)
  from_name   TEXT, from_addr TEXT NOT NULL,
  to_json     TEXT NOT NULL DEFAULT '[]',  -- [{name,addr}]
  cc_json     TEXT NOT NULL DEFAULT '[]',
  subject     TEXT NOT NULL DEFAULT '',
  snippet     TEXT NOT NULL DEFAULT '',    -- ~140 chars, from body when fetched else header-only
  date        TEXT NOT NULL,               -- message Date header (UTC ISO)
  flags       TEXT NOT NULL DEFAULT '',    -- space-sep: seen flagged answered draft
  has_attachments INTEGER NOT NULL DEFAULT 0,
  body_text   TEXT,                        -- NULL = not fetched yet (cache)
  body_fetched_at TEXT,
  size        INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  UNIQUE (folder_id, uid)
);
CREATE INDEX idx_emails_account_date ON emails(account_id, date DESC);
CREATE INDEX idx_emails_thread ON emails(thread_id, date);
CREATE INDEX idx_emails_unread ON emails(account_id) WHERE flags NOT LIKE '%seen%';
CREATE TRIGGER trg_emails_updated AFTER UPDATE ON emails
BEGIN UPDATE emails SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;

CREATE TABLE email_attachments (
  id       TEXT PRIMARY KEY,
  email_id TEXT NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
  filename TEXT NOT NULL,
  mime     TEXT NOT NULL,
  size     INTEGER NOT NULL DEFAULT 0,
  content  BLOB,                           -- NULL until explicitly saved/fetched
  UNIQUE (email_id, filename)
);

CREATE TABLE email_outbox (
  id          TEXT PRIMARY KEY,
  account_id  TEXT NOT NULL REFERENCES email_accounts(id) ON DELETE CASCADE,
  state       TEXT NOT NULL DEFAULT 'staged'
              CHECK (state IN ('staged','approved','sending','sent','failed','discarded')),
  to_json     TEXT NOT NULL,
  cc_json     TEXT NOT NULL DEFAULT '[]',
  bcc_json    TEXT NOT NULL DEFAULT '[]',
  subject     TEXT NOT NULL DEFAULT '',
  body        TEXT NOT NULL DEFAULT '',
  in_reply_to TEXT,                        -- message_id of source email (reply threading)
  source_email_id TEXT REFERENCES emails(id),  -- set for replies/forwards
  origin      TEXT NOT NULL DEFAULT 'ui' CHECK (origin IN ('ui','agent')),  -- M4 uses 'agent'
  error       TEXT,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  sent_at     TEXT
);
CREATE TRIGGER trg_email_outbox_updated AFTER UPDATE ON email_outbox
BEGIN UPDATE email_outbox SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;

-- Offline-tolerant server write queue (mark read, archive, delete)
CREATE TABLE email_write_ops (
  id         TEXT PRIMARY KEY,
  email_id   TEXT NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
  op         TEXT NOT NULL CHECK (op IN ('set_seen','unset_seen','move_folder')),
  arg        TEXT,                          -- target folder name for move_folder
  state      TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','done','failed')),
  error      TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE email_sync_log (              -- lightweight diagnostics, no secrets
  id         TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES email_accounts(id) ON DELETE CASCADE,
  event      TEXT NOT NULL,                 -- 'sync_start','sync_ok','sync_err','send_ok','send_err'
  detail     TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
```

## 4. Sync engine (src/email/sync.rs)

- One worker thread owns a `Vec<AccountSync>`; UI sends commands over mpsc (`SyncNow(account?)`, `FetchBody(email_id)`, `PushWriteOps`, `Shutdown`). Engine emits events (`Progress`, `FolderSynced`, `BodyReady`, `SendResult`, `Error`) on a channel the UI drains each frame (reactive repaint: `ctx.request_repaint()` on event).
- Engine owns its OWN rusqlite connection (open via same Db::open path + pragmas). UI connection stays the primary.
- Account sync cycle: connect (rustls, imap) → LIST folders → upsert email_folders + detect role by name (\INBOX/Sent/Drafts/Trash/Archive/Junk special-use attrs or name match) → per folder: if UIDVALIDITY changed → full resync of that folder; else incremental `UID FETCH last_uid+1:*` headers (ENVELOPE + FLAGS + RFC822.SIZE) → ingest rows (compute thread_id) → update last_uid.
- Initial folder sync bound: if last_uid=0, fetch only newest 500 UIDs (via SEARCH ALL, take tail).
- Poll: engine sleeps with recv_timeout(sync_interval) — per-account interval, min 60s.
- Body fetch: on email open, UI sends FetchBody → engine `UID FETCH <uid> BODY[]` → mailparse → prefer text/plain; HTML-only → strip to text (simple tag-strip + entity decode — no html2text dep; keep it dumb, note quality limitation) → store body_text + body_fetched_at + snippet refresh + attachments metadata (no content). Cache eviction: at startup, `UPDATE emails SET body_text=NULL, body_fetched_at=NULL WHERE body_fetched_at < datetime('now','-90 days')`.
- Write-op push: engine applies pending email_write_ops (UID STORE FLAGS / UID MOVE or COPY+DELETE fallback) → mark done/failed. UI applies ops locally immediately (optimistic), engine reconciles.
- Outbox send: approved item → build RFC822 via lettre Message builder → SMTP (rustls) → on success APPEND to Sent folder (best-effort; failure logged, mail still counts sent) → state=sent. state=failed keeps error; "Retry" re-approves.
- All secrets: engine fetches password from keyring per connection, holds in memory only, zeroized on drop (secrecy crate NOT added — simple scope-dropped String is acceptable M2; note for hardening later).

## 5. Threading (src/email/thread.rs)

JWZ-lite at ingest:
1. Normalize subject: strip `Re:/Fwd:/Fw:` prefixes (loop), collapse whitespace, lowercase.
2. Candidates: In-Reply-To, last References entry, else Message-ID.
3. `thread_id` = first existing emails.id/message_id matching candidates within same account; else if any email in account shares normalized subject → reuse its thread_id; else new uuid.
Container grouping happens in SQL (GROUP BY thread_id, latest date) — no threads table. Thread with messages across folders (Inbox + Sent) merges by message-id match.

## 6. UI (design language per M1.5/§8)

- Module nav: **Email becomes enabled**. Todo-style layout reused: sidebar gains "Accounts" section (per-account: colored dot, label, unread count) + "Unified" pseudo-account at top (default). Views: Inbox (merged or single account), Outbox, Accounts (manage).
- Thread rows as cards (folded card language): account color dot, from (bold if unseen), subject, snippet, right side: date + unread gold dot + attachment paperclip icon + count badge `N` for thread size. Click thread → reading pane.
- Reading pane replaces list area (back chevron returns; ←/→ prev/next thread; keyboard j/k aliases documented in F1). Header block, body text, attachment rows (icon + filename + size → click saves via rfd? NO new dep — save to ~/Downloads with sanitised name, toast path).
- Compose: modal-less full-pane editor (To/CC/BCC/subject/body) with "Stage to outbox" primary + "Discard" ghost. Reply pre-fills + quoted body (`On <date>, <name> wrote:` + `> ` lines).
- Outbox view: staged items as cards with account dot, to, subject, age; per-item Approve (primary) / Edit / Discard (danger ghost); "Approve all" primary at top (batch per locked decision). State badges: staged/approved/sending/sent/failed(+error tooltip).
- Account form: fields per intent; password field writes ONLY to keyring (never touches DB struct); "Test connection" runs imap login + smtp EHLO via engine, toast result.
- Empty states designed per §7a. All colors from palette; email module inherits theme system with zero new color literals.

## 7. Security

- Keyring: service `adjutant`, key `account/<uuid>`; password never in DB/logs/toasts/sync_log. Errors sanitized (imap error strings may echo credentials — log error.kind only).
- DB perms already 0600/0700. Bodies/attachments local-only. No remote image loading ever (bodies are plain text by design).
- secret-service used via blocking API on the sync thread only (no dbus on UI thread).

## 8. Risks (for plan to address)

1. imap 3.0.0-alpha API unknowns — first task is a spike: connect/list/fetch against Purelymail with Jimmy's real creds entered via UI (his manual step, not agent's).
2. UIDVALIDITY edge cases + Purelymail (Dovecot) quirks — mitigated by full-resync fallback.
3. UID MOVE vs COPY+DELETE server support — capability check, fallback path.
4. mailparse edge cases (broken encodings) — lossy decode, never fail ingest.
5. Sent-folder APPEND naming variance ("Sent" vs "Sent Items") — role detection with name fallbacks.
6. Sync thread panic must not kill app — catch at thread boundary, surface via event.
7. Concurrent DB writes from engine + UI — WAL + busy_timeout (already on), writes kept short.

## 9. Test strategy

- `tests/email_store.rs` — in-memory: account/folder CRUD, ingest idempotence (folder_id+uid unique), threading (reply chains, subject fallback, cross-folder merge), outbox state machine, write-op queue lifecycle, body cache eviction.
- `tests/email_sync.rs` — mock ImapTransport scripted with folder/UID fixtures: initial 500-cap sync, incremental, UIDVALIDITY resync, flag push, body fetch, send flow with mock SmtpTransport (success + failure retry).
- `tests/email_ui.rs` — kittest: merged vs single-account view, unread counts, outbox approve-all flow, compose staging.
- Threading unit tests in-crate (pure functions).
