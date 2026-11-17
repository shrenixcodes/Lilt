//! The glass appearances' blur, and Liquid Glass's tint, follow what is behind the island.
//!
//! The desktop behind the island window is read, blurred as much as the transparency
//! setting asks (none for clear glass; see [`crate::window::sample`]) and drawn beneath
//! the glass surfaces by the window itself; Liquid Glass also bends it along its edges.
//! The tint is set from it too: as clear as chosen over dark content and only as dense as
//! legible text requires over bright content.
//!
//! The backdrop is read when it may have changed: when the island appears, opens, closes
//! or moves, shortly after another window comes to the front, finishes moving, scrolling
//! or being restored, and every few seconds while the island is open. Nothing is read for
//! Midnight or for frosted glass.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use slint::ComponentHandle;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_SCROLLINGEND,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

use super::{App, CURRENT};
use crate::ambient::Rgb;
use crate::appearance::{
    GlassText, LIQUID_REFRACTION, Layer, blur_radius, contrast_targets, legible_alpha, preferred_alpha,
};
use crate::settings::AppearanceMode;
use crate::ui::{Appearance, Theme};
use crate::window;

/// While the island is open, how often the backdrop is sampled again.
const OPEN_INTERVAL: Duration = Duration::from_secs(3);
/// After another window changes, how long it is given to finish drawing.
const SETTLE: Duration = Duration::from_millis(300);
/// Smaller changes of the tint are not worth a redraw.
const STEP: f32 = 0.02;

#[derive(Default)]
pub(super) struct GlassSampler {
    hook: Cell<Option<HWINEVENTHOOK>>,
    /// The brightest colour behind the island at the last reading.
    backdrop: Cell<Option<Rgb>>,
    soon: slint::Timer,
    repeat: slint::Timer,
}

fn colour(c: slint::Color) -> Rgb {
    Rgb::new(c.red(), c.green(), c.blue())
}

/// Called by Windows on this thread, from the message loop, when another window changes.
unsafe extern "system" fn window_changed(
    _hook: HWINEVENTHOOK,
    event: u32,
    _hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if matches!(
        event,
        EVENT_SYSTEM_FOREGROUND | EVENT_SYSTEM_MOVESIZEEND | EVENT_SYSTEM_SCROLLINGEND | EVENT_SYSTEM_MINIMIZEEND
    ) && let Some(app) = CURRENT.with(|c| c.borrow().upgrade())
    {
        App::sample_glass_soon(&app, SETTLE);
    }
}

impl App {
    /// Whether the glass shows the blurred backdrop now.
    fn glass_follows_backdrop(&self) -> bool {
        let theme = self.island.global::<Theme>();
        theme.get_appearance() != Appearance::Midnight && theme.get_live_blur() && self.island_visible()
    }

    /// Starts or stops sampling for the current appearance, visibility and state. Called
    /// whenever one of them changes.
    pub(super) fn update_glass_sampling(app: &Rc<Self>) {
        let sampler = &app.glass;
        if !app.glass_follows_backdrop() {
            sampler.soon.stop();
            sampler.repeat.stop();
            if let Some(hook) = sampler.hook.take() {
                let _ = unsafe { UnhookWinEvent(hook) };
            }
            return;
        }
        if sampler.hook.get().is_none() {
            let hook = unsafe {
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_MINIMIZEEND,
                    None,
                    Some(window_changed),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };
            sampler.hook.set((!hook.is_invalid()).then_some(hook));
        }
        // The whole window is read, so its shape does not matter; what lies behind may
        // just have been uncovered, though.
        App::sample_glass_soon(app, Duration::from_millis(40));
        if app.expanded() {
            let weak = Rc::downgrade(app);
            sampler.repeat.start(slint::TimerMode::Repeated, OPEN_INTERVAL, move || {
                if let Some(app) = weak.upgrade() {
                    app.sample_glass();
                }
            });
        } else {
            sampler.repeat.stop();
        }
    }

    /// Samples after `delay`; further requests within it are merged into one.
    pub(super) fn sample_glass_soon(app: &Rc<Self>, delay: Duration) {
        if !app.glass_follows_backdrop() {
            return;
        }
        let weak = Rc::downgrade(app);
        app.glass.soon.start(slint::TimerMode::SingleShot, delay, move || {
            if let Some(app) = weak.upgrade() {
                app.sample_glass();
            }
        });
    }

    /// Reads the desktop behind the island, as blurred as the transparency setting asks,
    /// and sets the tint for its brightest text-sized area.
    fn sample_glass(&self) {
        if !self.glass_follows_backdrop() {
            return;
        }
        let Some(native) = window::find(self.island.window()) else { return };
        let scale = self.island.window().scale_factor();
        let transparency = self.settings.borrow().appearance.glass_transparency;
        let liquid = self.island.global::<Theme>().get_appearance() == Appearance::LiquidGlass;
        let mode = if liquid { AppearanceMode::LiquidGlass } else { AppearanceMode::SmokedGlass };
        let blur = blur_radius(mode, transparency) * scale;
        let (Some(area), Some(own)) = (native.screen_rect(), native.hwnd()) else { return };
        let Some(backdrop) = window::sample::backdrop_behind(area, blur, own) else { return };
        let brightest = backdrop.brightest_area((16.0 * scale).round() as usize);
        self.glass.backdrop.set(brightest.map(|[r, g, b]| Rgb::new(r, g, b)));
        native.set_backdrop(backdrop, if liquid { LIQUID_REFRACTION } else { 0.0 });
        self.apply_glass_tint();
    }

    /// Sets the active glass appearance's tint from the transparency setting, darkened as
    /// far as the last backdrop requires for legible text.
    pub(super) fn apply_glass_tint(&self) {
        let theme = self.island.global::<Theme>();
        let mode = match theme.get_appearance() {
            Appearance::SmokedGlass => AppearanceMode::SmokedGlass,
            Appearance::LiquidGlass => AppearanceMode::LiquidGlass,
            _ => return,
        };
        let transparency = self.settings.borrow().appearance.glass_transparency;
        let floor = preferred_alpha(mode, transparency);
        let tokens =
            if mode == AppearanceMode::SmokedGlass { theme.get_smoked_glass() } else { theme.get_liquid_glass() };
        let tile = tokens.tile;
        let glass = GlassText {
            tint: colour(tokens.blurred_surface),
            tile: Layer { rgb: colour(tile), alpha: tile.alpha() as f32 / 255.0 },
            text: colour(tokens.text),
            secondary: colour(tokens.text_secondary),
            tertiary: colour(tokens.text_tertiary),
        };
        let alpha = match self.glass.backdrop.get() {
            Some(backdrop) if theme.get_live_blur() => {
                legible_alpha(&glass, backdrop, floor, contrast_targets(mode, transparency))
            }
            _ => floor,
        };
        let current =
            if mode == AppearanceMode::SmokedGlass { theme.get_smoked_alpha() } else { theme.get_glass_alpha() };
        if (alpha - current).abs() >= STEP || alpha == floor {
            tracing::debug!(?mode, alpha, "glass tint");
            if mode == AppearanceMode::SmokedGlass {
                theme.set_smoked_alpha(alpha);
            } else {
                theme.set_glass_alpha(alpha);
            }
        }
    }
}

impl Drop for GlassSampler {
    fn drop(&mut self) {
        if let Some(hook) = self.hook.take() {
            let _ = unsafe { UnhookWinEvent(hook) };
        }
    }
}
