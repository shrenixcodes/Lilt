//! Native Windows dialogs.

use std::path::PathBuf;

use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH,
};
use windows::core::w;

/// Shows the system "Open" dialog for an `.ics` calendar file. Returns `None` if the
/// user cancels. Must be called on a thread initialised for COM in a single-threaded
/// apartment, and not from inside UI event dispatch, because the dialog runs a modal
/// message loop.
pub fn pick_calendar_file() -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let filters = [
            COMDLG_FILTERSPEC { pszName: w!("Calendar files (*.ics)"), pszSpec: w!("*.ics") },
            COMDLG_FILTERSPEC { pszName: w!("All files"), pszSpec: w!("*.*") },
        ];
        dialog.SetFileTypes(&filters).ok()?;
        dialog.SetTitle(w!("Choose a calendar file")).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM).ok()?;
        // Cancelling returns an error, which ends here as `None`.
        dialog.Show(None).ok()?;
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const _));
        path.map(PathBuf::from)
    }
}

/// Shows the system "Open" dialog for an application or shortcut file (.exe, .lnk, .url, .cmd, .bat).
pub fn pick_app_file() -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let filters = [
            COMDLG_FILTERSPEC {
                pszName: w!("Applications & Shortcuts (*.exe, *.lnk, *.url, *.cmd, *.bat)"),
                pszSpec: w!("*.exe;*.lnk;*.url;*.cmd;*.bat"),
            },
            COMDLG_FILTERSPEC { pszName: w!("Executables (*.exe)"), pszSpec: w!("*.exe") },
            COMDLG_FILTERSPEC { pszName: w!("Shortcuts (*.lnk)"), pszSpec: w!("*.lnk") },
            COMDLG_FILTERSPEC { pszName: w!("All files (*.*)"), pszSpec: w!("*.*") },
        ];
        dialog.SetFileTypes(&filters).ok()?;
        dialog.SetTitle(w!("Choose an application or shortcut")).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM).ok()?;
        dialog.Show(None).ok()?;
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const _));
        path.map(PathBuf::from)
    }
}
