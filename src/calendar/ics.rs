//! A small, defensive reader for iCalendar (RFC 5545) files.
//!
//! Only what an agenda preview needs is supported: event summaries, locations,
//! start/end times in UTC, floating or `TZID` form, all-day events, durations,
//! cancellations, `EXDATE`, and the common recurrence subset (`DAILY`, `WEEKLY` with
//! `BYDAY`, `MONTHLY`, `YEARLY`, `INTERVAL`, `COUNT`, `UNTIL`). Recurrence rules using
//! other parts are reported as only their first occurrence rather than guessed.

use super::date::{DAY, Date};

/// Upper bound on events read from one file, to keep memory bounded.
const MAX_EVENTS: usize = 20_000;
/// Upper bound on recurrence iterations per event.
const MAX_ITERATIONS: u32 = 10_000;

/// A timestamp as written in the file.
#[derive(Clone, Debug, PartialEq)]
pub enum Stamp {
    /// UTC minutes since the epoch (`...Z`).
    Utc(i64),
    /// Wall-clock minutes with no zone; interpreted in the user's local zone.
    Floating(i64),
    /// Wall-clock minutes in a named zone.
    Zoned { wall: i64, tzid: String },
    /// An all-day date.
    Date(Date),
}

impl Stamp {
    fn wall(&self) -> i64 {
        match self {
            Stamp::Utc(m) | Stamp::Floating(m) | Stamp::Zoned { wall: m, .. } => *m,
            Stamp::Date(d) => d.to_days() * DAY,
        }
    }

    fn with_wall(&self, wall: i64) -> Stamp {
        match self {
            Stamp::Utc(_) => Stamp::Utc(wall),
            Stamp::Floating(_) => Stamp::Floating(wall),
            Stamp::Zoned { tzid, .. } => Stamp::Zoned { wall, tzid: tzid.clone() },
            Stamp::Date(_) => Stamp::Date(Date::from_days(wall.div_euclid(DAY))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Recurrence {
    pub frequency: Frequency,
    pub interval: u32,
    pub count: Option<u32>,
    pub until: Option<Stamp>,
    /// ISO weekdays (Monday = 0) for weekly rules.
    pub by_day: Vec<u32>,
    /// False when the rule used parts that are not expanded.
    pub exact: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub summary: String,
    pub location: Option<String>,
    pub start: Stamp,
    /// Length of each occurrence in minutes.
    pub duration: i64,
    pub recurrence: Option<Recurrence>,
    pub exdates: Vec<Stamp>,
}

impl Event {
    pub fn all_day(&self) -> bool {
        matches!(self.start, Stamp::Date(_))
    }
}

/// Resolves stamps to the user's local wall-clock time.
pub trait TimeZones {
    /// Converts UTC minutes to local wall-clock minutes.
    fn utc_to_local(&self, utc: i64) -> i64;
    /// Converts wall-clock minutes in `tzid` to UTC, or `None` for unknown zones.
    fn zoned_to_utc(&self, tzid: &str, wall: i64) -> Option<i64>;

    fn to_local(&self, stamp: &Stamp) -> i64 {
        match stamp {
            Stamp::Utc(m) => self.utc_to_local(*m),
            Stamp::Floating(m) => *m,
            Stamp::Date(d) => d.to_days() * DAY,
            // An unknown zone is treated as local wall-clock time rather than dropped.
            Stamp::Zoned { wall, tzid } => self.zoned_to_utc(tzid, *wall).map_or(*wall, |u| self.utc_to_local(u)),
        }
    }
}

/// One concrete occurrence in local wall-clock minutes.
#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence {
    pub summary: String,
    pub location: Option<String>,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum IcsError {
    NotACalendar,
}

impl std::fmt::Display for IcsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the file is not an iCalendar (.ics) calendar")
    }
}

impl std::error::Error for IcsError {}

/// Parses all valid, non-cancelled events. Malformed events are skipped.
pub fn parse(text: &str) -> Result<Vec<Event>, IcsError> {
    let lines = unfold(text);
    if !lines.iter().any(|l| l.eq_ignore_ascii_case("BEGIN:VCALENDAR")) {
        return Err(IcsError::NotACalendar);
    }

    let mut events = Vec::new();
    let mut current: Option<Vec<(String, String, String)>> = None;
    let mut depth = 0u32;
    for line in &lines {
        let Some((name, params, value)) = split_property(line) else { continue };
        match (name.as_str(), value.to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") if current.is_none() => {
                current = Some(Vec::new());
                depth = 0;
            }
            ("BEGIN", _) if current.is_some() => depth += 1,
            ("END", "VEVENT") if depth == 0 => {
                if let Some(props) = current.take()
                    && let Some(event) = build_event(&props)
                {
                    events.push(event);
                    if events.len() >= MAX_EVENTS {
                        break;
                    }
                }
            }
            ("END", _) if current.is_some() => depth = depth.saturating_sub(1),
            _ => {
                // Properties of nested components (e.g. VALARM) are ignored.
                if let (Some(props), 0) = (current.as_mut(), depth) {
                    props.push((name, params, value));
                }
            }
        }
    }
    Ok(events)
}

fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t'])
            && let Some(last) = out.last_mut()
        {
            last.push_str(rest);
            continue;
        }
        if !line.is_empty() {
            out.push(line.to_owned());
        }
    }
    out
}

