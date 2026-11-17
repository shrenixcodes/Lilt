//! Reading and writing the Windows clipboard for the shelf.
//!
//! Lilt is told about clipboard changes with `AddClipboardFormatListener` on its control
//! window (`WM_CLIPBOARDUPDATE`); there is no polling. Reading happens on the clipboard
//! worker thread, never on the UI thread, and checks the privacy formats and the copying
//! app before it touches the text.

use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardOwner,
    GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
    RemoveClipboardFormatListener, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
use windows::core::{PCWSTR, PWSTR, w};

use super::{MAX_CHARS, Skip};

pub use windows::Win32::UI::WindowsAndMessaging::WM_CLIPBOARDUPDATE;

/// Starts clipboard change notifications to `window`.
pub fn listen(window: HWND) -> bool {
    unsafe { AddClipboardFormatListener(window) }.is_ok()
}

pub fn stop_listening(window: HWND) {
    unsafe {
        let _ = RemoveClipboardFormatListener(window);
    }
}

/// Changes with every clipboard change; used to skip a change already seen.
pub fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// What the capture checks need to know.
#[derive(Clone, Debug, Default)]
pub struct Policy {
    pub excluded_apps: Vec<String>,
    pub skip_unknown_sources: bool,
}

/// Copied text and the executable name of the app that copied it, when Windows says.
#[derive(Debug, PartialEq)]
pub struct Copied {
    pub text: String,
    pub image: Option<Vec<u8>>,
    pub source: Option<String>,
}

fn format(name: PCWSTR) -> u32 {
    unsafe { RegisterClipboardFormatW(name) }
}

fn available(format: u32) -> bool {
    format != 0 && unsafe { IsClipboardFormatAvailable(format) }.is_ok()
}

/// Reads the clipboard's text or image if it may be remembered. Formats an app uses to keep its
/// copy private are checked first, then the app, and only then is the content read.
pub fn read(policy: &Policy) -> Result<Copied, Skip> {
    // Set by password managers and other apps that copy sensitive data.
    if available(format(w!("ExcludeClipboardContentFromMonitorProcessing")))
        || available(format(w!("Clipboard Viewer Ignore")))
    {
        return Err(Skip::ExcludedByApp);
    }
    let (pid, source) = owner();
    if pid == Some(unsafe { GetCurrentProcessId() }) {
        return Err(Skip::OwnCopy);
    }
    match &source {
        Some(exe) if super::is_excluded(exe, &policy.excluded_apps) => return Err(Skip::ExcludedApp),
        None if policy.skip_unknown_sources => return Err(Skip::UnknownSource),
        _ => {}
    }

    let history = format(w!("CanIncludeInClipboardHistory"));

    // Check for text first
    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32) }.is_ok() {
        let text = with_clipboard(|| Some(unsafe { open_text(history) })).unwrap_or(Err(Skip::Unavailable))?;
        if text.trim().is_empty() {
            return Err(Skip::Empty);
        }
        return Ok(Copied { text, image: None, source });
    }

    // Check for image formats: PNG or CF_DIB (8) or CF_DIBV5 (17)
    let png_fmt = format(w!("PNG"));
    const CF_DIB: u32 = 8;
    const CF_DIBV5: u32 = 17;
    if available(png_fmt) || available(CF_DIB) || available(CF_DIBV5) {
        let image =
            with_clipboard(|| Some(unsafe { open_image(png_fmt, history) })).unwrap_or(Err(Skip::Unavailable))?;
        return Ok(Copied { text: "Screenshot".to_owned(), image: Some(image), source });
    }

    Err(Skip::NotText)
}

/// Reads the text of the open clipboard, unless the copying app asked for it to be kept
/// out of clipboard history (`CanIncludeInClipboardHistory` set to zero).
///
/// # Safety
/// The clipboard must be open on this thread.
unsafe fn open_text(history: u32) -> Result<String, Skip> {
    unsafe {
        if available(history)
            && let Ok(handle) = GetClipboardData(history)
        {
            let memory = HGLOBAL(handle.0);
            let value = GlobalLock(memory) as *const u32;
            let allowed = value.is_null() || GlobalSize(memory) < 4 || *value != 0;
            let _ = GlobalUnlock(memory);
            if !allowed {
                return Err(Skip::ExcludedByApp);
            }
        }
        let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).map_err(|_| Skip::Unavailable)?;
        let memory = HGLOBAL(handle.0);
        let units = GlobalSize(memory) / 2;
        if units > MAX_CHARS + 1 {
            return Err(Skip::TooLarge);
        }
        let data = GlobalLock(memory) as *const u16;
        if data.is_null() {
            return Err(Skip::Unavailable);
        }
        let slice = std::slice::from_raw_parts(data, units);
        let len = slice.iter().position(|&c| c == 0).unwrap_or(units);
        let text = String::from_utf16_lossy(&slice[..len]);
        let _ = GlobalUnlock(memory);
        Ok(text)
    }
}

