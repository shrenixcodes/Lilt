//! Collapsed island App Launcher: scroll or middle-click on the compact island
//! to browse and launch frequently used applications, games, and custom configured apps.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, VecModel};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HGDIOBJ,
    SelectObject,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::UI::Shell::{
    ExtractIconExW, SHFILEINFOW, SHGFI_DISPLAYNAME, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DI_NORMAL, DestroyIcon, DrawIconEx, HICON, PrivateExtractIconsW, SW_SHOWNORMAL,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::settings::Settings;
use crate::ui::{AppLauncherItem, AppState, Island};

/// Delay after last interaction before the compact island returns to its default tool/clock/media view.
const REVERT_DELAY: Duration = Duration::from_millis(3800);
const ICON_SIZE: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct AppItem {
    pub name: String,
    pub path: String,
    pub icon_kind: String,
    pub icon: Option<slint::Image>,
}

pub struct LauncherController {
    pub apps: RefCell<Vec<AppItem>>,
    revert_timer: slint::Timer,
}

impl LauncherController {
    pub fn new() -> Self {
        Self { apps: RefCell::new(Vec::new()), revert_timer: slint::Timer::default() }
    }

    /// Loads user-configured custom apps or initial defaults from settings.
    pub fn detect_apps(&self, settings: &Settings) {
        let mut detected = Vec::new();

        for custom in &settings.launcher.custom_apps {
            if !custom.name.trim().is_empty() && !custom.path.trim().is_empty() {
                detected.push(AppItem {
                    name: custom.name.clone(),
                    path: custom.path.clone(),
                    icon_kind: classify_icon(&custom.name, &custom.path),
                    icon: None,
                });
            }
        }

        detected.truncate(7);

        // Fetch genuine application icons
        for item in &mut detected {
            item.icon = extract_file_icon(&item.path);
        }

        *self.apps.borrow_mut() = detected;
    }

    /// Scans desktop shortcuts and well-known applications to populate initial shortcuts.
    pub fn scan_default_apps() -> Vec<crate::settings::CustomApp> {
        let mut detected = Vec::new();

        let search_dirs = [
            std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Desktop")),
            std::env::var_os("PUBLIC").map(|p| PathBuf::from(p).join("Desktop")),
            std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs")),
            std::env::var_os("PROGRAMDATA").map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs")),
        ];

        for dir in search_dirs.into_iter().flatten() {
            scan_shortcuts(&dir, 0, &mut detected);
        }

        for &(name, binary, kind) in COMMON_APPS {
            if !detected.iter().any(|a| a.name.eq_ignore_ascii_case(name)) && is_available(binary) {
                detected.push(AppItem {
                    name: name.to_owned(),
                    path: binary.to_owned(),
                    icon_kind: kind.to_owned(),
                    icon: None,
                });
            }
        }

        if detected.is_empty() {
            detected.push(AppItem {
                name: "File Explorer".into(),
                path: "explorer.exe".into(),
                icon_kind: "app".into(),
                icon: None,
            });
            detected.push(AppItem {
                name: "Terminal".into(),
                path: "wt.exe".into(),
                icon_kind: "terminal".into(),
                icon: None,
            });
            detected.push(AppItem {
                name: "Notepad".into(),
                path: "notepad.exe".into(),
                icon_kind: "app".into(),
                icon: None,
            });
        }

        let mut unique: Vec<crate::settings::CustomApp> = Vec::new();
        for item in detected {
            if !unique.iter().any(|u| u.name.eq_ignore_ascii_case(&item.name)) {
                unique.push(crate::settings::CustomApp { name: item.name, path: item.path });
            }
        }
        unique.truncate(7);
        unique
    }

    /// Publishes detected apps to Slint AppState.
    pub fn publish(&self, island: &Island) {
        let state = island.global::<AppState>();
        let apps: Vec<AppLauncherItem> = self
            .apps
            .borrow()
            .iter()
            .map(|a| AppLauncherItem {
                name: a.name.as_str().into(),
                path: a.path.as_str().into(),
                icon_kind: a.icon_kind.as_str().into(),
                has_icon: a.icon.is_some(),
                icon: a.icon.clone().unwrap_or_default(),
            })
            .collect();
        state.set_launcher_apps(ModelRc::new(VecModel::from(apps)));
    }

    /// Scrolls to next/previous app with directional animation and restarts the auto-revert timer.
    /// Includes the initial music/idle view as a step in the infinite scroll loop.
    pub fn scroll(&self, delta: i32, island: &Island) {
        let total = self.apps.borrow().len();
        if total == 0 {
            return;
        }
        let state = island.global::<AppState>();
        let is_active = state.get_launcher_active();

        if !is_active {
            let start_idx = if delta > 0 { 0 } else { total as i32 - 1 };
            state.set_launcher_direction(delta);
            state.set_launcher_anim_token(state.get_launcher_anim_token() + 1);
            state.set_launcher_index(start_idx);
            state.set_launcher_active(true);
            self.restart_revert_timer(island);
        } else {
            let current = state.get_launcher_index();
            if delta > 0 {
                if current + 1 < total as i32 {
                    state.set_launcher_direction(delta);
                    state.set_launcher_anim_token(state.get_launcher_anim_token() + 1);
                    state.set_launcher_index(current + 1);
                    self.restart_revert_timer(island);
                } else {
                    state.set_launcher_active(false);
                    self.revert_timer.stop();
                }
            } else if current > 0 {
                state.set_launcher_direction(delta);
                state.set_launcher_anim_token(state.get_launcher_anim_token() + 1);
                state.set_launcher_index(current - 1);
                self.restart_revert_timer(island);
            } else {
                state.set_launcher_active(false);
                self.revert_timer.stop();
            }
        }
    }

    /// Toggles the launcher on the compact island.
    pub fn toggle(&self, island: &Island) {
        let state = island.global::<AppState>();
        let active = !state.get_launcher_active();
        if active {
            state.set_launcher_direction(0);
            state.set_launcher_anim_token(state.get_launcher_anim_token() + 1);
            state.set_launcher_active(true);
            self.restart_revert_timer(island);
        } else {
            self.reset(island);
        }
    }

    /// Resets the compact launcher back to the default clock/media view.
    pub fn reset(&self, island: &Island) {
        let state = island.global::<AppState>();
        if state.get_launcher_active() {
            state.set_launcher_active(false);
        }
        self.revert_timer.stop();
    }

    /// Launches the selected app and closes the launcher view.
    pub fn launch(&self, index: i32, island: &Island) {
        let apps = self.apps.borrow();
        if let Some(app) = usize::try_from(index).ok().and_then(|i| apps.get(i)) {
            launch_process(&app.path);
        }
        self.reset(island);
    }

    /// Starts or restarts the auto-revert timer.
    pub fn restart_revert_timer(&self, island: &Island) {
        let island_weak = island.as_weak();
        self.revert_timer.start(slint::TimerMode::SingleShot, REVERT_DELAY, move || {
            if let Some(island) = island_weak.upgrade() {
                let state = island.global::<AppState>();
                state.set_launcher_active(false);
            }
        });
    }

    /// Invoked when pointer leaves the island; resets the launcher back to default tool view.
    pub fn on_pointer_left(&self, island: &Island) {
        self.reset(island);
    }
}

/// Categorizes an app icon into a visual icon family for display.
fn classify_icon(name: &str, path: &str) -> String {
    let lower = format!("{name} {path}").to_ascii_lowercase();
    if lower.contains("code") || lower.contains("studio") || lower.contains("rust") || lower.contains("py") {
        "code".into()
    } else if lower.contains("game")
        || lower.contains("steam")
        || lower.contains("epic")
        || lower.contains("play")
        || lower.contains("riot")
        || lower.contains("craft")
        || lower.contains("strike")
    {
        "game".into()
    } else if lower.contains("photo")
        || lower.contains("draw")
        || lower.contains("blender")
        || lower.contains("paint")
        || lower.contains("art")
        || lower.contains("figma")
    {
        "art".into()
    } else if lower.contains("term") || lower.contains("bash") || lower.contains("cmd") || lower.contains("powershell")
    {
        "terminal".into()
    } else if lower.contains("chrome")
        || lower.contains("edge")
        || lower.contains("brave")
        || lower.contains("firefox")
        || lower.contains("browser")
    {
        "browser".into()
    } else {
        "app".into()
    }
}

pub const COMMON_APPS: &[(&str, &str, &str)] = &[
    ("VS Code", "code.cmd", "code"),
    ("Terminal", "wt.exe", "terminal"),
    ("Discord", "Discord.exe", "app"),
    ("Spotify", "spotify.exe", "app"),
    ("Steam", "steam.exe", "game"),
    ("Google Chrome", "chrome.exe", "browser"),
    ("Microsoft Edge", "msedge.exe", "browser"),
    ("File Explorer", "explorer.exe", "app"),
    ("Notepad", "notepad.exe", "app"),
    ("Calculator", "calc.exe", "app"),
];

/// Resolves a friendly display name for an executable or shortcut target.
pub fn resolve_app_name(target: &str) -> String {
    let clean_target = target.trim().trim_matches('"');
    let path = Path::new(clean_target);

    // 1. If it's a shortcut (.lnk/.url), the filename stem is already the friendly name.
    let is_shortcut = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk") || ext.eq_ignore_ascii_case("url"));
    if is_shortcut {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().trim();
        if !stem.is_empty() {
            return stem.to_owned();
        }
    }

    // 2. Check well-known common app binary names
    let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or(clean_target);
    for &(name, binary, _) in COMMON_APPS {
        if filename.eq_ignore_ascii_case(binary) || clean_target.eq_ignore_ascii_case(binary) {
            return name.to_owned();
        }
    }

    // 3. Try SHGetFileInfoW for shell display name
    if let Some(resolved) = resolve_executable_path(clean_target) {
        let wide: Vec<u16> = resolved.to_string_lossy().encode_utf16().chain(Some(0)).collect();
        let mut shinfo = SHFILEINFOW::default();
        let result = unsafe {
            SHGetFileInfoW(
                PCWSTR(wide.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut shinfo),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_DISPLAYNAME,
            )
        };
        if result != 0 {
            let len = shinfo.szDisplayName.iter().position(|&c| c == 0).unwrap_or(shinfo.szDisplayName.len());
            let name = String::from_utf16_lossy(&shinfo.szDisplayName[..len]).trim().to_string();
            if !name.is_empty() {
                let stripped = name
                    .strip_suffix(".exe")
                    .or_else(|| name.strip_suffix(".lnk"))
                    .or_else(|| name.strip_suffix(".url"))
                    .unwrap_or(&name);
                if !stripped.is_empty() {
                    return stripped.to_owned();
                }
            }
        }
    }

    // 4. Fallback: file stem capitalized
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        let s = stem.trim();
        if !s.is_empty() {
            let mut chars = s.chars();
            if let Some(first) = chars.next() {
                return format!("{}{}", first.to_uppercase(), chars.as_str());
            }
        }
    }

    clean_target.to_owned()
}

