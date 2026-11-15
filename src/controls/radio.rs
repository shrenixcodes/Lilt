//! Turning the Wi-Fi and Bluetooth radios on and off with `Windows.Devices.Radios`, the
//! API Windows' own quick settings use. Windows may refuse (for example when the user has
//! denied apps radio control in Privacy settings); the refusal is reported.

use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};

fn find(kind: RadioKind) -> Option<Radio> {
    let radios = Radio::GetRadiosAsync().ok()?.join().ok()?;
    radios.into_iter().find(|r| r.Kind().is_ok_and(|k| k == kind))
}

pub fn wifi() -> RadioKind {
    RadioKind::WiFi
}

pub fn bluetooth() -> RadioKind {
    RadioKind::Bluetooth
}

/// Whether the radio is on; `None` without such a radio.
pub fn is_on(kind: RadioKind) -> Option<bool> {
    find(kind).and_then(|r| r.State().ok()).map(|s| s == RadioState::On)
}

pub fn set(kind: RadioKind, on: bool) -> Result<(), String> {
    let refused = || "Windows did not let Lilt change this. Check Privacy & security → Radios.".to_owned();
    let access = Radio::RequestAccessAsync().and_then(|a| a.join()).map_err(|_| refused())?;
    if access != RadioAccessStatus::Allowed {
        return Err(refused());
    }
    let radio = find(kind).ok_or("This PC has no such radio.")?;
    let status = radio
        .SetStateAsync(if on { RadioState::On } else { RadioState::Off })
        .and_then(|s| s.join())
        .map_err(|_| refused())?;
    if status == RadioAccessStatus::Allowed { Ok(()) } else { Err(refused()) }
}
