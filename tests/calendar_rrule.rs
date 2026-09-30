//! Recurrence expansion unit tests (spec §10): subset grammar, DST
//! boundary, UNTIL/COUNT, exceptions, iteration cap.
#![allow(clippy::unwrap_used)]

use adjutant::calendar::model::*;
use adjutant::calendar::rrule::{expand, parse_rrule, RruleError};
use chrono::{DateTime, NaiveDate, Utc};

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn timed(start: &str, end: &str, tz: &str, rrule: Option<&str>) -> Event {
    Event {
        id: uuid::Uuid::new_v4(),
        title: "Test".into(),
        description: String::new(),
        location: String::new(),
        all_day: false,
        start_utc: Some(utc(start)),
        end_utc: Some(utc(end)),
        start_date: None,
        end_date: None,
        tz: tz.into(),
        rrule: rrule.map(str::to_string),
        color_idx: 0,
        trashed_at: None,
        created_at: utc("2026-01-01T00:00:00Z"),
        updated_at: utc("2026-01-01T00:00:00Z"),
        attendees: vec![],
        exceptions: vec![],
    }
}

fn all_day(start: &str, end: &str, rrule: Option<&str>) -> Event {
    let mut e = timed("2026-01-01T00:00:00Z", "2026-01-01T01:00:00Z", "UTC", rrule);
    e.all_day = true;
    e.start_utc = None;
    e.end_utc = None;
    e.start_date = Some(NaiveDate::parse_from_str(start, "%Y-%m-%d").unwrap());
    e.end_date = Some(NaiveDate::parse_from_str(end, "%Y-%m-%d").unwrap());
    e
}

fn skip_exception(event: &Event, occurrence: DateTime<Utc>) -> Exception {
    Exception {
        id: uuid::Uuid::new_v4(),
        event_id: event.id,
        occurrence_utc: occurrence,
        kind: ExceptionKind::Skip,
        changes: None,
        created_at: utc("2026-01-01T00:00:00Z"),
        updated_at: utc("2026-01-01T00:00:00Z"),
    }
}

fn modify_exception(
    event: &Event,
    occurrence: DateTime<Utc>,
    changes: OccurrenceChanges,
) -> Exception {
    Exception {
        id: uuid::Uuid::new_v4(),
        event_id: event.id,
        occurrence_utc: occurrence,
        kind: ExceptionKind::Modify,
        changes: Some(changes),
        created_at: utc("2026-01-01T00:00:00Z"),
        updated_at: utc("2026-01-01T00:00:00Z"),
    }
}

fn starts(occs: &[Occurrence]) -> Vec<DateTime<Utc>> {
    occs.iter().map(|o| o.start_utc.unwrap()).collect()
}

// ── FREQ / INTERVAL ────────────────────────────────────────────────────────

#[test]
fn daily_basic_and_window_clipping() {
    let e = timed(
        "2026-03-09T09:00:00Z",
        "2026-03-09T10:00:00Z",
        "UTC",
        Some("FREQ=DAILY"),
    );
    // Window: Mar 11 09:30 → Mar 12 00:00. Mar 11's 09:00-10:00 block
    // intersects; Mar 12's block starts after the window ends.
    let occs = expand(&e, utc("2026-03-11T09:30:00Z"), utc("2026-03-12T00:00:00Z")).unwrap();
    assert_eq!(starts(&occs), vec![utc("2026-03-11T09:00:00Z")]);
}

