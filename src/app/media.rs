//! Connects the media service to the island.

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::ComponentHandle;

use super::App;
use crate::ambient::{AmbientPalette, Rgb};
use crate::media::{Command, MediaState, MediaUpdate, PlaybackStatus};
use crate::ui::{AppActions, AppState, MediaAvailability, MediaInfo, Theme};

/// How long the track-change banner stays in the compact island.
const ACTIVITY_DURATION: Duration = Duration::from_millis(3500);
/// Progress refresh interval while the timeline is visible and playing.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);

impl App {
    pub(super) fn connect_media(app: &Rc<Self>) {
        let actions = app.island.global::<AppActions>();
        actions.on_toggle_play(super::with(&Rc::downgrade(app), |app| app.send_media(Command::TogglePlayPause)));
        actions.on_next_track(super::with(&Rc::downgrade(app), |app| app.send_media(Command::Next)));
        actions.on_previous_track(super::with(&Rc::downgrade(app), |app| app.send_media(Command::Previous)));
        actions.on_cycle_source(super::with(&Rc::downgrade(app), |app| app.send_media(Command::CycleSession)));
        let weak = Rc::downgrade(app);
        actions.on_seek(move |fraction| {
            let Some(app) = weak.upgrade() else { return };
            let duration = app.media_state.borrow().as_ref().and_then(|m| m.timeline).map(|t| t.duration);
            if let Some(duration) = duration {
                app.send_media(Command::Seek(duration.mul_f32(fraction.clamp(0.0, 1.0))));
            }
        });
    }

    fn send_media(&self, command: Command) {
        if let Some(media) = self.media.borrow().as_ref() {
            media.send(command);
        }
    }

