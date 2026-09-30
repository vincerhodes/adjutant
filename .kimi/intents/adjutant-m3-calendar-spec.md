# Adjutant — Technical Spec (M3: Calendar)

Source: `.kimi/intents/adjutant-m3-calendar-intent.md` (normative for decisions). Builds on M1/M2 layout + design language. Zero changes to existing migrations — all schema via `migrations/0003_calendar.sql`. `EntityType::Event` and `EntityType::Reminder` already exist in `src/core/entity.rs` (`src/core/entity.rs:14-22`) — no link-layer change needed; links to todos/emails work today via `LinkStore` (`src/core/link.rs`).

## 1. Dependencies (additions)

| Crate | Version | Purpose |
|---|---|---|
| chrono-tz | 0.10 | IANA timezone database (chrono 0.4 already present; chrono-tz is the standard pairing — chrono alone has no TZ names). Used for: recurrence expansion, wall-clock display, DST-correct occurrence math |
| notify-rust | 4 | XDG desktop notifications (dbus/zbus, Wayland-safe; degrades gracefully if no daemon — error is swallowed to in-app-only). Default features; NO tokio/async runtime |

No other new deps. No tokio (app-wide rule). RRULE expansion is hand-rolled over the small subset in §5 — no rrule-parser crate (all existing options pull heavy deps or are stale; the subset is ~150 lines).

## 2. Module layout (additions, dependency rules unchanged)

```
src/calendar/
├── mod.rs            # CalendarStore: SQL CRUD, window queries, trash, reminder queries
├── model.rs          # Event, EventInput, Recurrence, Attendee, Reminder, Exception, enums
├── rrule.rs          # pure functions: parse RRULE subset → expand occurrences in [start,end) window
├── reminders.rs      # ReminderScheduler: due-scan, catch-up, snooze/dismiss state machine
├── notify.rs         # Notifier trait + notify-rust live impl (test seam, mock in tests)
└── ui/
    ├── mod.rs        # CalendarUi state + routing (view switcher)
    ├── dashboard.rs  # combined: upcoming-5 strip + week grid
    ├── week.rs       # week grid (day columns, timed blocks, all-day band)
    ├── upcoming.rs   # upcoming list view
    ├── event_form.rs # create/edit modal: timed/all-day, recurrence, attendees, reminders, TZ
    └── reminder_banner.rs  # due-reminders banner (dismiss/snooze)
tests/
├── calendar_store.rs     # in-memory DB: CRUD, trash/purge, reminder state machine
├── calendar_rrule.rs     # expansion unit tests: subset grammar, DST boundary, UNTIL/COUNT
└── calendar_ui.rs        # kittest: dashboard layout, view switcher, event form, banner
```

Dependency direction strictly **UI → module → core/db**. `core/` and `db/` never import `calendar/`. Zero SQL outside `mod.rs` files.

## 3. Schema — migrations/0003_calendar.sql

```sql
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
  UNIQUE (event_id, email)
);
CREATE TRIGGER trg_calendar_attendees_updated AFTER UPDATE ON calendar_attendees
BEGIN UPDATE calendar_attendees SET updated_at = ... END;  -- (updated_at column included; trigger verbatim pattern as above)

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
  UNIQUE (reminder_id, occurrence_utc)
);
CREATE TRIGGER trg_calendar_reminder_fires_updated AFTER UPDATE ON calendar_reminder_fires
BEGIN UPDATE calendar_reminder_fires SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END;
```

Timestamps: ISO-8601 UTC text (`chrono::Utc::now().to_rfc3339()`), matching M2. Every calendar table has `updated_at` + trigger — M5 last-write-wins reads these. `trashed_at` soft delete keeps link rows resolvable; hard delete purges links via `LinkStore::delete_links_for` in the same transaction (todo pattern).

## 4. CalendarStore API surface (src/calendar/mod.rs)

