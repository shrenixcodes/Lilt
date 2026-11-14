//! Time-zone conversion through the Windows time-zone database.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::{FreeLibrary, SYSTEMTIME};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows::Win32::System::Time::{
    DYNAMIC_TIME_ZONE_INFORMATION, EnumDynamicTimeZoneInformation, SystemTimeToTzSpecificLocalTimeEx,
    TzSpecificLocalTimeToSystemTimeEx,
};
use windows::core::{s, w};

use super::date::{Date, split};
use super::ics::TimeZones;

/// Resolves `TZID`s written either as Windows zone names (Outlook) or IANA names
/// (Google, Apple). Lookups are cached for the lifetime of the value.
#[derive(Default)]
pub struct WindowsZones {
    cache: RefCell<HashMap<String, Option<DYNAMIC_TIME_ZONE_INFORMATION>>>,
}

impl TimeZones for WindowsZones {
    fn utc_to_local(&self, utc: i64) -> i64 {
        let mut local = SYSTEMTIME::default();
        match unsafe { SystemTimeToTzSpecificLocalTimeEx(None, &to_systemtime(utc), &mut local) } {
            Ok(()) => from_systemtime(&local),
            Err(_) => utc,
        }
    }

    fn zoned_to_utc(&self, tzid: &str, wall: i64) -> Option<i64> {
        let zone = self.cache.borrow_mut().entry(tzid.to_owned()).or_insert_with(|| lookup(tzid)).to_owned()?;
        let mut utc = SYSTEMTIME::default();
        unsafe { TzSpecificLocalTimeToSystemTimeEx(Some(&zone), &to_systemtime(wall), &mut utc) }.ok()?;
        Some(from_systemtime(&utc))
    }
}

fn to_systemtime(minutes: i64) -> SYSTEMTIME {
    let (date, minute) = split(minutes);
    SYSTEMTIME {
        wYear: date.year as u16,
        wMonth: date.month as u16,
        wDayOfWeek: 0,
        wDay: date.day as u16,
        wHour: (minute / 60) as u16,
        wMinute: (minute % 60) as u16,
        wSecond: 0,
        wMilliseconds: 0,
    }
}

pub(crate) fn from_systemtime(st: &SYSTEMTIME) -> i64 {
    Date { year: st.wYear as i32, month: st.wMonth as u32, day: st.wDay as u32 }.at(st.wHour as u32, st.wMinute as u32)
}

fn lookup(tzid: &str) -> Option<DYNAMIC_TIME_ZONE_INFORMATION> {
    find_windows_zone(tzid).or_else(|| iana_to_windows(tzid).and_then(|id| find_windows_zone(&id)))
}

fn find_windows_zone(key: &str) -> Option<DYNAMIC_TIME_ZONE_INFORMATION> {
    (0..)
        .map_while(|index| {
            let mut zone = DYNAMIC_TIME_ZONE_INFORMATION::default();
            (unsafe { EnumDynamicTimeZoneInformation(index, &mut zone) } == 0).then_some(zone)
        })
        .find(|zone| {
            let name = &zone.TimeZoneKeyName;
            let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            String::from_utf16_lossy(&name[..len]).eq_ignore_ascii_case(key)
        })
}

type GetWindowsTimeZoneId = unsafe extern "C" fn(*const u16, i32, *mut u16, i32, *mut i32) -> i32;

/// Maps an IANA zone name to a Windows zone name using the ICU library shipped with
/// Windows 10 1903 and later. Loaded dynamically so its absence is not fatal.
fn iana_to_windows(iana: &str) -> Option<String> {
    let id: Vec<u16> = iana.encode_utf16().collect();
    unsafe {
        let module = LoadLibraryExW(w!("icu.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
        let result = GetProcAddress(module, s!("ucal_getWindowsTimeZoneID")).and_then(|f| {
            let f: GetWindowsTimeZoneId = std::mem::transmute(f);
            let mut out = [0u16; 128];
            let mut status = 0i32;
            let len = f(id.as_ptr(), id.len() as i32, out.as_mut_ptr(), out.len() as i32, &mut status);
            (status <= 0 && len > 0).then(|| String::from_utf16_lossy(&out[..len as usize]))
        });
        let _ = FreeLibrary(module);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_windows_and_iana_names() {
        let zones = WindowsZones::default();
        let wall = Date { year: 2026, month: 1, day: 15 }.at(12, 0);
        // UTC+1 in winter for both spellings of Central European time.
        assert_eq!(zones.zoned_to_utc("W. Europe Standard Time", wall), Some(wall - 60));
        assert_eq!(zones.zoned_to_utc("Europe/Berlin", wall), Some(wall - 60));
        // Daylight saving time is applied in summer.
        let summer = Date { year: 2026, month: 7, day: 15 }.at(12, 0);
        assert_eq!(zones.zoned_to_utc("America/New_York", summer), Some(summer + 240));
        assert_eq!(zones.zoned_to_utc("Not/AZone", wall), None);
    }

    #[test]
    fn systemtime_round_trip() {
        let m = Date { year: 2026, month: 11, day: 14 }.at(23, 59);
        assert_eq!(from_systemtime(&to_systemtime(m)), m);
    }
}
