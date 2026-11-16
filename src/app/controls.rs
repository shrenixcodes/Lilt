//! The quick-control pop-ups: connects the controls worker to the UI.
//!
//! The worker starts the first time a pop-up opens. While one is open its state is read
//! again every few seconds (Wi-Fi also scans, and reports changes as Windows announces
//! them); when it closes, the refresh timer stops.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::{App, post};
use crate::actions::QuickAction;
use crate::controls::audio;
use crate::controls::{BluetoothView, Command, ControlsService, Panel, Update, WifiView, wifi};
use crate::ui::{AppState, ControlsActions, ControlsState, DeviceRow, NetworkRow, Page, SessionRow};

const MESSAGE_TIME: Duration = Duration::from_secs(6);

#[derive(Default)]
pub(crate) struct ControlsController {
    service: RefCell<Option<ControlsService>>,
    refresh_timer: slint::Timer,
    message_timer: slint::Timer,
    open: Cell<Option<Panel>>,
    /// A password is being typed.
    pub busy: Cell<bool>,
}

fn panel(index: i32) -> Option<Panel> {
    match index {
        0 => Some(Panel::Sound),
        1 => Some(Panel::Display),
        2 => Some(Panel::Bluetooth),
        3 => Some(Panel::Wifi),
        _ => None,
    }
}

fn page(panel: Panel) -> Page {
    match panel {
        Panel::Sound => Page::Sound,
        Panel::Display => Page::Brightness,
        Panel::Bluetooth => Page::Bluetooth,
        Panel::Wifi => Page::Wifi,
    }
}

/// How often an open pop-up is read again.
fn refresh_interval(panel: Panel) -> Duration {
    match panel {
        // A scan takes a few seconds; Windows reports changes in between.
        Panel::Wifi => Duration::from_secs(10),
        Panel::Bluetooth => Duration::from_secs(4),
        // Volume keys and brightness keys change these outside Lilt.
        Panel::Sound | Panel::Display => Duration::from_secs(2),
    }
}

/// Signal quality (0–100) as 1 to 4 bars.
pub fn bars(signal: u32) -> i32 {
    match signal {
        0..=25 => 1,
        26..=50 => 2,
        51..=75 => 3,
        _ => 4,
    }
}

pub fn network_rows(view: &WifiView) -> Vec<NetworkRow> {
    view.networks
        .iter()
        .map(|n| NetworkRow {
            ssid: n.ssid.as_str().into(),
            bars: bars(n.signal),
            secured: n.security != wifi::Security::Open,
            connected: n.connected,
            saved: n.profile.is_some(),
            joinable: n.security != wifi::Security::Other,
        })
        .collect()
}

pub fn session_rows(sessions: &[audio::Session]) -> Vec<SessionRow> {
    sessions
        .iter()
        .map(|s| SessionRow {
            id: s.id.as_str().into(),
            name: s.name.as_str().into(),
            icon: s.icon.as_ref().map_or_else(slint::Image::default, |pixels| {
                slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                    pixels,
                    audio::ICON_SIZE,
                    audio::ICON_SIZE,
                ))
            }),
            volume: s.volume,
            muted: s.muted,
            is_system: s.is_system,
        })
        .collect()
}

pub fn device_rows(view: &BluetoothView) -> Vec<DeviceRow> {
    view.devices
        .iter()
        .map(|(d, can)| DeviceRow {
            name: d.name.as_str().into(),
            address: d.address.as_str().into(),
            connected: d.connected,
            can_connect: *can,
        })
        .collect()
}

