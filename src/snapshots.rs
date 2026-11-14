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

use crate::ui::{
    AgendaEntry, AppState, Appearance, AppearanceTokens, ArchiveRow, ArchiveRowKind, CalendarState, Island,
    MediaAvailability, MediaInfo, Page, SettingsData, SettingsWindow, SystemInfo, TaskForm, TaskView, TasksActions,
    TasksLayout, TasksState, Theme,
};

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
    pub window: Rc<MinimalSoftwareWindow>,
    pub clock: Rc<Cell<Duration>>,
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

    pub fn theme(&self) -> Theme<'_> {
        self.island.global::<Theme>()
    }

    /// Renders the window composited over a light backdrop.
    pub fn render(&self) -> RgbaImage {
        self.render_on(|_, _| BACKDROP)
    }

    /// Renders the window composited over the given wallpaper.
    pub fn render_on(&self, wallpaper: impl Fn(u32, u32) -> [u8; 3]) -> RgbaImage {
        // The window's logical size as laid out by ui/island.slint.
        let (width, height) = (self.island.get_window_width(), self.island.get_window_height());
        let size = PhysicalSize::new((width * self.scale).round() as u32, (height * self.scale).round() as u32);
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
            let back = wallpaper(x, y);
            Rgba([over(p.red, back[0]), over(p.green, back[1]), over(p.blue, back[2]), 255])
        })
    }
}

/// A smooth, colourful wallpaper, as the desktop looks once blurred behind glass.
fn wallpaper(x: u32, y: u32) -> [u8; 3] {
    let (u, v) = (x as f32 / 480.0, y as f32 / 270.0);
    let mix = |a: f32, b: f32, t: f32| (a + (b - a) * t.clamp(0.0, 1.0)) as u8;
    [mix(40.0, 236.0, u * 0.9 + v * 0.2), mix(70.0, 140.0, v), mix(190.0, 90.0, u)]
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
    assert!((450..=465).contains(&island_width(&expanded, 120)));
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
    // In the panel away from the artwork, inside the tint.
    let wash = tinted.get_pixel(300, 120);
    assert!(wash[2] > wash[0] + 6, "tint should be blue: {wash:?}");
}

#[test]
fn ambient_effects_can_be_disabled() {
    let plain = ambient_scene(0.0);
    let glow = plain.get_pixel(10 + 14 + 72 + 6, 64);
    assert!(glow[2].abs_diff(glow[0]) <= 3, "no tint expected: {glow:?}");
}

fn readings() -> SystemInfo {
    SystemInfo {
        ready: true,
        cpu: 0.23,
        cpu_text: "23%".into(),
        memory: 0.61,
        memory_text: "61%".into(),
        memory_detail: "4.7 GB of 7.7 GB in use".into(),
        down_text: "1.4 MB/s".into(),
        up_text: "86 KB/s".into(),
    }
}

#[test]
fn system_tile_and_page_render_readings() {
    let h = Harness::new(1.0);
    h.state().set_system(readings());
    h.state().set_expanded(true);
    let dashboard = h.render();
    save(&dashboard, "expanded-system-tile");
    // The CPU and RAM meter fills (accent) inside the system tile, the middle tile.
    let lit = (130..200)
        .flat_map(|y| (170..300).map(move |x| (x, y)))
        .filter(|&(x, y)| dashboard.get_pixel(x, y)[0] > 200)
        .count();
    assert!(lit > 10, "meter fill expected, found {lit} lit pixels");

    h.state().set_page(Page::System);
    let page = h.render();
    save(&page, "expanded-system-page");
    assert_ne!(dashboard.as_raw(), page.as_raw());
}

fn entry(day: &str, time: &str, title: &str, now: bool) -> AgendaEntry {
    AgendaEntry { title: title.into(), location: "".into(), day: day.into(), time: time.into(), now }
}

#[test]
fn calendar_tile_and_agenda_render_events() {
    let h = Harness::new(1.0);
    h.state().set_calendar_state(CalendarState::Ready);
    h.state().set_calendar_source("work.ics".into());
    h.state().set_agenda(slint::ModelRc::new(slint::VecModel::from(vec![
        entry("Today", "14:00 – 14:30", "Design review", true),
        entry("Tomorrow", "09:00 – 09:15", "Standup", false),
        entry("Mon 16", "All day", "Release day", false),
    ])));
    h.state().set_expanded(true);
    let dashboard = h.render();
    save(&dashboard, "expanded-calendar-tile");
    h.state().set_page(Page::Calendar);
    let agenda = h.render();
    save(&agenda, "expanded-agenda");
    assert_ne!(dashboard.as_raw(), agenda.as_raw());
}

#[test]
fn agenda_without_calendar_explains_itself() {
    let h = Harness::new(1.0);
    h.state().set_expanded(true);
    h.state().set_page(Page::Calendar);
    let image = h.render();
    save(&image, "expanded-agenda-empty");
}

