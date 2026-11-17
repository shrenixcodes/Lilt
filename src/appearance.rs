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

/// Liquid Glass's tint opacity over dark backdrops at the default transparency.
#[cfg(test)]
pub const CLEAR_ALPHA: f32 = 0.30;
/// Dense enough for legible text over pure white with either glass appearance.
pub const DENSE_ALPHA: f32 = 0.86;

/// How strongly Liquid Glass bends the desktop along its edges, as a share of the bending
/// band's width (the band is about as wide as the corner radius).
pub const LIQUID_REFRACTION: f32 = 0.6;

/// How far the glass blurs the desktop, in logical pixels, for a transparency setting
/// (0..=100 percent): Smoked Glass uses a heavy frosted blur, while Liquid Glass stays
/// crisper and clearer to preserve fluid shapes and colors beneath it.
pub fn blur_radius(appearance: AppearanceMode, transparency: u32) -> f32 {
    let t = transparency.min(100) as f32 / 100.0;
    match appearance {
        AppearanceMode::Midnight => 0.0,
        AppearanceMode::SmokedGlass => 40.0 * (1.0 - t),
        AppearanceMode::LiquidGlass => 22.0 * (1.0 - t),
    }
}

/// The tint opacity over the live blur that a transparency setting (0..=100 percent) asks
/// for, before Lilt darkens it over bright content. 50 percent is each appearance as
/// designed. Liquid Glass stays distinctly clearer and more luminous than Smoked Glass.
pub fn preferred_alpha(appearance: AppearanceMode, transparency: u32) -> f32 {
    let t = transparency.min(100) as f32 / 100.0;
    match appearance {
        AppearanceMode::Midnight => 1.0,
        // 0.97 .. 0.84 .. 0.71
        AppearanceMode::SmokedGlass => 0.97 - 0.26 * t,
        // 0.64 .. 0.30 .. 0.05
        AppearanceMode::LiquidGlass => (0.64 - 0.68 * t).max(0.05),
    }
}

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

/// The contrast the glass keeps for primary, secondary and tertiary text at a
/// transparency setting. Up to 50 percent it is WCAG AA (4.5:1, 4.5:1 and 3:1). Above
/// that, the choice of clear glass wins progressively: at 100 percent the glass dims only
/// enough for 2:1, 1.6:1 and 1.3:1, so it stays clear over most content and darkens a
/// little over very bright windows.
pub fn contrast_targets(appearance: AppearanceMode, transparency: u32) -> [f32; 3] {
    let _ = appearance;
    let clear = ((transparency.min(100) as f32 - 50.0) / 50.0).clamp(0.0, 1.0);
    let ease = |aa: f32, clearest: f32| aa + (clearest - aa) * clear;
    [ease(4.5, 2.0), ease(4.5, 1.6), ease(3.0, 1.3)]
}

