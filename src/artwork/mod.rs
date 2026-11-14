//! Album artwork decoding, normalisation and caching.
//!
//! Thumbnails arrive as encoded bytes from the media session. Each distinct image is
//! decoded once, cropped to a square and scaled to a small fixed size with rounded
//! corners baked in. Only the small result is kept; the encoded bytes and the
//! full-size decode are dropped immediately.

use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::sync::Arc;

use image::imageops::FilterType;
use image::{ImageReader, Limits, RgbaImage};

/// Edge length of the stored artwork. Large enough for the expanded player at 200%
/// display scaling (64 logical px), small enough to keep each entry at 64 KiB.
pub const SIZE: u32 = 128;
/// Corner radius as a fraction of the edge length; matches the UI artwork tiles.
const CORNER_RATIO: f32 = 0.19;
/// Number of processed artworks kept in memory.
const CACHE_ENTRIES: usize = 8;

/// A processed artwork as straight (not premultiplied) RGBA8 pixels.
#[derive(Clone, Debug)]
pub struct Artwork {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

#[derive(Default)]
pub struct ArtworkCache {
    entries: VecDeque<(u64, Artwork)>,
}

impl ArtworkCache {
    /// Returns the processed artwork for `bytes`, decoding only on a cache miss.
    pub fn process(&mut self, bytes: &[u8]) -> Option<Artwork> {
        let key = fingerprint(bytes);
        if let Some(index) = self.entries.iter().position(|(k, _)| *k == key) {
            let entry = self.entries.remove(index)?;
            let artwork = entry.1.clone();
            self.entries.push_front(entry);
            return Some(artwork);
        }
        let artwork = match decode(bytes) {
            Ok(artwork) => artwork,
            Err(err) => {
                tracing::debug!(%err, "artwork could not be decoded");
                return None;
            }
        };
        self.entries.push_front((key, artwork.clone()));
        self.entries.truncate(CACHE_ENTRIES);
        Some(artwork)
    }
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.len().hash(&mut hasher);
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn decode(bytes: &[u8]) -> image::ImageResult<Artwork> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    reader.limits(limits);
    let image = reader.decode()?;

    // Video thumbnails are often 16:9; album tiles are square, so crop the centre.
    let side = image.width().min(image.height());
    let square = image.crop_imm((image.width() - side) / 2, (image.height() - side) / 2, side, side);
    let mut tile = square.resize_exact(SIZE, SIZE, FilterType::Triangle).to_rgba8();
    drop(square);

    round_corners(&mut tile, SIZE as f32 * CORNER_RATIO);
    Ok(Artwork { width: SIZE, height: SIZE, pixels: tile.into_raw().into() })
}

/// Applies an anti-aliased rounded-rectangle alpha mask.
fn round_corners(image: &mut RgbaImage, radius: f32) {
    let (w, h) = (image.width() as f32, image.height() as f32);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let cx = px.clamp(radius, w - radius);
        let cy = py.clamp(radius, h - radius);
        let distance = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
        let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
        if coverage < 1.0 {
            pixel[3] = (pixel[3] as f32 * coverage).round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
        let img = RgbaImage::from_pixel(width, height, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn decodes_and_normalises_size() {
        let art = ArtworkCache::default().process(&png(300, 300, [30, 80, 210])).unwrap();
        assert_eq!((art.width, art.height), (SIZE, SIZE));
        assert_eq!(art.pixels.len(), (SIZE * SIZE * 4) as usize);
    }

    #[test]
    fn crops_wide_thumbnails() {
        let art = ArtworkCache::default().process(&png(320, 180, [200, 40, 40])).unwrap();
        assert_eq!((art.width, art.height), (SIZE, SIZE));
    }

    #[test]
    fn corners_are_transparent_and_centre_is_opaque() {
        let art = ArtworkCache::default().process(&png(64, 64, [10, 200, 90])).unwrap();
        let alpha = |x: u32, y: u32| art.pixels[((y * SIZE + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(SIZE - 1, SIZE - 1), 0);
        assert_eq!(alpha(SIZE / 2, SIZE / 2), 255);
        assert_eq!(alpha(SIZE / 2, 0), 255);
    }

    #[test]
    fn rejects_garbage() {
        assert!(ArtworkCache::default().process(b"definitely not an image").is_none());
        assert!(ArtworkCache::default().process(&[]).is_none());
    }

    #[test]
    fn cache_reuses_and_bounds_entries() {
        let mut cache = ArtworkCache::default();
        let first = png(16, 16, [1, 2, 3]);
        let a = cache.process(&first).unwrap();
        let b = cache.process(&first).unwrap();
        assert!(Arc::ptr_eq(&a.pixels, &b.pixels));
        for i in 0..20u8 {
            cache.process(&png(16, 16, [i, 100, 200]));
        }
        assert_eq!(cache.entries.len(), CACHE_ENTRIES);
    }
}