fn settings_data() -> SettingsData {
    SettingsData {
        auto_collapse: true,
        auto_collapse_seconds: 8,
        collapse_on_outside_click: true,
        activity_banners: true,
        compact_width: 196,
        expanded_width: 460,
        allow_drag: true,
        show_media: true,
        show_calendar: true,
        show_system: true,
        show_actions: true,
        show_artwork: true,
        show_progress: true,
        ambient_enabled: true,
        ambient_intensity: 60,
        hide_in_fullscreen: true,
        top_margin: 8,
        shortcut: "Ctrl+Alt+Space".into(),
        ..Default::default()
    }
}

/// Renders one page of the settings window at 760 x 540.
fn render_settings(window: &MinimalSoftwareWindow, clock: &Cell<Duration>) -> RgbaImage {
    let size = PhysicalSize::new(760, 540);
    window.set_size(size);
    let mut buffer = vec![PremultipliedRgbaColor::default(); (size.width * size.height) as usize];
    for _ in 0..2 {
        clock.set(clock.get() + Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, size.width as usize);
        });
    }
    RgbaImage::from_fn(size.width, size.height, |x, y| {
        let p = buffer[(y * size.width + x) as usize];
        Rgba([p.red, p.green, p.blue, 255])
    })
}

#[test]
fn settings_sections_render() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform { window: window.clone(), clock: clock.clone() }))
        .expect("one platform per test thread");
    let settings = SettingsWindow::new().expect("create settings window");
    settings.set_version("0.1.0".into());
    settings.set_data_folder("%APPDATA%/Lilt".into());
    settings.set_data(settings_data());
    settings.show().expect("show settings");
    let mut previous: Option<RgbaImage> = None;
    for section in 0..9 {
        settings.set_section(section);
        let image = render_settings(&window, &clock);
        save(&image, &format!("settings-{section}"));
        assert!(previous.as_ref() != Some(&image), "section {section} rendered the same as the previous one");
        previous = Some(image);
    }
}

#[test]
fn appearance_and_position_share_a_page() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform { window: window.clone(), clock: clock.clone() }))
        .expect("one platform per test thread");
    let settings = SettingsWindow::new().expect("create settings window");
    settings.set_data(settings_data());
    let reset = Rc::new(Cell::new(false));
    let seen = reset.clone();
    settings.on_reset_appearance(move || seen.set(true));
    settings.show().expect("show settings");
    settings.set_section(1);
    // A tall window shows both groups and the reset button.
    let size = PhysicalSize::new(760, 1000);
    window.set_size(size);
    let mut buffer = vec![PremultipliedRgbaColor::default(); (size.width * size.height) as usize];
    for _ in 0..2 {
        clock.set(clock.get() + Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, size.width as usize);
        });
    }
    let image = RgbaImage::from_fn(size.width, size.height, |x, y| {
        let p = buffer[(y * size.width + x) as usize];
        Rgba([p.red, p.green, p.blue, 255])
    });
    save(&image, "settings-appearance-position");
}

#[test]
fn settings_follow_the_appearance() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform { window: window.clone(), clock: clock.clone() }))
        .expect("one platform per test thread");
    let settings = SettingsWindow::new().expect("create settings window");
    settings.set_data(settings_data());
    settings.show().expect("show settings");
    settings.set_section(1);
    let midnight = render_settings(&window, &clock);
    save(&midnight, "settings-appearance-midnight");

    // Choosing the Liquid Glass card selects it; the window follows once the
    // application applies the appearance.
    settings.set_data(SettingsData { appearance: 1, ..settings_data() });
    settings.set_appearance_note(crate::appearance::Fallback::TransparencyOff.explanation().into());
    let selected = render_settings(&window, &clock);
    assert_ne!(midnight, selected, "selecting a card must show");
    let theme = settings.global::<Theme>();
    theme.set_appearance(Appearance::LiquidGlass);
    let glass = render_settings(&window, &clock);
    save(&glass, "settings-appearance-glass");
    assert_ne!(selected, glass);
    for section in [0, 5] {
        settings.set_section(section);
        save(&render_settings(&window, &clock), &format!("settings-{section}-glass"));
    }

    // Back to Midnight restores the original window exactly.
    theme.set_appearance(Appearance::Midnight);
    settings.set_section(1);
    settings.set_appearance_note("".into());
    settings.set_data(settings_data());
    assert_eq!(render_settings(&window, &clock), midnight);
}

