use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::ComponentHandle;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

use super::App;
use crate::ui::{AppActions, AppState, SettingsWindow};
use crate::updates;

thread_local! {
    /// The latest release we found, kept so the "Install update" action can act on it.
    static LATEST: RefCell<Option<updates::Release>> = const { RefCell::new(None) };
}

impl App {
    pub(super) fn connect_updates(app: &Rc<Self>) {
        let weak = Rc::downgrade(app);
        app.island.global::<AppActions>().on_check_for_updates(move || {
            if let Some(app) = weak.upgrade() {
                App::check_updates(&app, false);
            }
        });
        let weak = Rc::downgrade(app);
        app.island.global::<AppActions>().on_open_update(move || {
            if let Some(app) = weak.upgrade() {
                App::open_update(&app);
            }
        });

        let weak = Rc::downgrade(app);
        app.island.global::<AppActions>().on_install_update(move || {
            if let Some(app) = weak.upgrade() {
                App::install_update(&app);
            }
        });

        let weak = Rc::downgrade(app);
        app.update_timer.start(slint::TimerMode::Repeated, Duration::from_secs(30 * 60), move || {
            if let Some(app) = weak.upgrade() {
                App::check_updates(&app, false);
            }
        });
        App::check_updates(app, !cfg!(feature = "dev-build"));
    }

    pub(super) fn connect_settings_updates(app: &Rc<Self>, window: &SettingsWindow) {
        let state = app.island.global::<AppState>();
        window.set_update_checking(state.get_update_checking());
        window.set_update_available(state.get_update_available());
        window.set_update_message(state.get_update_message());
        let weak = Rc::downgrade(app);
        window.on_check_for_updates(move || {
            if let Some(app) = weak.upgrade() {
                App::check_updates(&app, false);
            }
        });
        let weak = Rc::downgrade(app);
        window.on_install_update(move || {
            if let Some(app) = weak.upgrade() {
                App::install_update(&app);
            }
        });
        let weak = Rc::downgrade(app);
        window.on_open_update(move || {
            if let Some(app) = weak.upgrade() {
                App::open_update(&app);
            }
        });
    }

    fn open_update(app: &Rc<Self>) {
        App::set_expanded(app, false);
        let url = HSTRING::from(updates::RELEASES_URL);
        let result = unsafe { ShellExecuteW(None, w!("open"), &url, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
        if result.0 as isize <= 32 {
            tracing::warn!(code = result.0 as isize, "could not open GitHub updates");
        }
    }

    fn refresh_settings_updates(&self) {
        if let Some(window) = self.settings_window.borrow().as_ref() {
            let state = self.island.global::<AppState>();
            window.set_update_checking(state.get_update_checking());
            window.set_update_available(state.get_update_available());
            window.set_update_message(state.get_update_message());
        }
    }

    fn check_updates(app: &Rc<Self>, install_on_found: bool) {
        let state = app.island.global::<AppState>();
        if app.update_checking.replace(true) {
            return;
        }
        state.set_update_checking(true);
        state.set_update_message("Checking GitHub…".into());
        app.refresh_settings_updates();
        std::thread::spawn(move || {
            let result = updates::latest_release();
            super::post(move |app| {
                app.update_checking.set(false);
                let state = app.island.global::<AppState>();
                state.set_update_checking(false);
                match result {
                    Ok(release) => {
                        let available = updates::newer_version(updates::LOCAL_VERSION, &release.version);
                        state.set_update_available(available);
                        state.set_update_message(
                            if available {
                                format!("Update to {} available", release.version)
                            } else {
                                "Up to date".to_string()
                            }
                            .into(),
                        );
                        LATEST.with(|slot| slot.replace(Some(release)));
                        if available && install_on_found {
                            App::install_update(app);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "GitHub update check failed");
                        state.set_update_available(false);
                        state.set_update_message("Could not check for updates".into());
                    }
                }
                app.refresh_settings_updates();
            });
        });
    }

    fn install_update(app: &Rc<Self>) {
        let release = LATEST.with(|slot| slot.borrow().clone());
        let Some(release) = release else {
            app.island.global::<AppState>().set_update_message("Nothing downloaded yet".into());
            app.refresh_settings_updates();
            return;
        };
        let Some(url) = release.installer_url.clone() else {
            app.island
                .global::<AppState>()
                .set_update_message("This release has no Windows installer; open GitHub to download it.".into());
            app.refresh_settings_updates();
            return;
        };
        let sha_url = release.sha256_url.clone();
        let asset_sha256 = release.installer_sha256.clone();
        let target = updates::installer_target(&release.version);
        let state = app.island.global::<AppState>();
        state.set_update_checking(true);
        state.set_update_message(format!("Downloading {}…", release.version).into());
        app.refresh_settings_updates();
        std::thread::spawn(move || {
            let outcome = install_in_background(&url, sha_url.as_deref(), asset_sha256.as_deref(), &target);
            super::post(move |app| {
                let state = app.island.global::<AppState>();
                app.update_checking.set(false);
                state.set_update_checking(false);
                match outcome {
                    Ok(()) => {
                        state.set_update_message("Installing… Lilt will restart.".into());
                        // Give the UI one frame to render the message before we hand off.
                        let path = target.clone();
                        slint::Timer::single_shot(Duration::from_millis(80), move || {
                            if let Err(err) = updates::run_installer(&path) {
                                tracing::warn!(%err, "running the installer failed");
                            } else {
                                let _ = slint::quit_event_loop();
                            }
                        });
                    }
                    Err(error) => {
                        tracing::warn!(%error, "update install failed");
                        state.set_update_message(format!("Update failed: {error}").into());
                    }
                }
                app.refresh_settings_updates();
            });
        });
    }
}

fn install_in_background(
    url: &str,
    sha_url: Option<&str>,
    asset_sha256: Option<&str>,
    target: &std::path::Path,
) -> Result<(), String> {
    updates::download(url, target)?;
    if sha_url.is_none() && asset_sha256.is_none() {
        let _ = std::fs::remove_file(target);
        return Err("This release has no SHA-256 checksum; install cancelled.".into());
    }
    if let Some(sha_url) = sha_url {
        let sha_target = target.with_extension("exe.sha256");
        // Failing to fetch the checksum is non-fatal; without it we still trust the
        // host verification above.
        if let Ok(()) = updates::download(sha_url, &sha_target) {
            let text = std::fs::read_to_string(&sha_target).map_err(|e| e.to_string())?;
            if let Some(expected) = updates::sha256_from_text(&text) {
                let actual = updates::sha256_of(target)?;
                if !actual.eq_ignore_ascii_case(&expected) {
                    let _ = std::fs::remove_file(target);
                    return Err("checksum mismatch — install cancelled.".into());
                }
            }
            let _ = std::fs::remove_file(&sha_target);
        }
    }
    if let Some(expected) = asset_sha256 {
        let actual = updates::sha256_of(target)?;
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(target);
            return Err("checksum mismatch - install cancelled.".into());
        }
    }
    Ok(())
}
