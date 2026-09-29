-- migrations tracked via PRAGMA user_version
CREATE TABLE todos (
  id            TEXT PRIMARY KEY,
  group_id      TEXT NOT NULL REFERENCES todo_groups(id) ON DELETE CASCADE,
  parent_id     TEXT REFERENCES todos(id) ON DELETE CASCADE,   -- NULL = top-level in group
  title         TEXT NOT NULL,
  notes         TEXT NOT NULL DEFAULT '',
  status        TEXT NOT NULL DEFAULT 'open'
                CHECK (status IN ('open','in_progress','done','cancelled')),
  priority      INTEGER NOT NULL DEFAULT 2 CHECK (priority BETWEEN 0 AND 3), -- 0=low..3=urgent
  position      INTEGER NOT NULL DEFAULT 0,                     -- sibling ordering
  due_date      TEXT,                                           -- ISO-8601 date (YYYY-MM-DD), optional
  deleted_at    TEXT,                                           -- trash: NULL = live, set = trashed at timestamp
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  completed_at  TEXT
);
CREATE INDEX idx_todos_group ON todos(group_id, parent_id, position) ;
CREATE INDEX idx_todos_due ON todos(due_date) WHERE deleted_at IS NULL;

CREATE TABLE todo_groups (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  position   INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- updated_at maintained by triggers, NOT app code: M4's MCP server writes
-- directly to the DB and must not be able to skip timestamp bumps
-- (last-write-wins sync depends on accurate updated_at).
CREATE TRIGGER trg_todos_updated AFTER UPDATE ON todos
BEGIN UPDATE todos SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;
CREATE TRIGGER trg_todo_groups_updated AFTER UPDATE ON todo_groups
BEGIN UPDATE todo_groups SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;

-- Generic entity links — the core graph. Entity rows live in their own
-- tables per type; this table references them loosely (no FK, entities
-- from future modules don't exist yet). Integrity enforced in link layer code.
CREATE TABLE entity_links (
  id          TEXT PRIMARY KEY,
  source_type TEXT NOT NULL CHECK (source_type IN ('email','thread','event','todo','note','reminder')),
  source_id   TEXT NOT NULL,
  target_type TEXT NOT NULL CHECK (target_type IN ('email','thread','event','todo','note','reminder')),
  target_id   TEXT NOT NULL,
  relation    TEXT NOT NULL,   -- v1 vocab: derived_from|blocks|scheduled_as|mentions; open enum by design
  created_at  TEXT NOT NULL,
  UNIQUE (source_type, source_id, target_type, target_id, relation)
);
CREATE INDEX idx_links_source ON entity_links(source_type, source_id);
CREATE INDEX idx_links_target ON entity_links(target_type, target_id);

CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL          -- JSON-encoded
);