#[test]
fn daily_interval_three() {
    let e = timed(
        "2026-03-09T09:00:00Z",
        "2026-03-09T09:30:00Z",
        "UTC",
        Some("FREQ=DAILY;INTERVAL=3"),
    );
    let occs = expand(&e, utc("2026-03-01T00:00:00Z"), utc("2026-04-01T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occs),
        vec![
            utc("2026-03-09T09:00:00Z"),
            utc("2026-03-12T09:00:00Z"),
            utc("2026-03-15T09:00:00Z"),
            utc("2026-03-18T09:00:00Z"),
            utc("2026-03-21T09:00:00Z"),
            utc("2026-03-24T09:00:00Z"),
            utc("2026-03-27T09:00:00Z"),
            utc("2026-03-30T09:00:00Z"),
        ]
    );
}

#[test]
fn weekly_byday_monday_wednesday() {
    // 2026-01-05 is a Monday.
    let e = timed(
        "2026-01-05T09:00:00Z",
        "2026-01-05T10:00:00Z",
        "UTC",
        Some("FREQ=WEEKLY;BYDAY=MO,WE"),
    );
    let occs = expand(&e, utc("2026-01-05T00:00:00Z"), utc("2026-01-14T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occs),
        vec![
            utc("2026-01-05T09:00:00Z"),
            utc("2026-01-07T09:00:00Z"),
            utc("2026-01-12T09:00:00Z"),
        ]
    );
}

#[test]
fn weekly_without_byday_defaults_to_start_weekday() {
    // Start is a Wednesday: only Wednesdays recur.
    let e = timed(
        "2026-01-07T09:00:00Z",
        "2026-01-07T10:00:00Z",
        "UTC",
        Some("FREQ=WEEKLY"),
    );
    let occs = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2026-02-01T00:00:00Z")).unwrap();
    assert_eq!(occs.len(), 4); // Jan 7, 14, 21, 28
    assert_eq!(occs[0].start_utc, Some(utc("2026-01-07T09:00:00Z")));
    assert_eq!(occs[3].start_utc, Some(utc("2026-01-28T09:00:00Z")));
}

#[test]
fn weekly_interval_two_skips_weeks() {
    let e = timed(
        "2026-01-05T09:00:00Z",
        "2026-01-05T10:00:00Z",
        "UTC",
        Some("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO"),
    );
    let occs = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2026-02-15T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occs),
        vec![
            utc("2026-01-05T09:00:00Z"),
            utc("2026-01-19T09:00:00Z"),
            utc("2026-02-02T09:00:00Z"),
        ]
    );
}

#[test]
fn monthly_default_day_skips_short_months() {
    // Start Jan 31, monthly: Feb and Apr have no 31st — skipped.
    let e = timed(
        "2026-01-31T09:00:00Z",
        "2026-01-31T10:00:00Z",
        "UTC",
        Some("FREQ=MONTHLY"),
    );
    let occs = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2026-06-01T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occs),
        vec![
            utc("2026-01-31T09:00:00Z"),
            utc("2026-03-31T09:00:00Z"),
            utc("2026-05-31T09:00:00Z"),
        ]
    );
}

#[test]
fn monthly_bymonthday() {
    let e = timed(
        "2026-01-01T09:00:00Z",
        "2026-01-01T10:00:00Z",
        "UTC",
        Some("FREQ=MONTHLY;BYMONTHDAY=1,15"),
    );
    let occs = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2026-03-01T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occs),
        vec![
            utc("2026-01-01T09:00:00Z"),
            utc("2026-01-15T09:00:00Z"),
            utc("2026-02-01T09:00:00Z"),
            utc("2026-02-15T09:00:00Z"),
        ]
    );
}

#[test]
fn monthly_byday_every_monday_of_month() {
    let e = timed(
        "2026-01-01T09:00:00Z",
        "2026-01-01T10:00:00Z",
        "UTC",
        Some("FREQ=MONTHLY;BYDAY=MO"),
    );
    // January 2026 has Mondays on 5, 12, 19, 26.
    let occs = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2026-02-01T00:00:00Z")).unwrap();
    assert_eq!(occs.len(), 4);
    assert_eq!(occs[0].start_utc, Some(utc("2026-01-05T09:00:00Z")));
    assert_eq!(occs[3].start_utc, Some(utc("2026-01-26T09:00:00Z")));
}

