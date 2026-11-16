//! Quick controls: the pop-ups behind the Sound, Brightness, Bluetooth and Wi-Fi buttons.
//!
//! Every call to Windows (Native Wifi, radios, device enumeration, Core Audio, WMI) runs
//! on one worker thread, started the first time a pop-up opens, so the UI thread never
//! waits on hardware. Lists are refreshed when a pop-up opens, when Windows reports a
//! Wi-Fi change, after an action, and every few seconds while a pop-up is open; nothing
//! runs while they are closed.

pub mod audio;
pub mod bluetooth;
pub mod display;
pub mod radio;
pub mod wifi;

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

/// Which pop-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Sound,
    Display,
    Bluetooth,
    Wifi,
}

#[derive(Debug)]
pub enum Command {
    /// The pop-up opened or should be refreshed.
    Refresh(Panel),
    SetRadio(Panel, bool),
    WifiConnect {
        ssid: String,
        password: Option<String>,
    },
    WifiDisconnect,
    Bluetooth {
        address: String,
        name: String,
        connect: bool,
    },
    SetVolume(f32),
    SetMuted(bool),
    /// Per-application volume in the mixer, 0.0..=1.0.
    SetSessionVolume(String, f32),
    SetSessionMuted(String, bool),
    SetBrightness(u32),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WifiView {
    /// This PC has a Wi-Fi adapter.
    pub available: bool,
    pub radio: Option<bool>,
    pub networks: Vec<wifi::Network>,
    /// The network being joined.
    pub connecting: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BluetoothView {
    pub radio: Option<bool>,
    /// Paired devices, and whether Lilt can connect or disconnect each one.
    pub devices: Vec<(bluetooth::Device, bool)>,
}

#[derive(Debug)]
pub enum Update {
    Wifi(WifiView),
    Bluetooth(BluetoothView),
    Sound(Option<audio::Output>, Vec<audio::Session>),
    Display(Option<u32>),
    /// A short message for a pop-up, such as a failed connection.
    Message(Panel, String),
}

enum Message {
    Command(Command),
    Wifi(wifi::Event),
    Shutdown,
}

pub struct ControlsService {
    sender: Sender<Message>,
    worker: Option<JoinHandle<()>>,
}

impl ControlsService {
    /// Starts the worker. `publish` is called on the worker thread.
    pub fn start(publish: impl Fn(Update) + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::channel();
        let events = sender.clone();
        let worker = std::thread::Builder::new()
            .name("lilt-controls".into())
            .spawn(move || {
                let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                Worker::new(events, &publish).run(&receiver);
                if com.is_ok() {
                    unsafe { CoUninitialize() };
                }
            })
            .map_err(|err| tracing::error!(%err, "failed to start the controls worker"))
            .ok();
        Self { sender, worker }
    }

    pub fn send(&self, command: Command) {
        let _ = self.sender.send(Message::Command(command));
    }
}

impl Drop for ControlsService {
    fn drop(&mut self) {
        let _ = self.sender.send(Message::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// How long a Wi-Fi connection may take before Lilt reports it failed.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(25);

struct Worker<'a> {
    events: Sender<Message>,
    publish: &'a dyn Fn(Update),
    wlan: Option<wifi::Wlan>,
    wlan_tried: bool,
    /// The network being joined, the profile Lilt created for it, and since when.
    connecting: Option<(String, Option<String>, Instant)>,
}

impl<'a> Worker<'a> {
    fn new(events: Sender<Message>, publish: &'a dyn Fn(Update)) -> Self {
        Self { events, publish, wlan: None, wlan_tried: false, connecting: None }
    }

    fn run(&mut self, receiver: &Receiver<Message>) {
        loop {
            let timeout = self
                .connecting
                .as_ref()
                .map_or(Duration::from_secs(3600), |(_, _, since)| CONNECT_TIMEOUT.saturating_sub(since.elapsed()));
            let first = match receiver.recv_timeout(timeout) {
                Ok(message) => Some(message),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            let mut batch: Vec<Message> = first.into_iter().collect();
            batch.extend(receiver.try_iter());
            // Only the last position of a dragged slider matters.
            let last_volume = batch.iter().rposition(|m| matches!(m, Message::Command(Command::SetVolume(_))));
            let last_brightness = batch.iter().rposition(|m| matches!(m, Message::Command(Command::SetBrightness(_))));
            let mut refresh: Vec<Panel> = Vec::new();
            for (index, message) in batch.into_iter().enumerate() {
                match message {
                    Message::Shutdown => return,
                    Message::Command(Command::SetVolume(_)) if Some(index) != last_volume => {}
                    Message::Command(Command::SetBrightness(_)) if Some(index) != last_brightness => {}
                    Message::Command(command) => self.handle(command, &mut refresh),
                    Message::Wifi(event) => self.wifi_event(event, &mut refresh),
                }
            }
            if let Some((ssid, _, since)) = &self.connecting
                && since.elapsed() >= CONNECT_TIMEOUT
            {
                let ssid = ssid.clone();
                self.connect_failed(&ssid);
                refresh.push(Panel::Wifi);
            }
            refresh.dedup();
            for panel in refresh {
                self.publish_panel(panel);
            }
        }
    }

    fn wlan(&mut self) -> Option<&wifi::Wlan> {
        if !self.wlan_tried {
            self.wlan_tried = true;
            self.wlan = wifi::Wlan::open().map(|mut wlan| {
                let events = self.events.clone();
                wlan.notify(move |event| {
                    let _ = events.send(Message::Wifi(event));
                });
                wlan
            });
        }
        self.wlan.as_ref()
    }

    fn message(&self, panel: Panel, text: impl Into<String>) {
        (self.publish)(Update::Message(panel, text.into()));
    }

    fn handle(&mut self, command: Command, refresh: &mut Vec<Panel>) {
        match command {
            Command::Refresh(panel) => {
                if panel == Panel::Wifi
                    && let Some(wlan) = self.wlan()
                {
                    wlan.scan();
                }
                refresh.push(panel);
            }
            Command::SetRadio(panel, on) => {
                let kind = if panel == Panel::Wifi { radio::wifi() } else { radio::bluetooth() };
                if let Err(message) = radio::set(kind, on) {
                    self.message(panel, message);
                }
                refresh.push(panel);
            }
            Command::WifiConnect { ssid, password } => {
                refresh.push(Panel::Wifi);
                let Some(wlan) = self.wlan() else { return };
                let Some(network) = wlan.networks().into_iter().find(|n| n.ssid == ssid) else {
                    self.message(Panel::Wifi, format!("{ssid} is no longer in range."));
                    return;
                };
                match wlan.connect(&network, password.as_deref()) {
                    Ok(created) => self.connecting = Some((ssid, created, Instant::now())),
                    Err(message) => self.message(Panel::Wifi, message),
                }
            }
            Command::WifiDisconnect => {
                if let Some(wlan) = self.wlan() {
                    wlan.disconnect();
                }
                refresh.push(Panel::Wifi);
            }
            Command::Bluetooth { address, name, connect } => {
                let device = bluetooth::Device { name: name.clone(), address, connected: !connect };
                let sent =
                    audio::bluetooth_audio().iter().find(|a| a.belongs_to(&device)).is_some_and(|a| a.request(connect));
                if sent {
                    self.message(
                        Panel::Bluetooth,
                        format!("{} {name}…", if connect { "Connecting" } else { "Disconnecting" }),
                    );
                } else {
                    self.message(Panel::Bluetooth, format!("Windows did not accept the request for {name}."));
                }
                refresh.push(Panel::Bluetooth);
            }
            Command::SetVolume(level) => {
                if !audio::set_volume(level) {
                    self.message(Panel::Sound, "The volume could not be changed.");
                    refresh.push(Panel::Sound);
                }
            }
            Command::SetMuted(muted) => {
                audio::set_muted(muted);
                refresh.push(Panel::Sound);
            }
            Command::SetSessionVolume(id, level) => {
                if !audio::set_session_volume(&id, level) {
                    self.message(Panel::Sound, "That app's volume could not be changed.");
                }
                refresh.push(Panel::Sound);
            }
            Command::SetSessionMuted(id, muted) => {
                audio::set_session_muted(&id, muted);
                refresh.push(Panel::Sound);
            }
            Command::SetBrightness(level) => {
                if !display::set_brightness(level) {
                    self.message(Panel::Display, "Windows did not change the brightness.");
                    refresh.push(Panel::Display);
                }
            }
        }
    }

    fn wifi_event(&mut self, event: wifi::Event, refresh: &mut Vec<Panel>) {
        match event {
            wifi::Event::Connected => {
                if let Some((ssid, _, _)) = self.connecting.take() {
                    self.message(Panel::Wifi, format!("Connected to {ssid}."));
                }
            }
            wifi::Event::Failed { reason } => {
                if let Some((ssid, _, _)) = &self.connecting {
                    tracing::info!(reason, "Wi-Fi connection failed");
                    let ssid = ssid.clone();
                    self.connect_failed(&ssid);
                }
            }
            wifi::Event::Networks | wifi::Event::Disconnected => {}
        }
        refresh.push(Panel::Wifi);
    }

    /// Reports a failed connection and removes the profile Lilt made for it, so a
    /// mistyped password is not kept.
    fn connect_failed(&mut self, ssid: &str) {
        if let Some((_, created, _)) = self.connecting.take() {
            if let (Some(profile), Some(wlan)) = (created, self.wlan.as_ref()) {
                wlan.delete_profile(&profile);
            }
            self.message(Panel::Wifi, format!("Could not connect to {ssid}. Check the password and try again."));
        }
    }

    fn publish_panel(&mut self, panel: Panel) {
        let update = match panel {
            Panel::Wifi => {
                let connecting = self.connecting.as_ref().map(|(ssid, _, _)| ssid.clone());
                let view = match self.wlan() {
                    Some(wlan) => WifiView {
                        available: true,
                        radio: wlan.radio_on().or_else(|| radio::is_on(radio::wifi())),
                        networks: wlan.networks(),
                        connecting,
                    },
                    None => WifiView::default(),
                };
                Update::Wifi(view)
            }
            Panel::Bluetooth => {
                let radio = radio::is_on(radio::bluetooth());
                let devices = if radio == Some(true) { bluetooth::paired() } else { Vec::new() };
                let audio = if devices.is_empty() { Vec::new() } else { audio::bluetooth_audio() };
                let devices = devices
                    .into_iter()
                    .map(|d| {
                        let can = audio.iter().any(|a| a.belongs_to(&d));
                        (d, can)
                    })
                    .collect();
                Update::Bluetooth(BluetoothView { radio, devices })
            }
            Panel::Sound => Update::Sound(audio::output(), audio::sessions()),
            Panel::Display => Update::Display(display::brightness()),
        };
        (self.publish)(update);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads (never changes) this PC's state: `cargo test -- --ignored controls::tests --nocapture`.
    #[test]
    #[ignore = "reads the PC's Wi-Fi, Bluetooth, audio and display"]
    fn reads_this_pc() {
        let _com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let security = unsafe {
            windows::Win32::System::Com::CoInitializeSecurity(
                None,
                -1,
                None,
                None,
                windows::Win32::System::Com::RPC_C_AUTHN_LEVEL_DEFAULT,
                windows::Win32::System::Com::RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                windows::Win32::System::Com::EOAC_NONE,
                None,
            )
        };
        assert!(security.is_ok(), "{security:?}");
        let wlan = wifi::Wlan::open();
        println!("wifi adapter: {}", wlan.is_some());
        if let Some(wlan) = &wlan {
            println!("wifi radio: {:?}", wlan.radio_on());
            for n in wlan.networks().iter().take(6) {
                println!(
                    "  {} signal {} {:?} connected {} saved {}",
                    n.ssid.len(),
                    n.signal,
                    n.security,
                    n.connected,
                    n.profile.is_some()
                );
            }
        }
        println!("radios: wifi {:?} bluetooth {:?}", radio::is_on(radio::wifi()), radio::is_on(radio::bluetooth()));
        let devices = bluetooth::paired();
        let drivers = audio::bluetooth_audio();
        for d in &devices {
            println!(
                "  bt device: connected {} address? {} audio {}",
                d.connected,
                !d.address.is_empty(),
                drivers.iter().any(|a| a.belongs_to(d))
            );
        }
        for a in &drivers {
            println!("  bt audio driver: address? {}", a.address.is_some());
        }
        println!("output: {:?}", audio::output().map(|o| (o.volume, o.muted)));
        println!("brightness: {:?}", display::brightness());
    }

    /// Changes and restores the Bluetooth radio and the brightness:
    /// `cargo test -- --ignored controls::tests::changes_and_restores --nocapture`.
    #[test]
    #[ignore = "briefly changes the PC's Bluetooth radio and screen brightness"]
    fn changes_and_restores() {
        let _com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let _ = unsafe {
            windows::Win32::System::Com::CoInitializeSecurity(
                None,
                -1,
                None,
                None,
                windows::Win32::System::Com::RPC_C_AUTHN_LEVEL_DEFAULT,
                windows::Win32::System::Com::RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                windows::Win32::System::Com::EOAC_NONE,
                None,
            )
        };
        if let Some(before) = radio::is_on(radio::bluetooth()) {
            println!("bluetooth set {:?}", radio::set(radio::bluetooth(), !before));
            // Windows takes a few seconds to bring up Bluetooth audio endpoints.
            std::thread::sleep(Duration::from_secs(10));
            println!("bluetooth now {:?} (was {before})", radio::is_on(radio::bluetooth()));
            if !before {
                println!("paired devices while on: {}", bluetooth::paired().len());
                println!("bluetooth audio drivers: {}", audio::bluetooth_audio().len());
            }
            println!("bluetooth restore {:?}", radio::set(radio::bluetooth(), before));
            std::thread::sleep(Duration::from_secs(1));
            println!("bluetooth restored {:?}", radio::is_on(radio::bluetooth()) == Some(before));
        }
        if let Some(before) = display::brightness() {
            let target = if before > 50 { before - 10 } else { before + 10 };
            println!("brightness set {}", display::set_brightness(target));
            std::thread::sleep(Duration::from_millis(800));
            println!("brightness now {:?} (was {before})", display::brightness());
            println!("brightness restore {}", display::set_brightness(before));
            std::thread::sleep(Duration::from_millis(800));
            println!("brightness restored {:?}", display::brightness() == Some(before));
        }
    }
}
