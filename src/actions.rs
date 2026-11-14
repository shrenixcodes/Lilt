//! Quick actions. Each action opens a documented Windows Settings page or a built-in
//! application; Lilt does not change hardware or system settings itself.

use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickAction {
    Sound,
    Display,
    Bluetooth,
    Wifi,
    Settings,
    TaskManager,
    Snip,
    Explorer,
}

impl QuickAction {
    /// Order matches the icons in the UI (`ui/quick-actions.slint`).
    pub const ALL: [QuickAction; 8] = [
        Self::Sound,
        Self::Display,
        Self::Bluetooth,
        Self::Wifi,
        Self::Snip,
        Self::TaskManager,
        Self::Explorer,
        Self::Settings,
    ];

    pub fn from_index(index: i32) -> Option<Self> {
        usize::try_from(index).ok().and_then(|i| Self::ALL.get(i).copied())
    }

    /// URI or executable launched by the action.
    /// Settings URIs: <https://learn.microsoft.com/windows/apps/develop/launch/launch-settings-app>
    pub fn target(self) -> &'static str {
        match self {
            Self::Sound => "ms-settings:sound",
            // Brightness lives on the display page on devices that support it.
            Self::Display => "ms-settings:display",
            Self::Bluetooth => "ms-settings:bluetooth",
            Self::Wifi => "ms-settings:network-wifi",
            Self::Settings => "ms-settings:",
            Self::TaskManager => "taskmgr.exe",
            Self::Snip => "ms-screenclip:",
            Self::Explorer => "explorer.exe",
        }
    }

    pub fn launch(self) -> windows::core::Result<()> {
        let target = HSTRING::from(self.target());
        let result = unsafe { ShellExecuteW(None, w!("open"), &target, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
        // ShellExecuteW reports success with a value greater than 32.
        if result.0 as isize > 32 { Ok(()) } else { Err(windows::core::Error::from_thread()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_map_to_actions() {
        assert_eq!(QuickAction::from_index(0), Some(QuickAction::Sound));
        assert_eq!(QuickAction::from_index(7), Some(QuickAction::Settings));
        assert_eq!(QuickAction::from_index(8), None);
        assert_eq!(QuickAction::from_index(-1), None);
    }

    #[test]
    fn targets_are_settings_uris_or_system_tools() {
        for action in QuickAction::ALL {
            let t = action.target();
            assert!(t.ends_with(':') || t.starts_with("ms-settings:") || t.ends_with(".exe"), "{t}");
        }
    }
}
