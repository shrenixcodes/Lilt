//! Application controller: owns the island and tray, and keeps the island placed and
//! visible according to the displays and the foreground application.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use slint::ComponentHandle;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Shell::{ABM_NEW, ABM_REMOVE, ABN_FULLSCREENAPP, APPBARDATA, SHAppBarMessage};
use windows::Win32::UI::WindowsAndMessaging::{
    PBT_APMRESUMEAUTOMATIC, SPI_SETWORKAREA, SetForegroundWindow, WM_DISPLAYCHANGE, WM_POWERBROADCAST,
    WM_SETTINGCHANGE, WM_TIMECHANGE,
};
use windows::core::w;

use crate::clock;
use crate::ui::{AppActions, AppState, Island, Tray};
use crate::window::{self, Notice, WindowKind, placement};

/// How long the expanded island stays open after the pointer leaves it.
const AUTO_COLLAPSE: Duration = Duration::from_secs(8);

/// Shell notifications for the app bar registration (full-screen detection).
const WM_APP_APPBAR: u32 = window::WM_APP_USER;

/// Where the island sits. Persisted with the settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// Device name of the chosen monitor; `None` follows the primary monitor.
    pub monitor: Option<String>,
    /// Logical pixels from the centre of the work area.
    pub offset_x: i32,
    /// Logical pixels below the top of the work area.
    pub top_margin: u32,
}

impl Default for Placement {
    fn default() -> Self {
        Self { monitor: None, offset_x: 0, top_margin: 8 }
    }
}

pub struct App {
    island: Island,
    tray: Tray,
    placement: RefCell<Placement>,
    hidden_by_user: Cell<bool>,
    hidden_for_fullscreen: Cell<bool>,
    clock_timer: slint::Timer,
    h24: Cell<bool>,
    collapse_timer: slint::Timer,
    pointer_inside: Cell<bool>,
    /// Window that had the focus before the island was expanded, restored on collapse.
    previous_foreground: Cell<Option<HWND>>,
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
    window::install()?;
    window::create_next_as(WindowKind::Island);
    let island = Island::new()?;
    let tray = Tray::new()?;

    let app = Rc::new(App {
        island,
        tray,
        placement: RefCell::new(Placement::default()),
        hidden_by_user: Cell::new(false),
        hidden_for_fullscreen: Cell::new(false),
        clock_timer: slint::Timer::default(),
        h24: Cell::new(clock::uses_24_hour_clock()),
        collapse_timer: slint::Timer::default(),
        pointer_inside: Cell::new(false),
        previous_foreground: Cell::new(None),
    });
    App::connect(&app);
    App::refresh_clock(&app);
    app.tray.set_light_taskbar(light_taskbar());

    app.island.show()?;
    app.place();
    app.tray.show()?;
    let _app_bar = AppBar::register();

    slint::run_event_loop()?;
    app.island.hide()?;
    Ok(())
}

