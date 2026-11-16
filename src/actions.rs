//! Quick actions. The Sound, Brightness, Bluetooth and Wi-Fi buttons open pop-ups with
//! their controls (src/controls); each pop-up's "more settings" link, and the other
//! buttons, open a documented Windows Settings page or a built-in application.

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
    /// Opens the Tasks + Clipboard shelf; handled in the application, not launched.
    Shelf,
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
        Self::Shelf,
        Self::Explorer,
        Self::Settings,
    ];

    pub fn from_index(index: i32) -> Option<Self> {
        usize::try_from(index).ok().and_then(|i| Self::ALL.get(i).copied())
    }

    /// URI or executable launched by the action, or `None` when the application handles
    /// it in place (the shelf).
    /// Settings URIs: <https://learn.microsoft.com/windows/apps/develop/launch/launch-settings-app>
    pub fn target(self) -> Option<&'static str> {
        Some(match self {
            Self::Sound => "ms-settings:sound",
            // Brightness lives on the display page on devices that support it.
            Self::Display => "ms-settings:display",
            Self::Bluetooth => "ms-settings:bluetooth",
            Self::Wifi => "ms-settings:network-wifi",
            Self::Settings => return None,
            Self::Snip => "ms-screenclip:",
            Self::Explorer => "explorer.exe",
            // The shelf is opened inside the island.
            Self::Shelf => return None,
        })
    }

    pub fn launch(self) -> windows::core::Result<()> {
        let Some(target) = self.target() else { return Ok(()) };
        let target = HSTRING::from(target);
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
    fn targets_are_settings_uris_or_system_tools_or_absent() {
        for action in QuickAction::ALL {
            match action.target() {
                Some(t) => {
                    assert!(t.ends_with(':') || t.starts_with("ms-settings:") || t.ends_with(".exe"), "{t}")
                }
                None => assert!(matches!(action, QuickAction::Shelf | QuickAction::Settings)),
            }
        }
    }
}
