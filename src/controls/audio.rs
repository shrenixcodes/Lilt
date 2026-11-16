//! Audio: the default output's volume and mute (Core Audio), per-application sessions
//! (the volume mixer), and connecting Bluetooth audio devices.
//!
//! Windows has no public "connect this Bluetooth device" call. For headphones and
//! speakers, the Bluetooth audio driver itself accepts a one-shot request to reconnect
//! or disconnect (`KSPROPSETID_BtAudio`, documented in the Windows driver kit). Lilt
//! sends it to the driver behind the device's audio endpoint. Other Bluetooth devices,
//! such as mice and keyboards, connect by themselves when used.

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    AudioSessionStateExpired, DEVICE_STATE, DEVICE_STATEMASK_ALL, IAudioSessionControl, IAudioSessionControl2,
    IAudioSessionManager2, IConnector, IDeviceTopology, IMMDevice, IMMDeviceEnumerator, IPart, ISimpleAudioVolume,
    MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::Media::KernelStreaming::{
    IKsControl, KSIDENTIFIER, KSIDENTIFIER_0, KSIDENTIFIER_0_0, KSPROPERTY_ONESHOT_DISCONNECT,
    KSPROPERTY_ONESHOT_RECONNECT, KSPROPERTY_TYPE_GET, KSPROPSETID_BtAudio,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{GUID, Interface, PWSTR};

use super::bluetooth;

#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    /// 0.0 to 1.0.
    pub volume: f32,
    pub muted: bool,
    pub name: String,
}

fn enumerator() -> Option<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()
}

fn default_volume() -> Option<(IMMDevice, IAudioEndpointVolume)> {
    let device = unsafe { enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole) }.ok()?;
    let volume = unsafe { device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.ok()?;
    Some((device, volume))
}

fn friendly_name(device: &IMMDevice) -> String {
    unsafe {
        device
            .OpenPropertyStore(STGM_READ)
            .and_then(|store| store.GetValue(&PKEY_Device_FriendlyName))
            .map(|value| value.to_string())
            .unwrap_or_default()
    }
}

/// The default output device, or `None` without one.
pub fn output() -> Option<Output> {
    let (device, volume) = default_volume()?;
    unsafe {
        Some(Output {
            volume: volume.GetMasterVolumeLevelScalar().ok()?.clamp(0.0, 1.0),
            muted: volume.GetMute().ok()?.as_bool(),
            name: friendly_name(&device),
        })
    }
}

pub fn set_volume(level: f32) -> bool {
    let Some((_, volume)) = default_volume() else { return false };
    unsafe { volume.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null()) }.is_ok()
}

pub fn set_muted(muted: bool) -> bool {
    let Some((_, volume)) = default_volume() else { return false };
    unsafe { volume.SetMute(muted, std::ptr::null()) }.is_ok()
}

/// One application-level audio session on the default output, as the mixer shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// Session instance identifier returned by Windows, stable while the session exists.
    pub id: String,
    /// Human-friendly label (application display name, or the executable's base name).
    pub name: String,
    pub process_id: u32,
    /// 0.0 to 1.0.
    pub volume: f32,
    pub muted: bool,
    /// The "System sounds" session.
    pub is_system: bool,
}

/// Opens the session manager for the default output.
fn session_manager() -> Option<IAudioSessionManager2> {
    let device = unsafe { enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole) }.ok()?;
    unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) }.ok()
}

/// Returns every application audio session on the default output.
pub fn sessions() -> Vec<Session> {
    let mut out: Vec<Session> = Vec::new();
    let Some(manager) = session_manager() else { return out };
    let Ok(enumerator) = (unsafe { manager.GetSessionEnumerator() }) else { return out };
    let count = unsafe { enumerator.GetCount() }.unwrap_or(0);
    for index in 0..count {
        let Ok(control) = (unsafe { enumerator.GetSession(index) }) else { continue };
        if let Some(session) = describe(&control) {
            out.push(session);
        }
    }
    // One entry per process: the first (usually the main window's) wins. This mirrors
    // Windows' own volume mixer, which collapses several streams of an app.
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    out.retain(|s| seen.insert(s.process_id.max(1) + if s.is_system { 1 << 30 } else { 0 }));
    // System sounds last; otherwise alphabetical.
    out.sort_by(|a, b| {
        a.is_system.cmp(&b.is_system).then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
    });
    out
}

