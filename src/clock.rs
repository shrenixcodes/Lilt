//! Local date and time text for the island.
//!
//! Times follow the user's regional 12/24-hour preference. The caller refreshes the
//! text once per minute using [`millis_to_next_minute`] rather than ticking every second.

use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_STIMEFORMAT};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::core::PCWSTR;

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

#[derive(Clone, Debug, PartialEq)]
pub struct ClockText {
    /// e.g. `14:05` or `2:05 PM`.
    pub time: String,
    /// e.g. `Sat`.
    pub weekday: String,
    /// e.g. `14 Nov`.
    pub date: String,
}

#[cfg(test)]
pub fn now(h24: bool) -> ClockText {
    now_formatted(h24, 0)
}

pub fn resolved_h24(settings: &crate::settings::Clock) -> bool {
    match settings.time_format {
        1 => false,
        2 => true,
        _ => uses_24_hour_clock(),
    }
}

pub fn configured_now(settings: &crate::settings::Clock) -> ClockText {
    now_formatted(resolved_h24(settings), settings.date_format)
}

fn format_date(year: u16, month: u16, day: u16, format: u8) -> String {
    let name = MONTHS.get(month.saturating_sub(1) as usize).copied().unwrap_or_default();
    match format {
        1 => format!("{name} {day}"),
        2 => format!("{year:04}-{month:02}-{day:02}"),
        _ => format!("{day} {name}"),
    }
}

fn now_formatted(h24: bool, date_format: u8) -> ClockText {
    let t = unsafe { GetLocalTime() };
    ClockText {
        time: format_time(t.wHour as u32, t.wMinute as u32, h24),
        weekday: WEEKDAYS.get(t.wDayOfWeek as usize).copied().unwrap_or_default().to_owned(),
        date: format_date(t.wYear, t.wMonth, t.wDay, date_format),
    }
}

/// Milliseconds until the next minute begins.
pub fn millis_to_next_minute() -> u64 {
    let t = unsafe { GetLocalTime() };
    60_000 - (t.wSecond as u64 * 1000 + t.wMilliseconds as u64).min(59_999)
}

/// Whether the user's regional format uses a 24-hour clock.
pub fn uses_24_hour_clock() -> bool {
    let mut buffer = [0u16; 80];
    let len = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_STIMEFORMAT, Some(&mut buffer)) };
    if len <= 0 {
        return true;
    }
    // Format pictures use `H` for 24-hour and `h` for 12-hour hours.
    String::from_utf16_lossy(&buffer[..len as usize]).contains('H')
}

pub fn format_time(hour: u32, minute: u32, h24: bool) -> String {
    if h24 {
        format!("{hour:02}:{minute:02}")
    } else {
        let suffix = if hour < 12 { "AM" } else { "PM" };
        let hour12 = if hour.is_multiple_of(12) { 12 } else { hour % 12 };
        format!("{hour12}:{minute:02} {suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_four_hour_format() {
        assert_eq!(format_time(0, 5, true), "00:05");
        assert_eq!(format_time(23, 59, true), "23:59");
    }

    #[test]
    fn twelve_hour_format() {
        assert_eq!(format_time(0, 5, false), "12:05 AM");
        assert_eq!(format_time(12, 0, false), "12:00 PM");
        assert_eq!(format_time(13, 30, false), "1:30 PM");
    }

    #[test]
    fn chosen_time_format_overrides_the_windows_format() {
        assert!(!resolved_h24(&crate::settings::Clock { time_format: 1, date_format: 0, ..Default::default() }));
        assert!(resolved_h24(&crate::settings::Clock { time_format: 2, date_format: 0, ..Default::default() }));
        assert_eq!(resolved_h24(&crate::settings::Clock::default()), uses_24_hour_clock());
    }

    #[test]
    fn date_formats_distinguish_day_month_and_iso() {
        assert_eq!(format_date(2026, 11, 7, 0), "7 Nov");
        assert_eq!(format_date(2026, 11, 7, 1), "Nov 7");
        assert_eq!(format_date(2026, 11, 7, 2), "2026-11-07");
    }

    #[test]
    fn live_values_are_well_formed() {
        let text = now(true);
        assert_eq!(text.time.len(), 5);
        assert!(WEEKDAYS.contains(&text.weekday.as_str()));
        assert!((1..=60_000).contains(&millis_to_next_minute()));
    }
}
