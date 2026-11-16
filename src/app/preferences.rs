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
use crate::settings::{self, AppearanceMode, ClipboardRetention, LoadStatus, MotionPreference, Settings};
use crate::ui::{AppActions, AppState, Appearance, SettingsData, SettingsWindow, Theme};
use crate::window::{self, WindowKind, placement};
use crate::{dialogs, paths, startup};

/// The Clipboard page of the settings window.
pub(super) const CLIPBOARD_SECTION: i32 = 3;

/// Edits are written once they have stopped for this long.
const SAVE_DELAY: Duration = Duration::from_millis(600);

/// Lilt's two global shortcuts. The index matches `recording-target` in the settings
/// window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShortcutKind {
    Island = 0,
    Clipboard = 1,
}

impl ShortcutKind {
    fn from_index(index: i32) -> Self {
        if index == 1 { Self::Clipboard } else { Self::Island }
    }

    fn id(self) -> i32 {
        match self {
            Self::Island => hotkey::TOGGLE_ID,
            Self::Clipboard => hotkey::CLIPBOARD_ID,
        }
    }

    fn get(self, settings: &Settings) -> Option<String> {
        match self {
            Self::Island => settings.shortcuts.toggle.clone(),
            Self::Clipboard => settings.clipboard.shortcut.clone(),
        }
    }