/// Splits `NAME;PARAM=x:VALUE`, honouring quoted parameter values.
fn split_property(line: &str) -> Option<(String, String, String)> {
    let mut in_quotes = false;
    let colon = line.char_indices().find_map(|(i, c)| {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => return Some(i),
            _ => {}
        }
        None
    })?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let (name, params) = head.split_once(';').unwrap_or((head, ""));
    Some((name.trim().to_ascii_uppercase(), params.to_owned(), value.to_owned()))
}

fn param<'a>(params: &'a str, key: &str) -> Option<&'a str> {
    params.split(';').find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim().eq_ignore_ascii_case(key).then(|| v.trim().trim_matches('"'))
    })
}

fn build_event(props: &[(String, String, String)]) -> Option<Event> {
    let get = |name: &str| props.iter().find(|(n, _, _)| n == name);
    if get("STATUS").is_some_and(|(_, _, v)| v.eq_ignore_ascii_case("CANCELLED")) {
        return None;
    }
    let (_, params, value) = get("DTSTART")?;
    let start = parse_stamp(params, value)?;

    let duration = if let Some((p, v)) = get("DTEND").map(|(_, p, v)| (p, v)) {
        let end = parse_stamp(p, v)?;
        end.wall() - start.wall()
    } else if let Some((_, _, v)) = get("DURATION") {
        parse_duration(v)?
    } else if start_is_date(&start) {
        DAY
    } else {
        0
    };

    let exdates = props
        .iter()
        .filter(|(n, _, _)| n == "EXDATE")
        .flat_map(|(_, p, v)| v.split(',').filter_map(|part| parse_stamp(p, part)).collect::<Vec<_>>())
        .collect();

    Some(Event {
        summary: get("SUMMARY")
            .map(|(_, _, v)| unescape(v))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Untitled event".into()),
        location: get("LOCATION").map(|(_, _, v)| unescape(v)).filter(|s| !s.trim().is_empty()),
        recurrence: get("RRULE").and_then(|(_, _, v)| parse_rule(v)),
        start,
        duration: duration.max(0),
        exdates,
    })
}

fn start_is_date(stamp: &Stamp) -> bool {
    matches!(stamp, Stamp::Date(_))
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push(' '),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_owned()
}

fn digits(s: &str, range: std::ops::Range<usize>) -> Option<u32> {
    let part = s.get(range)?;
    part.bytes().all(|b| b.is_ascii_digit()).then(|| part.parse().ok())?
}

fn parse_date(s: &str) -> Option<Date> {
    if s.len() < 8 {
        return None;
    }
    Date::new(digits(s, 0..4)? as i32, digits(s, 4..6)?, digits(s, 6..8)?)
}

