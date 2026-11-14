//! Plain-text clipboard for Slint's text fields (copy, cut and paste).

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// Opens the clipboard for the duration of `f`.
fn with_clipboard<T>(f: impl FnOnce() -> Option<T>) -> Option<T> {
    // Another application may hold the clipboard for a moment.
    for _ in 0..5 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            let result = f();
            unsafe {
                let _ = CloseClipboard();
            }
            return result;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

pub fn text() -> Option<String> {
    unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32) }.ok()?;
    with_clipboard(|| unsafe {
        let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
        let memory = HGLOBAL(handle.0);
        let data = GlobalLock(memory) as *const u16;
        if data.is_null() {
            return None;
        }
        let mut len = 0;
        while *data.add(len) != 0 && len < 1 << 20 {
            len += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(data, len));
        let _ = GlobalUnlock(memory);
        Some(text)
    })
}

pub fn set_text(text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    with_clipboard(|| unsafe {
        EmptyClipboard().ok()?;
        let memory = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ok()?;
        let data = GlobalLock(memory) as *mut u16;
        if data.is_null() {
            let _ = GlobalFree(Some(memory));
            return None;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), data, wide.len());
        let _ = GlobalUnlock(memory);
        // On success the clipboard owns the memory.
        if SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(memory.0))).is_err() {
            let _ = GlobalFree(Some(memory));
        }
        Some(())
    });
}
