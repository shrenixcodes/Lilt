//! Application controller: owns the island and tray, and keeps the island placed and
//! visible according to the displays and the foreground application.

mod calendar;
pub(crate) mod clipboard;
pub(crate) mod controls;
mod glass;
mod idle;
pub(crate) mod launcher;
mod media;
mod preferences;
mod system;
pub(crate) mod tasks;
mod updates;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use slint::ComponentHandle;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoInitializeSecurity, EOAC_NONE, RPC_C_AUTHN_LEVEL_DEFAULT,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Shell::{
    ABM_NEW, ABM_REMOVE, ABN_FULLSCREENAPP, APPBARDATA, QUNS_BUSY, QUNS_PRESENTATION_MODE,
    QUNS_RUNNING_D3D_FULL_SCREEN, SHAppBarMessage, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, PBT_APMPOWERSTATUSCHANGE, PBT_APMRESUMEAUTOMATIC, SPI_SETCLIENTAREAANIMATION, SPI_SETDESKWALLPAPER,
    SPI_SETWORKAREA, SetForegroundWindow, WM_CLOSE, WM_DISPLAYCHANGE, WM_HOTKEY, WM_POWERBROADCAST, WM_SETTINGCHANGE,
    WM_TIMECHANGE,
};
use windows::core::w;

use crate::actions::QuickAction;
use crate::appearance::Fallback;
use crate::calendar::{CalendarFile, WindowsZones};
use crate::clock;
use crate::hotkey;
use crate::media::{MediaService, MediaState};
use crate::settings::Settings;
use crate::system::{SamplingPolicy, SystemMonitor};
use crate::ui::{AppActions, AppState, Island, SettingsWindow, Theme, Tray};
use crate::window::{self, Notice, WindowKind, placement};

/// Shell notifications for the app bar registration (full-screen detection).
const WM_APP_APPBAR: u32 = window::WM_APP_USER;

/// Cursor poll interval while the island is tucked away for auto-hide.
const PEEK_POLL: Duration = Duration::from_millis(90);
/// Delay before a revealed island tucks itself away again after the pointer leaves it.
const PEEK_HIDE_DELAY: Duration = Duration::from_millis(600);
/// Logical depth, from the very top edge of the screen, of the band that reveals the
/// island. Kept shallow so a tucked island only returns when the cursor hits the top.
const PEEK_BAND_HEIGHT: f32 = 3.0;
/// Logical padding on each side of the compact island within the reveal band.
const PEEK_BAND_PAD: f32 = 40.0;

pub struct App {
    island: Island,
    tray: Tray,
    settings: RefCell<Settings>,
    save_timer: slint::Timer,
    settings_window: RefCell<Option<SettingsWindow>>,
    /// Why the configured shortcut could not be registered, shown in the settings window.
    shortcut_problem: RefCell<[Option<String>; 2]>,
    hidden_by_user: Cell<bool>,
    /// A full-screen application, game or presentation owns the display.
    fullscreen_active: Cell<bool>,
    clock_timer: slint::Timer,
    idle_timer: slint::Timer,
    idle_grace_timer: slint::Timer,
    idle_active: Cell<bool>,
    idle_transition_timer: slint::Timer,
    weather_timer: slint::Timer,
    h24: Cell<bool>,
    collapse_timer: slint::Timer,
    pointer_inside: Cell<bool>,
    /// Auto-hide: whether the island is revealed right now despite being tucked away.
    auto_hide_revealed: Cell<bool>,
    /// Polls the cursor while the island is tucked, to reveal it at the top centre.
    peek_timer: slint::Timer,
    /// Re-tucks the island a short while after the pointer leaves it.
    peek_hide_timer: slint::Timer,
    /// Window that had the focus before the island was expanded, restored on collapse.
    previous_foreground: Cell<Option<HWND>>,
    media: RefCell<Option<MediaService>>,
    media_state: RefCell<Option<MediaState>>,
    /// Source, title and artist of the current track, to detect track changes.
    last_track: RefCell<Option<(String, String, String)>>,
    /// False until the first media update, so the track playing at start-up is not announced.
    media_ready: Cell<bool>,
    activity_timer: slint::Timer,
    progress_timer: slint::Timer,
    /// Steps the "now playing" bars while they should move.
    bars_timer: slint::Timer,
    system_monitor: RefCell<Option<SystemMonitor>>,
    sampling: Cell<SamplingPolicy>,
    cpu_timer: slint::Timer,
    memory_timer: slint::Timer,
    network_timer: slint::Timer,
    calendar: RefCell<Option<CalendarFile>>,
    zones: WindowsZones,
    /// Why Liquid Glass is not using the live blur, if it is not.
    glass_fallback: Cell<Option<Fallback>>,
    /// Whether the settings window should show the system acrylic.
    window_blur: Cell<bool>,
    tasks: Rc<tasks::TasksController>,
    clipboard: clipboard::ClipboardController,
    controls: controls::ControlsController,
    launcher: launcher::LauncherController,
    /// Liquid Glass's backdrop sampling.
    glass: glass::GlassSampler,
    update_checking: Cell<bool>,
    update_timer: slint::Timer,
}