unsafe fn open_image(png_fmt: u32, history: u32) -> Result<Vec<u8>, Skip> {
    unsafe {
        if available(history)
            && let Ok(handle) = GetClipboardData(history)
        {
            let memory = HGLOBAL(handle.0);
            let value = GlobalLock(memory) as *const u32;
            let allowed = value.is_null() || GlobalSize(memory) < 4 || *value != 0;
            let _ = GlobalUnlock(memory);
            if !allowed {
                return Err(Skip::ExcludedByApp);
            }
        }

        // Try PNG format first
        if available(png_fmt)
            && let Ok(handle) = GetClipboardData(png_fmt)
        {
            let memory = HGLOBAL(handle.0);
            let size = GlobalSize(memory);
            let data = GlobalLock(memory) as *const u8;
            if !data.is_null() && size > 0 {
                let slice = std::slice::from_raw_parts(data, size);
                let loaded = image::load_from_memory(slice);
                let _ = GlobalUnlock(memory);
                if let Ok(img) = loaded {
                    return compress_thumbnail(img);
                }
            } else {
                let _ = GlobalUnlock(memory);
            }
        }

        // Try CF_DIB (8) or CF_DIBV5 (17)
        const CF_DIB: u32 = 8;
        const CF_DIBV5: u32 = 17;
        let fmt = if available(CF_DIB) { CF_DIB } else { CF_DIBV5 };
        let handle = GetClipboardData(fmt).map_err(|_| Skip::Unavailable)?;
        let memory = HGLOBAL(handle.0);
        let size = GlobalSize(memory);
        let data = GlobalLock(memory) as *const u8;
        if data.is_null() || size < 40 {
            let _ = GlobalUnlock(memory);
            return Err(Skip::Unavailable);
        }
        let dib_slice = std::slice::from_raw_parts(data, size);

        let bi_size = u32::from_le_bytes(dib_slice[0..4].try_into().unwrap_or_default()) as usize;
        let bi_bit_count = u16::from_le_bytes(dib_slice[14..16].try_into().unwrap_or_default());
        let bi_compression = u32::from_le_bytes(dib_slice[16..20].try_into().unwrap_or_default());
        let bi_clr_used = if dib_slice.len() >= 36 {
            u32::from_le_bytes(dib_slice[32..36].try_into().unwrap_or_default()) as usize
        } else {
            0
        };
        let palette_or_masks = if bi_compression == 3 {
            12
        } else if bi_clr_used > 0 {
            bi_clr_used * 4
        } else if bi_bit_count <= 8 && bi_bit_count > 0 {
            (1 << bi_bit_count) * 4
        } else {
            0
        };
        let off_bits = (14 + bi_size + palette_or_masks) as u32;
        let file_size = (14 + size) as u32;

        let mut bmp = Vec::with_capacity(14 + size);
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&file_size.to_le_bytes());
        bmp.extend_from_slice(&[0u8; 4]);
        bmp.extend_from_slice(&off_bits.to_le_bytes());
        bmp.extend_from_slice(dib_slice);
        let _ = GlobalUnlock(memory);

        let img = image::load_from_memory(&bmp).map_err(|_| Skip::Unavailable)?;
        compress_thumbnail(img)
    }
}

fn compress_thumbnail(img: image::DynamicImage) -> Result<Vec<u8>, Skip> {
    let thumb = if img.width() > super::THUMBNAIL_MAX_SIDE || img.height() > super::THUMBNAIL_MAX_SIDE {
        img.thumbnail(super::THUMBNAIL_MAX_SIDE, super::THUMBNAIL_MAX_SIDE)
    } else {
        img
    };
    let mut png = Vec::new();
    thumb.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|_| Skip::Unavailable)?;
    if png.len() > super::MAX_THUMBNAIL_BYTES {
        return Err(Skip::TooLarge);
    }
    Ok(png)
}

/// Puts image data on the clipboard in DIB and PNG formats.
pub fn write_image(png_bytes: &[u8]) -> Option<u32> {
    let img = image::load_from_memory(png_bytes).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let dib_size = 40 + (width as usize) * (height as usize) * 4;
    let mut dib = Vec::with_capacity(dib_size);
    dib.extend_from_slice(&(40u32).to_le_bytes());
    dib.extend_from_slice(&(width as i32).to_le_bytes());
    dib.extend_from_slice(&(height as i32).to_le_bytes());
    dib.extend_from_slice(&(1u16).to_le_bytes());
    dib.extend_from_slice(&(32u16).to_le_bytes());
    dib.extend_from_slice(&(0u32).to_le_bytes());
    dib.extend_from_slice(&(width * height * 4).to_le_bytes());
    dib.extend_from_slice(&[0u8; 16]);

    for y in (0..height).rev() {
        for x in 0..width {
            let pixel = img.get_pixel(x, y);
            dib.push(pixel[2]);
            dib.push(pixel[1]);
            dib.push(pixel[0]);
            dib.push(pixel[3]);
        }
    }

    let png_fmt = format(w!("PNG"));
    const CF_DIB: u32 = 8;

    with_clipboard(|| unsafe {
        EmptyClipboard().ok()?;

        let dib_mem = GlobalAlloc(GMEM_MOVEABLE, dib.len()).ok()?;
        let data = GlobalLock(dib_mem) as *mut u8;
        if !data.is_null() {
            std::ptr::copy_nonoverlapping(dib.as_ptr(), data, dib.len());
            let _ = GlobalUnlock(dib_mem);
            let _ = SetClipboardData(CF_DIB, Some(HANDLE(dib_mem.0)));
        } else {
            let _ = GlobalFree(Some(dib_mem));
        }

        if png_fmt != 0
            && let Ok(png_mem) = GlobalAlloc(GMEM_MOVEABLE, png_bytes.len())
        {
            let data = GlobalLock(png_mem) as *mut u8;
            if !data.is_null() {
                std::ptr::copy_nonoverlapping(png_bytes.as_ptr(), data, png_bytes.len());
                let _ = GlobalUnlock(png_mem);
                let _ = SetClipboardData(png_fmt, Some(HANDLE(png_mem.0)));
            } else {
                let _ = GlobalFree(Some(png_mem));
            }
        }

        Some(())
    })?;
    Some(sequence())
}