fn describe(control: &IAudioSessionControl) -> Option<Session> {
    let state = unsafe { control.GetState() }.ok()?;
    if state == AudioSessionStateExpired {
        return None;
    }
    let control2: IAudioSessionControl2 = control.cast().ok()?;
    let volume: ISimpleAudioVolume = control.cast().ok()?;
    // `IsSystemSoundsSession` returns S_OK only for the system-sounds session and
    // S_FALSE for every normal application. `HRESULT::is_ok` treats both as success,
    // which previously labelled every mixer row "System sounds".
    let is_system = unsafe { control2.IsSystemSoundsSession() }.0 == 0;
    let process_id = unsafe { control2.GetProcessId() }.unwrap_or(0);
    let id = unsafe { control2.GetSessionInstanceIdentifier() }
        .ok()
        .and_then(|p| unsafe { p.to_string().ok() })
        .unwrap_or_default();
    let display =
        unsafe { control2.GetDisplayName() }.ok().and_then(|p| unsafe { p.to_string().ok() }).unwrap_or_default();
    let name = if is_system {
        "System sounds".to_string()
    } else if !display.is_empty() {
        display
    } else {
        process_name(process_id).unwrap_or_else(|| "Unknown".into())
    };
    let level = unsafe { volume.GetMasterVolume() }.ok()?.clamp(0.0, 1.0);
    let muted = unsafe { volume.GetMute() }.ok()?.as_bool();
    Some(Session { id, name, process_id, volume: level, muted, is_system })
}

/// Executable file name (without path and extension) for a running process, if the
/// process allows a limited-information query.
fn process_name(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buffer = [0u16; 512];
    let mut len = buffer.len() as u32;
    let result =
        unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_FORMAT(0), PWSTR(buffer.as_mut_ptr()), &mut len) };
    unsafe { CloseHandle(handle) }.ok()?;
    result.ok()?;
    let path = String::from_utf16_lossy(&buffer[..len as usize]);
    let base = path.rsplit(['\\', '/']).next().unwrap_or(&path);
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    Some(pretty_name(stem))
}

/// Capitalises an executable stem without going out of its way.
fn pretty_name(stem: &str) -> String {
    if stem.is_empty() {
        return String::new();
    }
    let mut chars = stem.chars();
    let first = chars.next().unwrap().to_ascii_uppercase();
    let rest: String = chars.collect();
    format!("{first}{rest}")
}

/// Finds a session by its instance id and applies `then`, returning whether it ran.
fn with_session<F>(id: &str, then: F) -> bool
where
    F: Fn(&IAudioSessionControl) -> bool,
{
    let Some(manager) = session_manager() else { return false };
    let Ok(enumerator) = (unsafe { manager.GetSessionEnumerator() }) else { return false };
    let count = unsafe { enumerator.GetCount() }.unwrap_or(0);
    for index in 0..count {
        let Ok(control) = (unsafe { enumerator.GetSession(index) }) else { continue };
        let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
        let Ok(session_id) = (unsafe { control2.GetSessionInstanceIdentifier() }) else { continue };
        let Ok(session_id) = (unsafe { session_id.to_string() }) else { continue };
        if session_id == id {
            return then(&control);
        }
    }
    false
}

pub fn set_session_volume(id: &str, level: f32) -> bool {
    with_session(id, |control| {
        let Ok(volume) = control.cast::<ISimpleAudioVolume>() else { return false };
        unsafe { volume.SetMasterVolume(level.clamp(0.0, 1.0), std::ptr::null()) }.is_ok()
    })
}