/// Recursively scans for desktop and start menu shortcuts up to a limited depth.
fn scan_shortcuts(dir: &Path, depth: u32, detected: &mut Vec<AppItem>) {
    if depth > 2 {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_shortcuts(&path, depth + 1, detected);
            } else if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                let is_shortcut = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk") || ext.eq_ignore_ascii_case("url"));
                if is_shortcut {
                    let lower = stem.to_ascii_lowercase();
                    if lower.contains("lilt")
                        || lower.contains("uninstall")
                        || lower.contains("readme")
                        || lower.contains("documentation")
                        || lower.contains("help")
                    {
                        continue;
                    }
                    let name = stem.to_owned();
                    let path_str = path.to_string_lossy().into_owned();
                    let icon_kind = classify_icon(&name, &path_str);
                    detected.push(AppItem { name, path: path_str, icon_kind, icon: None });
                }
            }
        }
    }
}

/// Draws an HICON onto a 32x32 32-bit RGBA bitmap buffer.
fn draw_hicon(icon: HICON) -> Option<Vec<u8>> {
    let mut info = BITMAPINFO::default();
    info.bmiHeader.biSize = std::mem::size_of_val(&info.bmiHeader) as u32;
    info.bmiHeader.biWidth = ICON_SIZE as i32;
    info.bmiHeader.biHeight = -(ICON_SIZE as i32);
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB.0;
    let mut bits = std::ptr::null_mut();
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
    let dc = unsafe { CreateCompatibleDC(None) };
    if dc.is_invalid() {
        let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
        return None;
    }
    let previous = unsafe { SelectObject(dc, HGDIOBJ(bitmap.0)) };
    let drawn = unsafe { DrawIconEx(dc, 0, 0, icon, ICON_SIZE as i32, ICON_SIZE as i32, 0, None, DI_NORMAL) }.is_ok();
    let pixels = if drawn && !bits.is_null() {
        let bgra = unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), ICON_SIZE * ICON_SIZE * 4) };
        let mut rgba = Vec::with_capacity(bgra.len());
        for pixel in bgra.as_chunks::<4>().0 {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
        Some(rgba)
    } else {
        None
    };
    unsafe {
        let _ = SelectObject(dc, previous);
        let _ = DeleteDC(dc);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
    }
    pixels
}