/// Expanded player with artwork and its palette applied, as in the showcase.
fn playing_with_artwork(h: &Harness) {
    let art = processed_cover();
    let state = h.state();
    state.set_media(MediaInfo { session_count: 2, ..playing("Northern Lights") });
    state.set_artwork(slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
        &art.pixels,
        art.width,
        art.height,
    )));
    state.set_has_artwork(true);
    state.set_progress(0.38);
    state.set_position_text("1:24".into());
    state.set_duration_text("3:41".into());
    let colour = |c: crate::ambient::Rgb| slint::Color::from_rgb_u8(c.r, c.g, c.b);
    h.theme().set_accent(colour(art.palette.accent));
    h.theme().set_wash(colour(art.palette.wash));
    h.theme().set_glow(colour(art.palette.glow));
    state.set_system(readings());
}

#[test]
fn switching_appearance_is_instant_and_reversible() {
    let h = Harness::new(1.0);
    // Still bars, so that renders taken at different times can be compared.
    h.theme().set_motion(0.0);
    playing_with_artwork(&h);
    let compact = h.render();
    h.state().set_expanded(true);
    let expanded = h.render();

    h.theme().set_appearance(Appearance::LiquidGlass);
    let glass = h.render();
    assert_ne!(glass, expanded, "the expanded island must change");
    h.state().set_expanded(false);
    assert_ne!(h.render(), compact, "the compact island must change");

    h.theme().set_appearance(Appearance::Midnight);
    assert_eq!(h.render(), compact, "Midnight must be restored exactly");
    h.state().set_expanded(true);
    assert_eq!(h.render(), expanded, "Midnight must be restored exactly");
}

#[test]
fn liquid_glass_renders_every_state() {
    let h = Harness::new(1.0);
    h.theme().set_appearance(Appearance::LiquidGlass);
    save(&h.render_on(wallpaper), "glass-compact-idle");
    playing_with_artwork(&h);
    save(&h.render_on(wallpaper), "glass-compact-media");
    h.state().set_activity(true);
    save(&h.render_on(wallpaper), "glass-activity");
    h.state().set_activity(false);
    h.state().set_expanded(true);

    // Frosted (no live blur) and over the blur.
    let frosted = h.render_on(wallpaper);
    save(&frosted, "glass-expanded-frosted");
    h.theme().set_live_blur(true);
    let blurred = h.render_on(wallpaper);
    save(&blurred, "glass-expanded");
    assert_ne!(frosted, blurred);

    // The surface is translucent: the wallpaper shows through, more so over the blur.
    let midnight_surface = {
        h.theme().set_appearance(Appearance::Midnight);
        h.render_on(wallpaper).get_pixel(240, 230).0
    };
    let (f, b) = (frosted.get_pixel(240, 230).0, blurred.get_pixel(240, 230).0);
    let brightness = |p: [u8; 4]| p[0] as u32 + p[1] as u32 + p[2] as u32;
    assert!(
        brightness(midnight_surface) < brightness(f) && brightness(f) < brightness(b),
        "{midnight_surface:?} {f:?} {b:?}"
    );

    // Nothing, not even the reflections, is drawn outside the rounded corners.
    for (x, y) in [(12, 3), (blurred.width() - 13, 3)] {
        assert_eq!(
            blurred.get_pixel(x, y).0,
            [wallpaper(x, y)[0], wallpaper(x, y)[1], wallpaper(x, y)[2], 255],
            "corner at {x},{y}"
        );
    }
    // The rim lights the top edge: compared with the row just inside it, the edge is far
    // brighter than Midnight's plain border.
    let edge =
        |image: &RgbaImage| brightness(image.get_pixel(240, 0).0) as i32 - brightness(image.get_pixel(240, 1).0) as i32;
    h.theme().set_appearance(Appearance::LiquidGlass);
    let glass_edge = edge(&h.render());
    h.theme().set_appearance(Appearance::Midnight);
    let midnight_edge = edge(&h.render());
    assert!(glass_edge > midnight_edge + 60, "rim {glass_edge} vs border {midnight_edge}");

    for page in [Page::System, Page::Calendar] {
        h.theme().set_appearance(Appearance::LiquidGlass);
        h.state().set_page(page);
        save(&h.render_on(wallpaper), &format!("glass-page-{page:?}").to_lowercase());
    }
}

#[test]
fn ambient_colours_work_in_liquid_glass() {
    let h = Harness::new(1.0);
    h.theme().set_appearance(Appearance::LiquidGlass);
    h.theme().set_live_blur(true);
    h.state().set_media(playing("Blue Hours"));
    h.state().set_artwork(solid_artwork([36, 92, 214]));
    h.state().set_has_artwork(true);
    h.theme().set_ambient(1.0);
    h.theme().set_glow(slint::Color::from_rgb_u8(60, 110, 230));
    h.theme().set_wash(slint::Color::from_rgb_u8(30, 60, 140));
    h.state().set_expanded(true);
    let tinted = h.render();
    save(&tinted, "glass-ambient");
    let glow = tinted.get_pixel(10 + 14 + 72 + 6, 64);
    assert!(glow[2] > glow[0] + 12, "glow should be blue: {glow:?}");
    let wash = tinted.get_pixel(300, 120);
    assert!(wash[2] > wash[0] + 6, "tint should be blue: {wash:?}");
}

