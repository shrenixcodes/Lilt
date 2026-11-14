//! Reads what lies behind the island, for the glass backdrop and its tint.
//!
//! Windows includes layered windows in screen reads, so the island is excluded from
//! capture (`WDA_EXCLUDEFROMCAPTURE`) for the one compositor frame of the read: the result
//! is the desktop and the windows behind Lilt, and the island stays on screen throughout.
//! Screenshots and recordings of Lilt are unaffected outside that frame. For a blurred
//! backdrop the area is reduced straight away (each pixel the average of a block of the
//! screen) and then blurred; for clear glass it is read at full resolution. That image is
//! all that is kept, in memory, to draw the glass.

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GdiFlush, GetDC, HALFTONE, ReleaseDC, SRCCOPY, SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchBlt,
};

use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE};

use super::glass::Backdrop;

/// How a blur of about `blur` screen pixels is made: read the screen reduced by `block`
/// in each direction, then blur that with three box passes of `radius` image pixels.
/// Below one pixel there is no blur and the screen is read at full resolution.
pub fn blur_plan(blur: f32) -> (usize, usize) {
    if blur < 1.0 {
        return (1, 0);
    }
    let block = (blur / 2.5).round().clamp(1.0, 8.0);
    let radius = (blur / block - 0.5).round().max(1.0);
    (block as usize, radius as usize)
}

/// The desktop behind `area` (screen pixels) without the window `own`, blurred by about
/// `blur` screen pixels.
pub fn backdrop_behind(area: RECT, blur: f32, own: HWND) -> Option<Backdrop> {
    let (width, height) = (area.right - area.left, area.bottom - area.top);
    if width <= 0 || height <= 0 {
        return None;
    }
    let (block, radius) = blur_plan(blur);
    let b = block as i32;
    let (columns, rows) = ((width + b - 1) / b, (height + b - 1) / b);
    let pixels = unsafe {
        // Leave the island out of the read; on Windows before 10 2004, where this is not
        // available, the read still works but includes it.
        let excluded = SetWindowDisplayAffinity(own, WDA_EXCLUDEFROMCAPTURE).is_ok();
        if excluded {
            let _ = DwmFlush();
        }
        let pixels = read(area, width, height, columns, rows);
        if excluded {
            let _ = SetWindowDisplayAffinity(own, WDA_NONE);
        }
        pixels
    }?;
    Some(Backdrop::new(columns as usize, rows as usize, block, pixels, radius))
}

unsafe fn read(area: RECT, width: i32, height: i32, columns: i32, rows: i32) -> Option<Vec<[u8; 3]>> {
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let dc = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: columns,
                biHeight: -rows,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let pixels = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => {
                let previous = SelectObject(dc, bitmap.into());
                // Halftone averages every source pixel into its block; at full size it is
                // a plain copy.
                SetStretchBltMode(dc, HALFTONE);
                let _ = SetBrushOrgEx(dc, 0, 0, None);
                let copied =
                    StretchBlt(dc, 0, 0, columns, rows, Some(screen), area.left, area.top, width, height, SRCCOPY)
                        .as_bool();
                let _ = GdiFlush();
                let pixels = copied.then(|| {
                    // SAFETY: the DIB section holds columns * rows 32-bit BGRA pixels.
                    let raw = std::slice::from_raw_parts(bits as *const [u8; 4], (columns * rows) as usize);
                    raw.iter().map(|p| [p[2], p[1], p[0]]).collect()
                });
                SelectObject(dc, previous);
                let _ = DeleteObject(bitmap.into());
                pixels
            }
            Err(_) => None,
        };
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three box passes of radius r approximate a Gaussian of sd sqrt(r (r + 1)).
    fn spread((block, radius): (usize, usize)) -> f32 {
        block as f32 * ((radius * (radius + 1)) as f32).sqrt()
    }

    #[test]
    fn the_plan_matches_the_requested_blur() {
        assert_eq!(blur_plan(0.0), (1, 0), "clear glass: full resolution, no blur");
        assert_eq!(blur_plan(20.0), (8, 2), "the default, as before");
        for blur in [2.0f32, 5.0, 10.0, 20.0, 30.0] {
            let got = spread(blur_plan(blur));
            assert!((got - blur).abs() / blur < 0.35, "{blur}: {got}");
        }
        // Heavier blurs read fewer pixels, never more than one per 8 x 8 block.
        assert_eq!(blur_plan(40.0).0, 8);
        assert!(blur_plan(40.0).1 > blur_plan(20.0).1);
    }
}