```rust
impl CalendarStore<'_> {
    // CRUD
    fn create_event(&self, input: &EventInput) -> Result<Uuid>;      // tx: event + attendees
    fn update_event(&self, id: Uuid, input: &EventInput) -> Result<()>;
    fn trash_event(&self, id: Uuid) -> Result<()>;                   // soft
    fn restore_event(&self, id: Uuid) -> Result<()>;
    fn delete_event(&self, id: Uuid) -> Result<()>;                  // hard + link purge, same tx

    // Window queries (expansion happens in Rust, §5 — SQL returns base rows only)
    fn events_in_window(&self, from_utc: DateTime<Utc>, to_utc: DateTime<Utc>) -> Result<Vec<Event>>; // excludes trashed, includes recurring base rows
    fn upcoming_events(&self, now: DateTime<Utc>, limit: usize) -> Result<Vec<Event>>;                // next N occurrences across all events (default 5)

    // Recurring edits
    fn add_exception(&self, event_id: Uuid, occurrence: DateTime<Utc>, kind: ExceptionKind) -> Result<()>;
    fn modify_occurrence(&self, event_id: Uuid, occurrence: DateTime<Utc>, changes: &OccurrenceChanges) -> Result<()>;
    fn edit_future(&self, event_id: Uuid, from_occurrence: DateTime<Utc>, input: &EventInput) -> Result<Uuid>; // series-split: UNTIL the occurrence before, clone remainder as new event; returns new id

    // Reminders
    fn reminders_for_event(&self, event_id: Uuid) -> Result<Vec<Reminder>>;
    fn add_reminder(&self, event_id: Uuid, offset_minutes: i64) -> Result<Uuid>;
    fn remove_reminder(&self, id: Uuid) -> Result<()>;
    fn due_reminders(&self, now: DateTime<Utc>) -> Result<Vec<DueReminder>>;          // pending fire/snooled, not dismissed
    fn mark_fired(&self, fire_id: Uuid, at: DateTime<Utc>) -> Result<()>;
    fn snooze_fire(&self, fire_id: Uuid, to: DateTime<Utc>) -> Result<()>;
    fn dismiss_fire(&self, fire_id: Uuid) -> Result<()>;
    fn catch_up_due(&self, now: DateTime<Utc>, horizon: Duration) -> Result<Vec<DueReminder>>; // missed fires in [now-horizon, now], once each
}
```

## 5. Recurrence (src/calendar/rrule.rs) — DECIDED: expand on read

**Store the RRULE text; expand occurrences in Rust for the query window. No materialized occurrence table in M3.**

Justification: local single-writer SQLite; query windows are small (week view = 7 days, upcoming = 5 items, month stretch = 31 days). Expansion is a few hundred iterations worst case; materializing would add a table to invalidate on every edit, bloat the DB for unbounded series, and complicate M5 conflict merges. Expansion cost is measured once, cache invalidation is measured forever.

Grammar (subset — parse failures are a form error, never a panic):
`FREQ=DAILY|WEEKLY|MONTHLY|YEARLY;INTERVAL=<n>;BYDAY=MO,TU,...;BYMONTHDAY=1,15,...;UNTIL=<UTC instant>|COUNT=<n>;WKST=MO`

Rules:
- Expansion is done in the event's `tz` wall-clock space, then converted to UTC. An event at 09:00 Europe/London recurs at 09:00 local across DST — never 09:00 UTC arithmetic. (This is the DST rule: **expand in IANA TZ, store/compare in UTC**.) All-day events expand in pure date arithmetic (`start_date` + n days).
- Window scan: if non-recurring, row intersects window or not. If recurring, generate candidate occurrences from series start forward with an iteration cap (10,000 — a runaway RRULE yields an error surfaced in UI, not a hang), filter to window, apply exceptions (`skip` removes; `modify` overrides fields).
- `UNTIL` compares occurrence start in UTC; `COUNT` counts non-excepted generated occurrences.
- Exceptions match by exact occurrence start instant (`occurrence_utc`), stored at edit time.

**Edit semantics** (event form offers the choice when RRULE present):
- **This occurrence** → row in `calendar_event_exceptions`: `skip` + a `modify` exception if fields changed (title/time/location/description). Series untouched.
- **This and future** → series split (`edit_future`): original event gets `UNTIL = occurrence - 1` (computed in TZ wall-clock); a NEW event (new uuid, same RRULE, new `start_utc` = modified occurrence) carries the changes forward. Links on the original stay on the original — new series is a new entity.
- **Entire series** → plain `update_event` on the base row. Existing per-occurrence exceptions are kept if their `occurrence_utc` still exists in the new series, else pruned.

## 6. Reminders & notification (src/calendar/reminders.rs, notify.rs)

- Scheduling is **app-session based, tick-driven**: `ReminderScheduler::tick(now)` called from `app.rs` `update()` every ~15s (plus on launch). No background process — app closed = reminders queue up as rows.
- Per occurrence: fire instant = `occurrence_utc - offset_minutes`. On tick, `due_reminders(now)` returns pending fires (incl. snoozed-to) → for each: `Notifier::send(title, body)` (notify-rust live impl; mock in tests), in-app toast via existing toast system, `mark_fired`. Notify failure (no daemon) is logged and swallowed — in-app banner still shows.
- **Catch-up on launch**: `catch_up_due(now, 24h)` — fires missed while closed, bounded to last 24h, exactly once each (ledger row). Snoozed fires persist across restarts (row survives; `snoozed_to_utc` in the past → fires).
- **In-app banner**: due/today fires listed in a banner strip at top of calendar view: event title, relative fire time, Dismiss (ghost) / Snooze (dropdown 5/15/60 min). Snooze re-fires in-app only (desktop notification once per occurrence, at first fire).
- Recurring events: ledger dedupe is per (reminder_id, occurrence_utc) — a daily event's 09:00 reminder fires daily, never twice for one occurrence.
- `Notifier` trait = the test seam. No live network anywhere; notify-rust is local dbus only, and tests never touch it (mock).