    fn set(self, settings: &mut Settings, value: Option<String>) {
        match self {
            Self::Island => settings.shortcuts.toggle = value,
            Self::Clipboard => settings.clipboard.shortcut = value,
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Island => Self::Clipboard,
            Self::Clipboard => Self::Island,
        }
    }
}

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
        state.set_show_tasks(settings.widgets.tasks);
        app.tasks.set_order(super::tasks::sort_mode(settings.tasks.order));
        state.set_tasks_layout(match settings.tasks.layout {
            settings::TaskLayout::Beside => crate::ui::TasksLayout::Beside,
            settings::TaskLayout::Below => crate::ui::TasksLayout::Below,
        });
        state.set_show_artwork(settings.music.show_artwork);
        state.set_show_progress(settings.music.show_progress);
        state.set_hover_controls(settings.music.hover_controls);

        let theme = app.island.global::<Theme>();
        let ambient = settings.ambient.enabled && !settings.performance.low_power;
        theme.set_ambient(if ambient { settings.ambient.intensity as f32 / 100.0 } else { 0.0 });
        theme.set_motion(if reduced_motion(settings.appearance.motion) { 0.0 } else { 1.0 });
        App::update_progress_timer(app);
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

        App::apply_clipboard(app);
        app.register_shortcuts();
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
        let live = window::find(app.island.window()).is_some_and(|native| native.set_glass(wanted));
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
        app.apply_glass_tint();
        App::update_glass_sampling(app);
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
        // The appearance cards preview the chosen transparency.
        let transparency = self.settings.borrow().appearance.glass_transparency;
        theme.set_smoked_alpha(appearance::preferred_alpha(AppearanceMode::SmokedGlass, transparency));
        theme.set_glass_alpha(appearance::preferred_alpha(AppearanceMode::LiquidGlass, transparency));
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

    /// Registers both shortcuts and remembers why one could not be.
    pub(super) fn register_shortcuts(&self) {
        for kind in [ShortcutKind::Island, ShortcutKind::Clipboard] {
            let problem = self.register_shortcut(kind);
            self.shortcut_problem.borrow_mut()[kind as usize] = problem;
        }
    }

    /// Registers one configured shortcut. Returns an explanation if it could not be.
    fn register_shortcut(&self, kind: ShortcutKind) -> Option<String> {
        let control = window::control_window();
        hotkey::unregister(control, kind.id());
        // The shelf's shortcut is only needed while the shelf is on.
        if kind == ShortcutKind::Clipboard && !self.settings.borrow().clipboard.show {
            return None;
        }
        let text = kind.get(&self.settings.borrow())?;
        let Some(shortcut) = Shortcut::parse(&text) else {
            return Some(format!("{text} is not a supported shortcut."));
        };
        if kind == ShortcutKind::Clipboard
            && kind.other().get(&self.settings.borrow()).as_deref() == Some(text.as_str())
        {
            return Some(format!("{shortcut} already expands the island."));
        }
        match shortcut.register(control, kind.id()) {
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

    /// Opens the settings window on one of its sections.
    pub(super) fn open_settings_at(app: &Rc<Self>, section: i32) {
        App::open_settings(app);
        if let Some(window) = app.settings_window.borrow().as_ref() {
            window.set_section(section);
        }
    }

    pub(super) fn open_settings(app: &Rc<Self>) {
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
        App::connect_settings_updates(app, &settings_window);
        app.settings_window.replace(Some(settings_window));
        app.apply_window_appearance();
        app.refresh_settings_window();
        if let Some(window) = app.settings_window.borrow().as_ref() {
            let _ = window.show();
        }
        if app.settings.borrow().clipboard.show {
            // Fills in the storage line on the Clipboard page.
            App::refresh_clipboard_usage(app);
        }
    }

    /// Fills the settings window from the current settings.
    pub(super) fn refresh_settings_window(&self) {
        let window = self.settings_window.borrow();
        let Some(window) = window.as_ref() else { return };
        let monitors = placement::monitors();
        let mut labels: Vec<SharedString> = vec!["Primary display".into()];
        labels.extend(monitors.iter().enumerate().map(|(i, m)| m.label(i).into()));
        window.set_monitors(ModelRc::new(VecModel::from(labels)));
        window.set_data(to_data(&self.settings.borrow(), &monitors));
        let excluded: Vec<SharedString> =
            self.settings.borrow().clipboard.excluded_apps.iter().map(|a| a.as_str().into()).collect();
        window.set_clipboard_excluded(ModelRc::new(VecModel::from(excluded)));
        if !window.get_recording_shortcut() {
            let problems = self.shortcut_problem.borrow();
            window.set_shortcut_message(problems[0].clone().unwrap_or_default().into());
            window.set_clipboard_shortcut_message(problems[1].clone().unwrap_or_default().into());
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
        window.on_reset_appearance(super::with(&weak, |app| {
            {
                let mut settings = app.settings.borrow_mut();
                settings.appearance = Default::default();
                settings.position = Default::default();
            }
            App::apply_settings(app);
            app.place();
            App::schedule_save(app);
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
        let weak_record = weak.clone();
        window.on_record_shortcut(move |target| {
            let Some(app) = weak_record.upgrade() else { return };
            // While recording, the old shortcut must not fire.
            hotkey::unregister(window::control_window(), ShortcutKind::from_index(target).id());
            if let Some(window) = app.settings_window.borrow().as_ref() {
                window.set_recording_target(target);
                window.set_recording_shortcut(true);
                window.set_shortcut_message("".into());
                window.set_clipboard_shortcut_message("".into());
            }
        });
        let weak_key = weak.clone();
        window.on_shortcut_key(move |text, ctrl, alt, shift, win| {
            let Some(app) = weak_key.upgrade() else { return };
            App::record_shortcut_key(&app, &text, ctrl, alt, shift, win);
        });
        let weak_clear = weak.clone();
        window.on_clear_shortcut(move |target| {
            let Some(app) = weak_clear.upgrade() else { return };
            ShortcutKind::from_index(target).set(&mut app.settings.borrow_mut(), None);
            app.register_shortcuts();
            App::schedule_save(&app);
            app.refresh_settings_window();
        });
        let weak_add = weak.clone();
        window.on_add_excluded_app(move |name| {
            let Some(app) = weak_add.upgrade() else { return };
            let name = name.trim().trim_matches('"').to_owned();
            if name.is_empty() {
                return;
            }
            app.settings.borrow_mut().clipboard.excluded_apps.push(name);
            let sanitized = app.settings.borrow().clone().sanitize();
            app.settings.replace(sanitized);
            App::apply_settings(&app);
            App::schedule_save(&app);
            app.refresh_settings_window();
        });
        let weak_remove = weak.clone();
        window.on_remove_excluded_app(move |index| {
            let Some(app) = weak_remove.upgrade() else { return };
            {
                let mut settings = app.settings.borrow_mut();
                let apps = &mut settings.clipboard.excluded_apps;
                if let Some(i) = usize::try_from(index).ok().filter(|&i| i < apps.len()) {
                    apps.remove(i);
                }
            }
            App::apply_settings(&app);
            App::schedule_save(&app);
            app.refresh_settings_window();
        });
        let weak_clear_clipboard = weak.clone();
        window.on_clear_clipboard(move |everything| {
            let Some(app) = weak_clear_clipboard.upgrade() else { return };
            App::clear_clipboard(&app, everything);
        });
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
        let kind = app
            .settings_window
            .borrow()
            .as_ref()
            .map_or(ShortcutKind::Island, |w| ShortcutKind::from_index(w.get_recording_target()));
        let message = if is(Key::Escape) {
            app.register_shortcuts();
            String::new()
        } else if let Some(shortcut) = hotkey::from_key_event(text, ctrl, alt, shift, win) {
            let previous = kind.get(&app.settings.borrow());
            kind.set(&mut app.settings.borrow_mut(), Some(shortcut.to_string()));
            match app.register_shortcut(kind) {
                None => {
                    app.register_shortcuts();
                    App::schedule_save(app);
                    match shortcut.known_conflict() {
                        Some(apps) => {
                            format!("{shortcut} is set. Note: {apps}; they no longer receive it while Lilt runs.")
                        }
                        None => String::new(),
                    }
                }
                Some(problem) => {
                    kind.set(&mut app.settings.borrow_mut(), previous);
                    app.register_shortcuts();
                    problem
                }
            }
        } else {
            app.register_shortcuts();
            "Use Ctrl, Alt or Win together with a letter, a number, Space or F1 to F12. Win+V stays with Windows."
                .into()
        };
        if let Some(window) = app.settings_window.borrow().as_ref() {
            window.set_recording_shortcut(false);
        }
        app.refresh_settings_window();
        if let Some(window) = app.settings_window.borrow().as_ref() {
            match kind {
                ShortcutKind::Island => window.set_shortcut_message(message.into()),
                ShortcutKind::Clipboard => window.set_clipboard_shortcut_message(message.into()),
            }
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
        self.register_shortcuts();
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
        AppearanceMode::SmokedGlass => Appearance::SmokedGlass,
        AppearanceMode::LiquidGlass => Appearance::LiquidGlass,
    }
}

/// The appearance's card in the settings window, in the order shown.
fn appearance_index(mode: AppearanceMode) -> i32 {
    match mode {
        AppearanceMode::Midnight => 0,
        AppearanceMode::SmokedGlass => 1,
        AppearanceMode::LiquidGlass => 2,
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
        glass_transparency: s.appearance.glass_transparency as i32,
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
        show_tasks: s.widgets.tasks,
        tasks_below: s.tasks.layout == settings::TaskLayout::Below,
        show_artwork: s.music.show_artwork,
        show_progress: s.music.show_progress,
        hover_controls: s.music.hover_controls,
        ambient_enabled: s.ambient.enabled,
        ambient_intensity: s.ambient.intensity as i32,
        low_power: s.performance.low_power,
        hide_in_fullscreen: s.performance.hide_in_fullscreen,
        launch_at_startup: startup::is_enabled(),
        shortcut: s.shortcuts.toggle.clone().unwrap_or_default().into(),
        calendar_path: s.calendar.ics_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default().into(),
        show_clipboard: s.clipboard.show,
        clipboard_capture: s.clipboard.capture,
        clipboard_skip_secrets: s.clipboard.skip_secrets,
        clipboard_skip_unknown: s.clipboard.skip_unknown_sources,
        clipboard_retention: ClipboardRetention::ALL.iter().position(|r| *r == s.clipboard.retention).unwrap_or(2)
            as i32,
        clipboard_max_items: s.clipboard.max_items as i32,
        clipboard_shortcut: s.clipboard.shortcut.clone().unwrap_or_default().into(),
    }
}

/// Settings edited in the window. The shortcut and calendar file are changed through
/// their own actions and carried over from `previous`.
fn from_data(d: &SettingsData, monitors: &[placement::Monitor], previous: &Settings) -> Settings {
    let mut s = previous.clone();
    s.appearance.theme = match d.appearance {
        1 => AppearanceMode::SmokedGlass,
        2 => AppearanceMode::LiquidGlass,
        _ => AppearanceMode::Midnight,
    };
    s.appearance.reduce_transparency = d.reduce_transparency;
    s.appearance.glass_transparency = d.glass_transparency.clamp(0, 100) as u32;
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
    s.widgets.tasks = d.show_tasks;
    s.tasks.layout = if d.tasks_below { settings::TaskLayout::Below } else { settings::TaskLayout::Beside };
    s.music.show_artwork = d.show_artwork;
    s.music.show_progress = d.show_progress;
    s.music.hover_controls = d.hover_controls;
    s.ambient.enabled = d.ambient_enabled;
    s.ambient.intensity = d.ambient_intensity.max(0) as u32;
    s.performance.low_power = d.low_power;
    s.performance.hide_in_fullscreen = d.hide_in_fullscreen;
    s.clipboard.show = d.show_clipboard;
    if d.clipboard_capture != s.clipboard.capture {
        s.clipboard.capture = d.clipboard_capture;
        // Turning capture on or off here answers the shelf's first-use question.
        s.clipboard.introduced = true;
    }
    s.clipboard.skip_secrets = d.clipboard_skip_secrets;
    s.clipboard.skip_unknown_sources = d.clipboard_skip_unknown;
    s.clipboard.retention = usize::try_from(d.clipboard_retention)
        .ok()
        .and_then(|i| ClipboardRetention::ALL.get(i))
        .copied()
        .unwrap_or_default();
    // In steps of 50, as the slider moves.
    s.clipboard.max_items = ((d.clipboard_max_items.max(0) as u32 + 25) / 50) * 50;
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
        s.appearance.glass_transparency = 80;
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
        assert_eq!(from_data(&data, &[], &Settings::default()).appearance.theme, AppearanceMode::SmokedGlass);
        data.appearance = 2;
        assert_eq!(from_data(&data, &[], &Settings::default()).appearance.theme, AppearanceMode::LiquidGlass);
        for mode in [AppearanceMode::Midnight, AppearanceMode::SmokedGlass, AppearanceMode::LiquidGlass] {
            let mut s = Settings::default();
            s.appearance.theme = mode;
            assert_eq!(from_data(&to_data(&s, &[]), &[], &s).appearance.theme, mode);
        }
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