/// Extracts the true application icon from an executable or shortcut (.lnk/.url).
pub fn extract_file_icon(target: &str) -> Option<slint::Image> {
    let resolved = resolve_executable_path(target)?;
    let path_str = resolved.to_string_lossy();
    let wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

    // 1. Try SHGetFileInfoW
    let mut shinfo = SHFILEINFOW::default();
    let result = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut shinfo),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if result != 0 && !shinfo.hIcon.is_invalid() {
        if let Some(pixels) = draw_hicon(shinfo.hIcon) {
            let _ = unsafe { DestroyIcon(shinfo.hIcon) };
            if pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0) {
                let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                    &pixels,
                    ICON_SIZE as u32,
                    ICON_SIZE as u32,
                );
                return Some(slint::Image::from_rgba8(buf));
            }
        } else {
            let _ = unsafe { DestroyIcon(shinfo.hIcon) };
        }
    }

    // 2. Try PrivateExtractIconsW
    if wide.len() <= 260 {
        let mut filename = [0u16; 260];
        filename[..wide.len()].copy_from_slice(&wide);
        let mut icon = HICON::default();
        let _ = unsafe {
            PrivateExtractIconsW(
                &filename,
                0,
                ICON_SIZE as i32,
                ICON_SIZE as i32,
                Some(std::slice::from_mut(&mut icon)),
                None,
                0,
            )
        };
        if !icon.is_invalid() {
            if let Some(pixels) = draw_hicon(icon) {
                let _ = unsafe { DestroyIcon(icon) };
                if pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0) {
                    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                        &pixels,
                        ICON_SIZE as u32,
                        ICON_SIZE as u32,
                    );
                    return Some(slint::Image::from_rgba8(buf));
                }
            } else {
                let _ = unsafe { DestroyIcon(icon) };
            }
        }
    }

    // 3. Try ExtractIconExW
    let mut icon = HICON::default();
    let hpath = HSTRING::from(path_str.as_ref());
    if unsafe { ExtractIconExW(&hpath, 0, Some(&mut icon), None, 1) } > 0 && !icon.is_invalid() {
        if let Some(pixels) = draw_hicon(icon) {
            let _ = unsafe { DestroyIcon(icon) };
            if pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0) {
                let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                    &pixels,
                    ICON_SIZE as u32,
                    ICON_SIZE as u32,
                );
                return Some(slint::Image::from_rgba8(buf));
            }
        } else {
            let _ = unsafe { DestroyIcon(icon) };
        }
    }

    None
}