/// Puts `text` on the clipboard. Returns the clipboard's new sequence number, so the
/// change it causes is not captured again.
pub fn write(text: &str) -> Option<u32> {
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
            return None;
        }
        Some(())
    })?;
    Some(sequence())
}

/// Opens the clipboard for `f`. Another app may hold it for a moment, so this retries
/// for up to about a quarter of a second; it runs on the worker thread.
fn with_clipboard<T>(f: impl FnOnce() -> Option<T>) -> Option<T> {
    for attempt in 0..8 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            let result = f();
            unsafe {
                let _ = CloseClipboard();
            }
            return result;
        }
        std::thread::sleep(Duration::from_millis(10 + attempt * 5));
    }
    None
}

/// The process that owns the clipboard and its executable's file name. Windows does not
/// always know (an app may copy without a window), and then neither does Lilt.
fn owner() -> (Option<u32>, Option<String>) {
    let window = unsafe { GetClipboardOwner() }.ok().filter(|w| !w.is_invalid());
    let Some(window) = window else { return (None, None) };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    if pid == 0 {
        return (None, None);
    }
    let name = unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok().and_then(|process| {
            let mut buffer = [0u16; 1024];
            let mut size = buffer.len() as u32;
            let result = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size);
            let _ = CloseHandle(process);
            result.ok()?;
            let path = String::from_utf16_lossy(&buffer[..size as usize]);
            path.rsplit(['\\', '/']).next().map(str::to_owned)
        })
    };
    (Some(pid), name)
}

/// Owner lookup and the privacy formats need the real clipboard. These tests replace
/// what is on it (text is put back, anything else is lost), so they only run on request:
/// `cargo test -- --ignored clipboard::capture`.
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The clipboard is shared by every test in the process.
    static CLIPBOARD: Mutex<()> = Mutex::new(());

    #[test]
    #[ignore = "uses the system clipboard"]
    fn own_copies_are_recognised_and_text_round_trips() {
        let _guard = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
        let saved = crate::window::clipboard::text();
        let before = sequence();
        let Some(after) = write("Grüße 👋\r\nzweite Zeile") else {
            // No interactive desktop (for example a service account): nothing to test.
            return;
        };
        assert_ne!(before, after);
        // Lilt wrote it without an owner window, so the owner is unknown, not Lilt; the
        // worker skips it by its sequence number instead.
        let copied = read(&Policy::default());
        assert_eq!(copied.map(|c| c.text), Ok("Grüße 👋\r\nzweite Zeile".to_owned()));
        assert_eq!(read(&Policy { skip_unknown_sources: true, ..Default::default() }), Err(Skip::UnknownSource));
        if let Some(saved) = saved {
            write(&saved);
        }
    }

    #[test]
    #[ignore = "uses the system clipboard"]
    fn private_copies_are_skipped_before_reading() {
        let _guard = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
        let saved = crate::window::clipboard::text();
        let wide: Vec<u16> = "hunter22".encode_utf16().chain(Some(0)).collect();
        let written = with_clipboard(|| unsafe {
            EmptyClipboard().ok()?;
            let memory = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ok()?;
            let data = GlobalLock(memory) as *mut u16;
            std::ptr::copy_nonoverlapping(wide.as_ptr(), data, wide.len());
            let _ = GlobalUnlock(memory);
            SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(memory.0))).ok()?;
            // The marker's content does not matter, only its presence.
            let marker = GlobalAlloc(GMEM_MOVEABLE, 4).ok()?;
            SetClipboardData(format(w!("ExcludeClipboardContentFromMonitorProcessing")), Some(HANDLE(marker.0)))
                .ok()?;
            Some(())
        });
        if written.is_none() {
            return;
        }
        assert_eq!(read(&Policy::default()), Err(Skip::ExcludedByApp));
        if let Some(saved) = saved {
            write(&saved);
        }
    }
}
