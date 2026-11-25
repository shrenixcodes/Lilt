//! A Slint window backed by a Win32 window and the software renderer.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use slint::platform::software_renderer::{RepaintBufferType, SoftwareRenderer};
use slint::platform::{PointerEventButton, Renderer, WindowAdapter, WindowEvent, WindowProperties};
use slint::{LogicalPosition, PhysicalPosition, PhysicalSize, WindowPosition, WindowSize};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWMSBT_AUTO, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetDC, InvalidateRect, PAINTSTRUCT, ReleaseDC, ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VIRTUAL_KEY, VK_CONTROL, VK_LMENU,
    VK_MENU, VK_RMENU,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, w};

use super::glass::{Backdrop, Shape};
use super::input;
use super::surface::Surface;

/// Not exported by `windows` under the features in use.
const WM_MOUSELEAVE: u32 = 0x02A3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WindowKind {
    /// Borderless, topmost, layered, never activated by clicks, hidden from the taskbar.
    Island,
    /// An ordinary top-level window.
    #[default]
    Standard,
}

/// Events the application may want to react to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Notice {
    Activated(bool),
    /// The pointer entered (`true`) or left (`false`) the window's visible pixels.
    Pointer(bool),
    /// The user finished dragging the window.
    MoveFinished,
    CenterSnap(bool),
    ScaleChanged,
}

type NoticeHandler = Rc<dyn Fn(Notice)>;
type ShapeSource = Box<dyn Fn() -> Vec<Shape>>;

pub struct Win32Window {
    window: slint::Window,
    renderer: SoftwareRenderer,
    kind: WindowKind,
    hwnd: Cell<Option<HWND>>,
    size: Cell<PhysicalSize>,
    min_size: Cell<PhysicalSize>,
    position: Cell<Option<PhysicalPosition>>,
    scale: Cell<f32>,
    needs_redraw: Cell<bool>,
    surface: RefCell<Option<Surface>>,
    tracking_mouse: Cell<bool>,
    last_pointer: Cell<LogicalPosition>,
    drag_top: Cell<Option<i32>>,
    drag_origin: Cell<Option<super::placement::HorizontalDrag>>,
    notice: RefCell<Option<NoticeHandler>>,
    /// The live blur beneath the island (Liquid Glass).
    /// The glass backdrop: whether it is on, the desktop behind the window and how much
    /// it bends along the glass's edges.
    glass: Cell<bool>,
    backdrop: RefCell<Option<Backdrop>>,
    refraction: Cell<f32>,
    /// The outlines of the island's surfaces in logical pixels, read after each frame for
    /// the backdrop.
    shape_source: RefCell<Option<ShapeSource>>,
    /// Whether an ordinary window shows the system acrylic behind its content.
    system_backdrop: Cell<bool>,
    /// Intercepts Alt/F10 system keys instead of letting Windows handle them (e.g. while recording shortcuts).
    consume_sys_keys: Cell<bool>,
    /// Consumes the trailing `WM_SYSKEYUP` after a system key press was intercepted.
    suppress_sys_keyup: Cell<bool>,
}

thread_local! {
    /// Every window adapter created on this thread, registered at creation so that it
    /// can be found before its native window exists.
    static WINDOWS: RefCell<Vec<Weak<Win32Window>>> = const { RefCell::new(Vec::new()) };
}

fn lookup(hwnd: HWND) -> Option<Rc<Win32Window>> {
    all().into_iter().find(|w| w.hwnd.get() == Some(hwnd))
}

/// All live windows, for the event loop.
pub(super) fn all() -> Vec<Rc<Win32Window>> {
    WINDOWS.with(|w| {
        let mut list = w.borrow_mut();
        list.retain(|w| w.strong_count() > 0);
        list.iter().filter_map(Weak::upgrade).collect()
    })
}

/// Finds the native window behind a Slint window created by this platform.
pub fn find(window: &slint::Window) -> Option<Rc<Win32Window>> {
    all().into_iter().find(|w| std::ptr::eq(&w.window, window))
}

#[cfg(test)]
thread_local! {
    /// Stands in for the system DPI scale in tests, to exercise other display scalings.
    pub(crate) static TEST_SCALE: Cell<Option<f32>> = const { Cell::new(None) };
}