pub fn set_session_muted(id: &str, muted: bool) -> bool {
    with_session(id, |control| {
        let Ok(volume) = control.cast::<ISimpleAudioVolume>() else { return false };
        unsafe { volume.SetMute(muted, std::ptr::null()) }.is_ok()
    })
}

/// A Bluetooth audio driver that accepts connect and disconnect requests.
pub struct BluetoothAudio {
    /// 12 lowercase hex digits, when the driver's path includes it.
    pub address: Option<String>,
    /// The endpoint's name, e.g. "Headphones (WH-1000XM4)".
    pub name: String,
    control: IKsControl,
}

impl BluetoothAudio {
    /// Asks the driver to connect (`true`) or disconnect the device.
    pub fn request(&self, connect: bool) -> bool {
        let id = if connect { KSPROPERTY_ONESHOT_RECONNECT } else { KSPROPERTY_ONESHOT_DISCONNECT };
        let property = KSIDENTIFIER {
            Anonymous: KSIDENTIFIER_0 {
                Anonymous: KSIDENTIFIER_0_0 { Set: KSPROPSETID_BtAudio, Id: id.0 as u32, Flags: KSPROPERTY_TYPE_GET },
            },
        };
        let mut returned = 0u32;
        unsafe {
            self.control.KsProperty(
                &property,
                std::mem::size_of::<KSIDENTIFIER>() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
            )
        }
        .is_ok()
    }

    /// Whether this driver belongs to `device`: by address, or else by name.
    pub fn belongs_to(&self, device: &bluetooth::Device) -> bool {
        match &self.address {
            Some(address) => !device.address.is_empty() && *address == device.address,
            None => !device.name.is_empty() && self.name.contains(&device.name),
        }
    }
}

/// Bluetooth audio drivers, connected or not, behind every audio endpoint.
pub fn bluetooth_audio() -> Vec<BluetoothAudio> {
    let mut found = Vec::new();
    let Some(enumerator) = enumerator() else { return found };
    let Ok(endpoints) = (unsafe {
        enumerator.EnumAudioEndpoints(windows::Win32::Media::Audio::eAll, DEVICE_STATE(DEVICE_STATEMASK_ALL))
    }) else {
        return found;
    };
    let count = unsafe { endpoints.GetCount() }.unwrap_or(0);
    for index in 0..count {
        let Ok(device) = (unsafe { endpoints.Item(index) }) else { continue };
        let Some((path, control)) = driver(&device) else { continue };
        if !path.to_ascii_lowercase().contains("bth") {
            continue;
        }
        let address = bluetooth::address_in_path(&path);
        // A device has several endpoints (stereo, hands-free); one per driver path.
        if found.iter().any(|f: &BluetoothAudio| f.address.is_some() && f.address == address) {
            continue;
        }
        found.push(BluetoothAudio { address, name: friendly_name(&device), control });
    }
    found
}

/// The device path of the driver behind an endpoint, and its property interface.
fn driver(device: &IMMDevice) -> Option<(String, IKsControl)> {
    unsafe {
        let topology = device.Activate::<IDeviceTopology>(CLSCTX_ALL, None).ok()?;
        let connector: IConnector = topology.GetConnector(0).ok()?;
        let other = connector.GetConnectedTo().ok()?;
        let part: IPart = other.cast().ok()?;
        let filter = part.GetTopologyObject().ok()?;
        let id = filter.GetDeviceId().ok()?;
        let path = id.to_string().unwrap_or_default();
        CoTaskMemFree(Some(id.0 as *const _));
        let mut control: Option<IKsControl> = None;
        let iid: GUID = IKsControl::IID;
        part.Activate(CLSCTX_ALL.0, &iid, Some(&mut control as *mut _ as *mut *mut core::ffi::c_void)).ok()?;
        Some((path, control?))
    }
}