/// Resolves an executable or shortcut target from PATH or known system directories.
fn resolve_executable_path(target: &str) -> Option<PathBuf> {
    let p = Path::new(target);
    if p.exists() {
        return Some(p.to_path_buf());
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let full = dir.join(target);
            if full.exists() {
                return Some(full);
            }
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let full = PathBuf::from(local).join("Programs").join(target);
        if full.exists() {
            return Some(full);
        }
    }
    if let Some(windir) = std::env::var_os("WINDIR") {
        let full = PathBuf::from(windir).join("System32").join(target);
        if full.exists() {
            return Some(full);
        }
    }
    None
}

/// Checks if an executable or command is accessible on this PC.
fn is_available(target: &str) -> bool {
    resolve_executable_path(target).is_some()
}

/// Executes an application, game, or shortcut via Windows ShellExecute.
fn launch_process(target: &str) {
    let wide: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = ShellExecuteW(None, w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{CustomApp, Settings};

    #[test]
    fn test_classify_icon_categories() {
        assert_eq!(classify_icon("VS Code", "code.exe"), "code");
        assert_eq!(classify_icon("Counter-Strike 2", "cs2.exe"), "game");
        assert_eq!(classify_icon("Adobe Photoshop", "photoshop.exe"), "art");
        assert_eq!(classify_icon("Windows Terminal", "wt.exe"), "terminal");
        assert_eq!(classify_icon("Google Chrome", "chrome.exe"), "browser");
        assert_eq!(classify_icon("Notepad", "notepad.exe"), "app");
    }

    #[test]
    fn test_custom_apps_are_prioritized() {
        let mut settings = Settings::default();
        settings
            .launcher
            .custom_apps
            .push(CustomApp { name: "My Custom Tool".into(), path: "C:\\tools\\mytool.exe".into() });

        let controller = LauncherController::new();
        controller.detect_apps(&settings);

        let apps = controller.apps.borrow();
        assert!(!apps.is_empty());
        assert_eq!(apps[0].name, "My Custom Tool");
        assert_eq!(apps[0].path, "C:\\tools\\mytool.exe");
    }

    #[test]
    fn test_launcher_limits_to_seven_shortcuts() {
        let mut settings = Settings::default();
        for i in 1..=12 {
            settings.launcher.custom_apps.push(CustomApp { name: format!("App {i}"), path: format!("C:\\app{i}.exe") });
        }

        let controller = LauncherController::new();
        controller.detect_apps(&settings);

        let apps = controller.apps.borrow();
        assert_eq!(apps.len(), 7);
    }

    #[test]
    fn test_resolve_app_name() {
        assert_eq!(resolve_app_name("wt.exe"), "Terminal");
        assert_eq!(resolve_app_name("code.cmd"), "VS Code");
        assert_eq!(resolve_app_name("notepad.exe"), "Notepad");
        assert_eq!(resolve_app_name("C:\\Users\\test\\Desktop\\Discord.lnk"), "Discord");
    }
}