/// The README image of Liquid Glass: the compact and expanded island over one desktop.
#[test]
fn liquid_glass_showcase() {
    let h = Harness::new(1.0);
    h.theme().set_appearance(Appearance::LiquidGlass);
    h.theme().set_live_blur(true);
    playing_with_artwork(&h);
    h.state().set_calendar_state(CalendarState::Ready);
    h.state().set_agenda(slint::ModelRc::new(slint::VecModel::from(vec![
        entry("Today", "16:30 – 17:00", "Design review", false),
        entry("Tomorrow", "09:00 – 09:15", "Standup", false),
    ])));
    let desktop = |x: u32, y: u32| {
        let (u, v) = (x as f32 / 1020.0, y as f32 / 290.0);
        let mix = |a: f32, b: f32, t: f32| (a + (b - a) * t.clamp(0.0, 1.0)) as u8;
        [mix(38.0, 238.0, u * 0.85 + v * 0.25), mix(64.0, 150.0, v * 0.8 + u * 0.1), mix(196.0, 84.0, u)]
    };
    let compact = h.render_on(|x, y| desktop(x + 20, y + 10));
    h.state().set_expanded(true);
    let expanded = h.render_on(|x, y| desktop(x + 530, y + 10));
    let mut sheet = RgbaImage::from_fn(1020, 290, |x, y| {
        let c = desktop(x, y);
        Rgba([c[0], c[1], c[2], 255])
    });
    image::imageops::overlay(&mut sheet, &compact, 20, 10);
    image::imageops::overlay(&mut sheet, &expanded, 530, 10);
    save(&sheet, "glass-overview");
}

/// Renders the compact island twice, letting `ms` of time pass in between.
fn two_frames(h: &Harness, ms: u64) -> (RgbaImage, RgbaImage) {
    let first = h.render();
    h.clock.set(h.clock.get() + Duration::from_millis(ms));
    slint::platform::update_timers_and_animations();
    (first, h.render())
}

/// The playing glyph's area in the compact media island.
fn glyph(image: &RgbaImage) -> Vec<u8> {
    let (x, width) = ((image.width() + 254) / 2 - 30, 30);
    image::imageops::crop_imm(image, x, 0, width, 38).to_image().into_raw()
}

#[test]
fn playing_bars_move_only_while_playing_and_visible() {
    let h = Harness::new(1.0);
    h.state().set_media(playing("Blue Hours"));
    let (a, b) = two_frames(&h, 70);
    save(&b, "compact-visualizer");
    assert_ne!(glyph(&a), glyph(&b), "the bars move while playing");

    // Reduced motion: the original still bars.
    h.theme().set_motion(0.0);
    let (a, b) = two_frames(&h, 70);
    assert_eq!(glyph(&a), glyph(&b));
    h.theme().set_motion(1.0);

    // Hidden or expanded: no updates.
    h.state().set_on_screen(false);
    let (a, b) = two_frames(&h, 70);
    assert_eq!(glyph(&a), glyph(&b));
    h.state().set_on_screen(true);

    // Paused: settled and still.
    h.state().set_media(MediaInfo { playing: false, ..playing("Blue Hours") });
    let (a, b) = two_frames(&h, 70);
    assert_eq!(glyph(&a), glyph(&b));
    save(&b, "compact-visualizer-paused");
}

#[test]
fn playing_bars_work_in_liquid_glass() {
    let h = Harness::new(1.0);
    h.theme().set_appearance(Appearance::LiquidGlass);
    h.state().set_media(playing("Blue Hours"));
    let (a, b) = two_frames(&h, 70);
    save(&b, "glass-compact-visualizer");
    assert_ne!(glyph(&a), glyph(&b));
}

/// Example tasks for rendering tests only; never shown to users.
fn sample_tasks() -> Vec<crate::tasks::Task> {
    use crate::tasks::tests::task;
    use crate::tasks::{Priority, Source, SyncState};
    let mut tasks = vec![
        task(1, Priority::Critical, Some(-1)),
        task(2, Priority::High, Some(0)),
        task(3, Priority::High, Some(2)),
        task(4, Priority::Medium, None),
        task(5, Priority::Low, Some(6)),
        task(6, Priority::None, None),
    ];
    let titles = [
        "Send the signed contract to Priya",
        "Prepare slides for the team meeting",
        "Review the pull request for the release",
        "Book a dentist appointment",
        "Renew the domain",
        "Read the design notes",
    ];
    for (task, title) in tasks.iter_mut().zip(titles) {
        task.title = title.into();
    }
    tasks[1].due_minute = Some(17 * 60);
    tasks[1].category = "Work".into();
    tasks[2].source = Source::GoogleTasks;
    tasks[2].sync = SyncState::Synced;
    tasks
}

