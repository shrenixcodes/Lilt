//! Settings: loading, applying, saving, the settings window and the global shortcut.

use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SW_SHOWNORMAL, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{BOOL, HSTRING, PCWSTR, w};

use super::App;
use crate::appearance::{self, Fallback, Material};
use crate::hotkey::{self, Shortcut};
use crate::settings::{self, AppearanceMode, LoadStatus, MotionPreference, Settings};
use crate::ui::{AppActions, AppState, Appearance, SettingsData, SettingsWindow, Theme};
use crate::window::{self, WindowKind, placement};
use crate::{dialogs, paths, startup};

/// Edits are written once they have stopped for this long.
const SAVE_DELAY: Duration = Duration::from_millis(600);

impl App {
    pub(super) fn load_settings() -> Settings {
        let (settings, status) = settings::load_from(&settings::settings_path());
        match status {
            LoadStatus::Loaded | LoadStatus::Missing => {}
            LoadStatus::Invalid => tracing::warn!("settings file was invalid; defaults are used and it was backed up"),
            LoadStatus::Newer(version) => {
                tracing::warn!(version, "settings file is from a newer version; it was backed up")
            }
        }
        settings
    }

    pub(super) fn connect_preferences(app: &Rc<Self>) {
        let weak = Rc::downgrade(app);
        app.tray.on_open_settings(super::with(&weak, App::open_settings));
        app.island.global::<AppActions>().on_open_settings(super::with(&weak, App::open_settings));
    }

    /// Applies every setting to the running application.
    pub(super) fn apply_settings(app: &Rc<Self>) {
        let settings = app.settings.borrow().clone();
        let state = app.island.global::<AppState>();
        state.set_compact_width(settings.appearance.compact_width as f32);
        state.set_expanded_width(settings.appearance.expanded_width as f32);
        state.set_allow_drag(settings.position.allow_drag);
        state.set_show_media(settings.widgets.media);
        state.set_show_calendar(settings.widgets.calendar);
        state.set_show_system(settings.widgets.system);
        state.set_show_actions(settings.widgets.quick_actions);
        state.set_show_artwork(settings.music.show_artwork);
        state.set_show_progress(settings.music.show_progress);

        let theme = app.island.global::<Theme>();
        let ambient = settings.ambient.enabled && !settings.performance.low_power;
        theme.set_ambient(if ambient { settings.ambient.intensity as f32 / 100.0 } else { 0.0 });
        theme.set_motion(if reduced_motion(settings.appearance.motion) { 0.0 } else { 1.0 });
        App::apply_appearance(app);

        let path = settings.calendar.ics_path.clone();
        let current = app.calendar.borrow().as_ref().map(|c| c.path().to_path_buf());
        match path {
            Some(path) if current.as_ref() != Some(&path) => App::open_calendar(app, &path),
            None if current.is_some() => {
                app.calendar.replace(None);
                app.refresh_agenda();
            }
            _ => {}
        }

        let problem = app.register_shortcut();
        app.shortcut_problem.replace(problem);
        App::update_visibility(app);
        App::update_sampling(app);
    }

    /// Applies the appearance and chooses its material: solid, frosted or the live blur.
    /// Called when settings change and when Windows' transparency or power state does.
    pub(super) fn apply_appearance(app: &Rc<Self>) {
        let settings = app.settings.borrow().clone();
        let env = appearance::Environment::current();
        let (material, mut fallback) = appearance::material(&settings, &env);
        let wanted = material == Material::LiveBlur;
        let live = window::find(app.island.window()).is_some_and(|native| native.set_live_backdrop(wanted));
        if wanted && !live {
            fallback = Some(Fallback::Failed);
        }
        let theme = app.island.global::<Theme>();
        theme.set_appearance(ui_appearance(settings.appearance.theme));
        theme.set_live_blur(live);
        app.glass_fallback.set(fallback);
        app.window_blur.set(live && appearance::window_backdrop_supported(&env));
        if material != Material::Solid {
            tracing::debug!(?material, live, ?fallback, "appearance applied");
        }
        app.apply_window_appearance();
    }

