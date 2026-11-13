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

pub fn now(h24: bool) -> ClockText {
    let t = unsafe { GetLocalTime() };
    ClockText {
        time: format_time(t.wHour as u32, t.wMinute as u32, h24),
        weekday: WEEKDAYS.get(t.wDayOfWeek as usize).copied().unwrap_or_default().to_owned(),
        date: format!("{} {}", t.wDay, MONTHS.get(t.wMonth as usize - 1).copied().unwrap_or_default()),
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
        let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
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
    fn live_values_are_well_formed() {
        let text = now(true);
        assert_eq!(text.time.len(), 5);
        assert!(WEEKDAYS.contains(&text.weekday.as_str()));
        assert!((1..=60_000).contains(&millis_to_next_minute()));
    }
}