fn set_appearance(h: &Harness, appearance: Appearance) {
    h.theme().set_appearance(appearance);
    h.theme().set_live_blur(appearance == Appearance::LiquidGlass);
}

fn tasks_harness(appearance: Appearance) -> Harness {
    let h = Harness::new(1.0);
    set_appearance(&h, appearance);
    fill_tasks(&h);
    h
}

fn fill_tasks(h: &Harness) {
    use crate::app::tasks::build_rows;
    use crate::tasks::tests::NOW;
    use crate::tasks::{SortMode, View};
    let state = h.state();
    state.set_show_tasks(true);
    state.set_expanded(true);
    let tasks = h.island.global::<TasksState>();
    tasks.set_ready(true);
    tasks.set_date_label("Sat 14 Nov".into());
    let sample = sample_tasks();
    tasks.set_rows(slint::ModelRc::new(slint::VecModel::from(build_rows(
        &sample,
        View::All,
        None,
        SortMode::Priority,
        NOW,
        true,
    ))));
    tasks.set_open_count(6);
    tasks.set_due_today_count(2);
    tasks.set_upcoming_count(2);
    tasks.set_done_today(3);
    tasks.set_left_today(2);
    let actions = h.island.global::<TasksActions>();
    actions.on_day_label(|offset| if offset == -10000 { "No date".into() } else { "Today".into() });
    actions.on_time_valid(|text| crate::app::tasks::parse_time(&text).is_some());
}

#[test]
fn tasks_box_sits_beside_or_below_in_both_appearances() {
    let h = tasks_harness(Appearance::Midnight);
    let midnight = h.render_on(wallpaper);
    save(&midnight, "tasks-midnight");
    // Beside: the dashboard (460) and the tasks box (400) with a 10 px gap, plus margins.
    assert_eq!((midnight.width(), midnight.height()), (890, 340));
    set_appearance(&h, Appearance::LiquidGlass);
    let glass = h.render_on(wallpaper);
    save(&glass, "tasks-glass");
    assert_ne!(midnight, glass);
    // The gap between the boxes is empty: clicks there reach the desktop.
    let gap = (h.island.get_tasks_x() as u32 - 5, 120);
    assert_eq!(glass.get_pixel(gap.0, gap.1).0[..3], wallpaper(gap.0, gap.1));

    h.state().set_tasks_layout(TasksLayout::Below);
    let below = h.render_on(wallpaper);
    save(&below, "tasks-glass-below");
    // Below: both boxes as wide as the dashboard, one above the other.
    assert_eq!((below.width(), below.height()), (480, 600));
    assert_eq!(h.island.get_tasks_box_width(), 460.0);
    assert!(h.island.get_tasks_y() > h.island.get_body_height());
    set_appearance(&h, Appearance::Midnight);
    save(&h.render_on(wallpaper), "tasks-midnight-below");

    // Without Tasks the island is exactly the original.
    h.state().set_show_tasks(false);
    let plain = h.render();
    assert_eq!((plain.width(), plain.height()), (480, 270));
}

#[test]
fn tasks_states_render() {
    let h = tasks_harness(Appearance::Midnight);
    for (appearance, name) in [(Appearance::Midnight, "midnight"), (Appearance::LiquidGlass, "glass")] {
        set_appearance(&h, appearance);
        fill_tasks(&h);
        let tasks = h.island.global::<TasksState>();

        tasks.set_adding(true);
        tasks.set_draft("Call the bank".into());
        tasks.set_draft_priority(3);
        tasks.set_draft_due(0);
        save(&h.render_on(wallpaper), &format!("tasks-quick-add-{name}"));
        tasks.set_adding(false);

        tasks.set_undo_text("Completed · Book a dentist appointment".into());
        tasks.set_undo_visible(true);
        save(&h.render_on(wallpaper), &format!("tasks-undo-{name}"));
        tasks.set_undo_visible(false);

        tasks.set_form(TaskForm {
            id: 2,
            title: "Prepare slides for the team meeting".into(),
            notes: "Ten slides at most; include the roadmap.".into(),
            priority: 3,
            due_offset: 0,
            time: "17:00".into(),
            category: "Work".into(),
            source: "Lilt".into(),
            time_allowed: true,
            ..Default::default()
        });
        tasks.set_editing(true);
        save(&h.render_on(wallpaper), &format!("tasks-editor-{name}"));
        tasks.set_editing(false);

        tasks.set_view(TaskView::Done);
        let archive = vec![
            ArchiveRow { kind: ArchiveRowKind::Header, title: "Today".into(), ..Default::default() },
            ArchiveRow {
                kind: ArchiveRowKind::Task,
                id: 9,
                title: "Pay the electricity bill".into(),
                detail: "Completed 09:12 · High · was due Today".into(),
                priority: 3,
                ..Default::default()
            },
            ArchiveRow { kind: ArchiveRowKind::Header, title: "Earlier this week".into(), ..Default::default() },
            ArchiveRow {
                kind: ArchiveRowKind::Task,
                id: 10,
                title: "Send the quarterly report".into(),
                detail: "Completed Tue 10, 16:40 · Google Tasks".into(),
                ..Default::default()
            },
        ];
        tasks.set_archive(slint::ModelRc::new(slint::VecModel::from(archive)));
        save(&h.render_on(wallpaper), &format!("tasks-archive-{name}"));
        tasks.set_archive(slint::ModelRc::default());
        save(&h.render_on(wallpaper), &format!("tasks-archive-empty-{name}"));
        tasks.set_view(TaskView::All);

        tasks.set_rows(slint::ModelRc::default());
        tasks.set_open_count(0);
        tasks.set_done_today(0);
        tasks.set_left_today(0);
        tasks.set_empty_title("Nothing competing for your attention.".into());
        tasks.set_empty_detail("Add a task to get started: press N or the + button.".into());
        save(&h.render_on(wallpaper), &format!("tasks-empty-{name}"));
    }
}

