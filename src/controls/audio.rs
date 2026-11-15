//! Audio: the default output's volume and mute (Core Audio), and connecting Bluetooth
//! audio devices.
//!
//! Windows has no public "connect this Bluetooth device" call. For headphones and
//! speakers, the Bluetooth audio driver itself accepts a one-shot request to reconnect
//! or disconnect (`KSPROPSETID_BtAudio`, documented in the Windows driver kit). Lilt
//! sends it to the driver behind the device's audio endpoint. Other Bluetooth devices,
//! such as mice and keyboards, connect by themselves when used.

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    DEVICE_STATE, DEVICE_STATEMASK_ALL, IConnector, IDeviceTopology, IMMDevice, IMMDeviceEnumerator, IPart,
    MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::Media::KernelStreaming::{
    IKsControl, KSIDENTIFIER, KSIDENTIFIER_0, KSIDENTIFIER_0_0, KSPROPERTY_ONESHOT_DISCONNECT,
    KSPROPERTY_ONESHOT_RECONNECT, KSPROPERTY_TYPE_GET, KSPROPSETID_BtAudio,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ};
use windows::core::{GUID, Interface};

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
