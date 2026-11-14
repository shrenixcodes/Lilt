//! "Start with Windows", stored as the per-user `Run` registry value that the installer
//! also writes, so both always agree.

use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{HSTRING, PCWSTR, w};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE: PCWSTR = w!("Lilt");

/// The command line registered for this executable.
fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

/// Whether Lilt is set to start at sign-in (for any install location).
pub fn is_enabled() -> bool {
    let mut size = 0u32;
    let result = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE, RRF_RT_REG_SZ, None, None, Some(&mut size)) };
    result.is_ok() && size > 2
}

pub fn set_enabled(enabled: bool) -> windows::core::Result<()> {
    unsafe {
        if enabled {
            let command = HSTRING::from(command().unwrap_or_default());
            let bytes = (command.len() + 1) * std::mem::size_of::<u16>();
            RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE, REG_SZ.0, Some(command.as_ptr().cast()), bytes as u32)
                .ok()
        } else {
            match RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE) {
                e if e == ERROR_FILE_NOT_FOUND => Ok(()),
                e => e.ok(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_quotes_the_executable_path() {
        let command = super::command().unwrap();
        assert!(command.starts_with('"') && command.ends_with(".exe\""), "{command}");
    }
}