    /// Gives the settings window the island's appearance, with the system acrylic behind
    /// it where the live blur is in use and Windows supports it for ordinary windows.
    pub(super) fn apply_window_appearance(&self) {
        let window = self.settings_window.borrow();
        let Some(window) = window.as_ref() else { return };
        let island = self.island.global::<Theme>();
        let theme = window.global::<Theme>();
        theme.set_appearance(island.get_appearance());
        theme.set_motion(island.get_motion());
        let blur =
            window::find(window.window()).is_some_and(|native| native.set_system_backdrop(self.window_blur.get()));
        theme.set_live_blur(blur);
        let note = match self.glass_fallback.get() {
            // The switch right below the cards already says so.
            Some(Fallback::UserReduced) | None => "",
            Some(reason) => reason.explanation(),
        };
        window.set_appearance_note(note.into());
    }

    /// Registers the configured shortcut. Returns an explanation if it could not be.
    fn register_shortcut(&self) -> Option<String> {
        let control = window::control_window();
        hotkey::unregister(control);
        let text = self.settings.borrow().shortcuts.toggle.clone()?;
        let Some(shortcut) = Shortcut::parse(&text) else {
            return Some(format!("{text} is not a supported shortcut."));
        };
        match shortcut.register(control) {
            Ok(()) => None,
            Err(err) => {
                tracing::warn!(%err, shortcut = %shortcut, "shortcut unavailable");
                Some(format!("{shortcut} is already used by another app."))
            }
        }
    }

    /// Persists the settings shortly after the last change.
    pub(super) fn schedule_save(app: &Rc<Self>) {
        let weak = Rc::downgrade(app);
        app.save_timer.start(slint::TimerMode::SingleShot, SAVE_DELAY, move || {
            if let Some(app) = weak.upgrade() {
                app.save_now();
            }
        });
    }

    pub(super) fn save_now(&self) {
        self.save_timer.stop();
        if let Err(err) = settings::save_to(&settings::settings_path(), &self.settings.borrow()) {
            tracing::warn!(%err, "settings could not be saved");
        }
    }

    fn open_settings(app: &Rc<Self>) {
        App::set_expanded(app, false);
        if let Some(existing) = app.settings_window.borrow().as_ref() {
            let _ = existing.show();
            if let Some(native) = window::find(existing.window()) {
                native.activate();
            }
            return;
        }
        window::create_next_as(WindowKind::Standard);
        let settings_window = match SettingsWindow::new() {
            Ok(window) => window,
            Err(err) => {
                tracing::error!(%err, "could not open the settings window");
                return;
            }
        };
        settings_window.set_version(env!("CARGO_PKG_VERSION").into());
        settings_window.set_data_folder(paths::roaming_data_dir().display().to_string().into());
        App::connect_settings_window(app, &settings_window);
        app.settings_window.replace(Some(settings_window));
        app.apply_window_appearance();
        app.refresh_settings_window();
        if let Some(window) = app.settings_window.borrow().as_ref() {
            let _ = window.show();
        }
    }

    /// Fills the settings window from the current settings.
    fn refresh_settings_window(&self) {
        let window = self.settings_window.borrow();
        let Some(window) = window.as_ref() else { return };
        let monitors = placement::monitors();
        let mut labels: Vec<SharedString> = vec!["Primary display".into()];
        labels.extend(monitors.iter().enumerate().map(|(i, m)| m.label(i).into()));
        window.set_monitors(ModelRc::new(VecModel::from(labels)));
        window.set_data(to_data(&self.settings.borrow(), &monitors));
        if !window.get_recording_shortcut() {
            window.set_shortcut_message(self.shortcut_problem.borrow().clone().unwrap_or_default().into());
        }
    }