/// The scale of the system's DPI: what a new window starts with until it knows its
/// monitor.
fn system_scale() -> f32 {
    #[cfg(test)]
    if let Some(scale) = TEST_SCALE.with(Cell::get) {
        return scale;
    }
    unsafe { GetDpiForSystem() as f32 / 96.0 }
}

impl Win32Window {
    pub fn new(kind: WindowKind) -> Rc<Self> {
        let scale = system_scale();
        let window = Rc::new_cyclic(|weak: &Weak<Self>| Self {
            window: slint::Window::new(weak.clone() as Weak<dyn WindowAdapter>),
            renderer: SoftwareRenderer::new_with_repaint_buffer_type(RepaintBufferType::ReusedBuffer),
            kind,
            hwnd: Cell::new(None),
            size: Cell::new(PhysicalSize::default()),
            min_size: Cell::new(PhysicalSize::default()),
            position: Cell::new(None),
            scale: Cell::new(scale),
            needs_redraw: Cell::new(true),
            surface: RefCell::new(None),
            tracking_mouse: Cell::new(false),
            last_pointer: Cell::new(LogicalPosition::default()),
            drag_top: Cell::new(None),
            drag_origin: Cell::new(None),
            notice: RefCell::new(None),
            glass: Cell::new(false),
            backdrop: RefCell::new(None),
            refraction: Cell::new(0.0),
            shape_source: RefCell::new(None),
            system_backdrop: Cell::new(false),
            consume_sys_keys: Cell::new(false),
            suppress_sys_keyup: Cell::new(false),
        });
        WINDOWS.with(|w| w.borrow_mut().push(Rc::downgrade(&window)));
        // Slint draws at a scale of 1 until told otherwise. Without this, at any display
        // scaling other than 100 % Slint would draw at 100 % while sizes, positions and
        // pointer coordinates here are converted at the real scale, so clicks would land
        // on the wrong controls.
        window.window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
        window
    }

    pub fn set_notice_handler(&self, handler: impl Fn(Notice) + 'static) {
        *self.notice.borrow_mut() = Some(Rc::new(handler));
    }

    /// Supplies the current outlines of the island's surfaces, in logical pixels relative
    /// to the window.
    pub fn set_shape_source(&self, source: impl Fn() -> Vec<Shape> + 'static) {
        *self.shape_source.borrow_mut() = Some(Box::new(source));
    }

    fn physical_shapes(&self) -> Vec<Shape> {
        let source = self.shape_source.borrow();
        let Some(source) = source.as_ref() else { return Vec::new() };
        let k = self.scale.get();
        source().into_iter().map(|s| s.scaled(k)).collect()
    }

    /// Turns the glass blur behind the island's surfaces on or off. Returns whether it is
    /// on. The desktop itself arrives with [`Self::set_backdrop`].
    pub fn set_glass(&self, enabled: bool) -> bool {
        let enabled = enabled && self.kind == WindowKind::Island;
        if self.glass.replace(enabled) != enabled {
            self.backdrop.replace(None);
            self.repaint_all();
        }
        enabled
    }

    /// The native window, once created.
    pub fn hwnd(&self) -> Option<HWND> {
        self.hwnd.get()
    }

    /// The screen rectangle of the window, in physical pixels: what the blur must show.
    pub fn screen_rect(&self) -> Option<RECT> {
        let hwnd = self.hwnd.get()?;
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
        Some(rect)
    }

    /// Shows a newly read desktop behind the glass surfaces, bent along their edges by
    /// `refraction` (0 for none).
    pub fn set_backdrop(&self, backdrop: Backdrop, refraction: f32) {
        let same = self.backdrop.borrow().as_ref() == Some(&backdrop) && self.refraction.get() == refraction;
        if !self.glass.get() || same {
            return;
        }
        self.backdrop.replace(Some(backdrop));
        self.refraction.set(refraction);
        self.repaint_all();
    }

    /// Redraws the whole window on the next frame.
    fn repaint_all(&self) {
        self.renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
        self.renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
        self.window.request_redraw();
        self.needs_redraw.set(true);
    }

    /// Shows the system acrylic behind an ordinary window. Returns whether it is shown.
    pub fn set_system_backdrop(&self, enabled: bool) -> bool {
        if self.kind != WindowKind::Standard {
            return false;
        }
        self.system_backdrop.set(enabled);
        self.needs_redraw.set(true);
        match self.hwnd.get() {
            Some(hwnd) => apply_system_backdrop(hwnd, enabled),
            None => enabled,
        }
    }