#[test]
fn monthly_byday_and_bymonthday_intersect() {
    // RFC 5545: both present → a candidate must satisfy BOTH. In 2026 only
    // the 15th of June falls on a Monday.
    let e = timed(
        "2026-01-01T09:00:00Z",
        "2026-01-01T10:00:00Z",
        "UTC",
        Some("FREQ=MONTHLY;BYDAY=MO;BYMONTHDAY=15"),
    );
    let occs = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2027-01-01T00:00:00Z")).unwrap();
    assert_eq!(starts(&occs), vec![utc("2026-06-15T09:00:00Z")]);
}

#[test]
fn yearly_skips_non_leap_feb_29() {
    let e = timed(
        "2024-02-29T09:00:00Z",
        "2024-02-29T10:00:00Z",
        "UTC",
        Some("FREQ=YEARLY"),
    );
    let occs = expand(&e, utc("2024-01-01T00:00:00Z"), utc("2033-01-01T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occs),
        vec![
            utc("2024-02-29T09:00:00Z"),
            utc("2028-02-29T09:00:00Z"),
            utc("2032-02-29T09:00:00Z"),
        ]
    );
}

// ── UNTIL / COUNT ──────────────────────────────────────────────────────────

#[test]
fn until_bounds_series_inclusive() {
    let e = timed(
        "2026-01-05T09:00:00Z",
        "2026-01-05T10:00:00Z",
        "UTC",
        Some("FREQ=WEEKLY;BYDAY=MO;UNTIL=20260119T090000Z"),
    );
    let occs = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2026-03-01T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occs),
        vec![
            utc("2026-01-05T09:00:00Z"),
            utc("2026-01-12T09:00:00Z"),
            utc("2026-01-19T09:00:00Z"),
        ]
    );
}

#[test]
fn count_stops_series() {
    let e = timed(
        "2026-01-05T09:00:00Z",
        "2026-01-05T10:00:00Z",
        "UTC",
        Some("FREQ=DAILY;COUNT=3"),
    );
    let occs = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2027-01-01T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occs),
        vec![
            utc("2026-01-05T09:00:00Z"),
            utc("2026-01-06T09:00:00Z"),
            utc("2026-01-07T09:00:00Z"),
        ]
    );
}

#[test]
fn count_skips_are_not_counted() {
    // COUNT counts non-excepted occurrences: day 2 is skipped, so the
    // series runs day 1, day 3, day 4.
    let mut e = timed(
        "2026-01-05T09:00:00Z",
        "2026-01-05T10:00:00Z",
        "UTC",
        Some("FREQ=DAILY;COUNT=3"),
    );
    e.exceptions
        .push(skip_exception(&e, utc("2026-01-06T09:00:00Z")));
    let occs = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2027-01-01T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occs),
        vec![
            utc("2026-01-05T09:00:00Z"),
            utc("2026-01-07T09:00:00Z"),
            utc("2026-01-08T09:00:00Z"),
        ]
    );
}

// ── Exceptions ─────────────────────────────────────────────────────────────

#[test]
fn skip_exception_removes_occurrence() {
    let mut e = timed(
        "2026-01-05T09:00:00Z",
        "2026-01-05T10:00:00Z",
        "UTC",
        Some("FREQ=WEEKLY;BYDAY=MO"),
    );
    e.exceptions
        .push(skip_exception(&e, utc("2026-01-12T09:00:00Z")));
    let occs = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2026-02-01T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occs),
        vec![
            utc("2026-01-05T09:00:00Z"),
            utc("2026-01-19T09:00:00Z"),
            utc("2026-01-26T09:00:00Z"),
        ]
    );
}

