//! Monitor enumeration and island placement.
//!
//! The geometry is kept separate from the Win32 calls so it can be tested.

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::core::BOOL;

const MONITORINFOF_PRIMARY: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Self {
        Self { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    /// GDI device name, e.g. `\\.\DISPLAY1`. Stable across sessions for a given port.
    pub device: String,
    pub bounds: Rect,
    /// Area not covered by the taskbar or other app bars.
    pub work: Rect,
    pub scale: f32,
    pub primary: bool,
}

impl Monitor {
    /// e.g. `Display 2 · 2560 × 1440 (primary)`.
    pub fn label(&self, index: usize) -> String {
        let height = self.bounds.bottom - self.bounds.top;
        let primary = if self.primary { " (primary)" } else { "" };
        format!("Display {} · {} × {}{}", index + 1, self.bounds.width(), height, primary)
    }
}

/// All attached monitors in system enumeration order.
pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(monitor);
        true.into()
    }
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut handles as *mut _ as isize));
    }
    handles
        .into_iter()
        .filter_map(|handle| {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo as *mut _) }.ok().ok()?;
            let (mut dpi_x, mut dpi_y) = (96, 96);
            unsafe {
                let _ = GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            }
            let len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
            Some(Monitor {
                device: String::from_utf16_lossy(&info.szDevice[..len]),
                bounds: info.monitorInfo.rcMonitor.into(),
                work: info.monitorInfo.rcWork.into(),
                scale: dpi_x as f32 / 96.0,
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            })
        })
        .collect()
}

/// Picks the preferred monitor if it is attached, otherwise the primary one.
pub fn choose<'a>(monitors: &'a [Monitor], preferred: Option<&str>) -> Option<&'a Monitor> {
    preferred
        .and_then(|device| monitors.iter().find(|m| m.device == device))
        .or_else(|| monitors.iter().find(|m| m.primary))
        .or_else(|| monitors.first())
}

/// Top-left corner, in physical pixels, for a window of `size` whose centre sits
/// `offset_x` logical pixels from the centre of the monitor's work area and whose top
/// edge is `top_margin` logical pixels below the work area's top. The window is kept
/// fully on the monitor.
pub fn island_origin(monitor: &Monitor, size: (i32, i32), offset_x: i32, top_margin: u32) -> (i32, i32) {
    let work = monitor.work;
    let centre = work.left + work.width() / 2 + (offset_x as f32 * monitor.scale).round() as i32;
    let max_left = (monitor.bounds.right - size.0).max(monitor.bounds.left);
    let x = (centre - size.0 / 2).clamp(monitor.bounds.left, max_left);
    let y = work.top + (top_margin as f32 * monitor.scale).round() as i32;
    (x, y)
}

#[derive(Clone, Copy)]
pub struct HorizontalDrag {
    left: i32,
    pointer_x: i32,
}

impl HorizontalDrag {
    pub fn new(left: i32, pointer_x: i32) -> Self {
        Self { left, pointer_x }
    }

    pub fn left_at(self, pointer_x: i32) -> i32 {
        self.left.saturating_add(pointer_x.saturating_sub(self.pointer_x))
    }
}

/// A small magnet around the work area's horizontal center, in physical pixels.
pub fn snap_center(monitor: &Monitor, left: i32, width: i32) -> Option<i32> {
    let centered = monitor.work.left + monitor.work.width() / 2 - width / 2;
    ((left - centered).abs() <= (24.0 * monitor.scale).round() as i32).then_some(centered)
}

/// Inverse of [`island_origin`] for the horizontal axis: the logical offset of a window
/// whose left edge is at `left`.
pub fn offset_for(monitor: &Monitor, left: i32, width: i32) -> i32 {
    let work = monitor.work;
    let centre = left + width / 2;
    ((centre - (work.left + work.width() / 2)) as f32 / monitor.scale).round() as i32
}