    /// Intercepts system key chords (`Alt+*`, `F10`) instead of passing them to `DefWindowProcW`.
    pub fn set_consume_sys_keys(&self, enabled: bool) {
        self.consume_sys_keys.set(enabled);
        if !enabled {
            // Keep `suppress_sys_keyup` armed only if Alt is currently still down from a suppressed chord.
            if !input::modifier_held(VK_MENU) {
                self.suppress_sys_keyup.set(false);
            }
        }
    }

    fn notify(&self, notice: Notice) {
        let handler = self.notice.borrow().clone();
        if let Some(handler) = handler {
            handler(notice);
        }
    }

    fn ensure_hwnd(&self) -> Option<HWND> {
        if let Some(hwnd) = self.hwnd.get() {
            return Some(hwnd);
        }
        let (class, ex_style, style) = match self.kind {
            WindowKind::Island => {
                (super::ISLAND_CLASS, WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE, WS_POPUP)
            }
            WindowKind::Standard => (super::WINDOW_CLASS, WINDOW_EX_STYLE::default(), WS_OVERLAPPEDWINDOW),
        };
        let size = self.size.get();
        // Until the application positions it, the island is created off-screen so it never
        // flashes in the corner of the display.
        let default = if self.kind == WindowKind::Island { (-32000, -32000) } else { (CW_USEDEFAULT, CW_USEDEFAULT) };
        let (x, y) = self.position.get().map_or(default, |p| (p.x, p.y));
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style,
                class,
                w!("Lilt"),
                style,
                x,
                y,
                size.width.max(1) as i32,
                size.height.max(1) as i32,
                None,
                None,
                GetModuleHandleW(None).ok().map(Into::into),
                None,
            )
        };
        let hwnd = match hwnd {
            Ok(hwnd) => hwnd,
            Err(err) => {
                tracing::error!(%err, "could not create window");
                return None;
            }
        };
        self.hwnd.set(Some(hwnd));

        if self.kind == WindowKind::Standard {
            let dark: i32 = 1;
            unsafe {
                let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const i32 as *const _, 4);
            }
            if self.system_backdrop.get() {
                apply_system_backdrop(hwnd, true);
            }
        }

        let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
        if scale > 0.0 && (scale - self.scale.get()).abs() > f32::EPSILON {
            self.apply_scale(scale);
        }
        if self.kind == WindowKind::Standard {
            // The size Slint asked for is the client area; grow the frame around it.
            self.resize_native(self.size.get());
        }
        Some(hwnd)
    }

    fn apply_scale(&self, scale: f32) {
        let logical = self.size.get().to_logical(self.scale.get());
        self.scale.set(scale);
        self.window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
        self.set_physical_size(logical.to_physical(scale), true);
    }

    /// Updates the size Slint renders at and, if requested, the native window.
    fn set_physical_size(&self, size: PhysicalSize, resize_native: bool) {
        if size == self.size.get() {
            return;
        }
        self.size.set(size);
        if resize_native {
            self.resize_native(size);
        }
        self.window.dispatch_event(WindowEvent::Resized { size: size.to_logical(self.scale.get()) });
        self.needs_redraw.set(true);
    }

    fn resize_native(&self, size: PhysicalSize) {
        let Some(hwnd) = self.hwnd.get() else { return };
        unsafe {
            match self.kind {
                WindowKind::Island => {
                    // Keep the island's horizontal centre fixed when its window changes width.
                    let mut rect = RECT::default();
                    let _ = GetWindowRect(hwnd, &mut rect);
                    let centre = (rect.left + rect.right) / 2;
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        centre - size.width as i32 / 2,
                        rect.top,
                        size.width as i32,
                        size.height as i32,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                WindowKind::Standard => {
                    let mut rect = RECT { left: 0, top: 0, right: size.width as i32, bottom: size.height as i32 };
                    let dpi = GetDpiForWindow(hwnd);
                    let _ = windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi(
                        &mut rect,
                        WS_OVERLAPPEDWINDOW,
                        false,
                        WINDOW_EX_STYLE::default(),
                        dpi,
                    );
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
        }
    }

    /// Renders and presents if Slint requested a redraw. Returns true if it drew.
    pub(super) fn render_if_needed(&self) -> bool {
        if !self.needs_redraw.get() {
            return false;
        }
        let Some(hwnd) = self.hwnd.get() else { return false };
        if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
            return false;
        }
        self.needs_redraw.set(false);
        let size = self.size.get();
        if size.width == 0 || size.height == 0 {
            return false;
        }
        let mut slot = self.surface.borrow_mut();
        if slot.as_ref().is_none_or(|s| s.width != size.width || s.height != size.height) {
            *slot = Surface::new(size.width, size.height);
            // A new buffer has no previous content to reuse.
            self.renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
            self.renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
        }
        let Some(surface) = slot.as_mut() else { return false };
        let region = self.renderer.render(surface.pixels(), size.width as usize);
        let origin = region.bounding_box_origin();
        let extent = region.bounding_box_size();
        if extent.width == 0 || extent.height == 0 {
            return true;
        }
        let dirty = RECT {
            left: origin.x,
            top: origin.y,
            right: origin.x + extent.width as i32,
            bottom: origin.y + extent.height as i32,
        };
        let area = (
            dirty.left.max(0) as usize,
            dirty.top.max(0) as usize,
            dirty.right.max(0) as usize,
            dirty.bottom.max(0) as usize,
        );
        let shapes = self.physical_shapes();
        // The glass blur, beneath what was just drawn.
        if let Some(backdrop) = self.backdrop.borrow().as_ref() {
            super::glass::compose(
                surface.pixels(),
                size.width as usize,
                area,
                &shapes,
                backdrop,
                self.refraction.get(),
            );
        }
        // Always composite smooth SDF outer borders so rounded corners stay crisp in every theme.
        super::glass::compose_borders(surface.pixels(), size.width as usize, area, &shapes);
        match self.kind {
            WindowKind::Island => surface.present_layered(hwnd, Some(dirty)),
            WindowKind::Standard => unsafe {
                let dc = GetDC(Some(hwnd));
                surface.blit(dc, dirty);
                ReleaseDC(Some(hwnd), dc);
            },
        }
        drop(slot);
        true
    }

    /// Brings the window to the foreground so it receives keyboard input. Returns the
    /// window that was in the foreground before.
    pub fn activate(&self) -> Option<HWND> {
        let hwnd = self.hwnd.get()?;
        unsafe {
            let previous = GetForegroundWindow();
            let _ = SetForegroundWindow(hwnd);
            (!previous.is_invalid() && previous != hwnd).then_some(previous)
        }
    }

    /// Whether this window is the foreground window.
    pub fn is_foreground(&self) -> bool {
        self.hwnd.get().is_some_and(|hwnd| unsafe { GetForegroundWindow() } == hwnd)
    }

    /// Starts a native move loop. Must not be called from inside Slint event dispatch.
    pub fn begin_move(&self) {
        let Some(hwnd) = self.hwnd.get() else { return };
        unsafe {
            let mut rect = RECT::default();
            let _ = GetWindowRect(hwnd, &mut rect);
            self.drag_top.set(Some(rect.top));
            let mut pointer = POINT::default();
            self.drag_origin.set(
                GetCursorPos(&mut pointer).ok().map(|()| super::placement::HorizontalDrag::new(rect.left, pointer.x)),
            );
            let _ = ReleaseCapture();
            // Keep Slint feedback painting while Windows owns the modal move loop.
            let _ = SetTimer(Some(hwnd), 0x4c49, 16, None);
            // Runs a modal loop until the mouse button is released.
            SendMessageW(hwnd, WM_SYSCOMMAND, Some(WPARAM((SC_MOVE | 2) as usize)), Some(LPARAM(0)));
            let _ = KillTimer(Some(hwnd), 0x4c49);
            self.drag_top.set(None);
            self.drag_origin.set(None);
        }
        // The release happened inside the system loop; reset mouse tracking and hover
        // state so re-entering the pill immediately updates hover and auto-hide timers.
        let position = self.last_pointer.get();
        self.tracking_mouse.set(false);
        self.window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
        self.window.dispatch_event(WindowEvent::PointerExited);
        self.notify(Notice::Pointer(false));
    }

    fn logical_point(&self, lparam: LPARAM) -> LogicalPosition {
        let x = (lparam.0 & 0xffff) as i16 as f32;
        let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
        let scale = self.scale.get();
        LogicalPosition::new(x / scale, y / scale)
    }

    fn pointer(&self, event: WindowEvent) {
        if let Some(position) = event.position() {
            self.last_pointer.set(position);
        }
        self.window.dispatch_event(event);
    }

    fn key(&self, pressed: bool, repeat: bool, text: slint::SharedString) {
        let event = match (pressed, repeat) {
            (true, false) => WindowEvent::KeyPressed { text },
            (true, true) => WindowEvent::KeyPressRepeated { text },
            (false, _) => WindowEvent::KeyReleased { text },
        };
        self.window.dispatch_event(event);
    }

    /// Handles a message for this window; `None` falls through to `DefWindowProcW`.
    pub(super) fn handle(&self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        let button = |b| Some(b);
        match msg {
            WM_MOUSEACTIVATE if self.kind == WindowKind::Island => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_MOUSEMOVE => {
                if !self.tracking_mouse.replace(true) {
                    self.notify(Notice::Pointer(true));
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut tme);
                    }
                }
                self.pointer(WindowEvent::PointerMoved { position: self.logical_point(lparam) });
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.tracking_mouse.set(false);
                self.window.dispatch_event(WindowEvent::PointerExited);
                self.notify(Notice::Pointer(false));
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP => {
                let which = match msg {
                    WM_LBUTTONDOWN | WM_LBUTTONUP => button(PointerEventButton::Left),
                    WM_RBUTTONDOWN | WM_RBUTTONUP => button(PointerEventButton::Right),
                    _ => button(PointerEventButton::Middle),
                }?;
                let position = self.logical_point(lparam);
                if matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
                    unsafe { SetCapture(hwnd) };
                    self.pointer(WindowEvent::PointerPressed { position, button: which });
                } else {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    self.pointer(WindowEvent::PointerReleased { position, button: which });
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                let delta = input::wheel_pixels(((wparam.0 >> 16) & 0xffff) as i16);
                let mut point =
                    POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
                unsafe {
                    let _ = ScreenToClient(hwnd, &mut point);
                }
                let scale = self.scale.get();
                let position = LogicalPosition::new(point.x as f32 / scale, point.y as f32 / scale);
                let (delta_x, delta_y) = if msg == WM_MOUSEWHEEL { (0.0, delta) } else { (-delta, 0.0) };
                self.pointer(WindowEvent::PointerScrolled { position, delta_x, delta_y });
                Some(LRESULT(0))
            }
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
                let vk = VIRTUAL_KEY(wparam.0 as u16);
                let pressed = matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN);
                let consume_sys = self.consume_sys_keys.get();
                let repeat = pressed && (lparam.0 >> 30) & 1 == 1;
                let text = input::special_key(vk).or_else(|| {
                    // With Ctrl or Alt held, WM_CHAR carries control codes; report the base
                    // character instead. Plain typing is delivered through WM_CHAR.
                    let chord = input::modifier_held(VK_CONTROL) || input::modifier_held(VK_MENU);
                    if chord || !pressed { input::base_character(vk) } else { None }
                });
                if let Some(text) = text {
                    self.key(pressed, repeat, text);
                }
                if msg == WM_SYSKEYDOWN && consume_sys {
                    self.suppress_sys_keyup.set(true);
                    Some(LRESULT(0))
                } else if msg == WM_SYSKEYUP && (consume_sys || self.suppress_sys_keyup.get()) {
                    if matches!(vk, VK_MENU | VK_LMENU | VK_RMENU) || !input::modifier_held(VK_MENU) {
                        self.suppress_sys_keyup.set(false);
                    }
                    Some(LRESULT(0))
                } else if matches!(msg, WM_SYSKEYDOWN | WM_SYSKEYUP) {
                    // Let DefWindowProc see normal Alt combinations (e.g. Alt+F4) and system keys.
                    None
                } else {
                    Some(LRESULT(0))
                }
            }
            WM_CHAR => {
                let chord = input::modifier_held(VK_CONTROL) || input::modifier_held(VK_MENU);
                if let Some(c) = char::from_u32(wparam.0 as u32).filter(|c| !c.is_control())
                    && !chord
                {
                    let repeat = (lparam.0 >> 30) & 1 == 1;
                    self.key(true, repeat, c.to_string().into());
                }
                Some(LRESULT(0))
            }
            WM_ACTIVATE => {
                let active = (wparam.0 & 0xffff) as u32 != WA_INACTIVE;
                self.window.dispatch_event(WindowEvent::WindowActiveChanged(active));
                self.notify(Notice::Activated(active));
                None
            }
            WM_SIZE if self.kind == WindowKind::Standard => {
                let size = PhysicalSize::new((lparam.0 & 0xffff) as u32, ((lparam.0 >> 16) & 0xffff) as u32);
                if size.width > 0 && size.height > 0 {
                    self.set_physical_size(size, false);
                }
                Some(LRESULT(0))
            }
            WM_GETMINMAXINFO if self.kind == WindowKind::Standard => {
                let min = self.min_size.get();
                if min.width > 0 && min.height > 0 {
                    let mut rect = RECT { left: 0, top: 0, right: min.width as i32, bottom: min.height as i32 };
                    unsafe {
                        let dpi = GetDpiForWindow(hwnd);
                        let _ = windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi(
                            &mut rect,
                            WS_OVERLAPPEDWINDOW,
                            false,
                            WINDOW_EX_STYLE::default(),
                            dpi,
                        );
                        let info = &mut *(lparam.0 as *mut MINMAXINFO);
                        info.ptMinTrackSize = POINT { x: rect.right - rect.left, y: rect.bottom - rect.top };
                    }
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                let scale = (wparam.0 & 0xffff) as f32 / 96.0;
                if self.kind == WindowKind::Standard {
                    let suggested = unsafe { &*(lparam.0 as *const RECT) };
                    unsafe {
                        let _ = SetWindowPos(
                            hwnd,
                            None,
                            suggested.left,
                            suggested.top,
                            suggested.right - suggested.left,
                            suggested.bottom - suggested.top,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                    let logical = self.size.get().to_logical(self.scale.get());
                    self.scale.set(scale);
                    self.window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
                    self.set_physical_size(logical.to_physical(scale), false);
                } else {
                    self.apply_scale(scale);
                }
                self.notify(Notice::ScaleChanged);
                Some(LRESULT(0))
            }
            WM_PAINT if self.kind == WindowKind::Standard => {
                let mut paint = PAINTSTRUCT::default();
                unsafe {
                    let dc = BeginPaint(hwnd, &mut paint);
                    match self.surface.borrow().as_ref() {
                        Some(surface) => surface.blit(dc, paint.rcPaint),
                        None => self.needs_redraw.set(true),
                    }
                    let _ = EndPaint(hwnd, &paint);
                }
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_SETCURSOR if (lparam.0 & 0xffff) as u32 == HTCLIENT => {
                unsafe {
                    let _ = SetCursor(LoadCursorW(None, IDC_ARROW).ok());
                }
                Some(LRESULT(1))
            }
            WM_CLOSE => {
                self.window.dispatch_event(WindowEvent::CloseRequested);
                Some(LRESULT(0))
            }
            WM_TIMER if wparam.0 == 0x4c49 && self.drag_top.get().is_some() => {
                slint::platform::update_timers_and_animations();
                self.render_if_needed();
                Some(LRESULT(0))
            }
            WM_MOVING => {
                if let Some(top) = self.drag_top.get() {
                    let rect = unsafe { &mut *(lparam.0 as *mut RECT) };
                    let height = rect.bottom - rect.top;
                    rect.top = top;
                    rect.bottom = top + height;
                    let monitors = super::placement::monitors();
                    let width = rect.right - rect.left;
                    let mut pointer = POINT::default();
                    let proposed = self
                        .drag_origin
                        .get()
                        .filter(|_| unsafe { GetCursorPos(&mut pointer) }.is_ok())
                        .map(|origin| origin.left_at(pointer.x));
                    if let Some(left) = proposed {
                        rect.left = left;
                        rect.right = left + width;
                    }
                    let snapped = proposed.and_then(|left| {
                        super::placement::monitor_at(&monitors, left + width / 2, top + 1)
                            .and_then(|monitor| super::placement::snap_center(monitor, left, width))
                    });
                    if let Some(left) = snapped {
                        rect.left = left;
                        rect.right = left + width;
                    }
                    self.notify(Notice::CenterSnap(snapped.is_some()));
                    return Some(LRESULT(1));
                }
                None
            }
            WM_EXITSIZEMOVE if self.kind == WindowKind::Island => {
                self.notify(Notice::CenterSnap(false));
                self.notify(Notice::MoveFinished);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

impl WindowAdapter for Win32Window {
    fn window(&self) -> &slint::Window {
        &self.window
    }

    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }

    fn size(&self) -> PhysicalSize {
        self.size.get()
    }

    fn set_size(&self, size: WindowSize) {
        self.set_physical_size(size.to_physical(self.scale.get()), true);
    }

    fn position(&self) -> Option<PhysicalPosition> {
        let hwnd = self.hwnd.get()?;
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
        Some(PhysicalPosition::new(rect.left, rect.top))
    }

    fn set_position(&self, position: WindowPosition) {
        let position = position.to_physical(self.scale.get());
        self.position.set(Some(position));
        if let Some(hwnd) = self.hwnd.get() {
            unsafe {
                let _ =
                    SetWindowPos(hwnd, None, position.x, position.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
    }

    fn set_visible(&self, visible: bool) -> Result<(), slint::PlatformError> {
        if visible {
            let hwnd =
                self.ensure_hwnd().ok_or_else(|| slint::PlatformError::Other("window creation failed".into()))?;
            self.needs_redraw.set(true);
            unsafe {
                let _ = ShowWindow(hwnd, if self.kind == WindowKind::Island { SW_SHOWNOACTIVATE } else { SW_SHOW });
            }
            if self.kind == WindowKind::Island {
                // Present immediately so the window never appears empty.
                self.render_if_needed();
            }
        } else if let Some(hwnd) = self.hwnd.get() {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
        Ok(())
    }

    fn request_redraw(&self) {
        self.needs_redraw.set(true);
    }

    fn update_window_properties(&self, properties: WindowProperties<'_>) {
        let scale = self.scale.get();
        let constraints = properties.layout_constraints();
        if let Some(min) = constraints.min {
            self.min_size.set(min.to_physical(scale));
        }
        let size = self.size.get();
        let fixed = constraints.min.is_some() && constraints.min == constraints.max;
        if (fixed || size.width == 0 || size.height == 0)
            && let Some(preferred) = if fixed { constraints.min } else { Some(constraints.preferred) }
        {
            let physical = preferred.to_physical(scale);
            if physical.width > 0 && physical.height > 0 {
                self.set_physical_size(physical, true);
            }
        }
        if self.kind == WindowKind::Standard
            && let Some(hwnd) = self.hwnd.get()
        {
            let title = HSTRING::from(properties.title().as_str());
            unsafe {
                let _ = SetWindowTextW(hwnd, &title);
            }
        }
    }
}

impl Drop for Win32Window {
    fn drop(&mut self) {
        if let Some(hwnd) = self.hwnd.take() {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

/// Extends the frame over the client area and asks DWM for its acrylic material (Windows
/// 11 22H2 and later), or restores an ordinary opaque window.
fn apply_system_backdrop(hwnd: HWND, enabled: bool) -> bool {
    let extent = if enabled { -1 } else { 0 };
    let margins = MARGINS { cxLeftWidth: extent, cxRightWidth: extent, cyTopHeight: extent, cyBottomHeight: extent };
    let kind: DWM_SYSTEMBACKDROP_TYPE = if enabled { DWMSBT_TRANSIENTWINDOW } else { DWMSBT_AUTO };
    unsafe {
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
        let applied = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &kind as *const DWM_SYSTEMBACKDROP_TYPE as *const _,
            std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
        .is_ok();
        let _ = InvalidateRect(Some(hwnd), None, false);
        enabled && applied
    }
}

/// Window procedure shared by all Slint windows.
pub(super) unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(window) = lookup(hwnd)
        && let Some(result) = window.handle(hwnd, msg, wparam, lparam)
    {
        return result;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slint_draws_at_the_display_scale_from_the_start() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            TEST_SCALE.with(|s| s.set(Some(scale)));
            for kind in [WindowKind::Island, WindowKind::Standard] {
                let w = Win32Window::new(kind);
                // Slint draws at the scale the pointer coordinates are converted with.
                assert_eq!(w.window.scale_factor(), scale, "{kind:?} at {scale}");
                assert_eq!(w.scale.get(), scale);
                // A click on what Slint drew at logical (120, 40) arrives in physical
                // pixels and must map back to the same logical point.
                let (x, y) = ((120.0 * scale) as isize, (40.0 * scale) as isize);
                let point = w.logical_point(LPARAM((y << 16) | x));
                assert!((point.x - 120.0).abs() < 0.01 && (point.y - 40.0).abs() < 0.01, "{point:?} at {scale}");
            }
        }
        TEST_SCALE.with(|s| s.set(None));
    }
}
