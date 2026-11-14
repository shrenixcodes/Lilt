//! Ambient accent palette derived from album artwork.
//!
//! The extractor runs on a small thumbnail (typically 32x32), groups its pixels with
//! median cut, picks the most characteristic colour and then forces it into a narrow,
//! restrained lightness/chroma band. The result is only ever used as a subtle tint, so
//! the goal is a pleasant hue rather than a faithful reproduction of the artwork.

mod color;

use color::{Oklch, from_oklch, hue_distance, to_oklch};
pub use color::{Rgb, contrast_ratio, relative_luminance};

/// Island surface colour that accents must remain legible against.
pub const SURFACE: Rgb = Rgb::new(11, 11, 12);

/// Minimum WCAG contrast between the accent and the island surface.
const MIN_ACCENT_CONTRAST: f32 = 4.5;

/// Number of median-cut boxes. Eight is enough to separate a dominant background from
/// a smaller but more characteristic foreground colour.
const MAX_CLUSTERS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmbientPalette {
    /// Progress indicators and small highlights.
    pub accent: Rgb,
    /// Low-opacity colour wash behind the expanded player.
    pub wash: Rgb,
    /// Restrained radial glow behind the artwork.
    pub glow: Rgb,
    /// True when the artwork carries no meaningful colour.
    pub monochrome: bool,
}

impl AmbientPalette {
    /// Palette used when there is no artwork or ambient colours are disabled.
    pub const NEUTRAL: Self = Self {
        accent: Rgb::new(232, 232, 236),
        wash: Rgb::new(58, 58, 64),
        glow: Rgb::new(104, 104, 112),
        monochrome: true,
    };
}

struct Candidate {
    color: Oklch,
    share: f32,
}

/// Extracts a palette from tightly packed RGBA8 pixels.
pub fn extract(rgba: &[u8]) -> AmbientPalette {
    let pixels: Vec<[u8; 3]> =
        rgba.as_chunks::<4>().0.iter().filter(|p| p[3] >= 128).map(|p| [p[0], p[1], p[2]]).collect();
    if pixels.len() < 16 {
        return AmbientPalette::NEUTRAL;
    }

    let total = pixels.len() as f32;
    let candidates: Vec<Candidate> = median_cut(pixels, MAX_CLUSTERS)
        .into_iter()
        .map(|(rgb, count)| Candidate { color: to_oklch(rgb), share: count as f32 / total })
        .collect();

    let colourful_share: f32 = candidates.iter().filter(|c| c.color.c >= 0.04).map(|c| c.share).sum();
    let Some(best) = candidates.iter().max_by(|a, b| score(a).total_cmp(&score(b))) else {
        return AmbientPalette::NEUTRAL;
    };
    if best.color.c < 0.035 || colourful_share < 0.05 {
        return monochrome(&candidates);
    }

    let hue = best.color.h;
    let chroma = best.color.c;
    let secondary_hue = candidates
        .iter()
        .filter(|c| c.color.c >= 0.05 && c.share >= 0.08 && hue_distance(c.color.h, hue) >= 25.0)
        .max_by(|a, b| score(a).total_cmp(&score(b)))
        .map_or(hue, |c| c.color.h);

    AmbientPalette {
        accent: legible(Oklch { l: 0.76, c: chroma.clamp(0.05, 0.13), h: hue }),
        wash: from_oklch(Oklch { l: 0.34, c: (chroma * 0.6).clamp(0.02, 0.07), h: hue }),
        glow: from_oklch(Oklch { l: 0.52, c: (chroma * 0.8).clamp(0.03, 0.10), h: secondary_hue }),
        monochrome: false,
    }
}

