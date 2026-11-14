//! Which material the selected appearance can use on this PC.
//!
//! Midnight is always drawn with solid surfaces. Liquid Glass uses the live blur of the
//! desktop behind it when Windows allows it, and otherwise frosted surfaces: denser
//! translucent colours that need no blur. The decision is a pure function of the settings
//! and a snapshot of the Windows environment so that it can be tested.

use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::core::w;

use crate::settings::{AppearanceMode, Settings};

/// Windows 10 1903 added the rounded composition clip that shapes the island's blur.
pub const LIVE_BLUR_MIN_BUILD: u32 = 18362;
/// Windows 11 22H2 added documented system backdrops (acrylic) for ordinary windows,
/// used for the settings window.
pub const WINDOW_BACKDROP_MIN_BUILD: u32 = 22621;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    /// Opaque surfaces (Midnight).
    Solid,
    /// Liquid Glass without a live blur.
    Frosted,
    /// Liquid Glass over the blurred desktop.
    LiveBlur,
}

/// Why Liquid Glass is not using the live blur.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    UserReduced,
    LowPower,
    TransparencyOff,
    BatterySaver,
    Unsupported,
    /// The blur was allowed but could not be created.
    Failed,
}

impl Fallback {
    pub fn explanation(self) -> &'static str {
        match self {
            Self::UserReduced => {
                "Reduce transparency is on, so Liquid Glass uses frosted surfaces without a live blur."
            }
            Self::LowPower => "Low-power mode is on, so Liquid Glass uses frosted surfaces without a live blur.",
            Self::TransparencyOff => {
                "Transparency effects are off in Windows settings, so Liquid Glass uses frosted surfaces without a live blur."
            }
            Self::BatterySaver => "Battery saver is on, so Liquid Glass uses frosted surfaces without a live blur.",
            Self::Unsupported => {
                "This version of Windows cannot blur behind the island, so Liquid Glass uses frosted surfaces."
            }
            Self::Failed => "The live blur could not be started, so Liquid Glass uses frosted surfaces.",
        }
    }
}

/// The parts of the Windows environment that affect the material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Environment {
    /// "Transparency effects" in Windows settings.
    pub transparency: bool,
    pub battery_saver: bool,
    pub build: u32,
    /// Development override from `LILT_GLASS` (`frosted` or `blur`), used by benchmarks
    /// and tests to exercise one path regardless of the PC's settings.
    pub forced: Option<Material>,
}

impl Environment {
    pub fn current() -> Self {
        let forced = match std::env::var("LILT_GLASS").as_deref() {
            Ok("frosted") => Some(Material::Frosted),
            Ok("blur") => Some(Material::LiveBlur),
            _ => None,
        };
        Self { transparency: transparency_enabled(), battery_saver: battery_saver_on(), build: windows_build(), forced }
    }
}

/// Chooses the material for the configured appearance.
pub fn material(settings: &Settings, env: &Environment) -> (Material, Option<Fallback>) {
    if settings.appearance.theme == AppearanceMode::Midnight {
        return (Material::Solid, None);
    }
    if let Some(forced) = env.forced {
        return (forced, None);
    }
    let fallback = if settings.appearance.reduce_transparency {
        Some(Fallback::UserReduced)
    } else if settings.performance.low_power {
        Some(Fallback::LowPower)
    } else if !env.transparency {
        Some(Fallback::TransparencyOff)
    } else if env.battery_saver {
        Some(Fallback::BatterySaver)
    } else if env.build < LIVE_BLUR_MIN_BUILD {
        Some(Fallback::Unsupported)
    } else {
        None
    };
    match fallback {
        Some(reason) => (Material::Frosted, Some(reason)),
        None => (Material::LiveBlur, None),
    }
}

/// Whether ordinary windows (settings) can show the system acrylic backdrop.
pub fn window_backdrop_supported(env: &Environment) -> bool {
    env.build >= WINDOW_BACKDROP_MIN_BUILD
}

fn transparency_enabled() -> bool {
    let mut value = 1u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("EnableTransparency"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    // Missing means the Windows default, which is on.
    result.is_err() || value != 0
}

fn battery_saver_on() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status) }.is_ok() && status.SystemStatusFlag == 1
}

fn windows_build() -> u32 {
    let mut buffer = [0u16; 16];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            w!("CurrentBuildNumber"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if result.is_err() {
        return 0;
    }
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len]).trim().parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOWS_11: Environment =
        Environment { transparency: true, battery_saver: false, build: 26200, forced: None };

    fn glass() -> Settings {
        let mut s = Settings::default();
        s.appearance.theme = AppearanceMode::LiquidGlass;
        s
    }

    #[test]
    fn midnight_is_always_solid() {
        assert_eq!(material(&Settings::default(), &WINDOWS_11), (Material::Solid, None));
        let forced = Environment { forced: Some(Material::LiveBlur), ..WINDOWS_11 };
        assert_eq!(material(&Settings::default(), &forced).0, Material::Solid);
    }

    #[test]
    fn glass_uses_the_live_blur_when_windows_allows_it() {
        assert_eq!(material(&glass(), &WINDOWS_11), (Material::LiveBlur, None));
        let windows_10 = Environment { build: 19045, ..WINDOWS_11 };
        assert_eq!(material(&glass(), &windows_10).0, Material::LiveBlur);
        assert!(!window_backdrop_supported(&windows_10));
        assert!(window_backdrop_supported(&WINDOWS_11));
    }

    #[test]
    fn glass_falls_back_to_frosted_surfaces() {
        let cases = [
            (Environment { transparency: false, ..WINDOWS_11 }, Fallback::TransparencyOff),
            (Environment { battery_saver: true, ..WINDOWS_11 }, Fallback::BatterySaver),
            (Environment { build: 17763, ..WINDOWS_11 }, Fallback::Unsupported),
        ];
        for (env, reason) in cases {
            assert_eq!(material(&glass(), &env), (Material::Frosted, Some(reason)));
        }
        let mut reduced = glass();
        reduced.appearance.reduce_transparency = true;
        assert_eq!(material(&reduced, &WINDOWS_11), (Material::Frosted, Some(Fallback::UserReduced)));
        let mut low_power = glass();
        low_power.performance.low_power = true;
        assert_eq!(material(&low_power, &WINDOWS_11), (Material::Frosted, Some(Fallback::LowPower)));
    }

    #[test]
    fn environment_can_be_read() {
        let env = Environment::current();
        assert!(env.build >= 10240, "build {}", env.build);
    }
}
