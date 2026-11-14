//! Samples the colour of what lies behind the island, for Liquid Glass's tint.
//!
//! The screen is read without `CAPTUREBLT`, so layered windows, the island and its blur
//! among them, are not included: the result is the desktop and the windows behind Lilt.
//! The area is reduced to a 16 × 4 grid of averages straight away and only the brightest
//! cell is kept; no image of the screen is stored.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GdiFlush, GetDC, HALFTONE, ReleaseDC, SRCCOPY, SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchBlt,
};

use crate::ambient::{Rgb, relative_luminance};

const COLUMNS: i32 = 16;
const ROWS: i32 = 4;

/// The brightest of a grid of average colours over `area`, in screen pixels.
pub fn brightest_behind(area: RECT) -> Option<Rgb> {
    let (width, height) = (area.right - area.left, area.bottom - area.top);
    if width <= 0 || height <= 0 {
        return None;
    }
    let cells = unsafe { grid(area, width, height) }?;
    cells.into_iter().max_by(|a, b| relative_luminance(*a).total_cmp(&relative_luminance(*b)))
}

unsafe fn grid(area: RECT, width: i32, height: i32) -> Option<Vec<Rgb>> {
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let dc = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: COLUMNS,
                biHeight: -ROWS,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let cells = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => {
                let previous = SelectObject(dc, bitmap.into());
                // Halftone averages every source pixel into its cell.
                SetStretchBltMode(dc, HALFTONE);
                let _ = SetBrushOrgEx(dc, 0, 0, None);
                let copied =
                    StretchBlt(dc, 0, 0, COLUMNS, ROWS, Some(screen), area.left, area.top, width, height, SRCCOPY)
                        .as_bool();
                let _ = GdiFlush();
                let cells = copied.then(|| {
                    // SAFETY: the DIB section holds COLUMNS * ROWS 32-bit BGRA pixels.
                    let pixels = std::slice::from_raw_parts(bits as *const [u8; 4], (COLUMNS * ROWS) as usize);
                    pixels.iter().map(|p| Rgb::new(p[2], p[1], p[0])).collect()
                });
                SelectObject(dc, previous);
                let _ = DeleteObject(bitmap.into());
                cells
            }
            Err(_) => None,
        };
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        cells
    }
}