thread_local! {
    /// The running application, for work posted from other threads.
    static CURRENT: RefCell<Weak<App>> = const { RefCell::new(Weak::new()) };
}

/// Runs `f` on the UI thread with the running application, if it still exists.
fn post(f: impl FnOnce(&Rc<App>) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = CURRENT.with(|c| c.borrow().upgrade()) {
            f(&app);
        }
    });
}

/// Registration with the shell's app bar list, used only to be told when a full-screen
/// application takes over the display. No screen space is reserved.
struct AppBar(APPBARDATA);

impl AppBar {
    fn register() -> Self {
        let mut data = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: window::control_window(),
            uCallbackMessage: WM_APP_APPBAR,
            ..Default::default()
        };
        unsafe { SHAppBarMessage(ABM_NEW, &mut data) };
        Self(data)
    }
}

impl Drop for AppBar {
    fn drop(&mut self) {
        unsafe { SHAppBarMessage(ABM_REMOVE, &mut self.0) };
    }
}

pub fn run() -> Result<(), slint::PlatformError> {
    // Shell dialogs (the calendar file picker) require a single-threaded apartment.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    // Process-wide COM security, once, before any COM call leaves the process: Windows'
    // defaults with impersonation allowed, which WMI's brightness classes require
    // (src/controls/display.rs). The same settings PowerShell and most tools use.
    if let Err(err) = unsafe {
        CoInitializeSecurity(
            None,
            -1,
            None,
            None,
            RPC_C_AUTHN_LEVEL_DEFAULT,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
            None,
        )
    } {
        tracing::warn!(%err, "COM security could not be set; brightness control may be unavailable");
    }
    window::install()?;
    window::create_next_as(WindowKind::Island);
    let island = Island::new()?;
    let tray = Tray::new()?;
    let settings = App::load_settings();
    let tasks = tasks::TasksController::new(
        &island,
        crate::tasks::store::default_path(),
        clock::uses_24_hour_clock(),
        tasks::sort_mode(settings.tasks.order),
    );

    let app = Rc::new(App {
        island,
        tray,
        settings: RefCell::new(settings),
        save_timer: slint::Timer::default(),
        settings_window: RefCell::new(None),
        shortcut_problem: RefCell::new([None, None]),
        hidden_by_user: Cell::new(false),
        fullscreen_active: Cell::new(false),
        clock_timer: slint::Timer::default(),
        idle_timer: slint::Timer::default(),
        idle_grace_timer: slint::Timer::default(),
        idle_active: Cell::new(false),
        idle_transition_timer: slint::Timer::default(),
        weather_timer: slint::Timer::default(),
        h24: Cell::new(clock::uses_24_hour_clock()),
        collapse_timer: slint::Timer::default(),
        pointer_inside: Cell::new(false),
        auto_hide_revealed: Cell::new(false),
        peek_timer: slint::Timer::default(),
        peek_hide_timer: slint::Timer::default(),
        previous_foreground: Cell::new(None),
        media: RefCell::new(None),
        media_state: RefCell::new(None),
        last_track: RefCell::new(None),
        media_ready: Cell::new(false),
        activity_timer: slint::Timer::default(),
        progress_timer: slint::Timer::default(),
        bars_timer: slint::Timer::default(),
        system_monitor: RefCell::new(Some(SystemMonitor::new())),
        sampling: Cell::new(SamplingPolicy::OFF),
        cpu_timer: slint::Timer::default(),
        memory_timer: slint::Timer::default(),
        network_timer: slint::Timer::default(),
        calendar: RefCell::new(None),
        zones: WindowsZones::default(),
        glass_fallback: Cell::new(None),
        window_blur: Cell::new(false),
        tasks,
        clipboard: clipboard::ClipboardController::new(crate::clipboard::store::default_path()),
        controls: controls::ControlsController::default(),
        launcher: launcher::LauncherController::new(),
        glass: glass::GlassSampler::default(),
        update_checking: Cell::new(false),
        update_timer: slint::Timer::default(),
    });
    CURRENT.with(|c| *c.borrow_mut() = Rc::downgrade(&app));
    App::connect(&app);
    App::connect_media(&app);
    App::connect_calendar(&app);
    App::connect_preferences(&app);
    App::connect_tasks(&app);
    App::connect_clipboard(&app);
    App::connect_controls(&app);
    App::connect_updates(&app);
    App::refresh_clock(&app);
    App::connect_idle(&app);
    app.tray.set_light_taskbar(light_taskbar());

    // The shell reports full-screen changes only as they happen, so ask once whether one
    // is already in progress before showing the island.
    app.fullscreen_active.set(full_screen_app_active());
    App::apply_settings(&app);
    app.tray.show()?;
    let _app_bar = AppBar::register();
    app.media.replace(Some(MediaService::start(|update| post(move |app| App::media_update(app, update)))));

    slint::run_event_loop()?;
    app.island.hide()?;
    hotkey::unregister(window::control_window(), hotkey::TOGGLE_ID);
    hotkey::unregister(window::control_window(), hotkey::CLIPBOARD_ID);
    if app.save_timer.running() {
        app.save_now();
    }
    // Stops the media worker and unsubscribes from media session events.
    app.media.replace(None);
    // Stops clipboard notifications and the clipboard worker.
    app.clipboard.stop();
    // Stops the quick-controls worker and its Wi-Fi notifications.
    app.stop_controls();
    Ok(())
}