#[test]
fn completing_a_task_ticks_it_at_once() {
    let h = tasks_harness(Appearance::Midnight);
    let completed = Rc::new(Cell::new(0));
    let seen = completed.clone();
    h.island.global::<TasksActions>().on_complete(move |id| seen.set(id));
    let before = h.render();
    // The focus card's completion circle: the box's padding + 12 + 9 in, and below the
    // header, the filter chips and the card's "Focus now" label.
    let (x, y) = (h.island.get_tasks_x() + 14.0 + 12.0 + 9.0, 14.0 + 32.0 + 8.0 + 24.0 + 8.0 + 4.0 + 30.0 + 6.0);
    let position = slint::LogicalPosition::new(x, y);
    h.window.dispatch_event(slint::platform::WindowEvent::PointerPressed {
        position,
        button: slint::platform::PointerEventButton::Left,
    });
    h.window.dispatch_event(slint::platform::WindowEvent::PointerReleased {
        position,
        button: slint::platform::PointerEventButton::Left,
    });
    assert_eq!(completed.get(), 1, "the critical overdue task is first");
    let after = h.render();
    save(&after, "tasks-completing");
    // Inside the circle, away from the tick mark.
    let (px, py) = (x as u32 - 4, y as u32 - 4);
    // The circle fills with the accent while the row fades out.
    let (was, now) = (before.get_pixel(px, py)[0], after.get_pixel(px, py)[0]);
    assert!(now > was + 30, "circle {was} -> {now}");
}

fn key(h: &Harness, text: &str) {
    h.window.dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.into() });
    h.window.dispatch_event(slint::platform::WindowEvent::KeyReleased { text: text.into() });
}

#[test]
fn tasks_keyboard_quick_add() {
    let h = tasks_harness(Appearance::Midnight);
    let added = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = added.clone();
    h.island.global::<TasksActions>().on_quick_add(move |title, priority, due, _, _| {
        seen.borrow_mut().push((title.to_string(), priority, due));
    });
    let tasks = h.island.global::<TasksState>();
    h.render();
    // N opens quick entry when nothing else has the keyboard.
    key(&h, "n");
    assert!(tasks.get_adding(), "N opens quick add");
    h.render();
    for c in "Call the bank".chars() {
        key(&h, &c.to_string());
    }
    key(&h, "\n");
    assert_eq!(added.borrow().as_slice(), [("Call the bank".to_string(), 0, -10000)]);
    // Escape closes the field first, and only then the island.
    key(&h, "\u{1b}");
    assert!(!tasks.get_adding(), "Escape closes quick add");
    assert!(h.state().get_expanded(), "the island stays open");
}

fn advance(h: &Harness, ms: u64) {
    h.clock.set(h.clock.get() + Duration::from_millis(ms));
    slint::platform::update_timers_and_animations();
}

fn open_controller(h: &Harness, path: &std::path::Path) -> Rc<crate::app::tasks::TasksController> {
    use crate::app::tasks::{Hooks, TasksController};
    let controller = TasksController::new(&h.island, path.to_path_buf(), true, crate::tasks::SortMode::Priority);
    controller.connect(Hooks {
        busy: Box::new(|_| {}),
        order_changed: Box::new(|_| {}),
        layout_toggled: Box::new(|| {}),
    });
    controller
}