fn parse_stamp(params: &str, value: &str) -> Option<Stamp> {
    let value = value.trim();
    let date = parse_date(value)?;
    let is_date = param(params, "VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE")) || value.len() == 8;
    if is_date {
        return Some(Stamp::Date(date));
    }
    if value.as_bytes().get(8) != Some(&b'T') {
        return None;
    }
    let (hour, minute) = (digits(value, 9..11)?, digits(value, 11..13)?);
    if hour > 23 || minute > 59 {
        return None;
    }
    let wall = date.at(hour, minute);
    Some(if value.ends_with(['Z', 'z']) {
        Stamp::Utc(wall)
    } else if let Some(tzid) = param(params, "TZID") {
        Stamp::Zoned { wall, tzid: tzid.to_owned() }
    } else {
        Stamp::Floating(wall)
    })
}

/// Parses an RFC 5545 duration (e.g. `PT1H30M`, `P1D`, `-PT15M`) into minutes.
fn parse_duration(value: &str) -> Option<i64> {
    let value = value.trim();
    let (sign, rest) = match value.as_bytes().first()? {
        b'-' => (-1, &value[1..]),
        b'+' => (1, &value[1..]),
        _ => (1, value),
    };
    let rest = rest.strip_prefix(['P', 'p'])?;
    let mut total = 0i64;
    let mut number = String::new();
    let mut in_time = false;
    for c in rest.chars() {
        match c.to_ascii_uppercase() {
            '0'..='9' => number.push(c),
            'T' => in_time = true,
            unit => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                total += match (unit, in_time) {
                    ('W', false) => n * 7 * DAY,
                    ('D', false) => n * DAY,
                    ('H', true) => n * 60,
                    ('M', true) => n,
                    ('S', true) => n / 60,
                    _ => return None,
                };
            }
        }
    }
    number.is_empty().then_some(sign * total)
}

fn parse_rule(value: &str) -> Option<Recurrence> {
    let mut rule = Recurrence {
        frequency: Frequency::Daily,
        interval: 1,
        count: None,
        until: None,
        by_day: Vec::new(),
        exact: true,
    };
    let mut frequency = None;
    for part in value.split(';') {
        let (key, val) = part.split_once('=')?;
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                frequency = Some(match val.to_ascii_uppercase().as_str() {
                    "DAILY" => Frequency::Daily,
                    "WEEKLY" => Frequency::Weekly,
                    "MONTHLY" => Frequency::Monthly,
                    "YEARLY" => Frequency::Yearly,
                    _ => return None,
                })
            }
            "INTERVAL" => rule.interval = val.parse().ok().filter(|&n| n > 0)?,
            "COUNT" => rule.count = Some(val.parse().ok()?),
            "UNTIL" => rule.until = parse_stamp("", val),
            "BYDAY" => {
                for day in val.split(',') {
                    let code = day.trim().to_ascii_uppercase();
                    match ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].iter().position(|d| *d == code) {
                        Some(i) => rule.by_day.push(i as u32),
                        None => rule.exact = false, // ordinal forms such as 2MO
                    }
                }
            }
            "WKST" => {}
            _ => rule.exact = false,
        }
    }
    rule.frequency = frequency?;
    if !rule.by_day.is_empty() && rule.frequency != Frequency::Weekly {
        rule.exact = false;
    }
    rule.by_day.sort_unstable();
    rule.by_day.dedup();
    Some(rule)
}

/// Expands events into local occurrences overlapping `[from, to)`, sorted by start.
pub fn occurrences(events: &[Event], zones: &dyn TimeZones, from: i64, to: i64) -> Vec<Occurrence> {
    let mut out = Vec::new();
    for event in events {
        let excluded: Vec<i64> = event.exdates.iter().map(|s| zones.to_local(s)).collect();
        let mut emit = |stamp: &Stamp| {
            let start = zones.to_local(stamp);
            let end = start + event.duration.max(if event.all_day() { DAY } else { 0 });
            if end > from && start < to && !excluded.contains(&start) {
                out.push(Occurrence {
                    summary: event.summary.clone(),
                    location: event.location.clone(),
                    start,
                    end,
                    all_day: event.all_day(),
                });
            }
            start < to
        };
        match &event.recurrence {
            Some(rule) if rule.exact => expand(event, rule, zones, &mut emit),
            _ => {
                emit(&event.start);
            }
        }
    }
    out.sort_by(|a, b| (a.start, &a.summary).cmp(&(b.start, &b.summary)));
    out
}

