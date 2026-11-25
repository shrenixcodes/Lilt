//! Conversions between Unix time and local wall-clock minutes.

use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

use crate::calendar::date::Date;

/// Seconds between 1601-01-01 (FILETIME) and 1970-01-01 (Unix).
const EPOCH_DIFFERENCE: i64 = 11_644_473_600;

/// Current Unix time in seconds.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Local wall-clock minutes since the epoch for a Unix time, using the time-zone rules
/// in force at that moment (so daylight saving is applied correctly to past times).
pub fn local_minutes(unix: i64) -> i64 {
    let ticks = (unix + EPOCH_DIFFERENCE).max(0) as u64 * 10_000_000;
    let file = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        if FileTimeToSystemTime(&file, &mut utc).is_err()
            || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err()
        {
            return unix.div_euclid(60);
        }
    }
    Date::new(local.wYear as i32, local.wMonth as u32, local.wDay as u32)
        .map_or(unix.div_euclid(60), |d| d.at(local.wHour as u32, local.wMinute as u32))
}

/// Unix timestamp (seconds) of the start of the local day (00:00:00) containing `unix`.
pub fn local_midnight_unix(unix: i64) -> i64 {
    let ticks = (unix + EPOCH_DIFFERENCE).max(0) as u64 * 10_000_000;
    let file = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        if FileTimeToSystemTime(&file, &mut utc).is_err()
            || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err()
        {
            return unix - unix.rem_euclid(86_400);
        }
    }
    let elapsed_secs = i64::from(local.wHour) * 3600 + i64::from(local.wMinute) * 60 + i64::from(local.wSecond);
    unix - elapsed_secs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_time_is_close_to_utc() {
        // Every time zone is within 14 hours of UTC.
        let now = now_unix();
        let offset = local_minutes(now) - now.div_euclid(60);
        assert!(offset.abs() <= 14 * 60, "offset {offset}");
    }

    #[test]
    fn matches_the_calendar_clock() {
        let difference = local_minutes(now_unix()) - crate::calendar::now_local();
        assert!(difference.abs() <= 1, "difference {difference}");
    }

    #[test]
    fn local_midnight_is_within_one_day_before_now() {
        let now = now_unix();
        let midnight = local_midnight_unix(now);
        assert!((0..86_400).contains(&(now - midnight)));
    }
}