/// The monitor containing a physical point, if any.
pub fn monitor_at(monitors: &[Monitor], x: i32, y: i32) -> Option<&Monitor> {
    monitors.iter().find(|m| m.bounds.contains(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(left: i32, width: i32, scale: f32, primary: bool, taskbar_top: i32) -> Monitor {
        Monitor {
            device: format!(r"\\.\DISPLAY{left}"),
            bounds: Rect { left, top: 0, right: left + width, bottom: 1080 },
            work: Rect { left, top: taskbar_top, right: left + width, bottom: 1032 },
            scale,
            primary,
        }
    }

    #[test]
    fn slow_drags_escape_center_in_both_directions() {
        let m = monitor(0, 1920, 1.0, true, 0);
        for direction in [-1, 1] {
            let drag = HorizontalDrag::new(730, 960);
            let mut left = 730;
            for distance in 1..=60 {
                let proposed = drag.left_at(960 + direction * distance);
                left = snap_center(&m, proposed, 460).unwrap_or(proposed);
            }
            assert_eq!(left, 730 + direction * 60, "small pointer deltas must accumulate past the snap zone");
        }
    }

    #[test]
    fn center_snap_is_scaled_and_can_be_escaped() {
        let m = monitor(-1920, 1920, 1.5, false, 0);
        let center = -960 - 690 / 2;
        assert_eq!(snap_center(&m, center + 35, 690), Some(center));
        assert_eq!(snap_center(&m, center - 35, 690), Some(center));
        assert_eq!(snap_center(&m, center + 37, 690), None);
    }

    #[test]
    fn centred_below_work_area_top() {
        let m = monitor(0, 1920, 1.0, true, 0);
        assert_eq!(island_origin(&m, (460, 260), 0, 8), (730, 8));
    }

    #[test]
    fn respects_scale_and_top_taskbar() {
        let m = monitor(0, 2880, 1.5, true, 72);
        assert_eq!(island_origin(&m, (690, 390), 0, 8), (1095, 84));
        assert_eq!(island_origin(&m, (690, 390), 100, 0), (1245, 72));
    }

    #[test]
    fn stays_on_screen() {
        let m = monitor(1920, 1280, 1.0, false, 0);
        assert_eq!(island_origin(&m, (460, 260), -5000, 8).0, 1920);
        assert_eq!(island_origin(&m, (460, 260), 5000, 8).0, 1920 + 1280 - 460);
    }

    #[test]
    fn offset_round_trips() {
        let m = monitor(0, 2880, 1.5, true, 0);
        for offset in [-300, -1, 0, 1, 250] {
            let (left, _) = island_origin(&m, (690, 390), offset, 8);
            assert!((offset_for(&m, left, 690) - offset).abs() <= 1, "{offset}");
        }
    }

    #[test]
    fn chooses_preferred_then_primary() {
        let list = vec![monitor(0, 1920, 1.0, false, 0), monitor(1920, 1920, 1.0, true, 0)];
        assert_eq!(choose(&list, Some(r"\\.\DISPLAY0")).unwrap().bounds.left, 0);
        assert_eq!(choose(&list, Some(r"\\.\DISPLAY9")).unwrap().bounds.left, 1920);
        assert_eq!(choose(&list, None).unwrap().bounds.left, 1920);
        assert!(choose(&[], None).is_none());
    }

    #[test]
    fn finds_monitor_at_point() {
        let list = vec![monitor(0, 1920, 1.0, true, 0), monitor(1920, 1280, 1.0, false, 0)];
        assert_eq!(monitor_at(&list, 2000, 10).unwrap().bounds.left, 1920);
        assert!(monitor_at(&list, -10, 10).is_none());
    }

    #[test]
    fn real_monitors_are_enumerated() {
        let list = monitors();
        assert!(!list.is_empty());
        assert!(list.iter().any(|m| m.primary));
        assert!(list.iter().all(|m| m.scale >= 1.0 && m.bounds.width() > 0));
    }
}
