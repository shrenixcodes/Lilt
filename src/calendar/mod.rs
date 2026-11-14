//! Agenda preview built from a local `.ics` file.

pub mod date;
pub mod ics;
mod zones;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use date::{DAY, Date, split};
pub use zones::WindowsZones;

use crate::clock;

/// Calendar files larger than this are not read.
const MAX_ICS_BYTES: u64 = 8 * 1024 * 1024;
/// How far ahead the agenda looks.
const AGENDA_DAYS: i64 = 7;

/// ISO order, Monday first, matching [`Date::weekday`].
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Local wall-clock time, in minutes since the epoch.
pub fn now_local() -> i64 {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    zones::from_systemtime(&unsafe { GetLocalTime() })
}

fn format_time(minutes: i64, h24: bool) -> String {
    let minute_of_day = split(minutes).1;
    clock::format_time(minute_of_day / 60, minute_of_day % 60, h24)
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgendaItem {
    pub title: String,
    pub location: String,
    /// `Today`, `Tomorrow` or e.g. `Mon 17`.
    pub day: String,
    /// `09:00 – 09:30`, `All day`, …
    pub time: String,
    /// True while the event is in progress.
    pub now: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgendaStatus {
    /// The configured file could not be read or parsed.
    Unavailable(String),
    /// The file was read; `items` may be empty.
    Ready,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Agenda {
    pub status: AgendaStatus,
    pub items: Vec<AgendaItem>,
}

/// A parsed calendar file, re-read only when its modification time changes.
pub struct CalendarFile {
    path: PathBuf,
    modified: Option<SystemTime>,
    events: Result<Vec<ics::Event>, String>,
}

impl CalendarFile {
    pub fn open(path: &Path) -> Self {
        let mut file = Self { path: path.to_owned(), modified: None, events: Ok(Vec::new()) };
        file.reload();
        file
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Re-reads the file if it changed on disk. Returns true if it was re-read.
    pub fn refresh(&mut self) -> bool {
        let modified = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if modified.is_some() && modified == self.modified {
            return false;
        }
        self.reload();
        true
    }

    fn reload(&mut self) {
        self.modified = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        self.events = read(&self.path);
        match &self.events {
            Ok(events) => tracing::info!(count = events.len(), "calendar loaded"),
            Err(err) => tracing::warn!(%err, "calendar unavailable"),
        }
    }

    pub fn agenda(&self, zones: &dyn ics::TimeZones, now: i64, h24: bool, limit: usize) -> Agenda {
        match &self.events {
            Err(err) => Agenda { status: AgendaStatus::Unavailable(err.clone()), items: Vec::new() },
            Ok(events) => Agenda { status: AgendaStatus::Ready, items: build_agenda(events, zones, now, h24, limit) },
        }
    }
}

fn read(path: &Path) -> Result<Vec<ics::Event>, String> {
    let meta = std::fs::metadata(path).map_err(|_| "The calendar file could not be found.".to_owned())?;
    if meta.len() > MAX_ICS_BYTES {
        return Err("The calendar file is too large.".into());
    }
    let bytes = std::fs::read(path).map_err(|_| "The calendar file could not be read.".to_owned())?;
    let text = String::from_utf8_lossy(&bytes);
    ics::parse(&text).map_err(|e| format!("{}.", capitalise(&e.to_string())))
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

pub fn build_agenda(
    events: &[ics::Event],
    zones: &dyn ics::TimeZones,
    now: i64,
    h24: bool,
    limit: usize,
) -> Vec<AgendaItem> {
    let today = split(now).0;
    let window_end = (today.to_days() + AGENDA_DAYS) * DAY;
    ics::occurrences(events, zones, now, window_end)
        .into_iter()
        .take(limit)
        .map(|o| {
            let start_day = split(o.start.max(today.to_days() * DAY)).0;
            AgendaItem {
                day: day_label(start_day, today),
                time: if o.all_day {
                    "All day".into()
                } else if o.end > o.start {
                    format!("{} – {}", format_time(o.start, h24), format_time(o.end, h24))
                } else {
                    format_time(o.start, h24)
                },
                now: o.start <= now && now < o.end,
                title: o.summary,
                location: o.location.unwrap_or_default(),
            }
        })
        .collect()
}

fn day_label(day: Date, today: Date) -> String {
    match day.to_days() - today.to_days() {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        _ => format!("{} {}", WEEKDAYS[day.weekday() as usize], day.day),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Utc;
    impl ics::TimeZones for Utc {
        fn utc_to_local(&self, utc: i64) -> i64 {
            utc
        }
        fn zoned_to_utc(&self, _: &str, _: i64) -> Option<i64> {
            None
        }
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        Date::new(y, m, d).unwrap().at(h, min)
    }

    #[test]
    fn time_formats() {
        let t = at(2026, 11, 14, 0, 5);
        assert_eq!(format_time(t, true), "00:05");
        assert_eq!(format_time(t, false), "12:05 AM");
        assert_eq!(format_time(at(2026, 11, 14, 13, 30), false), "1:30 PM");
    }

    #[test]
    fn agenda_labels_and_ordering() {
        let text = "BEGIN:VCALENDAR\r\n\
BEGIN:VEVENT\r\nSUMMARY:Later\r\nDTSTART:20261116T090000Z\r\nDTEND:20261116T093000Z\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:Ongoing\r\nDTSTART:20261114T100000Z\r\nDTEND:20261114T120000Z\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:Past\r\nDTSTART:20261114T080000Z\r\nDTEND:20261114T083000Z\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261115\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n";
        let events = ics::parse(text).unwrap();
        let items = build_agenda(&events, &Utc, at(2026, 11, 14, 11, 0), true, 10);
        let summary: Vec<_> =
            items.iter().map(|i| (i.title.as_str(), i.day.as_str(), i.time.as_str(), i.now)).collect();
        assert_eq!(
            summary,
            vec![
                ("Ongoing", "Today", "10:00 – 12:00", true),
                ("Holiday", "Tomorrow", "All day", false),
                ("Later", "Mon 16", "09:00 – 09:30", false),
            ]
        );
    }

    #[test]
    fn reads_a_calendar_file_from_disk() {
        let dir = std::env::temp_dir().join(format!("lilt-calendar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.ics");
        std::fs::write(&path, "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Planning\r\nDTSTART:20261116T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n").unwrap();
        let mut file = CalendarFile::open(&path);
        let agenda = file.agenda(&Utc, at(2026, 11, 15, 12, 0), true, 5);
        assert_eq!(agenda.status, AgendaStatus::Ready);
        assert_eq!(agenda.items[0].title, "Planning");
        assert!(!file.refresh(), "unchanged file is not re-read");
    }

    #[test]
    fn missing_file_is_reported_not_invented() {
        let file = CalendarFile::open(Path::new(r"C:\definitely\missing\calendar.ics"));
        let agenda = file.agenda(&Utc, at(2026, 11, 14, 11, 0), true, 5);
        assert!(matches!(agenda.status, AgendaStatus::Unavailable(_)));
        assert!(agenda.items.is_empty());
    }
}