    fn connect_settings_window(app: &Rc<Self>, window: &SettingsWindow) {
        let weak = Rc::downgrade(app);
        window.on_edited(super::with(&weak, |app| {
            let Some(data) = app.settings_window.borrow().as_ref().map(|w| w.get_data()) else { return };
            let monitors = placement::monitors();
            let previous = app.settings.borrow().clone();
            let updated = from_data(&data, &monitors, &previous);
            if data.launch_at_startup != startup::is_enabled()
                && let Err(err) = startup::set_enabled(data.launch_at_startup)
            {
                tracing::warn!(%err, "could not change the startup setting");
            }
            app.settings.replace(updated);
            App::apply_settings(app);
            App::schedule_save(app);
        }));
        window.on_reset(super::with(&weak, |app| {
            app.settings.replace(Settings::default());
            App::apply_settings(app);
            app.save_now();
            app.refresh_settings_window();
        }));
        window.on_reset_position(super::with(&weak, |app| {
            app.settings.borrow_mut().position.offset_x = 0;
            app.place();
            App::schedule_save(app);
            app.refresh_settings_window();
        }));
        window.on_choose_calendar(super::with(&weak, |app| {
            let weak = Rc::downgrade(app);
            // The file dialog runs a modal loop, which must not start inside UI dispatch.
            slint::Timer::single_shot(Duration::ZERO, move || {
                let Some(app) = weak.upgrade() else { return };
                if let Some(path) = dialogs::pick_calendar_file() {
                    app.settings.borrow_mut().calendar.ics_path = Some(path);
                    App::apply_settings(&app);
                    app.save_now();
                    app.refresh_settings_window();
                }
            });
        }));
        window.on_clear_calendar(super::with(&weak, |app| {
            app.settings.borrow_mut().calendar.ics_path = None;
            App::apply_settings(app);
            App::schedule_save(app);
            app.refresh_settings_window();
        }));
        window.on_record_shortcut(super::with(&weak, |app| {
            // While recording, the old shortcut must not fire.
            hotkey::unregister(window::control_window());
            if let Some(window) = app.settings_window.borrow().as_ref() {
                window.set_recording_shortcut(true);
                window.set_shortcut_message("".into());
            }
        }));
        let weak_key = weak.clone();
        window.on_shortcut_key(move |text, ctrl, alt, shift, win| {
            let Some(app) = weak_key.upgrade() else { return };
            App::record_shortcut_key(&app, &text, ctrl, alt, shift, win);
        });
        window.on_clear_shortcut(super::with(&weak, |app| {
            app.settings.borrow_mut().shortcuts.toggle = None;
            app.shortcut_problem.replace(None);
            hotkey::unregister(window::control_window());
            App::schedule_save(app);
            app.refresh_settings_window();
        }));
        window.on_open_data_folder(|| {
            let folder = HSTRING::from(paths::roaming_data_dir().as_os_str());
            let _ = std::fs::create_dir_all(paths::roaming_data_dir());
            unsafe {
                ShellExecuteW(None, w!("open"), &folder, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
            }
        });
        window.window().on_close_requested(super::with_close(&weak));
    }

    fn record_shortcut_key(app: &Rc<Self>, text: &str, ctrl: bool, alt: bool, shift: bool, win: bool) {
        use slint::platform::Key;
        let is = |key: Key| SharedString::from(key).as_str() == text;
        // Modifier keys on their own: keep listening for the rest of the combination.
        if [Key::Control, Key::Alt, Key::Shift, Key::Meta, Key::ControlR, Key::AltGr, Key::ShiftR, Key::MetaR]
            .into_iter()
            .any(is)
        {
            return;
        }
        let message = if is(Key::Escape) {
            app.register_shortcut();
            String::new()
        } else if let Some(shortcut) = hotkey::from_key_event(text, ctrl, alt, shift, win) {
            let previous = app.settings.borrow().shortcuts.toggle.clone();
            app.settings.borrow_mut().shortcuts.toggle = Some(shortcut.to_string());
            match app.register_shortcut() {
                None => {
                    app.shortcut_problem.replace(None);
                    App::schedule_save(app);
                    String::new()
                }
                Some(problem) => {
                    app.settings.borrow_mut().shortcuts.toggle = previous;
                    app.register_shortcut();
                    problem
                }
            }
        } else {
            app.register_shortcut();
            "Use Ctrl, Alt or Win together with a letter, a number, Space or F1 to F12.".into()
        };
        if let Some(window) = app.settings_window.borrow().as_ref() {
            window.set_recording_shortcut(false);
        }
        app.refresh_settings_window();
        if let Some(window) = app.settings_window.borrow().as_ref() {
            window.set_shortcut_message(message.into());
        }
    }

    /// Releases the settings window so it uses no memory while closed.
    pub(super) fn close_settings(&self) {
        if let Some(window) = self.settings_window.take() {
            let _ = window.hide();
            // Dropped on the next turn of the event loop, outside the window's own
            // event handling.
            slint::Timer::single_shot(Duration::ZERO, move || drop(window));
        }
        self.register_shortcut();
    }
}

/// Whether animations should be reduced, honouring "Animation effects" in Windows.
pub(super) fn reduced_motion(preference: MotionPreference) -> bool {
    match preference {
        MotionPreference::Reduced => true,
        MotionPreference::Full => false,
        MotionPreference::System => {
            let mut enabled = BOOL(1);
            let ok = unsafe {
                SystemParametersInfoW(
                    SPI_GETCLIENTAREAANIMATION,
                    0,
                    Some(&mut enabled as *mut BOOL as *mut _),
                    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                )
            };
            ok.is_ok() && !enabled.as_bool()
        }
    }
}

fn ui_appearance(mode: AppearanceMode) -> Appearance {
    match mode {
        AppearanceMode::Midnight => Appearance::Midnight,
        AppearanceMode::LiquidGlass => Appearance::LiquidGlass,
    }
}

fn appearance_index(mode: AppearanceMode) -> i32 {
    match mode {
        AppearanceMode::Midnight => 0,
        AppearanceMode::LiquidGlass => 1,
    }
}

fn motion_index(preference: MotionPreference) -> i32 {
    match preference {
        MotionPreference::System => 0,
        MotionPreference::Reduced => 1,
        MotionPreference::Full => 2,
    }
}

fn to_data(s: &Settings, monitors: &[placement::Monitor]) -> SettingsData {
    let monitor = s
        .position
        .monitor
        .as_ref()
        .and_then(|device| monitors.iter().position(|m| &m.device == device))
        .map_or(0, |i| i as i32 + 1);
    SettingsData {
        appearance: appearance_index(s.appearance.theme),
        reduce_transparency: s.appearance.reduce_transparency,
        auto_collapse: s.general.auto_collapse,
        auto_collapse_seconds: s.general.auto_collapse_seconds as i32,
        collapse_on_outside_click: s.general.collapse_on_outside_click,
        activity_banners: s.general.activity_banners,
        compact_width: s.appearance.compact_width as i32,
        expanded_width: s.appearance.expanded_width as i32,
        motion: motion_index(s.appearance.motion),
        monitor,
        top_margin: s.position.top_margin as i32,
        offset_x: s.position.offset_x,
        allow_drag: s.position.allow_drag,
        show_media: s.widgets.media,
        show_calendar: s.widgets.calendar,
        show_system: s.widgets.system,
        show_actions: s.widgets.quick_actions,
        show_artwork: s.music.show_artwork,
        show_progress: s.music.show_progress,
        ambient_enabled: s.ambient.enabled,
        ambient_intensity: s.ambient.intensity as i32,
        low_power: s.performance.low_power,
        hide_in_fullscreen: s.performance.hide_in_fullscreen,
        launch_at_startup: startup::is_enabled(),
        shortcut: s.shortcuts.toggle.clone().unwrap_or_default().into(),
        calendar_path: s.calendar.ics_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default().into(),
    }
}

/// Settings edited in the window. The shortcut and calendar file are changed through
/// their own actions and carried over from `previous`.
fn from_data(d: &SettingsData, monitors: &[placement::Monitor], previous: &Settings) -> Settings {
    let mut s = previous.clone();
    s.appearance.theme = match d.appearance {
        1 => AppearanceMode::LiquidGlass,
        _ => AppearanceMode::Midnight,
    };
    s.appearance.reduce_transparency = d.reduce_transparency;
    s.general.auto_collapse = d.auto_collapse;
    s.general.auto_collapse_seconds = d.auto_collapse_seconds.max(0) as u32;
    s.general.collapse_on_outside_click = d.collapse_on_outside_click;
    s.general.activity_banners = d.activity_banners;
    s.appearance.compact_width = d.compact_width.max(0) as u32;
    s.appearance.expanded_width = d.expanded_width.max(0) as u32;
    s.appearance.motion = match d.motion {
        1 => MotionPreference::Reduced,
        2 => MotionPreference::Full,
        _ => MotionPreference::System,
    };
    s.position.monitor = usize::try_from(d.monitor - 1).ok().and_then(|i| monitors.get(i)).map(|m| m.device.clone());
    s.position.top_margin = d.top_margin.max(0) as u32;
    s.position.allow_drag = d.allow_drag;
    s.widgets.media = d.show_media;
    s.widgets.calendar = d.show_calendar;
    s.widgets.system = d.show_system;
    s.widgets.quick_actions = d.show_actions;
    s.music.show_artwork = d.show_artwork;
    s.music.show_progress = d.show_progress;
    s.ambient.enabled = d.ambient_enabled;
    s.ambient.intensity = d.ambient_intensity.max(0) as u32;
    s.performance.low_power = d.low_power;
    s.performance.hide_in_fullscreen = d.hide_in_fullscreen;
    s.sanitize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_the_window_model() {
        let monitors = vec![placement::Monitor {
            device: r"\\.\DISPLAY2".into(),
            bounds: Default::default(),
            work: Default::default(),
            scale: 1.0,
            primary: false,
        }];
        let mut s = Settings::default();
        s.position.monitor = Some(r"\\.\DISPLAY2".into());
        s.ambient.intensity = 35;
        s.appearance.motion = MotionPreference::Reduced;
        s.appearance.theme = AppearanceMode::LiquidGlass;
        s.appearance.reduce_transparency = true;
        s.widgets.system = false;
        s.shortcuts.toggle = Some("Ctrl+Alt+K".into());
        let data = to_data(&s, &monitors);
        assert_eq!(data.monitor, 1);
        assert_eq!(from_data(&data, &monitors, &s), s);
    }

    #[test]
    fn appearance_choice_maps_to_the_settings() {
        let mut data = to_data(&Settings::default(), &[]);
        assert_eq!(data.appearance, 0);
        data.appearance = 1;
        assert_eq!(from_data(&data, &[], &Settings::default()).appearance.theme, AppearanceMode::LiquidGlass);
        // An index this version does not know is treated as Midnight.
        data.appearance = 7;
        assert_eq!(from_data(&data, &[], &Settings::default()).appearance.theme, AppearanceMode::Midnight);
    }

    #[test]
    fn missing_monitor_falls_back_to_primary() {
        let mut s = Settings::default();
        s.position.monitor = Some(r"\\.\DISPLAY9".into());
        assert_eq!(to_data(&s, &[]).monitor, 0);
    }

    #[test]
    fn window_values_are_clamped() {
        let mut data = to_data(&Settings::default(), &[]);
        data.expanded_width = 10_000;
        data.ambient_intensity = -5;
        let s = from_data(&data, &[], &Settings::default());
        assert_eq!(s.appearance.expanded_width, settings::limits::EXPANDED_WIDTH.1);
        assert_eq!(s.ambient.intensity, 0);
    }
}
