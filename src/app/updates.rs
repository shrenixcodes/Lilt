use std::rc::Rc;
use std::time::Duration;

use slint::ComponentHandle;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

use super::App;
use crate::ui::{AppActions, AppState, SettingsWindow};
use crate::updates;

impl App {
    pub(super) fn connect_updates(app: &Rc<Self>) {
        let weak = Rc::downgrade(app);
        app.island.global::<AppActions>().on_check_for_updates(move || {
            if let Some(app) = weak.upgrade() {
                App::check_updates(&app);
            }
        });
        let weak = Rc::downgrade(app);
        app.island.global::<AppActions>().on_open_update(move || {
            if let Some(app) = weak.upgrade() {
                App::open_update(&app);
            }
        });

        let weak = Rc::downgrade(app);
        app.update_timer.start(slint::TimerMode::Repeated, Duration::from_secs(30 * 60), move || {
            if let Some(app) = weak.upgrade() {
                App::check_updates(&app);
            }
        });
        App::check_updates(app);
    }

    pub(super) fn connect_settings_updates(app: &Rc<Self>, window: &SettingsWindow) {
        let state = app.island.global::<AppState>();
        window.set_update_checking(state.get_update_checking());
        window.set_update_available(state.get_update_available());
        window.set_update_message(state.get_update_message());
        let weak = Rc::downgrade(app);
        window.on_check_for_updates(move || {
            if let Some(app) = weak.upgrade() {
                App::check_updates(&app);
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
        let url = HSTRING::from(updates::BRANCH_URL);
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

    fn check_updates(app: &Rc<Self>) {
        let state = app.island.global::<AppState>();
        if updates::BUILT_REVISION.is_empty() {
            state.set_update_message("Build revision unavailable".into());
            app.refresh_settings_updates();
            return;
        }
        if app.update_checking.replace(true) {
            return;
        }
        state.set_update_checking(true);
        state.set_update_message("Checking GitHub…".into());
        app.refresh_settings_updates();
        std::thread::spawn(|| {
            let result = updates::branch_revision();
            super::post(move |app| {
                app.update_checking.set(false);
                let state = app.island.global::<AppState>();
                state.set_update_checking(false);
                match result {
                    Ok(revision) => {
                        let available = updates::update_available(updates::BUILT_REVISION, &revision);
                        state.set_update_available(available);
                        state.set_update_message(if available { "Update available" } else { "Up to date" }.into());
                    }
                    Err(error) => {
                        tracing::warn!(%error, "GitHub update check failed");
                        state.set_update_message("Could not check for updates".into());
                    }
                }
                app.refresh_settings_updates();
            });
        });
    }
}