/// Favour colours that are both vivid and reasonably common. Near-black and near-white
/// clusters are usually backgrounds or text and are de-emphasised.
fn score(c: &Candidate) -> f32 {
    let vivid = smoothstep(0.02, 0.14, c.color.c);
    let tone = if c.color.l < 0.18 || c.color.l > 0.94 { 0.35 } else { 1.0 };
    c.share.sqrt() * (0.05 + vivid) * tone
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn monochrome(candidates: &[Candidate]) -> AmbientPalette {
    let lightness: f32 = candidates.iter().map(|c| c.color.l * c.share).sum();
    let grey = |l: f32| from_oklch(Oklch { l, c: 0.0, h: 0.0 });
    AmbientPalette {
        accent: grey(0.9),
        wash: grey(0.26 + 0.1 * lightness),
        glow: grey(0.42 + 0.1 * lightness),
        monochrome: true,
    }
}

/// Raises lightness until the accent is legible on the island surface.
fn legible(mut color: Oklch) -> Rgb {
    loop {
        let rgb = from_oklch(color);
        if contrast_ratio(rgb, SURFACE) >= MIN_ACCENT_CONTRAST || color.l >= 0.95 {
            return rgb;
        }
        color.l += 0.02;
    }
}

/// Classic median cut: repeatedly splits the box with the widest channel range
/// (weighted by population) at its median until `max_boxes` boxes exist.
fn median_cut(pixels: Vec<[u8; 3]>, max_boxes: usize) -> Vec<(Rgb, usize)> {
    let mut boxes = vec![pixels];
    while boxes.len() < max_boxes {
        let widest = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .map(|(i, b)| {
                let (channel, range) = widest_channel(b);
                (i, channel, range as usize * b.len())
            })
            .max_by_key(|&(_, _, weight)| weight);
        let Some((index, channel, weight)) = widest else { break };
        if weight == 0 {
            break;
        }
        let mut lower = boxes.swap_remove(index);
        lower.sort_unstable_by_key(|p| p[channel]);
        let upper = lower.split_off(lower.len() / 2);
        boxes.push(lower);
        boxes.push(upper);
    }
    boxes.iter().filter(|b| !b.is_empty()).map(|b| (mean(b), b.len())).collect()
}

fn widest_channel(pixels: &[[u8; 3]]) -> (usize, u8) {
    let mut min = [u8::MAX; 3];
    let mut max = [u8::MIN; 3];
    for p in pixels {
        for c in 0..3 {
            min[c] = min[c].min(p[c]);
            max[c] = max[c].max(p[c]);
        }
    }
    (0..3).map(|c| (c, max[c] - min[c])).max_by_key(|&(_, r)| r).unwrap_or((0, 0))
}

fn mean(pixels: &[[u8; 3]]) -> Rgb {
    let mut sum = [0u64; 3];
    for p in pixels {
        for c in 0..3 {
            sum[c] += p[c] as u64;
        }
    }
    let n = pixels.len() as u64;
    Rgb::new((sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 * 32 * 4);
        for y in 0..32 {
            for x in 0..32 {
                out.extend_from_slice(&f(x, y));
            }
        }
        out
    }

    fn hue_of(c: Rgb) -> f32 {
        to_oklch(c).h
    }

    #[test]
    fn blue_artwork_gives_cool_accent() {
        let p = extract(&image(|_, _| [30, 80, 210, 255]));
        assert!(!p.monochrome);
        assert!(hue_distance(hue_of(p.accent), hue_of(Rgb::new(30, 80, 210))) < 12.0);
    }

    #[test]
    fn orange_artwork_gives_warm_accent() {
        let p = extract(&image(|_, _| [235, 130, 30, 255]));
        let h = hue_of(p.accent);
        assert!((40.0..80.0).contains(&h), "hue {h}");
    }

    #[test]
    fn greyscale_artwork_is_monochrome() {
        let p = extract(&image(|x, y| {
            let v = ((x + y) * 4) as u8;
            [v, v, v, 255]
        }));
        assert!(p.monochrome);
        assert_eq!(p.accent.r, p.accent.g);
        assert_eq!(p.accent.g, p.accent.b);
    }

    #[test]
    fn small_vivid_subject_beats_dark_background() {
        let p = extract(&image(|x, y| if x < 10 && y < 10 { [210, 30, 40, 255] } else { [8, 8, 10, 255] }));
        assert!(!p.monochrome);
        let h = hue_of(p.accent);
        assert!(hue_distance(h, hue_of(Rgb::new(210, 30, 40))) < 15.0, "hue {h}");
    }

    #[test]
    fn transparent_or_tiny_input_is_neutral() {
        assert_eq!(extract(&image(|_, _| [255, 0, 0, 0])), AmbientPalette::NEUTRAL);
        assert_eq!(extract(&[255, 0, 0, 255]), AmbientPalette::NEUTRAL);
        assert_eq!(extract(&[]), AmbientPalette::NEUTRAL);
    }

    #[test]
    fn accents_stay_legible_and_restrained() {
        let inputs = [[0, 255, 0, 255], [255, 0, 255, 255], [20, 20, 120, 255], [250, 240, 40, 255], [90, 10, 10, 255]];
        for px in inputs {
            let p = extract(&image(|_, _| px));
            assert!(contrast_ratio(p.accent, SURFACE) >= MIN_ACCENT_CONTRAST, "{px:?} -> {p:?}");
            assert!(to_oklch(p.accent).c <= 0.135, "{px:?} too saturated: {p:?}");
            assert!(to_oklch(p.wash).l < 0.4, "{px:?} wash too bright: {p:?}");
        }
    }

    #[test]
    fn extraction_is_deterministic() {
        let img = image(|x, y| [(x * 8) as u8, (y * 8) as u8, 128, 255]);
        assert_eq!(extract(&img), extract(&img));
    }
}