/// The lightest tint opacity, from `floor` up to [`DENSE_ALPHA`], at which the glass's
/// text keeps the `targets` contrast (see [`contrast_targets`]) over `backdrop`, the
/// brightest text-sized area behind the island, both on the glass and on its tiles. A
/// floor above [`DENSE_ALPHA`] is returned as it is.
pub fn legible_alpha(glass: &GlassText, backdrop: Rgb, floor: f32, targets: [f32; 3]) -> f32 {
    const STEPS: u32 = 100;
    if floor >= DENSE_ALPHA {
        return floor;
    }
    for step in 0..=STEPS {
        let alpha = floor + (DENSE_ALPHA - floor) * step as f32 / STEPS as f32;
        let base = over(Layer { rgb: glass.tint, alpha }, backdrop);
        let tile = over(glass.tile, base);
        let legible = [base, tile].iter().all(|&bg| {
            contrast_ratio(glass.text, bg) >= targets[0]
                && contrast_ratio(glass.secondary, bg) >= targets[1]
                && contrast_ratio(glass.tertiary, bg) >= targets[2]
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
        tile: Layer { rgb: Rgb::new(255, 255, 255), alpha: 0x0d as f32 / 255.0 },
        text: Rgb::new(255, 255, 255),
        secondary: Rgb::new(0xf6, 0xf7, 0xfb),
        tertiary: Rgb::new(0xd6, 0xd9, 0xe4),
    };

    #[test]
    fn the_tint_is_clear_over_dark_desktops_and_dense_over_bright_ones() {
        let targets = contrast_targets(AppearanceMode::LiquidGlass, 50);
        let black = legible_alpha(&LIQUID, Rgb::new(0, 0, 0), CLEAR_ALPHA, targets);
        let navy = legible_alpha(&LIQUID, Rgb::new(30, 40, 80), CLEAR_ALPHA, targets);
        let grey = legible_alpha(&LIQUID, Rgb::new(128, 128, 128), CLEAR_ALPHA, targets);
        let white = legible_alpha(&LIQUID, Rgb::new(255, 255, 255), CLEAR_ALPHA, targets);
        assert_eq!(black, CLEAR_ALPHA);
        assert_eq!(navy, CLEAR_ALPHA);
        assert!(CLEAR_ALPHA <= grey && grey < white, "{grey} {white}");
        assert!(white < DENSE_ALPHA, "white is still legible below the densest tint: {white}");
    }

    const SMOKED: GlassText = GlassText {
        tint: Rgb::new(0x13, 0x13, 0x1a),
        tile: Layer { rgb: Rgb::new(255, 255, 255), alpha: 0x0f as f32 / 255.0 },
        text: Rgb::new(0xf6, 0xf6, 0xf8),
        secondary: Rgb::new(0xb7, 0xb7, 0xc0),
        tertiary: Rgb::new(0x95, 0x95, 0xa0),
    };

    #[test]
    fn text_stays_legible_over_any_backdrop_at_any_transparency() {
        for (glass, mode) in [(LIQUID, AppearanceMode::LiquidGlass), (SMOKED, AppearanceMode::SmokedGlass)] {
            for transparency in [0, 50, 100] {
                let floor = preferred_alpha(mode, transparency);
                let targets = contrast_targets(mode, transparency);
                for r in (0..=255).step_by(51) {
                    for g in (0..=255).step_by(15) {
                        for b in (0..=255).step_by(51) {
                            let backdrop = Rgb::new(r as u8, g as u8, b as u8);
                            let alpha = legible_alpha(&glass, backdrop, floor, targets);
                            assert!(alpha >= floor);
                            let base = over(Layer { rgb: glass.tint, alpha }, backdrop);
                            let tile = over(glass.tile, base);
                            for bg in [base, tile] {
                                let why = format!("{mode:?} {transparency} {backdrop:?}");
                                assert!(contrast_ratio(glass.text, bg) >= targets[0], "{why}");
                                assert!(contrast_ratio(glass.secondary, bg) >= targets[1], "{why}");
                                assert!(contrast_ratio(glass.tertiary, bg) >= targets[2], "{why}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn transparency_goes_from_blur_to_clear() {
        assert_eq!(blur_radius(AppearanceMode::SmokedGlass, 0), 40.0);
        assert_eq!(blur_radius(AppearanceMode::SmokedGlass, 50), 20.0, "the designed frosted blur");
        assert_eq!(blur_radius(AppearanceMode::SmokedGlass, 100), 0.0, "clear glass");
        assert_eq!(blur_radius(AppearanceMode::SmokedGlass, 300), 0.0);

        assert_eq!(blur_radius(AppearanceMode::LiquidGlass, 0), 22.0);
        assert_eq!(blur_radius(AppearanceMode::LiquidGlass, 50), 11.0, "crisp liquid blur");
        assert_eq!(blur_radius(AppearanceMode::LiquidGlass, 100), 0.0, "clear glass");
    }

    #[test]
    fn transparency_moves_the_tint_and_50_percent_is_the_design() {
        assert!((preferred_alpha(AppearanceMode::LiquidGlass, 50) - CLEAR_ALPHA).abs() < 1e-6);
        assert!((preferred_alpha(AppearanceMode::SmokedGlass, 50) - 0.84).abs() < 1e-6);
        for mode in [AppearanceMode::SmokedGlass, AppearanceMode::LiquidGlass] {
            assert!(preferred_alpha(mode, 0) > preferred_alpha(mode, 50));
            assert!(preferred_alpha(mode, 50) > preferred_alpha(mode, 100));
            assert_eq!(preferred_alpha(mode, 100), preferred_alpha(mode, 250), "clamped");
        }
        // Smoked Glass at its default needs no darkening even over white: it looks as before.
        assert_eq!(
            legible_alpha(&SMOKED, Rgb::new(255, 255, 255), 0.84, contrast_targets(AppearanceMode::SmokedGlass, 50)),
            0.84
        );
        // Fully clear Liquid Glass stays clear over a dark desktop.
        let clear = preferred_alpha(AppearanceMode::LiquidGlass, 100);
        assert_eq!(
            legible_alpha(&LIQUID, Rgb::new(10, 12, 20), clear, contrast_targets(AppearanceMode::LiquidGlass, 100)),
            clear
        );
    }

    #[test]
    fn clear_glass_dims_only_lightly_and_only_over_bright_content() {
        assert_eq!(contrast_targets(AppearanceMode::SmokedGlass, 0), [4.5, 4.5, 3.0]);
        assert_eq!(contrast_targets(AppearanceMode::SmokedGlass, 50), [4.5, 4.5, 3.0], "AA up to the default");
        let clearest = contrast_targets(AppearanceMode::SmokedGlass, 100);
        for (got, want) in clearest.iter().zip([2.0, 1.6, 1.3]) {
            assert!((got - want).abs() < 1e-5, "{clearest:?}");
        }
        let clear = preferred_alpha(AppearanceMode::LiquidGlass, 100);
        let targets = contrast_targets(AppearanceMode::LiquidGlass, 100);
        // A mid-bright, colourful page stays clear.
        assert_eq!(legible_alpha(&LIQUID, Rgb::new(90, 90, 255), clear, targets), clear);
        // A white page gets a light dimming, far from the frosted default.
        let white = legible_alpha(&LIQUID, Rgb::new(255, 255, 255), clear, targets);
        assert!(white > clear && white < 0.5, "{white}");
    }

    #[test]
    fn environment_can_be_read() {
        let env = Environment::current();
        assert!(env.build >= 10240, "build {}", env.build);
    }
}
