CREATE TABLE calendar_events (
  id            TEXT PRIMARY KEY,            -- app uuid
  title         TEXT NOT NULL DEFAULT '',
  description   TEXT NOT NULL DEFAULT '',
  location      TEXT NOT NULL DEFAULT '',
  all_day       INTEGER NOT NULL DEFAULT 0,
  -- Timed events: absolute instants, ISO-8601 UTC.
  start_utc     TEXT,                        -- NULL iff all_day
  end_utc       TEXT,                        -- NULL iff all_day
  -- All-day events: local dates, 'YYYY-MM-DD'. CHECK enforces exactly one shape.
  start_date    TEXT,
  end_date      TEXT,
  tz            TEXT NOT NULL DEFAULT 'UTC', -- IANA name (e.g. 'Europe/London'); wall-clock anchor for
                                             -- display + recurrence expansion. UTC for all-day (dates are tz-free)
  rrule         TEXT,                        -- NULL = non-recurring; iCalendar RRULE subset text, e.g.
                                             -- 'FREQ=WEEKLY;INTERVAL=1;BYDAY=MO,WE;UNTIL=20260701T000000Z'
  color_idx     INTEGER NOT NULL DEFAULT 0,  -- 6-color cycle, matches group dots
  trashed_at    TEXT,                        -- soft delete (30-day purge reuses startup rule)
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  CHECK ((all_day = 0 AND start_utc IS NOT NULL AND end_utc IS NOT NULL AND start_date IS NULL AND end_date IS NULL)
      OR (all_day = 1 AND start_date IS NOT NULL AND end_date IS NOT NULL AND start_utc IS NULL AND end_utc IS NULL))
);
CREATE INDEX idx_calendar_events_start ON calendar_events(start_utc);
CREATE INDEX idx_calendar_events_window ON calendar_events(rrule, start_utc);  -- recurring scan hint
CREATE TRIGGER trg_calendar_events_updated AFTER UPDATE ON calendar_events
BEGIN UPDATE calendar_events SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;

CREATE TABLE calendar_attendees (            -- separate table (not JSON) so M5 sync can diff/merge rows
  id          TEXT PRIMARY KEY,
  event_id    TEXT NOT NULL REFERENCES calendar_events(id) ON DELETE CASCADE,
  name        TEXT NOT NULL DEFAULT '',
  email       TEXT NOT NULL,
  rsvp        TEXT NOT NULL DEFAULT 'needs_action'
              CHECK (rsvp IN ('needs_action','accepted','declined','tentative')),
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  UNIQUE (event_id, email)
);
CREATE TRIGGER trg_calendar_attendees_updated AFTER UPDATE ON calendar_attendees
BEGIN UPDATE calendar_attendees SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;

-- Per-occurrence edits/cancels of recurring events ("this occurrence").
CREATE TABLE calendar_event_exceptions (
  id              TEXT PRIMARY KEY,
  event_id        TEXT NOT NULL REFERENCES calendar_events(id) ON DELETE CASCADE,
  occurrence_utc  TEXT NOT NULL,             -- original occurrence start (UTC) the exception applies to
  kind            TEXT NOT NULL CHECK (kind IN ('skip','modify')),
  changes_json    TEXT,                      -- kind='modify': JSON of changed fields
                                             -- {title?,start_utc?,end_utc?,tz?,location?,description?}
                                             -- kind='skip': NULL (occurrence cancelled)
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL,
  UNIQUE (event_id, occurrence_utc)
);
CREATE TRIGGER trg_calendar_event_exceptions_updated AFTER UPDATE ON calendar_event_exceptions
BEGIN UPDATE calendar_event_exceptions SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;

CREATE TABLE calendar_reminders (
  id               TEXT PRIMARY KEY,
  event_id         TEXT NOT NULL REFERENCES calendar_events(id) ON DELETE CASCADE,
  offset_minutes   INTEGER NOT NULL DEFAULT 10,  -- fire offset_minutes BEFORE occurrence start
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL
);
CREATE TRIGGER trg_calendar_reminders_updated AFTER UPDATE ON calendar_reminders
BEGIN UPDATE calendar_reminders SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;

-- Deduplication ledger: one row per (reminder, occurrence) fired. Drives "exactly once"
-- per occurrence, catch-up-on-launch, and snooze re-fire (snooze deletes/rewrites a pending row).
CREATE TABLE calendar_reminder_fires (
  id              TEXT PRIMARY KEY,
  reminder_id     TEXT NOT NULL REFERENCES calendar_reminders(id) ON DELETE CASCADE,
  occurrence_utc  TEXT NOT NULL,             -- occurrence start the reminder belongs to
  fire_at_utc     TEXT NOT NULL,             -- scheduled fire instant (occurrence - offset)
  fired_at        TEXT,                      -- NULL = pending (snoozed-to or catch-up queued)
  snoozed_to_utc  TEXT,                      -- pending snooze target; overrides fire_at_utc when set
  dismissed       INTEGER NOT NULL DEFAULT 0,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL,
  UNIQUE (reminder_id, occurrence_utc)
);
CREATE TRIGGER trg_calendar_reminder_fires_updated AFTER UPDATE ON calendar_reminder_fires
BEGIN UPDATE calendar_reminder_fires SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;
