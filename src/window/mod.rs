//! A minimal Slint platform for Windows.
//!
//! Slint renders with its software renderer into memory that Lilt presents with plain
//! Win32 calls. The island uses a layered window, so fully transparent pixels let mouse
//! input reach the windows underneath and nothing is drawn by the GPU. The event loop
//! sleeps in `MsgWaitForMultipleObjectsEx` until input arrives, a Slint timer is due,
//! another thread posts work, or an animation needs its next frame.

mod adapter;
mod clipboard;
pub mod glass;
mod input;
pub mod placement;
pub mod sample;
mod surface;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use slint::platform::{EventLoopProxy, Platform, WindowAdapter};
use slint::{EventLoopError, PlatformError};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::INFINITE;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

pub use adapter::{Notice, Win32Window, WindowKind, find};
pub use glass::Shape;

pub(crate) const ISLAND_CLASS: PCWSTR = w!("Lilt.Island");
pub(crate) const WINDOW_CLASS: PCWSTR = w!("Lilt.Window");
const CONTROL_CLASS: PCWSTR = w!("Lilt.Control");

/// Posted to the control window to run work queued from other threads.
const WM_APP_WAKE: u32 = WM_APP + 1;
/// Posted by a second instance to ask the running one to show itself.
pub const WM_APP_ACTIVATE: u32 = WM_APP + 2;
/// Base for application-defined messages handled by the system message handler.
pub const WM_APP_USER: u32 = WM_APP + 16;

/// Frame interval while an animation is running.
const FRAME: Duration = Duration::from_millis(16);

type Task = Box<dyn FnOnce() + Send>;

static QUEUE: Mutex<VecDeque<Task>> = Mutex::new(VecDeque::new());
static CONTROL: AtomicIsize = AtomicIsize::new(0);
static QUIT: AtomicBool = AtomicBool::new(false);

thread_local! {
    static NEXT_KIND: Cell<WindowKind> = const { Cell::new(WindowKind::Standard) };
    static SYSTEM_HANDLER: RefCell<Option<Rc<SystemHandler>>> = const { RefCell::new(None) };
}

/// Receives messages sent to the hidden control window that Lilt itself does not
/// handle: display changes, setting changes, hot keys and application messages.
pub type SystemHandler = dyn Fn(u32, WPARAM, LPARAM) -> Option<LRESULT>;

/// Declares the kind of the next window Slint creates (the next `Component::new()`).
pub fn create_next_as(kind: WindowKind) {
    NEXT_KIND.with(|k| k.set(kind));
}

pub fn set_system_handler(handler: impl Fn(u32, WPARAM, LPARAM) -> Option<LRESULT> + 'static) {
    SYSTEM_HANDLER.with(|h| *h.borrow_mut() = Some(Rc::new(handler)));
}

/// The hidden top-level window that receives broadcasts and cross-thread wake-ups.
pub fn control_window() -> HWND {
    HWND(CONTROL.load(Ordering::Acquire) as *mut _)
}

/// Asks an already running instance to show its island.
pub fn signal_running_instance() {
    unsafe {
        if let Ok(hwnd) = FindWindowW(CONTROL_CLASS, PCWSTR::null()) {
            let _ = PostMessageW(Some(hwnd), WM_APP_ACTIVATE, WPARAM(0), LPARAM(0));
        }
    }
}

struct Win32Platform {
    start: Instant,
}

impl Platform for Win32Platform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let kind = NEXT_KIND.with(|k| k.replace(WindowKind::Standard));
        Ok(Win32Window::new(kind))
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        run();
        Ok(())
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy))
    }

    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }

    fn click_interval(&self) -> Duration {
        Duration::from_millis(unsafe { GetDoubleClickTime() } as u64)
    }

    fn set_clipboard_text(&self, text: &str, clipboard: slint::platform::Clipboard) {
        if clipboard == slint::platform::Clipboard::DefaultClipboard {
            clipboard::set_text(text);
        }
    }

    fn clipboard_text(&self, clipboard: slint::platform::Clipboard) -> Option<String> {
        (clipboard == slint::platform::Clipboard::DefaultClipboard).then(clipboard::text).flatten()
    }
}

struct Proxy;

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        QUIT.store(true, Ordering::Release);
        wake();
        Ok(())
    }

    fn invoke_from_event_loop(&self, event: Box<dyn FnOnce() + Send>) -> Result<(), EventLoopError> {
        QUEUE.lock().map_err(|_| EventLoopError::EventLoopTerminated)?.push_back(event);
        wake();
        Ok(())
    }
}

fn wake() {
    let hwnd = control_window();
    if !hwnd.is_invalid() {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

fn run_queued() {
    loop {
        let task = QUEUE.lock().ok().and_then(|mut q| q.pop_front());
        match task {
            Some(task) => task(),
            None => break,
        }
    }
}

/// Registers window classes, creates the control window and installs the platform.
pub fn install() -> Result<(), PlatformError> {
    unsafe {
        // The manifest already requests this; the call is a fallback for builds whose
        // linker dropped it, and fails harmlessly otherwise.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let instance = GetModuleHandleW(None).map_err(|e| PlatformError::Other(e.to_string()))?;
        let icon = LoadIconW(Some(instance.into()), PCWSTR(1 as _)).ok();
        let cursor = LoadCursorW(None, IDC_ARROW).ok();
        let classes: [(PCWSTR, WNDPROC); 3] = [
            (ISLAND_CLASS, Some(adapter::window_proc as _)),
            (WINDOW_CLASS, Some(adapter::window_proc as _)),
            (CONTROL_CLASS, Some(control_proc as _)),
        ];
        for (class, procedure) in classes {
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: procedure,
                hInstance: instance.into(),
                hIcon: icon.unwrap_or_default(),
                hCursor: cursor.unwrap_or_default(),
                lpszClassName: class,
                ..Default::default()
            };
            if RegisterClassExW(&class) == 0 {
                return Err(PlatformError::Other("window class registration failed".into()));
            }
        }

        // A hidden top-level window rather than a message-only window, because the
        // latter does not receive broadcasts such as WM_DISPLAYCHANGE.
        let control = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CONTROL_CLASS,
            w!("Lilt"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|e| PlatformError::Other(e.to_string()))?;
        CONTROL.store(control.0 as isize, Ordering::Release);
    }
    slint::platform::set_platform(Box::new(Win32Platform { start: Instant::now() }))
        .map_err(|e| PlatformError::Other(e.to_string()))
}

unsafe extern "system" fn control_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_APP_WAKE {
        // Also reached from modal loops (window dragging, menus), so queued work is
        // not delayed by them.
        run_queued();
        return LRESULT(0);
    }
    let handler = SYSTEM_HANDLER.with(|h| h.borrow().clone());
    if let Some(result) = handler.and_then(|handler| handler(msg, wparam, lparam)) {
        return result;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn run() {
    QUIT.store(false, Ordering::Release);
    loop {
        let frame_start = Instant::now();
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        run_queued();
        if QUIT.load(Ordering::Acquire) {
            return;
        }

        slint::platform::update_timers_and_animations();
        let mut animating = false;
        for window in adapter::all() {
            window.render_if_needed();
            animating |= window.window().has_active_animations();
        }

        let wait = if animating {
            FRAME.saturating_sub(frame_start.elapsed())
        } else {
            slint::platform::duration_until_next_timer_update().unwrap_or(Duration::MAX)
        };
        let millis = if wait == Duration::MAX { INFINITE } else { wait.as_millis().min(u32::MAX as u128 - 1) as u32 };
        unsafe {
            MsgWaitForMultipleObjectsEx(None, millis, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }
}
