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
  body_html   TEXT,                        -- raw HTML part when present (NULL for plain-only mail); kept for future subset renderer (M3.5 candidate), never rendered in M2
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
