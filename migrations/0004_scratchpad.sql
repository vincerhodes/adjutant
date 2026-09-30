CREATE TABLE scratch_notes (
  id          TEXT PRIMARY KEY,            -- app uuid
  body        TEXT NOT NULL DEFAULT '',    -- plain text, no rendering
  pinned      INTEGER NOT NULL DEFAULT 0,
  color_idx   INTEGER NOT NULL DEFAULT 0,  -- 6-color cycle, matches group dots (0..=5)
  trashed_at  TEXT,                        -- soft delete (30-day purge reuses startup rule)
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);
CREATE TRIGGER trg_scratch_notes_updated AFTER UPDATE ON scratch_notes
BEGIN UPDATE scratch_notes SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;
