//! Translation of Win32 input messages into Slint events.

use slint::SharedString;
use slint::platform::Key;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

/// Text for keys that have a dedicated Slint key code.
pub fn special_key(vk: VIRTUAL_KEY) -> Option<SharedString> {
    let key = match vk {
        VK_ESCAPE => Key::Escape,
        VK_RETURN => Key::Return,
        VK_TAB => Key::Tab,
        VK_BACK => Key::Backspace,
        VK_DELETE => Key::Delete,
        VK_LEFT => Key::LeftArrow,
        VK_RIGHT => Key::RightArrow,
        VK_UP => Key::UpArrow,
        VK_DOWN => Key::DownArrow,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_PRIOR => Key::PageUp,
        VK_NEXT => Key::PageDown,
        VK_INSERT => Key::Insert,
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => Key::Shift,
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL => Key::Control,
        VK_MENU | VK_LMENU | VK_RMENU => Key::Alt,
        VK_LWIN | VK_RWIN => Key::Meta,
        VK_F1 => Key::F1,
        VK_F2 => Key::F2,
        VK_F3 => Key::F3,
        VK_F4 => Key::F4,
        VK_F5 => Key::F5,
        VK_F6 => Key::F6,
        VK_F7 => Key::F7,
        VK_F8 => Key::F8,
        VK_F9 => Key::F9,
        VK_F10 => Key::F10,
        VK_F11 => Key::F11,
        VK_F12 => Key::F12,
        _ => return None,
    };
    Some(key.into())
}

/// Base character of a key regardless of modifiers (letters in lower case), used while
/// Ctrl or Alt is held so that shortcuts are reported as e.g. `Ctrl` + `k`.
pub fn base_character(vk: VIRTUAL_KEY) -> Option<SharedString> {
    if vk == VK_SPACE {
        return Some(" ".into());
    }
    let code = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_CHAR) } & 0x7fff_ffff;
    char::from_u32(code).filter(|c| !c.is_control()).map(|c| c.to_lowercase().collect::<String>().into())
}

pub fn modifier_held(vk: VIRTUAL_KEY) -> bool {
    (unsafe { GetKeyState(vk.0 as i32) } as u16 & 0x8000) != 0
}

/// Converts a `WM_MOUSEWHEEL` delta into logical pixels, matching common desktop
/// toolkits (three lines of 20 px per notch).
pub fn wheel_pixels(delta: i16) -> f32 {
    delta as f32 / 120.0 * 60.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_navigation_keys() {
        assert_eq!(special_key(VK_ESCAPE), Some(SharedString::from(Key::Escape)));
        assert_eq!(special_key(VK_LCONTROL), Some(SharedString::from(Key::Control)));
        assert_eq!(special_key(VIRTUAL_KEY(b'A' as u16)), None);
    }

    #[test]
    fn base_characters_are_lowercase() {
        assert_eq!(base_character(VIRTUAL_KEY(b'K' as u16)).as_deref(), Some("k"));
        assert_eq!(base_character(VK_SPACE).as_deref(), Some(" "));
        assert_eq!(base_character(VIRTUAL_KEY(b'7' as u16)).as_deref(), Some("7"));
    }

    #[test]
    fn wheel_conversion() {
        assert_eq!(wheel_pixels(120), 60.0);
        assert_eq!(wheel_pixels(-240), -120.0);
    }
}