/// Walks recurrence instances in the event's own wall-clock frame so that daylight
/// saving transitions keep the written time of day.
fn expand(event: &Event, rule: &Recurrence, zones: &dyn TimeZones, emit: &mut dyn FnMut(&Stamp) -> bool) {
    let first = event.start.wall();
    let (first_date, minute) = super::date::split(first);
    let until = rule.until.as_ref().map(|u| zones.to_local(u));
    let mut produced = 0u32;
    let mut step = |wall: i64| -> bool {
        if wall < first {
            return true;
        }
        let stamp = event.start.with_wall(wall);
        if until.is_some_and(|u| zones.to_local(&stamp) > u) {
            return false;
        }
        produced += 1;
        let keep_going = emit(&stamp);
        keep_going && rule.count.is_none_or(|c| produced < c)
    };

    let interval = rule.interval as i64;
    for k in 0..MAX_ITERATIONS as i64 {
        let more = match rule.frequency {
            Frequency::Daily => step(first + k * interval * DAY),
            Frequency::Weekly if rule.by_day.is_empty() => step(first + k * interval * 7 * DAY),
            Frequency::Weekly => {
                let week_start = first_date.add_days(-(first_date.weekday() as i64) + k * interval * 7);
                rule.by_day.iter().all(|&wd| step(week_start.add_days(wd as i64).to_days() * DAY + minute as i64))
            }
            Frequency::Monthly | Frequency::Yearly => {
                let months = k * interval * if rule.frequency == Frequency::Yearly { 12 } else { 1 };
                match first_date.add_months(months) {
                    Some(date) => step(date.to_days() * DAY + minute as i64),
                    None => true,
                }
            }
        };
        if !more {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UTC+1 with no daylight saving; zones named "Fixed/+2" are UTC+2.
    struct TestZones;

    impl TimeZones for TestZones {
        fn utc_to_local(&self, utc: i64) -> i64 {
            utc + 60
        }
        fn zoned_to_utc(&self, tzid: &str, wall: i64) -> Option<i64> {
            (tzid == "Fixed/+2").then_some(wall - 120)
        }
    }

    fn cal(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n")
    }

    fn day(y: i32, m: u32, d: u32) -> Date {
        Date::new(y, m, d).unwrap()
    }

    #[test]
    fn rejects_non_calendar() {
        assert_eq!(parse("hello"), Err(IcsError::NotACalendar));
    }

    #[test]
    fn parses_basic_event_with_folding_and_escapes() {
        let text = cal(
            "BEGIN:VEVENT\r\nSUMMARY:Design review\\, round\r\n  two\r\nLOCATION:Room 4\\;B\r\nDTSTART:20261116T090000Z\r\nDTEND:20261116T100000Z\r\nEND:VEVENT\r\n",
        );
        let events = parse(&text).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, "Design review, round two");
        assert_eq!(events[0].location.as_deref(), Some("Room 4;B"));
        assert_eq!(events[0].duration, 60);
        let occ = occurrences(&events, &TestZones, day(2026, 11, 16).at(0, 0), day(2026, 11, 17).at(0, 0));
        assert_eq!(occ[0].start, day(2026, 11, 16).at(10, 0));
    }

    #[test]
    fn converts_zoned_and_floating_times() {
        let text = cal(
            "BEGIN:VEVENT\r\nSUMMARY:A\r\nDTSTART;TZID=\"Fixed/+2\":20261116T120000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:B\r\nDTSTART:20261116T080000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:C\r\nDTSTART;TZID=Unknown/Zone:20261116T070000\r\nEND:VEVENT\r\n",
        );
        let occ =
            occurrences(&parse(&text).unwrap(), &TestZones, day(2026, 11, 16).at(0, 0), day(2026, 11, 17).at(0, 0));
        let starts: Vec<_> = occ.iter().map(|o| (o.summary.as_str(), o.start)).collect();
        assert_eq!(
            starts,
            vec![
                ("C", day(2026, 11, 16).at(7, 0)),
                ("B", day(2026, 11, 16).at(8, 0)),
                ("A", day(2026, 11, 16).at(11, 0))
            ]
        );
    }

    #[test]
    fn all_day_events_span_the_day() {
        let text = cal("BEGIN:VEVENT\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261225\r\nEND:VEVENT\r\n");
        let occ =
            occurrences(&parse(&text).unwrap(), &TestZones, day(2026, 12, 25).at(12, 0), day(2026, 12, 26).at(0, 0));
        assert_eq!(occ.len(), 1);
        assert!(occ[0].all_day);
        assert_eq!(occ[0].end - occ[0].start, DAY);
    }

    #[test]
    fn skips_cancelled_and_ignores_alarm_properties() {
        let text = cal(
            "BEGIN:VEVENT\r\nSUMMARY:Gone\r\nSTATUS:CANCELLED\r\nDTSTART:20261116T090000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Kept\r\nDTSTART:20261116T090000Z\r\nBEGIN:VALARM\r\nSUMMARY:Alarm text\r\nEND:VALARM\r\nEND:VEVENT\r\n",
        );
        let events = parse(&text).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, "Kept");
    }

    #[test]
    fn weekly_byday_with_count_and_exdate() {
        // Starts Monday 2026-11-16; Mon/Wed; 4 instances; Wed 18th excluded.
        let text = cal(
            "BEGIN:VEVENT\r\nSUMMARY:Standup\r\nDTSTART:20261116T090000\r\nDURATION:PT15M\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=4\r\nEXDATE:20261118T090000\r\nEND:VEVENT\r\n",
        );
        let occ =
            occurrences(&parse(&text).unwrap(), &TestZones, day(2026, 11, 1).at(0, 0), day(2026, 12, 31).at(0, 0));
        let days: Vec<_> = occ.iter().map(|o| super::super::date::split(o.start).0.day).collect();
        assert_eq!(days, vec![16, 23, 25]);
        assert!(occ.iter().all(|o| o.end - o.start == 15));
    }

    #[test]
    fn monthly_rule_skips_missing_days_and_respects_until() {
        let text = cal(
            "BEGIN:VEVENT\r\nSUMMARY:Rent\r\nDTSTART;VALUE=DATE:20260131\r\nRRULE:FREQ=MONTHLY;UNTIL=20260601\r\nEND:VEVENT\r\n",
        );
        let occ = occurrences(&parse(&text).unwrap(), &TestZones, day(2026, 1, 1).at(0, 0), day(2027, 1, 1).at(0, 0));
        let months: Vec<_> = occ.iter().map(|o| super::super::date::split(o.start).0.month).collect();
        assert_eq!(months, vec![1, 3, 5]);
    }

    #[test]
    fn unsupported_rule_falls_back_to_first_instance() {
        let text = cal(
            "BEGIN:VEVENT\r\nSUMMARY:Second Tuesday\r\nDTSTART:20261110T090000\r\nRRULE:FREQ=MONTHLY;BYDAY=2TU\r\nEND:VEVENT\r\n",
        );
        let occ = occurrences(&parse(&text).unwrap(), &TestZones, day(2026, 11, 1).at(0, 0), day(2027, 6, 1).at(0, 0));
        assert_eq!(occ.len(), 1);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT1H30M"), Some(90));
        assert_eq!(parse_duration("P1D"), Some(DAY));
        assert_eq!(parse_duration("P1W"), Some(7 * DAY));
        assert_eq!(parse_duration("-PT15M"), Some(-15));
        assert_eq!(parse_duration("PT"), Some(0));
        assert_eq!(parse_duration("1H"), None);
        assert_eq!(parse_duration("PT5"), None);
    }

    #[test]
    fn malformed_events_are_skipped() {
        let text = cal(
            "BEGIN:VEVENT\r\nSUMMARY:No start\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART:2026BAD\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART:20261116T250000\r\nEND:VEVENT\r\n",
        );
        assert!(parse(&text).unwrap().is_empty());
    }

    #[test]
    fn unbounded_daily_rule_stops_at_window() {
        let text =
            cal("BEGIN:VEVENT\r\nSUMMARY:Daily\r\nDTSTART:20200101T080000\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\n");
        let occ =
            occurrences(&parse(&text).unwrap(), &TestZones, day(2026, 11, 16).at(0, 0), day(2026, 11, 23).at(0, 0));
        assert_eq!(occ.len(), 7);
    }
}
