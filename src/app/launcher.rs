//! Collapsed island App Launcher: scroll or middle-click on the compact island
//! to browse and launch frequently used applications, games, and custom configured apps.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, VecModel};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, w};

use crate::settings::Settings;
use crate::ui::{AppLauncherItem, AppState, Island};

/// Delay after last interaction before the compact island returns to its default tool/clock/media view.
const REVERT_DELAY: Duration = Duration::from_millis(3800);

#[derive(Clone, Debug, PartialEq)]
pub struct AppItem {
    pub name: String,
    pub path: String,
    pub icon_kind: String,
}

pub struct LauncherController {
    pub apps: RefCell<Vec<AppItem>>,
    revert_timer: slint::Timer,
}

impl LauncherController {
    pub fn new() -> Self {
        Self { apps: RefCell::new(Vec::new()), revert_timer: slint::Timer::default() }
    }

    /// Detects desktop shortcuts, popular installed applications, and user-configured custom apps.
    pub fn detect_apps(&self, settings: &Settings) {
        let mut detected = Vec::new();

        // 1. User custom apps from settings come first
        for custom in &settings.launcher.custom_apps {
            if !custom.name.trim().is_empty() && !custom.path.trim().is_empty() {
                detected.push(AppItem {
                    name: custom.name.clone(),
                    path: custom.path.clone(),
                    icon_kind: classify_icon(&custom.name, &custom.path),
                });
            }
        }

        // 2. Scan Desktop and Start Menu shortcuts (.lnk, .url)
        let search_dirs = [
            std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Desktop")),
            std::env::var_os("PUBLIC").map(|p| PathBuf::from(p).join("Desktop")),
            std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs")),
            std::env::var_os("PROGRAMDATA").map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs")),
        ];

        for dir in search_dirs.into_iter().flatten() {
            scan_shortcuts(&dir, 0, &mut detected);
        }

        // 3. Scan well-known daily developer, gaming, and creative tools
        let common_apps: &[(&str, &str, &str)] = &[
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

        for &(name, binary, kind) in common_apps {
            if !detected.iter().any(|a| a.name.eq_ignore_ascii_case(name)) && is_available(binary) {
                detected.push(AppItem { name: name.to_owned(), path: binary.to_owned(), icon_kind: kind.to_owned() });
            }
        }

        // Fallback default apps if empty
        if detected.is_empty() {
            detected.push(AppItem {
                name: "File Explorer".into(),
                path: "explorer.exe".into(),
                icon_kind: "app".into(),
            });
            detected.push(AppItem { name: "Terminal".into(), path: "wt.exe".into(), icon_kind: "terminal".into() });
            detected.push(AppItem { name: "Notepad".into(), path: "notepad.exe".into(), icon_kind: "app".into() });
        }

        // Deduplicate by name
        let mut unique: Vec<AppItem> = Vec::new();
        for item in detected {
            if !unique.iter().any(|u| u.name.eq_ignore_ascii_case(&item.name)) {
                unique.push(item);
            }
        }

        unique.truncate(20);
        *self.apps.borrow_mut() = unique;
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
            })
            .collect();
        state.set_launcher_apps(ModelRc::new(VecModel::from(apps)));
    }

    /// Scrolls to next/previous app with directional animation and restarts the auto-revert timer.
    pub fn scroll(&self, delta: i32, island: &Island) {
        let state = island.global::<AppState>();
        if !state.get_launcher_active() {
            state.set_launcher_active(true);
        }
        let total = self.apps.borrow().len();
        if total == 0 {
            return;
        }
        let current = state.get_launcher_index();
        let next = (current + delta).rem_euclid(total as i32);
        state.set_launcher_direction(delta);
        state.set_launcher_anim_token(state.get_launcher_anim_token() + 1);
        state.set_launcher_index(next);
        self.restart_revert_timer(island);
    }

    /// Toggles the launcher on the compact island.
    pub fn toggle(&self, island: &Island) {
        let state = island.global::<AppState>();
        let active = !state.get_launcher_active();
        state.set_launcher_active(active);
        if active {
            self.restart_revert_timer(island);
        } else {
            self.revert_timer.stop();
        }
    }

    /// Launches the selected app and closes the launcher view.
    pub fn launch(&self, index: i32, island: &Island) {
        let apps = self.apps.borrow();
        if let Some(app) = usize::try_from(index).ok().and_then(|i| apps.get(i)) {
            launch_process(&app.path);
        }
        let state = island.global::<AppState>();
        state.set_launcher_active(false);
        self.revert_timer.stop();
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
        let state = island.global::<AppState>();
        if state.get_launcher_active() {
            state.set_launcher_active(false);
            self.revert_timer.stop();
        }
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
                    detected.push(AppItem { name, path: path_str, icon_kind });
                }
            }
        }
    }
}

/// Checks if an executable or command is accessible on this PC.
fn is_available(target: &str) -> bool {
    if Path::new(target).exists() {
        return true;
    }
    // Check PATH
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let full = dir.join(target);
            if full.exists() {
                return true;
            }
        }
    }
    // Check LocalAppData Programs
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let full = PathBuf::from(local).join("Programs").join(target);
        if full.exists() {
            return true;
        }
    }
    false
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
}
