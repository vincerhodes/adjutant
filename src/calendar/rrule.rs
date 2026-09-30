//! Recurrence expansion (spec §5): RRULE subset parser + occurrence
//! expansion in the event's IANA timezone wall-clock space, converted to
//! UTC for storage/comparison.
//!
//! Design decisions (documented per plan risk 12):
//! - **BYDAY ∩ BYMONTHDAY (RFC 5545):** when both are present a candidate
//!   day must satisfy BOTH filters (weekday ∈ BYDAY AND day-of-month ∈
//!   BYMONTHDAY).
//! - **Defaults:** WEEKLY without BYDAY recurs on DTSTART's weekday;
//!   MONTHLY/YEARLY without BYDAY *and* without BYMONTHDAY recurs on
//!   DTSTART's day-of-month (invalid in a given month/year → that period
//!   yields nothing). BYDAY on MONTHLY means "every matching weekday of
//!   the month" (no ordinals in the subset).
//! - **DST:** expansion is done in the event's TZ wall-clock space, then
//!   converted to UTC. Spring-forward gaps shift forward to the next valid
//!   local time; autumn overlaps take the first (earlier) occurrence.
//! - **COUNT** counts non-excepted generated occurrences (skips do not
//!   consume count). **UNTIL** compares occurrence start in UTC.
//! - **Iteration cap:** 10,000 candidate evaluations from series start —
//!   a runaway RRULE is a typed error surfaced in the UI, never a hang.
//!
//! Pure functions over chrono + chrono-tz only; no I/O.

use std::collections::{HashMap, HashSet};

use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc, Weekday,
};
use chrono_tz::Tz;
use thiserror::Error as ThisError;

use super::model::{Event, ExceptionKind, Freq, Occurrence, OccurrenceChanges, Recurrence};

/// Candidate-evaluation cap: generation from series start stops (with a
/// typed error) after this many evaluated days. Spec §5.
pub const ITERATION_CAP: usize = 10_000;

#[derive(Debug, ThisError, PartialEq, Eq)]
pub enum RruleError {
    #[error("empty RRULE")]
    Empty,
    #[error("missing FREQ")]
    MissingFreq,
    #[error("unknown FREQ: {0}")]
    UnknownFreq(String),
    #[error("invalid INTERVAL: {0} (expected a positive integer)")]
    InvalidInterval(String),
    #[error("invalid BYDAY value: {0}")]
    InvalidByday(String),
    #[error("invalid BYMONTHDAY value: {0} (expected 1..=31)")]
    InvalidBymonthday(String),
    #[error("invalid UNTIL: {0} (expected YYYYMMDDTHHMMSSZ)")]
    InvalidUntil(String),
    #[error("invalid COUNT: {0} (expected a positive integer)")]
    InvalidCount(String),
    #[error("unsupported WKST: {0} (subset is MO only)")]
    UnsupportedWkst(String),
    #[error("duplicate RRULE key: {0}")]
    DuplicateKey(String),
    #[error("unknown RRULE key: {0}")]
    UnknownKey(String),
    #[error("malformed RRULE part: {0}")]
    InvalidPart(String),
    #[error("unknown timezone: {0}")]
    UnknownTz(String),
    #[error("invalid event: {0}")]
    InvalidEvent(String),
    #[error("recurrence exceeded the {ITERATION_CAP} iteration cap")]
    IterationCap,
}

pub type Result<T> = std::result::Result<T, RruleError>;

