//! Liquid Glass's tint follows what is behind the island: clear over dark content and
//! only as dense as legible text requires over bright content.
//!
//! The backdrop is sampled when it may have changed: when the island appears, opens,
//! closes or moves, shortly after another window comes to the front, finishes moving,
//! scrolling or being restored, and every few seconds while the island is open. Nothing
//! is sampled for Midnight, Smoked Glass or frosted glass.

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
use crate::appearance::{GlassText, Layer, legible_alpha};
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
    /// Whether the island's tint follows the backdrop now.
    fn glass_follows_backdrop(&self) -> bool {
        let theme = self.island.global::<Theme>();
        theme.get_appearance() == Appearance::LiquidGlass && theme.get_live_blur() && self.island_visible()
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
        // Once the island has finished changing shape.
        let duration = app.island.global::<Theme>().get_resize_duration();
        App::sample_glass_soon(app, Duration::from_millis(duration.max(0) as u64) + Duration::from_millis(40));
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

    /// Sets the tint for the brightest colour behind the island.
    fn sample_glass(&self) {
        if !self.glass_follows_backdrop() {
            return;
        }
        let Some(area) = window::find(self.island.window()).and_then(|native| native.glass_area()) else { return };
        let Some(backdrop) = window::sample::brightest_behind(area) else { return };
        let theme = self.island.global::<Theme>();
        let tokens = theme.get_liquid_glass();
        let tile = tokens.tile;
        let glass = GlassText {
            tint: colour(tokens.blurred_surface),
            tile: Layer { rgb: colour(tile), alpha: tile.alpha() as f32 / 255.0 },
            text: colour(tokens.text),
            secondary: colour(tokens.text_secondary),
            tertiary: colour(tokens.text_tertiary),
        };
        let alpha = legible_alpha(&glass, backdrop);
        if (alpha - theme.get_glass_alpha()).abs() >= STEP {
            tracing::debug!(?backdrop, alpha, "glass tint follows the backdrop");
            theme.set_glass_alpha(alpha);
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
