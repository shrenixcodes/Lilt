//! A 32-bit DIB section that Slint's software renderer draws into directly.

use slint::platform::software_renderer::{PremultipliedRgbaColor, TargetPixel};
use windows::Win32::Foundation::{HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, BitBlt, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{ULW_ALPHA, UPDATELAYEREDWINDOWINFO, UpdateLayeredWindowIndirect};

/// Premultiplied BGRA, the layout expected by GDI and `UpdateLayeredWindow`.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Bgra {
    pub b: u8,
    pub g: u8,
    pub r: u8,
    pub a: u8,
}

impl TargetPixel for Bgra {
    fn blend(&mut self, color: PremultipliedRgbaColor) {
        let inverse = 255 - color.alpha as u16;
        let mix = |dst: u8, src: u8| (src as u16 + (dst as u16 * inverse + 127) / 255) as u8;
        self.r = mix(self.r, color.red);
        self.g = mix(self.g, color.green);
        self.b = mix(self.b, color.blue);
        self.a = mix(self.a, color.alpha);
    }

    fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self { b, g, r, a: 255 }
    }

    fn background() -> Self {
        Self::default()
    }
}

pub struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut Bgra,
    pub width: u32,
    pub height: u32,
}

impl Surface {
    pub fn new(width: u32, height: u32) -> Option<Self> {
        let (width, height) = (width.max(1), height.max(1));
        unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    // Negative height: rows are stored top-down, matching Slint's buffer.
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(dc);
                return None;
            };
            let previous = SelectObject(dc, bitmap.into());
            Some(Self { dc, bitmap, previous, bits: bits.cast(), width, height })
        }
    }

    pub fn pixels(&mut self) -> &mut [Bgra] {
        // SAFETY: the DIB section owns width * height pixels for the lifetime of `self`.
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.width * self.height) as usize) }
    }

    /// Presents to a layered window. Fully transparent pixels let input pass through.
    pub fn present_layered(&self, hwnd: HWND, dirty: Option<RECT>) {
        let size = SIZE { cx: self.width as i32, cy: self.height as i32 };
        let origin = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let info = UPDATELAYEREDWINDOWINFO {
            cbSize: std::mem::size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
            psize: &size,
            hdcSrc: self.dc,
            pptSrc: &origin,
            pblend: &blend,
            dwFlags: ULW_ALPHA,
            prcDirty: dirty.as_ref().map_or(std::ptr::null(), |r| r),
            ..Default::default()
        };
        unsafe {
            if !UpdateLayeredWindowIndirect(hwnd, &info).as_bool() && dirty.is_some() {
                // A dirty rectangle outside the current window size is rejected;
                // fall back to a full update.
                let full = UPDATELAYEREDWINDOWINFO { prcDirty: std::ptr::null(), ..info };
                let _ = UpdateLayeredWindowIndirect(hwnd, &full);
            }
        }
    }

    /// Copies a region to an ordinary window's device context.
    pub fn blit(&self, target: HDC, area: RECT) {
        unsafe {
            let _ = BitBlt(
                target,
                area.left,
                area.top,
                area.right - area.left,
                area.bottom - area.top,
                Some(self.dc),
                area.left,
                area.top,
                SRCCOPY,
            );
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blending_is_premultiplied_source_over() {
        let mut px = Bgra::default();
        px.blend(PremultipliedRgbaColor { red: 100, green: 50, blue: 25, alpha: 128 });
        assert_eq!(px, Bgra { b: 25, g: 50, r: 100, a: 128 });
        px.blend(PremultipliedRgbaColor { red: 0, green: 0, blue: 0, alpha: 255 });
        assert_eq!(px, Bgra { b: 0, g: 0, r: 0, a: 255 });
    }

    #[test]
    fn surface_has_expected_pixel_count() {
        let mut s = Surface::new(7, 3).expect("DIB section");
        assert_eq!(s.pixels().len(), 21);
        assert!(s.pixels().iter().all(|p| *p == Bgra::default()));
    }
}