impl App {
    pub(super) fn connect_controls(app: &Rc<Self>) {
        let actions = app.island.global::<ControlsActions>();
        let weak = Rc::downgrade(app);

        let w = weak.clone();
        actions.on_opened(move |index| {
            let (Some(app), Some(panel)) = (w.upgrade(), panel(index)) else { return };
            App::open_controls(&app, panel);
        });
        let w = weak.clone();
        actions.on_set_radio(move |index, on| {
            let (Some(app), Some(panel)) = (w.upgrade(), panel(index)) else { return };
            app.send_control(Command::SetRadio(panel, on));
        });
        let w = weak.clone();
        actions.on_join(move |ssid, password| {
            let Some(app) = w.upgrade() else { return };
            let password = (!password.is_empty()).then(|| password.to_string());
            app.island.global::<ControlsState>().set_wifi_connecting(ssid.clone());
            app.send_control(Command::WifiConnect { ssid: ssid.to_string(), password });
        });
        let w = weak.clone();
        actions.on_disconnect_wifi(move || {
            if let Some(app) = w.upgrade() {
                app.send_control(Command::WifiDisconnect);
            }
        });
        let w = weak.clone();
        actions.on_connect_device(move |address, name, connect| {
            let Some(app) = w.upgrade() else { return };
            app.send_control(Command::Bluetooth { address: address.to_string(), name: name.to_string(), connect });
        });
        let w = weak.clone();
        actions.on_set_volume(move |level| {
            let Some(app) = w.upgrade() else { return };
            app.island.global::<ControlsState>().set_volume(level);
            app.send_control(Command::SetVolume(level));
        });
        let w = weak.clone();
        actions.on_set_muted(move |muted| {
            let Some(app) = w.upgrade() else { return };
            app.island.global::<ControlsState>().set_muted(muted);
            app.send_control(Command::SetMuted(muted));
        });
        let w = weak.clone();
        actions.on_set_session_volume(move |id, level| {
            let Some(app) = w.upgrade() else { return };
            // Update the row at once so the slider feels direct.
            let state = app.island.global::<ControlsState>();
            let rows = state.get_sessions();
            if let Some((index, mut row)) = rows.iter().enumerate().find(|(_, row)| row.id == id) {
                row.volume = level;
                rows.set_row_data(index, row);
            }
            app.send_control(Command::SetSessionVolume(id.to_string(), level));
        });
        let w = weak.clone();
        actions.on_set_session_muted(move |id, muted| {
            let Some(app) = w.upgrade() else { return };
            let state = app.island.global::<ControlsState>();
            let rows = state.get_sessions();
            if let Some((index, mut row)) = rows.iter().enumerate().find(|(_, row)| row.id == id) {
                row.muted = muted;
                rows.set_row_data(index, row);
            }
            app.send_control(Command::SetSessionMuted(id.to_string(), muted));
        });
        let w = weak.clone();
        actions.on_set_brightness(move |level| {
            let Some(app) = w.upgrade() else { return };
            app.island.global::<ControlsState>().set_brightness(level);
            app.send_control(Command::SetBrightness((level * 100.0).round() as u32));
        });
        let w = weak.clone();
        actions.on_more_settings(move |index| {
            let Some(app) = w.upgrade() else { return };
            let Some(action) = QuickAction::from_index(index) else { return };
            // Collapse first so the opened window is not covered and gets the focus.
            App::set_expanded(&app, false);
            if let Err(err) = action.launch() {
                tracing::warn!(%err, ?action, "could not open Windows settings");
            }
        });
        let w = weak;
        actions.on_busy(move |busy| {
            let Some(app) = w.upgrade() else { return };
            app.controls.busy.set(busy);
            if busy {
                app.collapse_timer.stop();
            } else if app.expanded() && !app.pointer_inside.get() {
                App::schedule_collapse(&app);
            }
        });
    }

    fn send_control(&self, command: Command) {
        if let Some(service) = self.controls.service.borrow().as_ref() {
            service.send(command);
        }
    }

    fn open_controls(app: &Rc<Self>, panel: Panel) {
        if app.controls.service.borrow().is_none() {
            let service = ControlsService::start(|update| post(move |app| App::controls_update(app, update)));
            app.controls.service.replace(Some(service));
        }
        let state = app.island.global::<ControlsState>();
        state.set_loading(true);
        state.set_message("".into());
        state.set_wifi_password_for("".into());
        app.controls.open.set(Some(panel));
        app.send_control(Command::Refresh(panel));
        let weak = Rc::downgrade(app);
        app.controls.refresh_timer.start(slint::TimerMode::Repeated, refresh_interval(panel), move || {
            let Some(app) = weak.upgrade() else { return };
            let showing = app.expanded() && app.island.global::<AppState>().get_page() == page(panel);
            if showing {
                app.send_control(Command::Refresh(panel));
            } else {
                app.controls.refresh_timer.stop();
                app.controls.open.set(None);
                app.controls.busy.set(false);
            }
        });
    }