    pub(super) fn media_update(app: &Rc<Self>, update: MediaUpdate) {
        let state = app.island.global::<AppState>();
        match update {
            MediaUpdate::Unavailable | MediaUpdate::Idle => {
                let availability = if matches!(update, MediaUpdate::Unavailable) {
                    MediaAvailability::Unavailable
                } else {
                    MediaAvailability::Idle
                };
                app.media_state.replace(None);
                app.last_track.replace(None);
                app.activity_timer.stop();
                state.set_activity(false);
                state.set_media(MediaInfo { availability, ..Default::default() });
                state.set_has_artwork(false);
                state.set_artwork(slint::Image::default());
                app.apply_palette(&AmbientPalette::NEUTRAL);
            }
            MediaUpdate::Artwork(Some(artwork)) => {
                tracing::debug!(palette = ?artwork.palette, "artwork received");
                app.apply_palette(&artwork.palette);
                let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                    &artwork.pixels,
                    artwork.width,
                    artwork.height,
                );
                state.set_artwork(slint::Image::from_rgba8(buffer));
                state.set_has_artwork(true);
            }
            MediaUpdate::Artwork(None) => {
                state.set_has_artwork(false);
                state.set_artwork(slint::Image::default());
                app.apply_palette(&AmbientPalette::NEUTRAL);
            }
            MediaUpdate::State(media) => {
                let track = (media.source_id.clone(), media.title.clone(), media.artist.clone());
                let previous = app.last_track.replace(Some(track.clone()));
                // Announce new tracks, but not the one already playing when Lilt starts.
                let announce = app.media_ready.get() && previous.as_ref() != Some(&track) && !media.title.is_empty();
                state.set_media(info(&media));
                app.media_state.replace(Some(media));
                if announce {
                    App::show_activity(app);
                }
            }
        }
        app.media_ready.set(true);
        App::update_progress_timer(app);
    }

    /// Hands the artwork palette to the UI, which cross-fades to it.
    fn apply_palette(&self, palette: &AmbientPalette) {
        let colour = |c: Rgb| slint::Color::from_rgb_u8(c.r, c.g, c.b);
        let theme = self.island.global::<Theme>();
        theme.set_accent(colour(palette.accent));
        theme.set_wash(colour(palette.wash));
        theme.set_glow(colour(palette.glow));
    }

    fn show_activity(app: &Rc<Self>) {
        if app.expanded() || !app.settings.borrow().general.activity_banners {
            return;
        }
        let state = app.island.global::<AppState>();
        state.set_activity(true);
        let weak = app.island.as_weak();
        app.activity_timer.start(slint::TimerMode::SingleShot, ACTIVITY_DURATION, move || {
            if let Some(island) = weak.upgrade() {
                island.global::<AppState>().set_activity(false);
            }
        });
    }

    /// Steps the "now playing" bars while music plays and the compact island is on
    /// screen with motion allowed, and stops them otherwise. Called on every change of
    /// playback, visibility, expansion or motion, and once a minute.
    pub(super) fn update_bars(app: &Rc<Self>) {
        let state = app.island.global::<AppState>();
        let moving = bars_move(
            state.get_media().playing,
            app.island_visible(),
            app.expanded(),
            app.island.global::<Theme>().get_motion() > 0.0,
        );
        if !moving {
            app.bars_timer.stop();
        } else if !app.bars_timer.running() {
            let weak = app.island.as_weak();
            app.bars_timer.start(slint::TimerMode::Repeated, BARS_INTERVAL, move || {
                if let Some(island) = weak.upgrade() {
                    let state = island.global::<AppState>();
                    // Every wave repeats after 360 steps, so wrapping at 7200 is seamless.
                    state.set_bars_step((state.get_bars_step() + 1) % 7200);
                }
            });
        }
    }

    /// Runs the progress timer only while the position is actually on screen and moving.
    pub(super) fn update_progress_timer(app: &Rc<Self>) {
        App::update_bars(app);
        app.refresh_progress();
        let moving = app
            .media_state
            .borrow()
            .as_ref()
            .is_some_and(|m| m.status == PlaybackStatus::Playing && m.timeline.is_some());
        let visible = app.island_visible() && {
            let settings = app.settings.borrow();
            settings.widgets.media && settings.music.show_progress
        };
        if visible && moving {
            if !app.progress_timer.running() {
                let weak = Rc::downgrade(app);
                app.progress_timer.start(slint::TimerMode::Repeated, PROGRESS_INTERVAL, move || {
                    if let Some(app) = weak.upgrade() {
                        app.refresh_progress();
                    }
                });
            }
        } else {
            app.progress_timer.stop();
        }
    }

    fn refresh_progress(&self) {
        let media = self.media_state.borrow();
        let Some(timeline) = media.as_ref().and_then(|m| m.timeline) else { return };
        let position = timeline.position_at(Instant::now());
        let state = self.island.global::<AppState>();
        state.set_progress((position.as_secs_f64() / timeline.duration.as_secs_f64().max(1.0)) as f32);
        state.set_position_text(format_duration(position).into());
        state.set_duration_text(format_duration(timeline.duration).into());
    }
}

fn info(media: &MediaState) -> MediaInfo {
    MediaInfo {
        availability: MediaAvailability::Active,
        playing: media.status == PlaybackStatus::Playing,
        title: media.title.as_str().into(),
        artist: media.artist.as_str().into(),
        source: media.source_name.as_str().into(),
        can_play_pause: media.capabilities.play_pause,
        can_next: media.capabilities.next,
        can_previous: media.capabilities.previous,
        can_seek: media.capabilities.seek && media.timeline.is_some(),
        has_timeline: media.timeline.is_some(),
        session_count: media.session_count as i32,
    }
}

/// About 14 steps a second.
const BARS_INTERVAL: Duration = Duration::from_millis(70);

/// Whether the "now playing" bars move: music plays and the compact island is on screen
/// with motion allowed.
fn bars_move(playing: bool, visible: bool, expanded: bool, motion: bool) -> bool {
    playing && visible && !expanded && motion
}

/// `m:ss`, or `h:mm:ss` from one hour.
fn format_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_move_only_while_playing_compact_visible_and_animated() {
        assert!(bars_move(true, true, false, true));
        assert!(!bars_move(false, true, false, true), "paused");
        assert!(!bars_move(true, false, false, true), "hidden, e.g. behind a full-screen app");
        assert!(!bars_move(true, true, true, true), "expanded: the bars are not shown");
        assert!(!bars_move(true, true, false, false), "reduced motion");
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0:00");
        assert_eq!(format_duration(Duration::from_secs(65)), "1:05");
        assert_eq!(format_duration(Duration::from_secs(3600 + 62)), "1:01:02");
    }
}
