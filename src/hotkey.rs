//! The global keyboard shortcut that toggles the island.
//!
//! Shortcuts are stored as text such as `Ctrl+Alt+Space` and registered with
//! `RegisterHotKey` on the platform's control window, so Windows delivers them as
//! messages and no keyboard hook is needed.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey, VK_F1,
    VK_SPACE,
};

/// Identifier of Lilt's only hot key.
pub const TOGGLE_ID: i32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Virtual-key code.
    pub key: u16,
}

impl Shortcut {
    /// Parses text such as `Ctrl+Alt+Space` or `Win+Shift+F9`.
    pub fn parse(text: &str) -> Option<Self> {
        let mut shortcut = Shortcut { ctrl: false, alt: false, shift: false, win: false, key: 0 };
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => shortcut.ctrl = true,
                "alt" => shortcut.alt = true,
                "shift" => shortcut.shift = true,
                "win" | "windows" => shortcut.win = true,
                _ if shortcut.key == 0 => shortcut.key = key_code(part)?,
                _ => return None,
            }
        }
        shortcut.is_valid().then_some(shortcut)
    }

    /// A shortcut needs a key and Ctrl, Alt or Win, so ordinary typing never triggers it.
    pub fn is_valid(&self) -> bool {
        self.key != 0 && (self.ctrl || self.alt || self.win)
    }

    fn modifiers(&self) -> HOT_KEY_MODIFIERS {
        let mut m = MOD_NOREPEAT;
        for (on, flag) in [(self.ctrl, MOD_CONTROL), (self.alt, MOD_ALT), (self.shift, MOD_SHIFT), (self.win, MOD_WIN)]
        {
            if on {
                m |= flag;
            }
        }
        m
    }

    /// Registers the shortcut with Windows. Fails if another application owns it.
    pub fn register(&self, window: HWND) -> windows::core::Result<()> {
        unsafe { RegisterHotKey(Some(window), TOGGLE_ID, self.modifiers(), self.key as u32) }
    }
}

impl std::fmt::Display for Shortcut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (on, name) in [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift"), (self.win, "Win")] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&key_name(self.key))
    }
}

pub fn unregister(window: HWND) {
    unsafe {
        let _ = UnregisterHotKey(Some(window), TOGGLE_ID);
    }
}

fn key_code(name: &str) -> Option<u16> {
    let upper = name.to_ascii_uppercase();
    if upper == "SPACE" {
        return Some(VK_SPACE.0);
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
        return (1..=24).contains(&n).then(|| VK_F1.0 + n - 1);
    }
    let mut chars = upper.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => Some(c as u16),
        _ => None,
    }
}

fn key_name(code: u16) -> String {
    match code {
        c if c == VK_SPACE.0 => "Space".into(),
        c if (VK_F1.0..VK_F1.0 + 24).contains(&c) => format!("F{}", c - VK_F1.0 + 1),
        c => char::from_u32(c as u32).map(String::from).unwrap_or_default(),
    }
}

/// Builds a shortcut from a key event reported by the UI: `text` is the key's text
/// (a character, or Slint's private-use code for keys such as F1).
pub fn from_key_event(text: &str, ctrl: bool, alt: bool, shift: bool, win: bool) -> Option<Shortcut> {
    let key = if text == " " {
        VK_SPACE.0
    } else if let Some(n) = function_key(text) {
        VK_F1.0 + n - 1
    } else {
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_alphanumeric() => c.to_ascii_uppercase() as u16,
            _ => return None,
        }
    };
    let shortcut = Shortcut { ctrl, alt, shift, win, key };
    shortcut.is_valid().then_some(shortcut)
}

fn function_key(text: &str) -> Option<u16> {
    use slint::platform::Key;
    let keys =
        [Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6, Key::F7, Key::F8, Key::F9, Key::F10, Key::F11, Key::F12];
    keys.iter().position(|k| slint::SharedString::from(*k).as_str() == text).map(|i| i as u16 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats() {
        let s = Shortcut::parse("Ctrl+Alt+Space").unwrap();
        assert!(s.ctrl && s.alt && !s.shift && !s.win);
        assert_eq!(s.to_string(), "Ctrl+Alt+Space");
        assert_eq!(Shortcut::parse("win + shift + f9").unwrap().to_string(), "Shift+Win+F9");
        assert_eq!(Shortcut::parse("Alt+k").unwrap().to_string(), "Alt+K");
    }

    #[test]
    fn rejects_unsafe_or_malformed_shortcuts() {
        assert_eq!(Shortcut::parse("Space"), None);
        assert_eq!(Shortcut::parse("Shift+A"), None);
        assert_eq!(Shortcut::parse("Ctrl+Alt"), None);
        assert_eq!(Shortcut::parse("Ctrl+A+B"), None);
        assert_eq!(Shortcut::parse("Ctrl+F25"), None);
        assert_eq!(Shortcut::parse("Ctrl+Tab"), None);
    }

    #[test]
    fn builds_from_key_events() {
        assert_eq!(from_key_event("k", true, true, false, false).unwrap().to_string(), "Ctrl+Alt+K");
        assert_eq!(from_key_event(" ", false, true, false, true).unwrap().to_string(), "Alt+Win+Space");
        let f5 = slint::SharedString::from(slint::platform::Key::F5);
        assert_eq!(from_key_event(&f5, true, false, false, false).unwrap().to_string(), "Ctrl+F5");
        assert_eq!(from_key_event("k", false, false, true, false), None);
    }
}