#[test]
fn modify_exception_overrides_fields() {
    let mut e = timed(
        "2026-01-05T09:00:00Z",
        "2026-01-05T10:00:00Z",
        "UTC",
        Some("FREQ=WEEKLY;BYDAY=MO"),
    );
    e.exceptions.push(modify_exception(
        &e,
        utc("2026-01-12T09:00:00Z"),
        OccurrenceChanges {
            title: Some("Moved standup".into()),
            start_utc: Some(utc("2026-01-12T11:00:00Z")),
            end_utc: Some(utc("2026-01-12T12:00:00Z")),
            tz: None,
            location: Some("Room 2".into()),
            description: None,
        },
    ));
    let occs = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2026-01-20T00:00:00Z")).unwrap();
    assert_eq!(occs.len(), 3); // Jan 5, 12 (modified), 19
    assert_eq!(occs[1].start_utc, Some(utc("2026-01-12T11:00:00Z")));
    assert_eq!(occs[1].end_utc, Some(utc("2026-01-12T12:00:00Z")));
    assert_eq!(occs[1].title, "Moved standup");
    assert_eq!(occs[1].location, "Room 2");
    assert_eq!(occs[0].start_utc, Some(utc("2026-01-05T09:00:00Z")));
    assert_eq!(occs[2].start_utc, Some(utc("2026-01-19T09:00:00Z")));
}

// ── DST (Europe/London 2026 transitions) ───────────────────────────────────

