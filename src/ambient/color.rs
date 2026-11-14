//! Colour-space helpers: sRGB <-> OKLab/OKLCH and WCAG contrast.
//!
//! OKLab is used because equal steps in lightness and chroma look roughly equal,
//! which makes it possible to clamp album colours into a restrained range without
//! shifting their perceived hue.

/// An 8-bit sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// A colour in OKLCH: lightness 0..1, chroma 0..~0.37, hue in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oklch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
}

fn srgb_to_linear(v: u8) -> f32 {
    let v = v as f32 / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Converts sRGB to OKLab `[L, a, b]`.
pub fn to_oklab(c: Rgb) -> [f32; 3] {
    let (r, g, b) = (srgb_to_linear(c.r), srgb_to_linear(c.g), srgb_to_linear(c.b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// Converts OKLab to linear sRGB; components may fall outside 0..1 when out of gamut.
fn oklab_to_linear([l, a, b]: [f32; 3]) -> [f32; 3] {
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ]
}

pub fn to_oklch(c: Rgb) -> Oklch {
    let [l, a, b] = to_oklab(c);
    let h = b.atan2(a).to_degrees();
    Oklch { l, c: (a * a + b * b).sqrt(), h: if h < 0.0 { h + 360.0 } else { h } }
}

/// Converts OKLCH to sRGB, reducing chroma until the colour fits the sRGB gamut so
/// the hue is preserved instead of being clipped per channel.
pub fn from_oklch(color: Oklch) -> Rgb {
    let (sin, cos) = color.h.to_radians().sin_cos();
    let mut chroma = color.c.max(0.0);
    loop {
        let rgb = oklab_to_linear([color.l, chroma * cos, chroma * sin]);
        let in_gamut = rgb.iter().all(|v| (-0.0005..=1.0005).contains(v));
        if in_gamut || chroma < 0.001 {
            let to_u8 = |v: f32| (linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0).round() as u8;
            return Rgb::new(to_u8(rgb[0]), to_u8(rgb[1]), to_u8(rgb[2]));
        }
        chroma *= 0.92;
    }
}

/// WCAG 2.x relative luminance.
pub fn relative_luminance(c: Rgb) -> f32 {
    0.2126 * srgb_to_linear(c.r) + 0.7152 * srgb_to_linear(c.g) + 0.0722 * srgb_to_linear(c.b)
}

/// WCAG 2.x contrast ratio between two colours (1.0 ..= 21.0).
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Shortest angular distance between two hues, in degrees.
pub fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    if d > 180.0 { 360.0 - d } else { d }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklch_round_trip_is_stable() {
        for c in [Rgb::new(30, 90, 200), Rgb::new(240, 140, 30), Rgb::new(128, 128, 128), Rgb::new(10, 200, 90)] {
            let back = from_oklch(to_oklch(c));
            for (x, y) in [(c.r, back.r), (c.g, back.g), (c.b, back.b)] {
                assert!((x as i16 - y as i16).abs() <= 1, "{c:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn grey_has_no_chroma() {
        assert!(to_oklch(Rgb::new(120, 120, 120)).c < 0.001);
    }

    #[test]
    fn out_of_gamut_keeps_hue() {
        let requested = Oklch { l: 0.7, c: 0.4, h: 145.0 };
        let got = to_oklch(from_oklch(requested));
        assert!(hue_distance(got.h, requested.h) < 4.0, "{got:?}");
    }

    #[test]
    fn contrast_matches_wcag_reference() {
        let ratio = contrast_ratio(Rgb::new(255, 255, 255), Rgb::new(0, 0, 0));
        assert!((ratio - 21.0).abs() < 0.01);
    }

    #[test]
    fn hue_distance_wraps() {
        assert_eq!(hue_distance(350.0, 10.0), 20.0);
        assert_eq!(hue_distance(10.0, 350.0), 20.0);
    }
}