impl App {
    fn connect(app: &Rc<Self>) {
        let weak = Rc::downgrade(app);

        app.tray.on_toggle_island(with(&weak, |app| App::set_hidden_by_user(app, !app.hidden_by_user.get())));
        app.tray.on_quit(|| {
            let _ = slint::quit_event_loop();
        });

        let actions = app.island.global::<AppActions>();
        let weak_action = weak.clone();
        actions.on_quick_action(move |index| {
            let Some(action) = QuickAction::from_index(index) else { return };
            if let Some(app) = weak_action.upgrade() {
                if action == QuickAction::Settings {
                    App::open_settings(&app);
                    return;
                }
                if action == QuickAction::AutoHide {
                    let current = app.settings.borrow().general.auto_hide;
                    let next = !current;
                    app.settings.borrow_mut().general.auto_hide = next;
                    app.island.global::<AppState>().set_auto_hide(next);
                    App::schedule_save(&app);
                    App::apply_peek(&app);
                    return;
                }
                // Collapse first so the opened window is not covered and gets the focus.
                App::set_expanded(&app, false);
            }
            if let Err(err) = action.launch() {
                tracing::warn!(%err, ?action, "quick action failed");
            }
        });
        actions.on_toggle_expanded(with(&weak, |app| App::set_expanded(app, !app.expanded())));
        actions.on_collapse(with(&weak, |app| App::set_expanded(app, false)));

        // The Tasks + Clipboard quick action toggles the side box, even when both are
        // turned off in Settings. The force is also cleared when the island collapses.
        let weak_shelf = weak.clone();
        actions.on_open_shelf(move || {
            let Some(app) = weak_shelf.upgrade() else { return };
            let state = app.island.global::<AppState>();
            // A second tap hides the box it opened, so the island goes back to its
            // dashboard instead of only ever opening.
            if state.get_force_side_panel() {
                state.set_force_side_panel(false);
                return;
            }
            state.set_force_side_panel(true);
            // If neither side is on in Settings, bias the first view towards Tasks;
            // honour the user's last pick when it is available.
            if !state.get_show_tasks() && state.get_show_clipboard() {
                state.set_side_panel(crate::ui::SidePanel::Clipboard);
            } else if !state.get_show_clipboard() && state.get_show_tasks() {
                state.set_side_panel(crate::ui::SidePanel::Tasks);
            }
            App::set_expanded(&app, true);
        });

        let weak_launch = weak.clone();
        actions.on_toggle_launcher(move || {
            if let Some(app) = weak_launch.upgrade() {
                app.launcher.toggle(&app.island);
            }
        });
        let weak_scroll = weak.clone();
        actions.on_scroll_launcher(move |delta| {
            if let Some(app) = weak_scroll.upgrade() {
                app.launcher.scroll(delta, &app.island);
            }
        });
        let weak_exec = weak.clone();
        actions.on_launch_app(move |index| {
            if let Some(app) = weak_exec.upgrade() {
                app.launcher.launch(index, &app.island);
            }
        });
        let weak_hide = weak.clone();
        actions.on_toggle_auto_hide(move || {
            if let Some(app) = weak_hide.upgrade() {
                let current = app.settings.borrow().general.auto_hide;
                let next = !current;
                app.settings.borrow_mut().general.auto_hide = next;
                app.island.global::<AppState>().set_auto_hide(next);
                App::schedule_save(&app);
                App::apply_peek(&app);
            }
        });
        if !app.settings.borrow().launcher.initialized && app.settings.borrow().launcher.custom_apps.is_empty() {
            let defaults = launcher::LauncherController::scan_default_apps();
            let mut settings = app.settings.borrow_mut();
            settings.launcher.custom_apps = defaults;
            settings.launcher.initialized = true;
            drop(settings);
            App::schedule_save(app);
        }
        app.launcher.detect_apps(&app.settings.borrow());
        app.launcher.publish(&app.island);

        let island_window = app.island.as_weak();
        actions.on_start_drag(move || {
            let island_window = island_window.clone();
            // Native dragging pumps Slint timers, so enter it outside both pointer and timer callbacks.
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(native) = island_window.upgrade().and_then(|i| window::find(i.window())) {
                    native.begin_move();
                }
            });
        });

        if let Some(native) = window::find(app.island.window()) {
            let island = app.island.as_weak();
            native.set_shape_source(move || {
                let Some(island) = island.upgrade() else { return Vec::new() };
                let mut shapes = vec![window::Shape {
                    x: island.get_body_x(),
                    y: island.get_body_y(),
                    width: island.get_body_width(),
                    height: island.get_body_height(),
                    radius: island.get_body_radius(),
                    opacity: island.get_body_opacity(),
                }];
                if island.get_tasks_shown() {
                    shapes.push(window::Shape {
                        x: island.get_tasks_x(),
                        y: island.get_tasks_y(),
                        width: island.get_tasks_box_width(),
                        height: island.get_tasks_box_height(),
                        radius: island.get_tasks_radius(),
                        opacity: island.get_tasks_opacity(),
                    });
                }
                shapes
            });
            let weak = weak.clone();
            native.set_notice_handler(move |notice| {
                let Some(app) = weak.upgrade() else { return };
                match notice {
                    Notice::CenterSnap(snapped) => app.island.global::<AppState>().set_center_snapped(snapped),
                    Notice::MoveFinished => App::finish_move(&app),
                    Notice::ScaleChanged => app.place(),
                    // A click elsewhere moved the focus away from the expanded island.
                    Notice::Activated(false)
                        if app.expanded() && app.settings.borrow().general.collapse_on_outside_click =>
                    {
                        App::set_expanded(&app, false)
                    }
                    Notice::Activated(_) => {}
                    Notice::Pointer(inside) => App::pointer_moved(&app, inside),
                }
            });
        }

        window::set_system_handler(move |msg, wparam, lparam| {
            let app = weak.upgrade()?;
            app.system_message(&app, msg, wparam, lparam)
        });
    }

    fn system_message(&self, app: &Rc<Self>, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_DISPLAYCHANGE => self.place(),
            WM_SETTINGCHANGE if wparam.0 as u32 == SPI_SETWORKAREA.0 => self.place(),
            // A new wallpaper may be brighter or darker behind Liquid Glass.
            WM_SETTINGCHANGE if wparam.0 as u32 == SPI_SETDESKWALLPAPER.0 => {
                App::sample_glass_soon(app, Duration::from_millis(500))
            }
            // "Animation effects" was switched in Windows.
            WM_SETTINGCHANGE if wparam.0 as u32 == SPI_SETCLIENTAREAANIMATION.0 => {
                let reduced = preferences::reduced_motion(self.settings.borrow().appearance.motion);
                self.island.global::<Theme>().set_motion(if reduced { 0.0 } else { 1.0 });
                self.apply_window_appearance();
                App::update_bars(app);
            }
            WM_SETTINGCHANGE => match setting_name(lparam).as_deref() {
                // Regional format changed: the 12/24-hour preference may differ.
                Some("intl") => {
                    App::refresh_clock(app);
                }
                // The Windows colour mode or "Transparency effects" changed.
                Some("ImmersiveColorSet") => {
                    self.tray.set_light_taskbar(light_taskbar());
                    App::apply_appearance(app);
                }
                _ => {}
            },
            WM_TIMECHANGE => App::refresh_clock(app),
            WM_POWERBROADCAST if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC => App::refresh_clock(app),
            // Battery saver may have been switched, which turns the live blur off.
            WM_POWERBROADCAST if wparam.0 as u32 == PBT_APMPOWERSTATUSCHANGE => App::apply_appearance(app),
            WM_APP_APPBAR if wparam.0 as u32 == ABN_FULLSCREENAPP => {
                let fullscreen = lparam.0 != 0;
                tracing::debug!(fullscreen, "full-screen application state changed");
                self.fullscreen_active.set(fullscreen);
                App::update_visibility(app);
            }
            crate::clipboard::capture::WM_CLIPBOARDUPDATE => self.clipboard_changed(),
            WM_HOTKEY if wparam.0 as i32 == hotkey::CLIPBOARD_ID => {
                if self.hidden_by_user.get() {
                    App::set_hidden_by_user(app, false);
                }
                if self.island_visible() {
                    App::open_clipboard_shelf(app);
                }
            }
            WM_HOTKEY if wparam.0 as i32 == hotkey::TOGGLE_ID => {
                if self.hidden_by_user.get() {
                    App::set_hidden_by_user(app, false);
                }
                if self.island_visible() {
                    App::set_expanded(app, !self.expanded());
                }
            }
            window::WM_APP_ACTIVATE => App::set_hidden_by_user(app, false),
            // Sent by the installer and uninstaller to close a running copy gracefully.
            WM_CLOSE => {
                tracing::info!("received shutdown request on the control window");
                let _ = slint::quit_event_loop();
            }
            _ => return None,
        }
        Some(LRESULT(0))
    }

    fn expanded(&self) -> bool {
        self.island.global::<AppState>().get_expanded()
    }

    fn island_visible(&self) -> bool {
        let hidden_for_fullscreen =
            self.fullscreen_active.get() && self.settings.borrow().performance.hide_in_fullscreen;
        !self.hidden_by_user.get() && !hidden_for_fullscreen
    }

    fn set_expanded(app: &Rc<Self>, expanded: bool) {
        if app.expanded() == expanded {
            return;
        }
        let state = app.island.global::<AppState>();
        state.set_expanded(expanded);
        state.set_activity(false);
        app.collapse_timer.stop();
        // Expanding reveals an auto-hidden island; collapsing lets it tuck away again.
        if expanded {
            app.auto_hide_revealed.set(true);
        }
        App::apply_peek(app);
        App::update_progress_timer(app);
        App::update_sampling(app);
        App::update_glass_sampling(app);
        app.refresh_agenda();
        if !expanded {
            app.reset_clipboard_ui();
            // The quick-action force only applies while expanded.
            state.set_force_side_panel(false);
        }
        let native = window::find(app.island.window());
        if expanded {
            // Taking the focus lets Escape close the island and a click elsewhere
            // collapse it; the user's window gets it back afterwards.
            app.previous_foreground.set(native.as_ref().and_then(|n| n.activate()));
            if !app.pointer_inside.get() {
                App::schedule_collapse(app);
            }
        } else {
            if let Some(previous) = app.previous_foreground.take()
                && native.is_some_and(|n| n.is_foreground())
            {
                unsafe {
                    let _ = SetForegroundWindow(previous);
                }
            }
            // After collapsing, tuck away again once the pointer has left.
            App::schedule_peek_hide(app);
        }
    }

    fn pointer_moved(app: &Rc<Self>, inside: bool) {
        app.pointer_inside.set(inside);
        app.island.global::<AppState>().set_pointer_inside(inside);
        if inside {
            app.collapse_timer.stop();
            // Keep a revealed island out while the pointer rests on it.
            app.peek_hide_timer.stop();
        } else {
            app.launcher.on_pointer_left(&app.island);
            if app.expanded() {
                App::schedule_collapse(app);
            }
            // The pointer left the island: tuck it away again if auto-hide is on.
            App::schedule_peek_hide(app);
        }
    }

    fn schedule_collapse(app: &Rc<Self>) {
        let general = app.settings.borrow().general.clone();
        // Never while a task, a search or a snippet is being typed or edited.
        if !general.auto_collapse || app.tasks.busy.get() || app.clipboard.busy.get() || app.controls.busy.get() {
            return;
        }
        let weak = Rc::downgrade(app);
        let delay = Duration::from_secs(general.auto_collapse_seconds as u64);
        app.collapse_timer.start(slint::TimerMode::SingleShot, delay, move || {
            if let Some(app) = weak.upgrade() {
                App::set_expanded(&app, false);
            }
        });
    }

    fn set_hidden_by_user(app: &Rc<Self>, hidden: bool) {
        app.hidden_by_user.set(hidden);
        app.tray.set_island_visible(!hidden);
        App::update_visibility(app);
    }

    fn update_visibility(app: &Rc<Self>) {
        let visible = app.island_visible();
        if !visible {
            App::set_expanded(app, false);
        }
        app.island.global::<AppState>().set_on_screen(visible);
        let result = if visible { app.island.show() } else { app.island.hide() };
        if let Err(err) = result {
            tracing::warn!(%err, visible, "could not change island visibility");
        }
        if visible {
            app.place();
        }
        // Under the cursor, auto-hide starts revealed rather than tucking out of reach.
        if app.settings.borrow().general.auto_hide && app.pointer_inside.get() {
            app.auto_hide_revealed.set(true);
        }
        App::apply_peek(app);
        App::update_progress_timer(app);
        App::update_sampling(app);
        App::update_glass_sampling(app);
    }

    /// Re-evaluates whether the island should be tucked away for auto-hide, and starts or
    /// stops the cursor poll accordingly.
    fn apply_peek(app: &Rc<Self>) {
        let governed = app.settings.borrow().general.auto_hide && app.island_visible();
        let revealed = !governed || app.expanded() || app.auto_hide_revealed.get();
        let tucked = governed && !revealed;
        app.island.global::<AppState>().set_tucked(tucked);
        if tucked {
            if !app.peek_timer.running() {
                let weak = Rc::downgrade(app);
                app.peek_timer.start(slint::TimerMode::Repeated, PEEK_POLL, move || {
                    if let Some(app) = weak.upgrade() {
                        App::peek_tick(&app);
                    }
                });
            }
        } else {
            app.peek_timer.stop();
        }
        if !governed {
            app.peek_hide_timer.stop();
            app.auto_hide_revealed.set(false);
        }
    }

    /// Reveals the island when the cursor reaches the band at the top centre.
    fn peek_tick(app: &Rc<Self>) {
        let Some(zone) = app.peek_trigger_zone() else { return };
        let mut point = POINT::default();
        if unsafe { GetCursorPos(&mut point) }.is_err() {
            return;
        }
        if zone.contains(point.x, point.y) {
            App::reveal_peek(app);
        }
    }

    /// Shows a tucked island and arms a fallback that re-tucks it if the pointer never
    /// lands on it.
    fn reveal_peek(app: &Rc<Self>) {
        if app.auto_hide_revealed.replace(true) {
            return;
        }
        app.peek_hide_timer.stop();
        App::apply_peek(app);
        App::schedule_peek_hide(app);
    }

    /// Tucks the island away again and resumes polling the cursor.
    fn tuck_peek(app: &Rc<Self>) {
        app.auto_hide_revealed.set(false);
        App::apply_peek(app);
    }

    /// Re-tucks the island a short while after the pointer leaves it, unless it is
    /// expanded or a field inside is being edited.
    fn schedule_peek_hide(app: &Rc<Self>) {
        if !(app.settings.borrow().general.auto_hide && app.island_visible())
            || app.expanded()
            || app.pointer_inside.get()
            || app.tasks.busy.get()
            || app.clipboard.busy.get()
            || app.controls.busy.get()
        {
            return;
        }
        let weak = Rc::downgrade(app);
        app.peek_hide_timer.start(slint::TimerMode::SingleShot, PEEK_HIDE_DELAY, move || {
            if let Some(app) = weak.upgrade() {
                App::tuck_peek(&app);
            }
        });
    }

    /// The band, in physical pixels, that reveals a tucked island: the top centre of the
    /// chosen monitor, around where the compact island sits.
    fn peek_trigger_zone(&self) -> Option<placement::Rect> {
        let monitors = placement::monitors();
        let settings = self.settings.borrow();
        let monitor = placement::choose(&monitors, settings.position.monitor.as_deref())?;
        let scale = monitor.scale;
        let half_width = (settings.appearance.compact_width as f32 * scale / 2.0).round() as i32;
        let pad = (PEEK_BAND_PAD * scale).round() as i32;
        let centre =
            monitor.work.left + monitor.work.width() / 2 + (settings.position.offset_x as f32 * scale).round() as i32;
        Some(placement::Rect {
            left: centre - half_width - pad,
            right: centre + half_width + pad,
            // Only a thin strip at the very top edge, so the island stays tucked until the
            // cursor reaches the top of the screen.
            top: monitor.bounds.top,
            bottom: monitor.bounds.top + (PEEK_BAND_HEIGHT * scale).round() as i32,
        })
    }

    /// Updates the clock text and schedules the next update for the start of the next
    /// minute. Re-arming per minute keeps the clock aligned after sleep or time changes.
    fn refresh_clock(app: &Rc<Self>) {
        app.h24.set(clock::resolved_h24(&app.settings.borrow().clock));
        let text = clock::configured_now(&app.settings.borrow().clock);
        let state = app.island.global::<AppState>();
        state.set_time(text.time.into());
        state.set_weekday(text.weekday.into());
        state.set_date(text.date.into());
        app.refresh_agenda();
        app.tick_tasks();
        // A safety net: whatever happened, the bars move again within a minute.
        App::update_bars(app);

        let weak = Rc::downgrade(app);
        let wait = Duration::from_millis(clock::millis_to_next_minute() + 20);
        app.clock_timer.start(slint::TimerMode::SingleShot, wait, move || {
            if let Some(app) = weak.upgrade() {
                App::refresh_clock(&app);
            }
        });
    }

    /// Moves the island to its configured position on the chosen monitor.
    pub fn place(&self) {
        let monitors = placement::monitors();
        let settings = self.settings.borrow();
        let placement = &settings.position;
        let Some(monitor) = placement::choose(&monitors, placement.monitor.as_deref()) else { return };
        let size = self.island.window().size();
        let (x, y) = placement::island_origin(
            monitor,
            (size.width as i32, size.height as i32),
            placement.offset_x,
            placement.top_margin,
        );
        self.island.window().set_position(slint::PhysicalPosition::new(x, y));
    }

    /// Records where the user dropped the island and snaps it back to the top edge.
    fn finish_move(app: &Rc<Self>) {
        app.record_move();
        app.place();
        App::schedule_save(app);
        App::sample_glass_soon(app, Duration::ZERO);
    }

    fn record_move(&self) {
        let position = self.island.window().position();
        let size = self.island.window().size();
        let monitors = placement::monitors();
        let centre = position.x + size.width as i32 / 2;
        let current = self.settings.borrow().position.monitor.clone();
        let Some(monitor) = placement::monitor_at(&monitors, centre, position.y + 1)
            .or_else(|| placement::choose(&monitors, current.as_deref()))
        else {
            return;
        };
        let mut settings = self.settings.borrow_mut();
        let placement = &mut settings.position;
        placement.monitor = if monitor.primary { None } else { Some(monitor.device.clone()) };
        placement.offset_x = placement::offset_for(monitor, position.x, size.width as i32);
        tracing::info!(monitor = ?placement.monitor, offset_x = placement.offset_x, "island moved");
    }
}