    fn controls_update(app: &Rc<Self>, update: Update) {
        let state = app.island.global::<ControlsState>();
        let open = app.controls.open.get();
        let radio = |on: Option<bool>| on.map_or(-1, i32::from);
        match update {
            Update::Wifi(view) => {
                state.set_wifi_available(view.available);
                state.set_wifi_radio(radio(view.radio));
                state.set_wifi_connecting(view.connecting.clone().unwrap_or_default().into());
                state.set_networks(ModelRc::new(VecModel::from(network_rows(&view))));
                if open == Some(Panel::Wifi) {
                    state.set_loading(false);
                }
            }
            Update::Bluetooth(view) => {
                state.set_bluetooth_radio(radio(view.radio));
                state.set_devices(ModelRc::new(VecModel::from(device_rows(&view))));
                if open == Some(Panel::Bluetooth) {
                    state.set_loading(false);
                }
            }
            Update::Sound(output, sessions) => {
                state.set_sound_available(output.is_some());
                if let Some(output) = output {
                    state.set_volume(output.volume);
                    state.set_muted(output.muted);
                    state.set_output_name(output.name.into());
                }
                let next = session_rows(&sessions);
                let current = state.get_sessions();
                if current.row_count() == next.len() && current.iter().zip(&next).all(|(old, new)| old.id == new.id) {
                    for (index, row) in next.into_iter().enumerate() {
                        current.set_row_data(index, row);
                    }
                } else {
                    state.set_sessions(ModelRc::new(VecModel::from(next)));
                }
                if open == Some(Panel::Sound) {
                    state.set_loading(false);
                }
            }
            Update::Display(level) => {
                state.set_brightness_available(level.is_some());
                if let Some(level) = level {
                    state.set_brightness(level as f32 / 100.0);
                }
                if open == Some(Panel::Display) {
                    state.set_loading(false);
                }
            }
            Update::Message(panel, text) => {
                if open == Some(panel) {
                    state.set_message(SharedString::from(text));
                    let island = app.island.as_weak();
                    app.controls.message_timer.start(slint::TimerMode::SingleShot, MESSAGE_TIME, move || {
                        if let Some(island) = island.upgrade() {
                            island.global::<ControlsState>().set_message("".into());
                        }
                    });
                }
            }
        }
    }

    /// Stops the controls worker, for shutdown.
    pub(super) fn stop_controls(&self) {
        self.controls.refresh_timer.stop();
        self.controls.service.replace(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::bluetooth::Device;

    #[test]
    fn signal_becomes_bars() {
        assert_eq!([0, 25, 26, 50, 51, 75, 76, 100].map(bars), [1, 1, 2, 2, 3, 3, 4, 4]);
    }

    #[test]
    fn rows_say_what_lilt_can_do() {
        let network = |security, profile: Option<&str>| wifi::Network {
            ssid: "Home".into(),
            ssid_bytes: b"Home".to_vec(),
            signal: 80,
            security,
            connected: false,
            profile: profile.map(str::to_owned),
        };
        let view = WifiView {
            available: true,
            radio: Some(true),
            networks: vec![
                network(wifi::Security::Open, None),
                network(wifi::Security::Personal { wpa3: false, tkip: false }, Some("Home")),
                network(wifi::Security::Other, None),
            ],
            connecting: None,
        };
        let rows = network_rows(&view);
        assert_eq!(
            rows.iter().map(|r| (r.secured, r.saved, r.joinable)).collect::<Vec<_>>(),
            [(false, false, true), (true, true, true), (true, false, false)]
        );
        let view = BluetoothView {
            radio: Some(true),
            devices: vec![(Device { name: "Buds".into(), address: "a".into(), connected: true }, true)],
        };
        let rows = device_rows(&view);
        assert!(rows[0].connected && rows[0].can_connect);
    }
}
