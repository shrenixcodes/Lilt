//! The live blur behind the Liquid Glass island.
//!
//! DWM's own materials (acrylic, Mica) fill a window's whole rectangle and ignore window
//! regions, so they cannot follow the island's pill shape. Instead a second window sits
//! directly beneath the island. It is layered and transparent to input, so it never
//! intercepts clicks, and it hosts a single Windows.UI.Composition visual: the host
//! backdrop brush (the blurred content behind the window) clipped to a rounded rectangle
//! that tracks the island's shape. DWM composites it on the GPU; Lilt only updates the
//! shape when the island's outline changes.

use std::cell::{Cell, RefCell};

use windows::System::DispatcherQueueController;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{CompositionRoundedRectangleGeometry, Compositor, SpriteVisual};
use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmSetWindowAttribute};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Interface, w};
use windows_numerics::{Vector2, Vector3};

thread_local! {
    /// The compositor, created on first use. A thread can host only one dispatcher queue,
    /// so it is kept for the life of the thread rather than recreated on every switch.
    static COMPOSITOR: RefCell<Option<(DispatcherQueueController, Compositor)>> = const { RefCell::new(None) };
}

fn compositor() -> windows::core::Result<Compositor> {
    COMPOSITOR.with(|slot| {
        if let Some((_, compositor)) = slot.borrow().as_ref() {
            return Ok(compositor.clone());
        }
        let options = DispatcherQueueOptions {
            dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_NONE,
        };
        let controller = unsafe { CreateDispatcherQueueController(options)? };
        let compositor = Compositor::new()?;
        *slot.borrow_mut() = Some((controller, compositor.clone()));
        Ok(compositor)
    })
}

/// The island's outline in physical pixels, relative to the island window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
}

pub struct Backdrop {
    hwnd: HWND,
    _target: DesktopWindowTarget,
    visual: SpriteVisual,
    geometry: CompositionRoundedRectangleGeometry,
    shape: Cell<Option<Shape>>,
}

impl Backdrop {
    pub fn new() -> windows::core::Result<Self> {
        let compositor = compositor()?;
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOREDIRECTIONBITMAP
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE,
                super::BACKDROP_CLASS,
                w!("Lilt"),
                WS_POPUP,
                -32000,
                -32000,
                1,
                1,
                None,
                None,
                GetModuleHandleW(None).ok().map(Into::into),
                None,
            )?
        };
        // Built step by step so that the window is destroyed if any step fails.
        let destroy = |err: windows::core::Error| {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            err
        };
        let build = || -> windows::core::Result<Self> {
            unsafe {
                // Layered and transparent: every click reaches the island or what is below.
                SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
                let on = BOOL(1);
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_HOSTBACKDROPBRUSH,
                    &on as *const BOOL as *const _,
                    std::mem::size_of::<BOOL>() as u32,
                )?;
            }
            let interop: ICompositorDesktopInterop = compositor.cast()?;
            let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, true)? };
            let visual = compositor.CreateSpriteVisual()?;
            visual.SetBrush(&compositor.CreateHostBackdropBrush()?)?;
            let geometry = compositor.CreateRoundedRectangleGeometry()?;
            visual.SetClip(&compositor.CreateGeometricClipWithGeometry(&geometry)?)?;
            target.SetRoot(&visual)?;
            Ok(Self { hwnd, _target: target, visual, geometry, shape: Cell::new(None) })
        };
        build().map_err(destroy)
    }

    /// Clips the blur to the island's outline. Cheap when nothing changed.
    pub fn set_shape(&self, shape: Shape) {
        if self.shape.replace(Some(shape)) == Some(shape) {
            return;
        }
        // Inset half a pixel so the blur never shows outside the island's antialiased edge.
        const INSET: f32 = 0.5;
        let size = Vector2 { X: (shape.width - 2.0 * INSET).max(0.0), Y: (shape.height - 2.0 * INSET).max(0.0) };
        let radius = (shape.radius - INSET).max(0.0);
        let result = (|| -> windows::core::Result<()> {
            self.visual.SetOffset(Vector3 { X: shape.x + INSET, Y: shape.y + INSET, Z: 0.0 })?;
            self.visual.SetSize(size)?;
            self.geometry.SetSize(size)?;
            self.geometry.SetCornerRadius(Vector2 { X: radius, Y: radius })
        })();
        if let Err(err) = result {
            tracing::debug!(%err, "backdrop shape not updated");
        }
    }

    /// Places the backdrop exactly beneath the island, with the island's visibility.
    pub fn follow(&self, island: HWND) {
        unsafe {
            let mut rect = RECT::default();
            let _ = GetWindowRect(island, &mut rect);
            let visibility = if IsWindowVisible(island).as_bool() { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
            let _ = SetWindowPos(
                self.hwnd,
                Some(island),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOACTIVATE | visibility,
            );
        }
    }
}

/// Window procedure of the backdrop window: it never takes input or activation.
pub(super) unsafe extern "system" fn backdrop_proc(
    hwnd: HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