/// Parse the RRULE subset: `FREQ=DAILY|WEEKLY|MONTHLY|YEARLY;INTERVAL=<n>;
/// BYDAY=MO,TU,...;BYMONTHDAY=1,15,...;UNTIL=<UTC instant>;COUNT=<n>;WKST=MO`.
pub fn parse_rrule(text: &str) -> Result<Recurrence> {
    let text = text.trim();
    if text.is_empty() {
        return Err(RruleError::Empty);
    }
    let mut freq: Option<Freq> = None;
    let mut interval: u32 = 1;
    let mut byday: Vec<Weekday> = Vec::new();
    let mut bymonthday: Vec<i32> = Vec::new();
    let mut until: Option<DateTime<Utc>> = None;
    let mut count: Option<u32> = None;
    let mut wkst: Weekday = Weekday::Mon;

    for part in text.split(';') {
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| RruleError::InvalidPart(part.to_string()))?;
        let key = key.trim().to_ascii_uppercase();
        let value = value.trim();
        match key.as_str() {
            "FREQ" => {
                if freq.is_some() {
                    return Err(RruleError::DuplicateKey(key));
                }
                freq = Some(match value {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => return Err(RruleError::UnknownFreq(value.to_string())),
                });
            }
            "INTERVAL" => {
                let n: u32 = value
                    .parse()
                    .map_err(|_| RruleError::InvalidInterval(value.to_string()))?;
                if n == 0 {
                    return Err(RruleError::InvalidInterval(value.to_string()));
                }
                interval = n;
            }
            "BYDAY" => {
                byday = value
                    .split(',')
                    .map(|d| {
                        parse_weekday(d).ok_or_else(|| RruleError::InvalidByday(d.to_string()))
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()?;
            }
            "BYMONTHDAY" => {
                bymonthday = value
                    .split(',')
                    .map(|d| {
                        let n: i32 = d
                            .parse()
                            .map_err(|_| RruleError::InvalidBymonthday(d.to_string()))?;
                        if !(1..=31).contains(&n) {
                            return Err(RruleError::InvalidBymonthday(d.to_string()));
                        }
                        Ok(n)
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()?;
            }
            "UNTIL" => {
                until = Some(parse_until(value)?);
            }
            "COUNT" => {
                let n: u32 = value
                    .parse()
                    .map_err(|_| RruleError::InvalidCount(value.to_string()))?;
                if n == 0 {
                    return Err(RruleError::InvalidCount(value.to_string()));
                }
                count = Some(n);
            }
            "WKST" => {
                let wd = parse_weekday(value)
                    .ok_or_else(|| RruleError::UnsupportedWkst(value.to_string()))?;
                if wd != Weekday::Mon {
                    return Err(RruleError::UnsupportedWkst(value.to_string()));
                }
                wkst = wd;
            }
            _ => return Err(RruleError::UnknownKey(key)),
        }
    }

    let freq = freq.ok_or(RruleError::MissingFreq)?;
    Ok(Recurrence {
        freq,
        interval,
        byday,
        bymonthday,
        until,
        count,
        wkst,
    })
}

/// Canonical serialization (used by series-split in the store). Round-trips
/// through [`parse_rrule`].
pub fn format_rrule(rule: &Recurrence) -> String {
    let mut parts = vec![format!(
        "FREQ={}",
        match rule.freq {
            Freq::Daily => "DAILY",
            Freq::Weekly => "WEEKLY",
            Freq::Monthly => "MONTHLY",
            Freq::Yearly => "YEARLY",
        }
    )];
    if rule.interval != 1 {
        parts.push(format!("INTERVAL={}", rule.interval));
    }
    if !rule.byday.is_empty() {
        let days: Vec<&str> = rule.byday.iter().map(|d| weekday_code(*d)).collect();
        parts.push(format!("BYDAY={}", days.join(",")));
    }
    if !rule.bymonthday.is_empty() {
        let days: Vec<String> = rule.bymonthday.iter().map(|d| d.to_string()).collect();
        parts.push(format!("BYMONTHDAY={}", days.join(",")));
    }
    if let Some(u) = rule.until {
        parts.push(format!("UNTIL={}", u.format("%Y%m%dT%H%M%SZ")));
    }
    if let Some(c) = rule.count {
        parts.push(format!("COUNT={c}"));
    }
    if rule.wkst != Weekday::Mon {
        parts.push(format!("WKST={}", weekday_code(rule.wkst)));
    }
    parts.join(";")
}

/// Expand `event` into concrete occurrences intersecting
/// `[window_from, window_to)`. Non-recurring events yield at most one.
/// Exceptions on the event are applied: `Skip` removes the occurrence,
/// `Modify` overrides fields. Never panics — malformed input is a typed
/// error.
pub fn expand(
    event: &Event,
    window_from: DateTime<Utc>,
    window_to: DateTime<Utc>,
) -> Result<Vec<Occurrence>> {
    if window_from >= window_to {
        return Ok(Vec::new());
    }
    let tz: Tz = event
        .tz
        .parse()
        .map_err(|_| RruleError::UnknownTz(event.tz.clone()))?;
    match &event.rrule {
        None => Ok(expand_single(event, window_from, window_to, &tz)),
        Some(text) => {
            let rule = parse_rrule(text)?;
            expand_recurring(event, &rule, window_from, window_to, &tz)
        }
    }
}

/// Convert a wall-clock local time to UTC with documented DST handling:
/// gaps shift forward to the next valid local time; overlaps take the first
/// (earlier) occurrence.
///
/// chrono-tz resolves an ambiguous (overlap) local time to the LATER
/// offset, so we additionally walk backwards minute-by-minute while the
/// instant still maps to the same wall clock — the earliest such instant
/// is the first occurrence.
pub fn local_to_utc(tz: &Tz, naive: NaiveDateTime) -> Result<DateTime<Utc>> {
    let primary = match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt.with_timezone(&Utc),
        LocalResult::Ambiguous(early, _) => return Ok(early.with_timezone(&Utc)),
        LocalResult::None => {
            // Spring-forward gap: probe forward minute-by-minute (gaps are
            // at most a couple of hours).
            let mut probe = naive;
            let mut resolved: Option<DateTime<Utc>> = None;
            for _ in 0..240 {
                probe += Duration::minutes(1);
                match tz.from_local_datetime(&probe) {
                    LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => {
                        resolved = Some(dt.with_timezone(&Utc));
                        break;
                    }
                    LocalResult::None => {}
                }
            }
            return resolved.ok_or_else(|| {
                RruleError::InvalidEvent(format!("no valid local time within gap at {naive}"))
            });
        }
    };
    let mut first = primary;
    let mut probe = primary - Duration::minutes(1);
    while primary - probe <= Duration::hours(3) {
        if probe.with_timezone(tz).naive_local() == naive {
            first = probe;
        }
        probe -= Duration::minutes(1);
    }
    Ok(first)
}

/// The instant one recurrence period before `instant`, computed in the TZ
/// wall-clock space (series-split UNTIL boundary). Month/year steps clamp
/// the day-of-month (Jan 31 → one month → Dec 31; Mar 31 → Feb 28/29).
pub fn prev_in_wall_clock(
    tz: &Tz,
    rule: &Recurrence,
    instant: DateTime<Utc>,
) -> Result<DateTime<Utc>> {
    let naive = instant.with_timezone(tz).naive_local();
    let steps = i64::from(rule.interval);
    let prev = match rule.freq {
        Freq::Daily => naive.checked_sub_signed(Duration::days(steps)),
        Freq::Weekly => naive.checked_sub_signed(Duration::weeks(steps)),
        Freq::Monthly => shift_months_clamped(naive, -steps),
        Freq::Yearly => shift_months_clamped(naive, -steps * 12),
    }
    .ok_or_else(|| RruleError::InvalidEvent("date out of range".into()))?;
    local_to_utc(tz, prev)
}

fn shift_months_clamped(naive: NaiveDateTime, months: i64) -> Option<NaiveDateTime> {
    let date = naive.date();
    let total = i64::from(date.year()) * 12 + i64::from(date.month0()) + months;
    let year = total.div_euclid(12);
    let month0 = total.rem_euclid(12) as u32;
    if !(0..=262_000).contains(&year) {
        return None;
    }
    let day = date.day().min(days_in_month(year, month0));
    let shifted = NaiveDate::from_ymd_opt(year as i32, month0 + 1, day)?;
    Some(shifted.and_time(naive.time()))
}

// ── internals ──────────────────────────────────────────────────────────────

/// `UNTIL=YYYYMMDDTHHMMSSZ` (subset grammar). Parsed positionally — no
/// format-string ambiguity, always a typed error on bad input.
fn parse_until(value: &str) -> Result<DateTime<Utc>> {
    let bad = || RruleError::InvalidUntil(value.to_string());
    let bytes = value.as_bytes();
    if bytes.len() != 16 || bytes[8] != b'T' || bytes[15] != b'Z' {
        return Err(bad());
    }
    let num = |r: std::ops::Range<usize>| -> Result<i64> {
        value[r]
            .parse::<i64>()
            .map_err(|_| RruleError::InvalidUntil(value.to_string()))
    };
    let year = num(0..4)?;
    let month = num(4..6)?;
    let day = num(6..8)?;
    let hour = num(9..11)?;
    let minute = num(11..13)?;
    let second = num(13..15)?;
    let date = NaiveDate::from_ymd_opt(
        i32::try_from(year).map_err(|_| bad())?,
        u32::try_from(month).map_err(|_| bad())?,
        u32::try_from(day).map_err(|_| bad())?,
    )
    .ok_or_else(bad)?;
    let time = chrono::NaiveTime::from_hms_opt(
        u32::try_from(hour).map_err(|_| bad())?,
        u32::try_from(minute).map_err(|_| bad())?,
        u32::try_from(second).map_err(|_| bad())?,
    )
    .ok_or_else(bad)?;
    Ok(date.and_time(time).and_utc())
}

fn parse_weekday(code: &str) -> Option<Weekday> {
    match code.trim().to_ascii_uppercase().as_str() {
        "MO" => Some(Weekday::Mon),
        "TU" => Some(Weekday::Tue),
        "WE" => Some(Weekday::Wed),
        "TH" => Some(Weekday::Thu),
        "FR" => Some(Weekday::Fri),
        "SA" => Some(Weekday::Sat),
        "SU" => Some(Weekday::Sun),
        _ => None,
    }
}

fn weekday_code(wd: Weekday) -> &'static str {
    match wd {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

fn days_in_month(year: i64, month0: u32) -> u32 {
    // Month lengths; February adjusted for leap years (proleptic Gregorian).
    const COMMON: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if month0 == 1 && is_leap(year) {
        29
    } else {
        COMMON[month0 as usize]
    }
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn expand_single(
    event: &Event,
    window_from: DateTime<Utc>,
    window_to: DateTime<Utc>,
    tz: &Tz,
) -> Vec<Occurrence> {
    let occ = Occurrence {
        event_id: event.id,
        title: event.title.clone(),
        description: event.description.clone(),
        location: event.location.clone(),
        all_day: event.all_day,
        start_utc: event.start_utc,
        end_utc: event.end_utc,
        start_date: event.start_date,
        end_date: event.end_date,
        tz: event.tz.clone(),
        color_idx: event.color_idx,
    };
    let intersects = if event.all_day {
        match (event.start_date, event.end_date) {
            (Some(s), Some(e)) => {
                let from_local = window_from.with_timezone(tz).date_naive();
                let to_local = (window_to - Duration::seconds(1))
                    .with_timezone(tz)
                    .date_naive();
                s <= to_local && e >= from_local
            }
            _ => false,
        }
    } else {
        match (event.start_utc, event.end_utc) {
            (Some(s), Some(e)) => s < window_to && e > window_from,
            _ => false,
        }
    };
    if intersects {
        vec![occ]
    } else {
        Vec::new()
    }
}

struct Filters {
    byday: Option<Vec<Weekday>>,
    bymonthday: Option<Vec<i32>>,
}

fn filters_for(rule: &Recurrence, start_date: NaiveDate) -> Filters {
    let byday = if !rule.byday.is_empty() {
        Some(rule.byday.clone())
    } else if rule.freq == Freq::Weekly {
        // WEEKLY without BYDAY recurs on DTSTART's weekday.
        Some(vec![start_date.weekday()])
    } else {
        None
    };
    let bymonthday = if !rule.bymonthday.is_empty() {
        Some(rule.bymonthday.clone())
    } else if matches!(rule.freq, Freq::Monthly | Freq::Yearly) && rule.byday.is_empty() {
        // MONTHLY/YEARLY without BYDAY *and* BYMONTHDAY recurs on
        // DTSTART's day-of-month.
        Some(vec![start_date.day() as i32])
    } else {
        None
    };
    Filters { byday, bymonthday }
}

/// All candidate dates in recurrence `period` (0-based), unsorted
/// pre-filter raw material. Dates are generated in the TZ wall-clock space;
/// period alignment: DAILY from DTSTART, WEEKLY from the WKST week
/// containing DTSTART, MONTHLY/YEARLY from DTSTART's month.
fn period_dates(rule: &Recurrence, start: NaiveDate, period: u64) -> Vec<NaiveDate> {
    let interval = i64::from(rule.interval).saturating_mul(period as i64);
    match rule.freq {
        Freq::Daily => {
            // `period` here is candidate index; interval applied by caller steps.
            [start.checked_add_signed(Duration::days(interval))]
                .into_iter()
                .flatten()
                .collect()
        }
        Freq::Weekly => {
            let week_start = match start.checked_sub_signed(Duration::days(i64::from(
                start.weekday().num_days_from_monday(),
            ))) {
                Some(d) => d,
                None => return Vec::new(),
            };
            match week_start.checked_add_signed(Duration::weeks(interval)) {
                Some(ws) => (0..7)
                    .filter_map(|d| ws.checked_add_signed(Duration::days(d)))
                    .collect(),
                None => Vec::new(),
            }
        }
        Freq::Monthly => month_dates(start, interval, start.month0()),
        Freq::Yearly => month_dates(start, interval * 12, start.month0()),
    }
}

/// Every day of the month `start.month0()` shifted by `month_offset`.
fn month_dates(start: NaiveDate, month_offset: i64, month0: u32) -> Vec<NaiveDate> {
    let start_total = i64::from(start.year()) * 12 + i64::from(month0);
    let total = start_total.saturating_add(month_offset);
    let year = total.div_euclid(12);
    let m0 = total.rem_euclid(12) as u32;
    if !(0..=262_000).contains(&year) {
        return Vec::new();
    }
    let year = year as i32;
    let ndays = days_in_month(i64::from(year), m0);
    (1..=ndays)
        .filter_map(|d| NaiveDate::from_ymd_opt(year, m0 + 1, d))
        .collect()
}

fn expand_recurring(
    event: &Event,
    rule: &Recurrence,
    window_from: DateTime<Utc>,
    window_to: DateTime<Utc>,
    tz: &Tz,
) -> Result<Vec<Occurrence>> {
    let filters = filters_for(rule, start_date_of(event)?);
    let skip: HashSet<DateTime<Utc>> = event
        .exceptions
        .iter()
        .filter(|e| e.kind == ExceptionKind::Skip)
        .map(|e| e.occurrence_utc)
        .collect();
    let modify: HashMap<DateTime<Utc>, &OccurrenceChanges> = event
        .exceptions
        .iter()
        .filter_map(|e| e.changes.as_ref().map(|c| (e.occurrence_utc, c)))
        .collect();

    let (timed, duration) = if event.all_day {
        (false, Duration::zero())
    } else {
        let (s, e) = (event.start_utc, event.end_utc);
        match (s, e) {
            (Some(s), Some(e)) => {
                let dur = e - s;
                if dur < Duration::zero() {
                    return Err(RruleError::InvalidEvent("end before start".into()));
                }
                (true, dur)
            }
            _ => {
                return Err(RruleError::InvalidEvent(
                    "timed event missing instants".into(),
                ))
            }
        }
    };

    // Wall-clock anchors.
    let start_naive = if timed {
        event
            .start_utc
            .map(|s| s.with_timezone(tz).naive_local())
            .ok_or_else(|| RruleError::InvalidEvent("timed event missing start".into()))?
    } else {
        event
            .start_date
            .ok_or_else(|| RruleError::InvalidEvent("all-day event missing dates".into()))?
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| RruleError::InvalidEvent("invalid start date".into()))?
    };
    let start_date = start_naive.date();
    let until_local = rule.until.map(|u| u.with_timezone(tz).naive_local());
    let bound_local = {
        let win_end = window_to.with_timezone(tz).naive_local();
        match until_local {
            Some(u) => u.min(win_end),
            None => win_end,
        }
    };

    let win_from_local = window_from.with_timezone(tz).date_naive();
    let win_to_local = (window_to - Duration::seconds(1))
        .with_timezone(tz)
        .date_naive();

    let mut out: Vec<Occurrence> = Vec::new();
    let mut evals: usize = 0;
    let mut seen: u32 = 0;

    'periods: for period in 0..(2 * ITERATION_CAP as u64) {
        // Early exit: the whole period starts past the bound.
        let period_first = match period_dates(rule, start_date, period).first().copied() {
            Some(d) => d,
            None => {
                // Degenerate period (date range overflow) — stop entirely.
                break;
            }
        };
        if period_first > bound_local.date() {
            break;
        }
        for date in period_dates(rule, start_date, period) {
            evals += 1;
            if evals > ITERATION_CAP {
                return Err(RruleError::IterationCap);
            }
            // BYDAY ∩ BYMONTHDAY filters (RFC 5545 intersection).
            if filters
                .byday
                .as_ref()
                .is_some_and(|d| !d.contains(&date.weekday()))
            {
                continue;
            }
            if filters
                .bymonthday
                .as_ref()
                .is_some_and(|d| !d.contains(&(date.day() as i32)))
            {
                continue;
            }
            let naive = date.and_time(start_naive.time());
            // Occurrences before DTSTART are never generated (RFC 5545).
            if naive < start_naive {
                continue;
            }
            let start_utc = local_to_utc(tz, naive)?;
            if let Some(u) = rule.until {
                if start_utc > u {
                    break 'periods;
                }
            }
            if skip.contains(&start_utc) {
                continue;
            }
            if let Some(limit) = rule.count {
                if seen >= limit {
                    break 'periods;
                }
                seen += 1;
            }

            // Build the occurrence (all-day events carry shifted dates).
            let mut occ = Occurrence {
                event_id: event.id,
                title: event.title.clone(),
                description: event.description.clone(),
                location: event.location.clone(),
                all_day: event.all_day,
                start_utc: None,
                end_utc: None,
                start_date: None,
                end_date: None,
                tz: event.tz.clone(),
                color_idx: event.color_idx,
            };
            if timed {
                occ.start_utc = Some(start_utc);
                occ.end_utc = Some(start_utc + duration);
            } else {
                let span = match (event.start_date, event.end_date) {
                    (Some(s), Some(e)) => (e - s).num_days(),
                    _ => {
                        return Err(RruleError::InvalidEvent(
                            "all-day event missing dates".into(),
                        ))
                    }
                };
                occ.start_date = Some(date);
                occ.end_date = Some(date + Duration::days(span));
            }

            if let Some(changes) = modify.get(&start_utc) {
                apply_changes(&mut occ, changes, duration)?;
            }

            // Window intersection (post-override instants).
            let intersects = if occ.all_day {
                match (occ.start_date, occ.end_date) {
                    (Some(s), Some(e)) => s <= win_to_local && e >= win_from_local,
                    _ => false,
                }
            } else {
                match (occ.start_utc, occ.end_utc) {
                    (Some(s), Some(e)) => s < window_to && e > window_from,
                    _ => false,
                }
            };
            if intersects {
                out.push(occ);
            }
        }
    }

    out.sort_by_key(|o| {
        o.start_utc.unwrap_or_else(|| {
            o.start_date
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|n| n.and_utc())
                .unwrap_or(window_from)
        })
    });
    Ok(out)
}

fn start_date_of(event: &Event) -> Result<NaiveDate> {
    if event.all_day {
        event
            .start_date
            .ok_or_else(|| RruleError::InvalidEvent("all-day event missing start date".into()))
    } else {
        let tz: Tz = event
            .tz
            .parse()
            .map_err(|_| RruleError::UnknownTz(event.tz.clone()))?;
        event
            .start_utc
            .map(|s| s.with_timezone(&tz).date_naive())
            .ok_or_else(|| RruleError::InvalidEvent("timed event missing start".into()))
    }
}

fn apply_changes(
    occ: &mut Occurrence,
    changes: &OccurrenceChanges,
    duration: Duration,
) -> Result<()> {
    if let Some(t) = &changes.title {
        occ.title = t.clone();
    }
    if let Some(l) = &changes.location {
        occ.location = l.clone();
    }
    if let Some(d) = &changes.description {
        occ.description = d.clone();
    }
    if let Some(tz_name) = &changes.tz {
        tz_name
            .parse::<Tz>()
            .map_err(|_| RruleError::UnknownTz(tz_name.clone()))?;
        occ.tz = tz_name.clone();
    }
    if occ.all_day {
        return Ok(());
    }
    let (old_start, old_end) = match (occ.start_utc, occ.end_utc) {
        (Some(s), Some(e)) => (s, e),
        _ => return Ok(()),
    };
    match (changes.start_utc, changes.end_utc) {
        (Some(s), Some(e)) => {
            occ.start_utc = Some(s);
            occ.end_utc = Some(e);
        }
        (Some(s), None) => {
            // Start moved, end not: preserve the event's wall-clock duration.
            occ.start_utc = Some(s);
            occ.end_utc = Some(s + duration);
        }
        (None, Some(e)) => {
            let _ = old_start;
            occ.end_utc = Some(e);
        }
        (None, None) => {
            let _ = (old_start, old_end);
        }
    }
    Ok(())
}