/// Whether a full-screen application, game or presentation currently owns the display.
fn full_screen_app_active() -> bool {
    matches!(
        unsafe { SHQueryUserNotificationState() },
        Ok(QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)
    )
}

/// The setting name broadcast with `WM_SETTINGCHANGE`, if any.
fn setting_name(lparam: LPARAM) -> Option<String> {
    if lparam.0 == 0 {
        return None;
    }
    // SAFETY: for WM_SETTINGCHANGE a non-null lParam points to a NUL-terminated string.
    unsafe { windows::core::PCWSTR(lparam.0 as *const u16).to_string().ok() }
}

/// Whether the taskbar uses the light theme ("Choose your default Windows mode").
fn light_taskbar() -> bool {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    result.is_ok() && value == 1
}

/// Close handler for the settings window: releases it instead of keeping it hidden.
fn with_close(weak: &Weak<App>) -> impl Fn() -> slint::CloseRequestResponse + 'static {
    let weak = weak.clone();
    move || {
        if let Some(app) = weak.upgrade() {
            app.close_settings();
        }
        slint::CloseRequestResponse::HideWindow
    }
}

/// Wraps a method call on a weakly held `App` as a callback.
fn with(weak: &Weak<App>, f: impl Fn(&Rc<App>) + 'static) -> impl Fn() + 'static {
    let weak = weak.clone();
    move || {
        if let Some(app) = weak.upgrade() {
            f(&app);
        }
    }
}
