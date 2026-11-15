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
    pub source: Option<String>,
}

fn format(name: PCWSTR) -> u32 {
    unsafe { RegisterClipboardFormatW(name) }
}

fn available(format: u32) -> bool {
    format != 0 && unsafe { IsClipboardFormatAvailable(format) }.is_ok()
}

/// Reads the clipboard's text if it may be remembered. Formats an app uses to keep its
/// copy private are checked first, then the app, and only then is the text read.
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
    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32) }.is_err() {
        return Err(Skip::NotText);
    }
    let history = format(w!("CanIncludeInClipboardHistory"));
    let text = with_clipboard(|| Some(unsafe { open_text(history) })).unwrap_or(Err(Skip::Unavailable))?;
    if text.trim().is_empty() {
        return Err(Skip::Empty);
    }
    Ok(Copied { text, source })
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