## 7. UI (design language per M1.5/§8)

- Module nav: **Calendar becomes enabled** (`Module::Calendar` wired in `src/app.rs` like Email). Placeholder removed.
- **Default = dashboard**: top strip lists next 5 upcoming occurrences (date chip, time or "All day", title, location) as compact cards (`ui::card_frame`); beneath it the week grid (7 day columns, all-day band on top, timed blocks stacked by hour, now-line). If window height is short the whole column scrolls — upcoming strip scrolls away, week stays reachable. View switcher (segmented control, palette semantics): Dashboard / Week / Upcoming. Month: stretch only.
- Event rendering: color-dot per `color_idx` (same 6-color cycle as account/group dots), title, time range; all-day events sit in the day-band, multi-day spilling across columns. Past occurrences render dimmed via existing palette weak/text-dim colors — no new literals.
- Event form (modal): title, location, description; timed/all-day toggle (all-day swaps datetime pickers for date pickers); TZ picker (IANA dropdown, default system TZ, "floating" not offered); recurrence section (frequency/interval/by-day/until/count, "Custom" collapses to RRULE text shown read-only); attendees (name+email rows, RSVP select); reminders (offset rows + add); color tag. Save primary / Cancel ghost / Trash (danger ghost, existing pattern).
- Event click → detail card in pane: fields, linked entities (existing link layer UI patterns — todo/email pickers gain calendar entries), "Link reminder to…" via link picker (relation vocabulary: reminders link to todos/emails with existing relations, e.g. `scheduled_as`/`mentions`).
- F1 overlay additions for calendar shortcuts. Empty states designed (`ui::empty_state`). All colors from palette; zero new color literals; painter icons from `ui::icons` only — never font glyphs.

## 8. Edge cases

1. **DST gap/overlap**: occurrence landing in a spring-forward gap (02:30 local) shifts forward to 03:00 local (chrono-tz `from_local_datetime` handling, single-sided); overlap (autumn) takes the first occurrence. Documented behavior, unit-tested on Europe/London 2026-03-29.
2. **Recurring event edited while standing in a different TZ**: `occurrence_utc` matching is exact-instant — the edit dialog identifies occurrences by their original UTC start, stable regardless of viewer TZ.
3. **Event end ≤ start** → form validation error (incl. all-day `end_date < start_date`), save blocked.
4. **Trash + recurrence**: trashing a recurring event trashes the series (exceptions included via cascade); restore brings all back.
5. **Reminder on trashed event**: not scheduled (`trashed_at IS NULL` filter in every reminder query).
6. **Clock skew / back-in-time `updated_at`**: irrelevant locally; M5 conflict resolution reads triggers, no M3 logic depends on monotonicity beyond display.
7. **All-day crossing window edge**: `events_in_window` includes all-day events whose `[start_date, end_date]` intersects the window's local-date span (dates compared as text — `YYYY-MM-DD` lexicographic order is chronological).

## 9. Security

- No secrets anywhere in M3 (no accounts, no network). notify-rust is local dbus only.
- DB perms already 0600/0700. `quality_reports/` untouched, never committed.

## 10. Test strategy

- `tests/calendar_store.rs` — in-memory: CRUD, CHECK constraint violations (timed/all-day shape), trash/restore/purge, exception CRUD + uniqueness, reminder state machine (fire/snooze/dismiss/re-fire), catch-up exactly-once with injected clock.
- `tests/calendar_rrule.rs` — expansion: each FREQ, INTERVAL, BYDAY/BYMONTHDAY, UNTIL vs COUNT, window clipping, exception skip/modify, series-split (`edit_future`), DST boundary (Europe/London 2026 transitions), iteration-cap runaway, malformed RRULE → typed error.
- `tests/calendar_ui.rs` — kittest: dashboard layout + view switcher, event form validation, reminder banner dismiss/snooze, link picker gains calendar entries.
- Reminder tick tests use injected `now` + mock `Notifier` — zero live network, zero real notifications (assert on mock call log).
