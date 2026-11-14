//! Headless rendering tests for the island UI.
//!
//! The real `Island` component is rendered with Slint's software renderer into memory,
//! with a controllable clock so that animations can be completed instantly. Each test
//! checks a few properties of the image and writes it to `target/ui-snapshots/` for
//! visual review.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use image::{Rgba, RgbaImage};
use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, PlatformError};

use crate::ui::{AppState, Island, MediaAvailability, MediaInfo, Theme};

/// Desktop-like backdrop the transparent island is composited onto.
const BACKDROP: [u8; 3] = [214, 219, 228];

struct TestPlatform {
    window: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
}

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}

pub struct Harness {
    pub island: Island,
    window: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
    scale: f32,
}

impl Harness {
    /// Creates the island on the current test thread at the given scale factor.
    pub fn new(scale: f32) -> Self {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(TestPlatform { window: window.clone(), clock: clock.clone() }))
            .expect("one platform per test thread");
        window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: scale });
        let island = Island::new().expect("create island");
        let state = island.global::<AppState>();
        state.set_time("14:05".into());
        state.set_weekday("Sat".into());
        state.set_date("14 Nov".into());
        island.show().expect("show island");
        Self { island, window, clock, scale }
    }

    pub fn state(&self) -> AppState<'_> {
        self.island.global::<AppState>()
    }

    /// Lets every running animation finish.
    pub fn settle(&self) {
        for _ in 0..4 {
            self.clock.set(self.clock.get() + Duration::from_millis(500));
            slint::platform::update_timers_and_animations();
        }
    }

    /// Renders the window composited over a light backdrop.
    pub fn render(&self) -> RgbaImage {
        // The island window is 460 x 270 logical pixels (see ui/island.slint).
        let size = PhysicalSize::new((460.0 * self.scale).round() as u32, (270.0 * self.scale).round() as u32);
        self.window.set_size(size);
        let mut buffer = vec![PremultipliedRgbaColor::default(); (size.width * size.height) as usize];
        // Conditional elements are created during the first render and start their fade-in
        // animations then, so render, let animations finish, and render again.
        for _ in 0..2 {
            self.settle();
            self.window.request_redraw();
            self.window.draw_if_needed(|renderer| {
                renderer.render(&mut buffer, size.width as usize);
            });
        }
        RgbaImage::from_fn(size.width, size.height, |x, y| {
            let p = buffer[(y * size.width + x) as usize];
            let inverse = 255 - p.alpha as u16;
            let over = |src: u8, dst: u8| (src as u16 + (dst as u16 * inverse) / 255) as u8;
            Rgba([over(p.red, BACKDROP[0]), over(p.green, BACKDROP[1]), over(p.blue, BACKDROP[2]), 255])
        })
    }
}

pub fn save(image: &RgbaImage, name: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("ui-snapshots");
    let _ = std::fs::create_dir_all(&dir);
    let _ = image.save(dir.join(format!("{name}.png")));
}

/// Width of the run of island pixels (darker than the backdrop) on row `y`.
fn island_width(image: &RgbaImage, y: u32) -> u32 {
    (0..image.width()).filter(|&x| image.get_pixel(x, y)[0] < 60).count() as u32
}

/// Height of the island at column `x`.
fn island_height(image: &RgbaImage, x: u32) -> u32 {
    (0..image.height()).filter(|&y| image.get_pixel(x, y)[0] < 60).count() as u32
}

fn playing(title: &str) -> MediaInfo {
    MediaInfo {
        availability: MediaAvailability::Active,
        playing: true,
        title: title.into(),
        artist: "Lilt Test Ensemble".into(),
        source: "Media Player".into(),
        can_play_pause: true,
        can_next: true,
        can_previous: true,
        can_seek: true,
        has_timeline: true,
        session_count: 1,
    }
}

fn solid_artwork(rgb: [u8; 3]) -> slint::Image {
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(128, 128);
    for pixel in buffer.make_mut_slice() {
        *pixel = slint::Rgba8Pixel { r: rgb[0], g: rgb[1], b: rgb[2], a: 255 };
    }
    slint::Image::from_rgba8(buffer)
}

#[test]
fn compact_idle_shows_a_small_pill() {
    let h = Harness::new(1.0);
    let image = h.render();
    save(&image, "compact-idle");
    // Row 11 lies above the text, where the rounded ends are nearly full width.
    let width = island_width(&image, 11);
    assert!((185..=200).contains(&width), "compact width {width}");
    assert!((36..=40).contains(&island_height(&image, image.width() / 2)));
}

#[test]
fn compact_media_widens_and_expanded_grows() {
    let h = Harness::new(1.0);
    h.state().set_media(playing("Blue Hours"));
    let compact = h.render();
    save(&compact, "compact-media");
    let width = island_width(&compact, 11);
    assert!((245..=258).contains(&width), "media width {width}");

    h.state().set_expanded(true);
    let expanded = h.render();
    save(&expanded, "expanded-media");
    assert!((430..=445).contains(&island_width(&expanded, 120)));
    assert!(island_height(&expanded, expanded.width() / 2) > 200);
}

#[test]
fn artwork_is_drawn_in_the_player() {
    let h = Harness::new(1.0);
    h.state().set_media(playing("Amber Hours"));
    h.state().set_artwork(solid_artwork([232, 132, 34]));
    h.state().set_has_artwork(true);
    h.state().set_expanded(true);
    let image = h.render();
    save(&image, "expanded-artwork");
    // Centre of the 72 px artwork tile: x = margin + padding + 36, y = padding + 50.
    let (x, y) = (10 + 14 + 36, 14 + 50);
    let p = image.get_pixel(x, y);
    assert!(p[0] > 200 && p[1] > 100 && p[1] < 160 && p[2] < 70, "artwork pixel {p:?}");
}

#[test]
fn high_dpi_rendering_scales_the_window() {
    let h = Harness::new(1.5);
    let image = h.render();
    save(&image, "compact-idle-150");
    let width = island_width(&image, 16);
    assert!((280..=296).contains(&width), "scaled compact width {width}");
}

fn ambient_scene(intensity: f32) -> RgbaImage {
    let h = Harness::new(1.0);
    h.state().set_media(playing("Blue Hours"));
    h.state().set_artwork(solid_artwork([36, 92, 214]));
    h.state().set_has_artwork(true);
    let theme = h.island.global::<Theme>();
    theme.set_ambient(intensity);
    theme.set_glow(slint::Color::from_rgb_u8(60, 110, 230));
    theme.set_wash(slint::Color::from_rgb_u8(30, 60, 140));
    h.state().set_expanded(true);
    h.render()
}

#[test]
fn ambient_colours_tint_the_expanded_player() {
    let tinted = ambient_scene(1.0);
    save(&tinted, "expanded-ambient");
    // Just right of the artwork tile, inside the glow.
    let glow = tinted.get_pixel(10 + 14 + 72 + 6, 64);
    assert!(glow[2] > glow[0] + 12, "glow should be blue: {glow:?}");
    // Near the top of the panel, inside the wash.
    let wash = tinted.get_pixel(300, 8);
    assert!(wash[2] > wash[0] + 6, "wash should be blue: {wash:?}");
}

#[test]
fn ambient_effects_can_be_disabled() {
    let plain = ambient_scene(0.0);
    let glow = plain.get_pixel(10 + 14 + 72 + 6, 64);
    assert!(glow[2].abs_diff(glow[0]) <= 3, "no tint expected: {glow:?}");
}