#[test]
fn dst_wall_clock_anchor_across_spring_forward() {
    // Weekly Sundays 09:00 Europe/London: 2026-03-22 is GMT (09:00 UTC),
    // 2026-03-29 is BST (08:00 UTC) — local wall clock holds, UTC shifts.
    let e = timed(
        "2026-03-22T09:00:00Z",
        "2026-03-22T10:00:00Z",
        "Europe/London",
        Some("FREQ=WEEKLY;BYDAY=SU"),
    );
    let occs = expand(&e, utc("2026-03-01T00:00:00Z"), utc("2026-04-15T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occs),
        vec![
            utc("2026-03-22T09:00:00Z"),
            utc("2026-03-29T08:00:00Z"),
            utc("2026-04-05T08:00:00Z"),
            utc("2026-04-12T08:00:00Z"),
        ]
    );
}

#[test]
fn dst_gap_shifts_forward_to_next_valid_local_time() {
    // Daily 01:30 London: on 2026-03-29 local times 01:00-01:59 do not
    // exist (clocks jump to 02:00 BST), so the occurrence lands at 02:00
    // local = 01:00 UTC.
    let e = timed(
        "2026-03-28T01:30:00Z",
        "2026-03-28T02:00:00Z",
        "Europe/London",
        Some("FREQ=DAILY"),
    );
    let occs = expand(&e, utc("2026-03-27T00:00:00Z"), utc("2026-03-31T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occs),
        vec![
            utc("2026-03-28T01:30:00Z"),
            utc("2026-03-29T01:00:00Z"),
            utc("2026-03-30T00:30:00Z"),
        ]
    );
}

#[test]
fn dst_overlap_takes_first_occurrence() {
    // Daily 01:30 London: on 2026-10-25 local 01:00-01:59 happens twice
    // (clocks fall back at 01:00 UTC); the first (01:30 BST = 00:30 UTC)
    // wins.
    let e = timed(
        "2026-10-23T00:30:00Z",
        "2026-10-23T01:00:00Z",
        "Europe/London",
        Some("FREQ=DAILY"),
    );
    let occs = expand(&e, utc("2026-10-22T00:00:00Z"), utc("2026-10-27T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occs),
        vec![
            utc("2026-10-23T00:30:00Z"),
            utc("2026-10-24T00:30:00Z"),
            utc("2026-10-25T00:30:00Z"),
            utc("2026-10-26T01:30:00Z"),
        ]
    );
}

// ── All-day ────────────────────────────────────────────────────────────────

#[test]
fn all_day_daily_pure_date_arithmetic() {
    let e = all_day("2026-01-05", "2026-01-05", Some("FREQ=DAILY"));
    let occs = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2026-01-10T00:00:00Z")).unwrap();
    assert_eq!(occs.len(), 5);
    assert_eq!(
        occs[0].start_date,
        Some(NaiveDate::from_ymd_opt(2026, 1, 5).unwrap())
    );
    assert_eq!(
        occs[4].start_date,
        Some(NaiveDate::from_ymd_opt(2026, 1, 9).unwrap())
    );
}

#[test]
fn all_day_multi_day_spans_window_edge() {
    // Edge 7: a 3-day event is included when its [start,end] date span
    // intersects the window, even if the window starts mid-event.
    let e = all_day("2026-01-10", "2026-01-12", None);
    let before = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2026-01-09T23:00:00Z")).unwrap();
    assert!(before.is_empty());
    let mid = expand(&e, utc("2026-01-11T00:00:00Z"), utc("2026-01-20T00:00:00Z")).unwrap();
    assert_eq!(mid.len(), 1);
    assert_eq!(
        mid[0].start_date,
        Some(NaiveDate::from_ymd_opt(2026, 1, 10).unwrap())
    );
    assert_eq!(
        mid[0].end_date,
        Some(NaiveDate::from_ymd_opt(2026, 1, 12).unwrap())
    );
}

// ── Non-recurring / errors ─────────────────────────────────────────────────

#[test]
fn non_recurring_respects_window() {
    let e = timed("2026-01-05T09:00:00Z", "2026-01-05T10:00:00Z", "UTC", None);
    let in_win = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2026-01-06T00:00:00Z")).unwrap();
    assert_eq!(in_win.len(), 1);
    let out = expand(&e, utc("2026-01-06T00:00:00Z"), utc("2026-01-07T00:00:00Z")).unwrap();
    assert!(out.is_empty());
}

#[test]
fn malformed_rrules_are_typed_errors() {
    let cases = [
        ("", RruleError::Empty),
        (
            "FREQ=FORTNIGHTLY",
            RruleError::UnknownFreq("FORTNIGHTLY".into()),
        ),
        (
            "FREQ=DAILY;INTERVAL=zero",
            RruleError::InvalidInterval("zero".into()),
        ),
        (
            "FREQ=DAILY;INTERVAL=0",
            RruleError::InvalidInterval("0".into()),
        ),
        (
            "FREQ=WEEKLY;BYDAY=XX",
            RruleError::InvalidByday("XX".into()),
        ),
        (
            "FREQ=MONTHLY;BYMONTHDAY=32",
            RruleError::InvalidBymonthday("32".into()),
        ),
        ("FREQ=DAILY;COUNT=-1", RruleError::InvalidCount("-1".into())),
        (
            "FREQ=DAILY;UNTIL=notadate",
            RruleError::InvalidUntil("notadate".into()),
        ),
        (
            "FREQ=DAILY;FREQ=WEEKLY",
            RruleError::DuplicateKey("FREQ".into()),
        ),
        ("FREQ=DAILY;BOGUS=1", RruleError::UnknownKey("BOGUS".into())),
        (
            "FREQ=WEEKLY;WKST=SU",
            RruleError::UnsupportedWkst("SU".into()),
        ),
    ];
    for (text, expected) in cases {
        let got = parse_rrule(text).unwrap_err();
        assert_eq!(got, expected, "for {text:?}");
    }
}

#[test]
fn iteration_cap_runaway_is_an_error_not_a_hang() {
    let e = timed(
        "2000-01-01T09:00:00Z",
        "2000-01-01T10:00:00Z",
        "UTC",
        Some("FREQ=DAILY"),
    );
    let err = expand(&e, utc("2100-01-01T00:00:00Z"), utc("2100-02-01T00:00:00Z")).unwrap_err();
    assert_eq!(err, RruleError::IterationCap);
}

#[test]
fn expand_errors_on_unknown_timezone() {
    let e = timed(
        "2026-01-05T09:00:00Z",
        "2026-01-05T10:00:00Z",
        "Mars/Olympus_Mons",
        Some("FREQ=DAILY"),
    );
    let err = expand(&e, utc("2026-01-01T00:00:00Z"), utc("2026-02-01T00:00:00Z")).unwrap_err();
    assert!(matches!(err, RruleError::UnknownTz(_)));
}
