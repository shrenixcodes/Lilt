//! Single-instance guard based on a named mutex in the session namespace.

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::w;

/// Held for the lifetime of the process; releasing it lets a new instance start.
pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Returns `None` if another Lilt process already holds the guard in this session.
pub fn acquire() -> Option<InstanceGuard> {
    unsafe {
        let handle = CreateMutexW(None, true, w!("Local\\Lilt.SingleInstance")).ok()?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            return None;
        }
        Some(InstanceGuard(handle))
    }
}