/// Titles of the tasks in the list, in display order.
fn listed(h: &Harness) -> Vec<String> {
    use slint::Model;
    h.island
        .global::<TasksState>()
        .get_rows()
        .iter()
        .filter(|r| r.kind != crate::ui::TaskRowKind::Header)
        .map(|r| r.title.to_string())
        .collect()
}

fn id_of(h: &Harness, title: &str) -> i32 {
    use slint::Model;
    let rows = h.island.global::<TasksState>().get_rows();
    rows.iter().find(|r| r.title == title).map(|r| r.id).unwrap_or_else(|| panic!("{title} not listed"))
}

#[test]
fn tasks_work_end_to_end() {
    let dir = std::env::temp_dir().join(format!("lilt-tasks-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("tasks.sqlite3");
    let h = Harness::new(1.0);
    let _controller = open_controller(&h, &path);
    h.state().set_show_tasks(true);
    h.state().set_expanded(true);
    // Showing the box opens the database.
    h.render();
    let state = h.island.global::<TasksState>();
    let actions = h.island.global::<TasksActions>();
    assert!(state.get_ready(), "{}", state.get_error());
    assert!(state.get_first_use());

    // Create: with only a title, and with every option.
    actions.invoke_quick_add("Read the design notes".into(), 0, -10000, "".into(), "".into());
    actions.invoke_quick_add("Send the contract".into(), 4, -1, "".into(), "Work".into());
    actions.invoke_quick_add("Prepare slides".into(), 3, 0, "17:30".into(), "".into());
    assert_eq!(listed(&h), ["Send the contract", "Prepare slides", "Read the design notes"]);
    assert_eq!((state.get_open_count(), state.get_due_today_count()), (3, 2));
    assert_eq!(state.get_draft(), "", "the field is cleared for the next task");
    assert!(!state.get_first_use());

    // Bad input is refused with an explanation.
    actions.invoke_quick_add("Bad time".into(), 0, 0, "25:00".into(), "".into());
    assert!(state.get_draft_error().contains("time"));
    actions.invoke_quick_add("   ".into(), 0, -10000, "".into(), "".into());
    assert!(state.get_draft_error().contains("title"));
    assert_eq!(state.get_open_count(), 3);

    // Complete: the counts change at once, the row leaves after its tick, undo restores.
    let contract = id_of(&h, "Send the contract");
    actions.invoke_complete(contract);
    assert_eq!(state.get_open_count(), 2);
    assert!(state.get_undo_visible());
    assert!(listed(&h).contains(&"Send the contract".to_string()), "still shown while the tick plays");
    advance(&h, 500);
    assert!(!listed(&h).contains(&"Send the contract".to_string()));
    assert_eq!(state.get_done_today(), 1);
    actions.invoke_undo();
    assert_eq!(state.get_open_count(), 3);
    assert_eq!(listed(&h)[0], "Send the contract");

    // Edit: a new priority moves the task at once.
    actions.invoke_edit(id_of(&h, "Read the design notes"));
    assert!(state.get_editing());
    let mut form = state.get_form();
    assert_eq!(form.source, "Lilt task, stored on this PC");
    form.title = "Read the design notes again".into();
    form.priority = 4;
    actions.invoke_save(form);
    assert!(!state.get_editing());
    assert_eq!(listed(&h), ["Send the contract", "Read the design notes again", "Prepare slides"]);

    // Defer to tomorrow, and undo.
    actions.invoke_defer(id_of(&h, "Prepare slides"));
    assert_eq!(state.get_due_today_count(), 1);
    actions.invoke_undo();
    assert_eq!(state.get_due_today_count(), 2);

    // Your own order, within a priority.
    actions.invoke_toggle_order();
    assert!(state.get_custom_order());
    actions.invoke_move(id_of(&h, "Read the design notes again"), -1);
    assert_eq!(listed(&h)[..2], ["Read the design notes again", "Send the contract"]);
    actions.invoke_toggle_order();

    // Archive: search, restore and delete.
    actions.invoke_complete(id_of(&h, "Prepare slides"));
    advance(&h, 500);
    state.set_view(TaskView::Done);
    actions.invoke_view_changed();
    let archived = |h: &Harness| {
        use slint::Model;
        h.island.global::<TasksState>().get_archive().iter().map(|r| r.title.to_string()).collect::<Vec<_>>()
    };
    assert_eq!(archived(&h), ["Today", "Prepare slides"]);
    state.set_search("nothing like this".into());
    actions.invoke_search_changed("nothing like this".into());
    advance(&h, 200);
    assert!(archived(&h).is_empty());
    state.set_search("".into());
    actions.invoke_search_changed("".into());
    advance(&h, 200);
    let slides = {
        use slint::Model;
        state.get_archive().iter().find(|r| r.title == "Prepare slides").map(|r| r.id).unwrap()
    };
    actions.invoke_restore(slides);
    assert!(archived(&h).is_empty());
    state.set_view(TaskView::All);
    actions.invoke_view_changed();
    assert!(listed(&h).contains(&"Prepare slides".to_string()));
    actions.invoke_complete(slides);
    advance(&h, 500);
    actions.invoke_delete_completed(slides);
    actions.invoke_clear_completed();
    assert_eq!(state.get_open_count(), 2);

    // Everything survives reopening the database.
    drop(_controller);
    let again = open_controller(&h, &path);
    again.open();
    assert_eq!(listed(&h), ["Send the contract", "Read the design notes again"]);
    drop(again);
    let _ = std::fs::remove_dir_all(&dir);
}

/// WCAG relative luminance of an sRGB colour.
fn luminance(rgb: [f32; 3]) -> f32 {
    let channel = |c: f32| {
        let c = c / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
}

fn over(top: slint::Color, below: [f32; 3]) -> [f32; 3] {
    let a = top.alpha() as f32 / 255.0;
    [
        top.red() as f32 * a + below[0] * (1.0 - a),
        top.green() as f32 * a + below[1] * (1.0 - a),
        top.blue() as f32 * a + below[2] * (1.0 - a),
    ]
}

fn contrast(text: slint::Color, background: [f32; 3]) -> f32 {
    let (a, b) = (luminance(over(text, background)) + 0.05, luminance(background) + 0.05);
    a.max(b) / a.min(b)
}

#[test]
fn liquid_glass_text_stays_legible_on_any_wallpaper() {
    let h = Harness::new(1.0);
    let tokens: AppearanceTokens = h.theme().get_liquid_glass();
    for wallpaper in [[255.0, 255.0, 255.0], [0.0, 0.0, 0.0], [214.0, 120.0, 40.0]] {
        for surface in [tokens.surface, tokens.blurred_surface] {
            let base = over(surface, wallpaper);
            let tile = over(tokens.tile, base);
            for background in [base, tile] {
                assert!(contrast(tokens.text, background) >= 7.0, "text on {background:?}");
                assert!(contrast(tokens.text_secondary, background) >= 4.5, "secondary text on {background:?}");
                assert!(contrast(tokens.text_tertiary, background) >= 3.0, "tertiary text on {background:?}");
            }
        }
    }
}

/// A cover with a soft diagonal gradient and a light disc, run through the real artwork
/// pipeline so the image has rounded corners and its own palette.
fn processed_cover() -> crate::artwork::Artwork {
    let cover = RgbaImage::from_fn(300, 300, |x, y| {
        let t = (x + y) as f32 / 600.0;
        let (dx, dy) = (x as f32 - 190.0, y as f32 - 110.0);
        let disc = ((dx * dx + dy * dy).sqrt() < 70.0) as u8 as f32;
        let mix = |a: f32, b: f32| (a + (b - a) * t + disc * 60.0).min(255.0) as u8;
        Rgba([mix(24.0, 96.0), mix(52.0, 46.0), mix(150.0, 190.0), 255])
    });
    let mut encoded = Vec::new();
    cover.write_to(&mut std::io::Cursor::new(&mut encoded), image::ImageFormat::Png).expect("encode cover");
    crate::artwork::ArtworkCache::default().process(&encoded).expect("process cover")
}

/// Renders the states used in the README with realistic content.
#[test]
fn showcase() {
    let h = Harness::new(1.0);
    let art = processed_cover();
    let state = h.state();
    state.set_media(MediaInfo { session_count: 2, ..playing("Northern Lights") });
    state.set_artwork(slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
        &art.pixels,
        art.width,
        art.height,
    )));
    state.set_has_artwork(true);
    state.set_progress(0.38);
    state.set_position_text("1:24".into());
    state.set_duration_text("3:41".into());
    let colour = |c: crate::ambient::Rgb| slint::Color::from_rgb_u8(c.r, c.g, c.b);
    let theme = h.island.global::<Theme>();
    theme.set_accent(colour(art.palette.accent));
    theme.set_wash(colour(art.palette.wash));
    theme.set_glow(colour(art.palette.glow));
    state.set_system(readings());
    state.set_calendar_state(CalendarState::Ready);
    state.set_agenda(slint::ModelRc::new(slint::VecModel::from(vec![
        entry("Today", "16:30 – 17:00", "Design review", false),
        entry("Tomorrow", "09:00 – 09:15", "Standup", false),
    ])));
    save(&h.render(), "showcase-compact");
    state.set_expanded(true);
    let expanded = h.render();
    save(&expanded, "showcase-expanded");
    // Nothing may be drawn outside the panel's rounded top corners.
    for (x, y) in [(12, 3), (expanded.width() - 13, 3)] {
        assert_eq!(expanded.get_pixel(x, y), &Rgba([BACKDROP[0], BACKDROP[1], BACKDROP[2], 255]), "corner at {x},{y}");
    }
}
