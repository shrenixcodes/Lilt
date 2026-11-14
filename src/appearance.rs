//! Which material the selected appearance can use on this PC.
//!
//! Midnight is always drawn with solid surfaces. The glass appearances, Smoked Glass and
//! Liquid Glass, use the live blur of the desktop behind them when Windows allows it, and
//! otherwise frosted surfaces: denser translucent colours that need no blur. The decision
//! is a pure function of the settings and a snapshot of the Windows environment so that it
//! can be tested.
//!
//! Liquid Glass's tint over the blur is as clear as the desktop behind it allows:
//! [`legible_alpha`] finds the lightest tint that keeps its text legible over a sampled
//! backdrop colour.

use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::core::w;

use crate::ambient::{Rgb, contrast_ratio};
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
    /// Glass without a live blur.
    Frosted,
    /// Glass over the blurred desktop.
    LiveBlur,
}

/// Why the glass is not using the live blur.
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
            Self::UserReduced => "Reduce transparency is on, so the glass uses frosted surfaces without a live blur.",
            Self::LowPower => "Low-power mode is on, so the glass uses frosted surfaces without a live blur.",
            Self::TransparencyOff => {
                "Transparency effects are off in Windows settings, so the glass uses frosted surfaces without a live blur."
            }
            Self::BatterySaver => "Battery saver is on, so the glass uses frosted surfaces without a live blur.",
            Self::Unsupported => {
                "This version of Windows cannot blur behind the island, so the glass uses frosted surfaces."
            }
            Self::Failed => "The live blur could not be started, so the glass uses frosted surfaces.",
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

/// Liquid Glass's tint opacity over the clearest backdrops. Lower would let bright
/// spots in a dark backdrop compete with the text.
pub const CLEAR_ALPHA: f32 = 0.36;
/// The densest tint, enough for legible text over pure white.
pub const DENSE_ALPHA: f32 = 0.86;

/// A colour with straight (not premultiplied) alpha, 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub rgb: Rgb,
    pub alpha: f32,
}

/// What Liquid Glass's text must stay legible against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassText {
    /// The tint, without its alpha.
    pub tint: Rgb,
    /// Tiles drawn on the glass; text sits on them too.
    pub tile: Layer,
    pub text: Rgb,
    pub secondary: Rgb,
    pub tertiary: Rgb,
}

fn over(top: Layer, below: Rgb) -> Rgb {
    let mix = |t: u8, b: u8| (t as f32 * top.alpha + b as f32 * (1.0 - top.alpha)).round() as u8;
    Rgb::new(mix(top.rgb.r, below.r), mix(top.rgb.g, below.g), mix(top.rgb.b, below.b))
}

/// The lightest tint opacity between [`CLEAR_ALPHA`] and [`DENSE_ALPHA`] at which the
/// glass's text meets WCAG AA over `backdrop`, the brightest colour sampled behind the
/// island: 4.5:1 for primary and secondary text and 3:1 for tertiary text, both on the
/// glass and on its tiles.
pub fn legible_alpha(glass: &GlassText, backdrop: Rgb) -> f32 {
    const STEPS: u32 = 88;
    for step in 0..=STEPS {
        let alpha = CLEAR_ALPHA + (DENSE_ALPHA - CLEAR_ALPHA) * step as f32 / STEPS as f32;
        let base = over(Layer { rgb: glass.tint, alpha }, backdrop);
        let tile = over(glass.tile, base);
        let legible = [base, tile].iter().all(|&bg| {
            contrast_ratio(glass.text, bg) >= 4.5
                && contrast_ratio(glass.secondary, bg) >= 4.5
                && contrast_ratio(glass.tertiary, bg) >= 3.0
        });
        if legible {
            return alpha;
        }
    }
    DENSE_ALPHA
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

    const LIQUID: GlassText = GlassText {
        tint: Rgb::new(0x10, 0x13, 0x1c),
        tile: Layer { rgb: Rgb::new(255, 255, 255), alpha: 0x12 as f32 / 255.0 },
        text: Rgb::new(255, 255, 255),
        secondary: Rgb::new(0xd2, 0xd4, 0xdc),
        tertiary: Rgb::new(0xa9, 0xac, 0xb8),
    };

    #[test]
    fn the_tint_is_clear_over_dark_desktops_and_dense_over_bright_ones() {
        let black = legible_alpha(&LIQUID, Rgb::new(0, 0, 0));
        let navy = legible_alpha(&LIQUID, Rgb::new(30, 40, 80));
        let grey = legible_alpha(&LIQUID, Rgb::new(128, 128, 128));
        let white = legible_alpha(&LIQUID, Rgb::new(255, 255, 255));
        assert_eq!(black, CLEAR_ALPHA);
        assert_eq!(navy, CLEAR_ALPHA);
        assert!(CLEAR_ALPHA < grey && grey < white, "{grey} {white}");
        assert!(white < DENSE_ALPHA, "white is still legible below the densest tint: {white}");
    }

    #[test]
    fn text_stays_legible_over_any_backdrop() {
        for r in (0..=255).step_by(15) {
            for g in (0..=255).step_by(15) {
                for b in (0..=255).step_by(51) {
                    let backdrop = Rgb::new(r as u8, g as u8, b as u8);
                    let alpha = legible_alpha(&LIQUID, backdrop);
                    let base = over(Layer { rgb: LIQUID.tint, alpha }, backdrop);
                    let tile = over(LIQUID.tile, base);
                    for bg in [base, tile] {
                        assert!(contrast_ratio(LIQUID.text, bg) >= 4.5, "{backdrop:?}");
                        assert!(contrast_ratio(LIQUID.secondary, bg) >= 4.5, "{backdrop:?}");
                        assert!(contrast_ratio(LIQUID.tertiary, bg) >= 3.0, "{backdrop:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn environment_can_be_read() {
        let env = Environment::current();
        assert!(env.build >= 10240, "build {}", env.build);
    }
}
