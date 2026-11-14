//! Minimal proleptic-Gregorian date arithmetic.
//!
//! Times inside the calendar module are expressed as minutes since
//! 1970-01-01T00:00 in a given frame (UTC or local wall-clock). Minute resolution is
//! sufficient for an agenda and keeps the arithmetic in plain integers.

/// Minutes in a day.
pub const DAY: i64 = 24 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    pub fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        (1..=12).contains(&month).then_some(())?;
        (1..=days_in_month(year, month)).contains(&day).then_some(Self { year, month, day })
    }

    /// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`).
    pub fn to_days(self) -> i64 {
        let y = self.year as i64 - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = self.month as i64;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + self.day as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    pub fn from_days(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Self { year, month, day }
    }

    /// ISO weekday: Monday = 0 … Sunday = 6.
    pub fn weekday(self) -> u32 {
        (self.to_days() + 3).rem_euclid(7) as u32
    }

    pub fn add_days(self, days: i64) -> Self {
        Self::from_days(self.to_days() + days)
    }

    /// Adds calendar months. Returns `None` when the day does not exist in the target
    /// month (e.g. the 31st), matching RFC 5545 recurrence semantics.
    pub fn add_months(self, months: i64) -> Option<Self> {
        let index = self.year as i64 * 12 + (self.month as i64 - 1) + months;
        Self::new(index.div_euclid(12) as i32, (index.rem_euclid(12) + 1) as u32, self.day)
    }

    pub fn at(self, hour: u32, minute: u32) -> i64 {
        self.to_days() * DAY + (hour * 60 + minute) as i64
    }
}

pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Splits minutes-since-epoch into a date and minute-of-day.
pub fn split(minutes: i64) -> (Date, u32) {
    (Date::from_days(minutes.div_euclid(DAY)), minutes.rem_euclid(DAY) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_round_trip() {
        assert_eq!(Date::new(1970, 1, 1).unwrap().to_days(), 0);
        assert_eq!(Date::from_days(0), Date::new(1970, 1, 1).unwrap());
        for days in [-1_000_000, -1, 59, 60, 11_016, 20_000, 2_932_896] {
            assert_eq!(Date::from_days(days).to_days(), days);
        }
    }

    #[test]
    fn known_weekdays() {
        assert_eq!(Date::new(1970, 1, 1).unwrap().weekday(), 3); // Thursday
        assert_eq!(Date::new(2000, 2, 29).unwrap().weekday(), 1); // Tuesday
        assert_eq!(Date::new(2026, 11, 14).unwrap().weekday(), 5); // Saturday
    }

    #[test]
    fn leap_rules() {
        assert!(Date::new(2024, 2, 29).is_some());
        assert!(Date::new(2023, 2, 29).is_none());
        assert!(Date::new(1900, 2, 29).is_none());
        assert!(Date::new(2000, 2, 29).is_some());
    }

    #[test]
    fn month_arithmetic_skips_missing_days() {
        let jan31 = Date::new(2026, 1, 31).unwrap();
        assert_eq!(jan31.add_months(1), None);
        assert_eq!(jan31.add_months(2), Date::new(2026, 3, 31));
        assert_eq!(jan31.add_months(-2), None); // 31 November does not exist
        assert_eq!(Date::new(2026, 1, 15).unwrap().add_months(-2), Date::new(2025, 11, 15));
        assert_eq!(Date::new(2026, 12, 15).unwrap().add_months(1), Date::new(2027, 1, 15));
    }

    #[test]
    fn split_handles_negative_minutes() {
        let (date, minute) = split(-1);
        assert_eq!(date, Date::new(1969, 12, 31).unwrap());
        assert_eq!(minute, DAY as u32 - 1);
    }
}
