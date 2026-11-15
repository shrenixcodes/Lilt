//! Paired Bluetooth devices, from Windows' device association endpoints (classic and Low
//! Energy). Pairing new devices needs Windows' own dialog, so Lilt links to it.

use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationKind};
use windows::Foundation::IPropertyValue;
use windows::core::{HSTRING, Interface};
use windows_collections::IIterable;

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub name: String,
    /// The Bluetooth address as 12 lowercase hex digits, to match audio endpoints.
    pub address: String,
    pub connected: bool,
}

/// Bluetooth Classic and Bluetooth LE.
const PROTOCOLS: [&str; 2] = ["{e0cbf06c-cd8b-4647-bb8a-263b43f0f974}", "{bb7bb05e-5972-42b5-94fc-76eaa7084d49}"];

/// Paired devices, connected first, then by name. Empty if Windows cannot list them.
pub fn paired() -> Vec<Device> {
    let mut devices: Vec<Device> = Vec::new();
    for protocol in PROTOCOLS {
        let filter = HSTRING::from(format!(
            "System.Devices.Aep.ProtocolId:=\"{protocol}\" AND System.Devices.Aep.IsPaired:=System.StructuredQueryType.Boolean#True"
        ));
        let properties: IIterable<HSTRING> =
            vec![HSTRING::from("System.Devices.Aep.IsConnected"), HSTRING::from("System.Devices.Aep.DeviceAddress")]
                .into();
        let found = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
            &filter,
            &properties,
            DeviceInformationKind::AssociationEndpoint,
        )
        .and_then(|op| op.join());
        let Ok(found) = found else { continue };
        for info in found {
            let name = info.Name().map(|n| n.to_string()).unwrap_or_default();
            let Ok(props) = info.Properties() else { continue };
            let lookup =
                |key: &str| props.Lookup(&HSTRING::from(key)).ok().and_then(|v| v.cast::<IPropertyValue>().ok());
            let connected = lookup("System.Devices.Aep.IsConnected").and_then(|v| v.GetBoolean().ok()).unwrap_or(false);
            let address = lookup("System.Devices.Aep.DeviceAddress")
                .and_then(|v| v.GetString().ok())
                .map(|a| normalize_address(&a.to_string()))
                .unwrap_or_default();
            if name.trim().is_empty() {
                continue;
            }
            // The same device can appear for both protocols.
            match devices.iter_mut().find(|d| !address.is_empty() && d.address == address) {
                Some(existing) => existing.connected |= connected,
                None => devices.push(Device { name, address, connected }),
            }
        }
    }
    devices
        .sort_by(|a, b| b.connected.cmp(&a.connected).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    devices
}

/// "38:18:4C:4F:0A:36" → "38184c4f0a36".
pub fn normalize_address(address: &str) -> String {
    address.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>().to_ascii_lowercase()
}

/// The Bluetooth address in an audio driver's device path, such as
/// `…\bthenum#{0000110b-…}_vid&…#7&2c4b&0&38184c4f0a36_c00000000#{…}`.
pub fn address_in_path(path: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    if !lower.contains("bth") {
        return None;
    }
    lower
        .split(['#', '&', '_', '\\', '{', '}', '.'])
        .find(|t| t.len() == 12 && t.chars().all(|c| c.is_ascii_hexdigit()) && t.chars().any(|c| c != '0'))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_found_in_driver_paths() {
        assert_eq!(normalize_address("38:18:4C:4F:0A:36"), "38184c4f0a36");
        let path = r"{2}.\\?\bthenum#{0000110b-0000-1000-8000-00805f9b34fb}_vid&0001004c_pid&200e#7&2c4b2a1&0&38184C4F0A36_C00000000#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\wave";
        assert_eq!(address_in_path(path).as_deref(), Some("38184c4f0a36"));
        assert_eq!(address_in_path(r"{2}.\\?\hdaudio#func_01&ven_10ec&dev_0257#4&1&0&0001"), None);
    }
}