impl App {
    fn connect(app: &Rc<Self>) {
        let weak = Rc::downgrade(app);

        app.tray.on_toggle_island(with(&weak, |app| app.set_hidden_by_user(!app.hidden_by_user.get())));
        app.tray.on_quit(|| {
            let _ = slint::quit_event_loop();
        });

        let actions = app.island.global::<AppActions>();
        actions.on_toggle_expanded(with(&weak, |app| App::set_expanded(app, !app.expanded())));
        actions.on_collapse(with(&weak, |app| App::set_expanded(app, false)));

        let island_window = app.island.as_weak();
        actions.on_start_drag(move || {
            let island_window = island_window.clone();
            // The native move loop must not run inside Slint's pointer event dispatch.
            slint::Timer::single_shot(Duration::ZERO, move || {
                if let Some(native) = island_window.upgrade().and_then(|i| window::find(i.window())) {
                    native.begin_move();
                }
            });
        });

        if let Some(native) = window::find(app.island.window()) {
            let weak = weak.clone();
            native.set_notice_handler(move |notice| {
                let Some(app) = weak.upgrade() else { return };
                match notice {
                    Notice::MoveFinished => app.finish_move(),
                    Notice::ScaleChanged => app.place(),
                    // A click elsewhere moved the focus away from the expanded island.
                    Notice::Activated(false) if app.expanded() => App::set_expanded(&app, false),
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
            WM_SETTINGCHANGE => match setting_name(lparam).as_deref() {
                // Regional format changed: the 12/24-hour preference may differ.
                Some("intl") => {
                    self.h24.set(clock::uses_24_hour_clock());
                    App::refresh_clock(app);
                }
                // The Windows colour mode changed.
                Some("ImmersiveColorSet") => self.tray.set_light_taskbar(light_taskbar()),
                _ => {}
            },
            WM_TIMECHANGE => App::refresh_clock(app),
            WM_POWERBROADCAST if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC => App::refresh_clock(app),
            WM_APP_APPBAR if wparam.0 as u32 == ABN_FULLSCREENAPP => {
                let fullscreen = lparam.0 != 0;
                tracing::debug!(fullscreen, "full-screen application state changed");
                self.hidden_for_fullscreen.set(fullscreen);
                self.update_visibility();
            }
            window::WM_APP_ACTIVATE => self.set_hidden_by_user(false),
            _ => return None,
        }
        Some(LRESULT(0))
    }

    fn expanded(&self) -> bool {
        self.island.global::<AppState>().get_expanded()
    }

    fn set_expanded(app: &Rc<Self>, expanded: bool) {
        if app.expanded() == expanded {
            return;
        }
        app.island.global::<AppState>().set_expanded(expanded);
        app.collapse_timer.stop();
        let native = window::find(app.island.window());
        if expanded {
            // Taking the focus lets Escape close the island and a click elsewhere
            // collapse it; the user's window gets it back afterwards.
            app.previous_foreground.set(native.as_ref().and_then(|n| n.activate()));
            if !app.pointer_inside.get() {
                App::schedule_collapse(app);
            }
        } else if let Some(previous) = app.previous_foreground.take()
            && native.is_some_and(|n| n.is_foreground())
        {
            unsafe {
                let _ = SetForegroundWindow(previous);
            }
        }
    }

    fn pointer_moved(app: &Rc<Self>, inside: bool) {
        app.pointer_inside.set(inside);
        if inside {
            app.collapse_timer.stop();
        } else if app.expanded() {
            App::schedule_collapse(app);
        }
    }

    fn schedule_collapse(app: &Rc<Self>) {
        let weak = Rc::downgrade(app);
        app.collapse_timer.start(slint::TimerMode::SingleShot, AUTO_COLLAPSE, move || {
            if let Some(app) = weak.upgrade() {
                App::set_expanded(&app, false);
            }
        });
    }

    fn set_hidden_by_user(&self, hidden: bool) {
        self.hidden_by_user.set(hidden);
        self.tray.set_island_visible(!hidden);
        self.update_visibility();
    }

    fn update_visibility(&self) {
        let visible = !self.hidden_by_user.get() && !self.hidden_for_fullscreen.get();
        let result = if visible { self.island.show() } else { self.island.hide() };
        if let Err(err) = result {
            tracing::warn!(%err, visible, "could not change island visibility");
        }
        if visible {
            self.place();
        }
    }

    /// Updates the clock text and schedules the next update for the start of the next
    /// minute. Re-arming per minute keeps the clock aligned after sleep or time changes.
    fn refresh_clock(app: &Rc<Self>) {
        let text = clock::now(app.h24.get());
        let state = app.island.global::<AppState>();
        state.set_time(text.time.into());
        state.set_weekday(text.weekday.into());
        state.set_date(text.date.into());

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
        let placement = self.placement.borrow();
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
    fn finish_move(&self) {
        let position = self.island.window().position();
        let size = self.island.window().size();
        let monitors = placement::monitors();
        let centre = position.x + size.width as i32 / 2;
        let current = self.placement.borrow().monitor.clone();
        let Some(monitor) = placement::monitor_at(&monitors, centre, position.y + 1)
            .or_else(|| placement::choose(&monitors, current.as_deref()))
        else {
            return;
        };
        {
            let mut placement = self.placement.borrow_mut();
            placement.monitor = if monitor.primary { None } else { Some(monitor.device.clone()) };
            placement.offset_x = placement::offset_for(monitor, position.x, size.width as i32);
            tracing::info!(?placement, "island moved");
        }
        self.place();
    }
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

/// Wraps a method call on a weakly held `App` as a callback.
fn with(weak: &Weak<App>, f: impl Fn(&Rc<App>) + 'static) -> impl Fn() + 'static {
    let weak = weak.clone();
    move || {
        if let Some(app) = weak.upgrade() {
            f(&app);
        }
    }
}
