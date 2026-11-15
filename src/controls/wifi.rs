//! Wi-Fi through the Native Wifi API (`wlanapi.dll`): the networks in range, connecting
//! and disconnecting. Passwords typed into Lilt go straight into a Windows Wi-Fi profile,
//! exactly as Windows' own Wi-Fi flyout stores them; Lilt keeps no copy and never logs them.

use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::WiFi::*;
use windows::core::{GUID, HSTRING, PCWSTR};

/// How a network is protected, as far as Lilt can join it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    Open,
    /// A password (WPA2- or WPA3-Personal).
    Personal {
        wpa3: bool,
        tkip: bool,
    },
    /// Enterprise sign-in (802.1X) and older schemes: joined in Windows' own settings.
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Network {
    pub ssid: String,
    /// The SSID's bytes; a few networks use names that are not valid UTF-8.
    pub ssid_bytes: Vec<u8>,
    /// 0 to 100.
    pub signal: u32,
    pub security: Security,
    pub connected: bool,
    /// Windows already has a profile (saved password) for it.
    pub profile: Option<String>,
}

/// Wi-Fi notifications the worker cares about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A scan finished or the list of networks changed.
    Networks,
    Connected,
    /// A connection attempt failed; `reason` is the WLAN reason code.
    Failed {
        reason: u32,
    },
    Disconnected,
}

/// Receives Wi-Fi events on a Windows thread-pool thread.
type Notify = Box<dyn Fn(Event) + Send + Sync>;

pub struct Wlan {
    handle: HANDLE,
    interface: GUID,
    /// Kept alive while notifications are registered.
    events: Option<Box<Notify>>,
}

// The handle is only used from the worker thread; notifications arrive through `events`.
unsafe impl Send for Wlan {}

impl Wlan {
    /// Opens the first Wi-Fi adapter, or `None` without one.
    pub fn open() -> Option<Self> {
        let mut version = 0;
        let mut handle = HANDLE::default();
        if unsafe { WlanOpenHandle(2, None, &mut version, &mut handle) } != 0 {
            return None;
        }
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        let interface = unsafe {
            if WlanEnumInterfaces(handle, None, &mut list) != 0 || list.is_null() {
                None
            } else {
                let count = (*list).dwNumberOfItems as usize;
                let items = std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), count);
                let first = items.first().map(|i| i.InterfaceGuid);
                WlanFreeMemory(list as *const _);
                first
            }
        };
        match interface {
            Some(interface) => Some(Self { handle, interface, events: None }),
            None => {
                unsafe { WlanCloseHandle(handle, None) };
                None
            }
        }
    }

    /// Calls `notify` with scan, connection and disconnection events until dropped.
    pub fn notify(&mut self, notify: impl Fn(Event) + Send + Sync + 'static) {
        let boxed: Box<Notify> = Box::new(Box::new(notify));
        let context = &*boxed as *const Notify as *const core::ffi::c_void;
        let result = unsafe {
            WlanRegisterNotification(
                self.handle,
                WLAN_NOTIFICATION_SOURCE_ACM,
                true,
                Some(on_notification),
                Some(context),
                None,
                None,
            )
        };
        if result == 0 {
            self.events = Some(boxed);
        }
    }

    /// Whether the radio is on: `None` if Windows does not say.
    pub fn radio_on(&self) -> Option<bool> {
        let mut size = 0u32;
        let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
        unsafe {
            if WlanQueryInterface(
                self.handle,
                &self.interface,
                wlan_intf_opcode_radio_state,
                None,
                &mut size,
                &mut data,
                None,
            ) != 0
                || data.is_null()
            {
                return None;
            }
            let state = &*(data as *const WLAN_RADIO_STATE);
            let count = (state.dwNumberOfPhys as usize).min(state.PhyRadioState.len());
            let on = state.PhyRadioState[..count].iter().any(|p| {
                p.dot11SoftwareRadioState == dot11_radio_state_on && p.dot11HardwareRadioState == dot11_radio_state_on
            });
            WlanFreeMemory(data);
            Some(on)
        }
    }

    /// Asks the adapter to look for networks; `Event::Networks` follows when it is done.
    pub fn scan(&self) {
        unsafe { WlanScan(self.handle, &self.interface, None, None, None) };
    }

    /// The networks in range, one per name, strongest signal first, the connected one on top.
    pub fn networks(&self) -> Vec<Network> {
        let mut list: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
        let mut networks = Vec::new();
        unsafe {
            if WlanGetAvailableNetworkList(self.handle, &self.interface, 0, None, &mut list) != 0 || list.is_null() {
                return networks;
            }
            let count = (*list).dwNumberOfItems as usize;
            for n in std::slice::from_raw_parts((*list).Network.as_ptr(), count) {
                if n.dot11BssType != dot11_BSS_type_infrastructure {
                    continue;
                }
                let len = (n.dot11Ssid.uSSIDLength as usize).min(32);
                let bytes = n.dot11Ssid.ucSSID[..len].to_vec();
                // Hidden networks have no name to show.
                if bytes.iter().all(|&b| b == 0) {
                    continue;
                }
                let profile = (n.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0).then(|| wide(&n.strProfileName));
                networks.push(Network {
                    ssid: String::from_utf8_lossy(&bytes).into_owned(),
                    ssid_bytes: bytes,
                    signal: n.wlanSignalQuality.min(100),
                    security: security(
                        n.bSecurityEnabled.as_bool(),
                        n.dot11DefaultAuthAlgorithm,
                        n.dot11DefaultCipherAlgorithm,
                    ),
                    connected: n.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0,
                    profile: profile.filter(|p| !p.is_empty()),
                });
            }
            WlanFreeMemory(list as *const _);
        }
        merge(networks)
    }

    /// Connects with the saved profile or, for a new network, a profile made from the
    /// password (none for open networks). Returns the name of a profile Lilt created, so
    /// it can be removed again if the connection fails.
    pub fn connect(&self, network: &Network, password: Option<&str>) -> Result<Option<String>, String> {
        let (profile, created) = match (&network.profile, network.security) {
            (Some(profile), _) => (profile.clone(), None),
            (None, Security::Other) => return Err("This network needs Windows' Wi-Fi settings to join.".into()),
            (None, security) => {
                let password = match (security, password.map(|p| p.trim_end_matches(['\r', '\n']).to_owned())) {
                    (Security::Open, _) => None,
                    (_, Some(p)) if (8..=63).contains(&p.chars().count()) => Some(p),
                    _ => return Err("Wi-Fi passwords have 8 to 63 characters.".into()),
                };
                let xml = profile_xml(network, password.as_deref());
                let mut reason = 0u32;
                let result = unsafe {
                    WlanSetProfile(
                        self.handle,
                        &self.interface,
                        0,
                        &HSTRING::from(xml),
                        PCWSTR::null(),
                        true,
                        None,
                        &mut reason,
                    )
                };
                if result != 0 {
                    tracing::warn!(result, reason, "could not save a Wi-Fi profile");
                    return Err("Windows did not accept these network settings.".into());
                }
                (network.ssid.clone(), Some(network.ssid.clone()))
            }
        };
        let name = HSTRING::from(profile.as_str());
        let parameters = WLAN_CONNECTION_PARAMETERS {
            wlanConnectionMode: wlan_connection_mode_profile,
            strProfile: PCWSTR(name.as_ptr()),
            pDot11Ssid: std::ptr::null_mut(),
            pDesiredBssidList: std::ptr::null_mut(),
            dot11BssType: dot11_BSS_type_infrastructure,
            dwFlags: 0,
        };
        if unsafe { WlanConnect(self.handle, &self.interface, &parameters, None) } != 0 {
            if let Some(created) = &created {
                self.delete_profile(created);
            }
            return Err("Windows could not start connecting.".into());
        }
        Ok(created)
    }

    pub fn disconnect(&self) {
        unsafe { WlanDisconnect(self.handle, &self.interface, None) };
    }

    pub fn delete_profile(&self, name: &str) {
        unsafe { WlanDeleteProfile(self.handle, &self.interface, &HSTRING::from(name), None) };
    }
}

impl Drop for Wlan {
    fn drop(&mut self) {
        unsafe {
            if self.events.is_some() {
                // Waits for callbacks in progress, so the sender can be dropped after.
                WlanRegisterNotification(self.handle, WLAN_NOTIFICATION_SOURCE_NONE, true, None, None, None, None);
            }
            WlanCloseHandle(self.handle, None);
        }
    }
}

unsafe extern "system" fn on_notification(data: *mut L2_NOTIFICATION_DATA, context: *mut core::ffi::c_void) {
    if data.is_null() || context.is_null() {
        return;
    }
    let notify = unsafe { &*(context as *const Notify) };
    let data = unsafe { &*data };
    let code = WLAN_NOTIFICATION_ACM(data.NotificationCode as i32);
    let event = if code == wlan_notification_acm_scan_complete
        || code == wlan_notification_acm_scan_fail
        || code == wlan_notification_acm_network_available
        || code == wlan_notification_acm_interface_arrival
        || code == wlan_notification_acm_interface_removal
    {
        Event::Networks
    } else if code == wlan_notification_acm_connection_complete || code == wlan_notification_acm_connection_attempt_fail
    {
        let reason = if data.pData.is_null()
            || (data.dwDataSize as usize) < std::mem::size_of::<WLAN_CONNECTION_NOTIFICATION_DATA>() - 2
        {
            0
        } else {
            unsafe { (*(data.pData as *const WLAN_CONNECTION_NOTIFICATION_DATA)).wlanReasonCode }
        };
        if code == wlan_notification_acm_connection_complete && reason == 0 {
            Event::Connected
        } else {
            Event::Failed { reason }
        }
    } else if code == wlan_notification_acm_disconnected {
        Event::Disconnected
    } else {
        return;
    };
    notify(event);
}

fn wide(buffer: &[u16]) -> String {
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

fn security(enabled: bool, auth: DOT11_AUTH_ALGORITHM, cipher: DOT11_CIPHER_ALGORITHM) -> Security {
    if !enabled {
        return Security::Open;
    }
    let tkip = cipher == DOT11_CIPHER_ALGO_TKIP;
    if auth == DOT11_AUTH_ALGO_RSNA_PSK || auth == DOT11_AUTH_ALGO_WPA_PSK {
        Security::Personal { wpa3: false, tkip }
    } else if auth == DOT11_AUTH_ALGO_WPA3_SAE {
        Security::Personal { wpa3: true, tkip: false }
    } else {
        Security::Other
    }
}

/// One entry per network name: Windows lists a name once per profile and band.
fn merge(networks: Vec<Network>) -> Vec<Network> {
    let mut merged: Vec<Network> = Vec::new();
    for n in networks {
        match merged.iter_mut().find(|m| m.ssid_bytes == n.ssid_bytes) {
            Some(m) => {
                m.signal = m.signal.max(n.signal);
                m.connected |= n.connected;
                if m.profile.is_none() {
                    m.profile = n.profile;
                }
            }
            None => merged.push(n),
        }
    }
    merged.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.signal.cmp(&a.signal)).then(a.ssid.cmp(&b.ssid)));
    merged
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// A Windows Wi-Fi profile for a new network, connecting automatically like one joined
/// in Windows' own flyout.
pub fn profile_xml(network: &Network, password: Option<&str>) -> String {
    let hex: String = network.ssid_bytes.iter().map(|b| format!("{b:02X}")).collect();
    let name = escape(&network.ssid);
    let security = match (network.security, password) {
        (Security::Personal { wpa3, tkip }, Some(password)) => format!(
            "<authEncryption><authentication>{}</authentication><encryption>{}</encryption><useOneX>false</useOneX></authEncryption>\
             <sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>",
            if wpa3 { "WPA3SAE" } else { "WPA2PSK" },
            if tkip { "TKIP" } else { "AES" },
            escape(password)
        ),
        _ => "<authEncryption><authentication>open</authentication><encryption>none</encryption><useOneX>false</useOneX></authEncryption>"
            .into(),
    };
    format!(
        "<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\">\
         <name>{name}</name><SSIDConfig><SSID><hex>{hex}</hex><name>{name}</name></SSID></SSIDConfig>\
         <connectionType>ESS</connectionType><connectionMode>auto</connectionMode>\
         <MSM><security>{security}</security></MSM></WLANProfile>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network(ssid: &str, signal: u32, connected: bool) -> Network {
        Network {
            ssid: ssid.into(),
            ssid_bytes: ssid.as_bytes().to_vec(),
            signal,
            security: Security::Personal { wpa3: false, tkip: false },
            connected,
            profile: None,
        }
    }

    #[test]
    fn networks_are_merged_and_ordered() {
        let mut saved = network("Home", 40, false);
        saved.profile = Some("Home".into());
        let merged =
            merge(vec![network("Cafe", 90, false), network("Home", 70, true), saved, network("Office", 90, false)]);
        let names: Vec<&str> = merged.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(names, ["Home", "Cafe", "Office"]);
        assert_eq!((merged[0].signal, merged[0].connected, merged[0].profile.as_deref()), (70, true, Some("Home")));
    }

    #[test]
    fn security_is_read_from_the_algorithms() {
        assert_eq!(security(false, DOT11_AUTH_ALGO_80211_OPEN, DOT11_CIPHER_ALGORITHM(0)), Security::Open);
        assert_eq!(
            security(true, DOT11_AUTH_ALGO_RSNA_PSK, DOT11_CIPHER_ALGO_CCMP),
            Security::Personal { wpa3: false, tkip: false }
        );
        assert_eq!(
            security(true, DOT11_AUTH_ALGO_WPA3_SAE, DOT11_CIPHER_ALGO_CCMP),
            Security::Personal { wpa3: true, tkip: false }
        );
        assert_eq!(security(true, DOT11_AUTH_ALGO_RSNA, DOT11_CIPHER_ALGO_CCMP), Security::Other);
    }

    #[test]
    fn profiles_escape_names_and_passwords() {
        let n = network("Tom & Jerry's <5G>", 50, false);
        let xml = profile_xml(&n, Some("p<a>ss&\"word'"));
        assert!(xml.contains("<name>Tom &amp; Jerry&apos;s &lt;5G&gt;</name>"));
        assert!(xml.contains("<hex>546F6D"));
        assert!(xml.contains("<authentication>WPA2PSK</authentication><encryption>AES</encryption>"));
        assert!(xml.contains("<keyMaterial>p&lt;a&gt;ss&amp;&quot;word&apos;</keyMaterial>"));
        let mut open = network("Library", 50, false);
        open.security = Security::Open;
        assert!(profile_xml(&open, None).contains("<authentication>open</authentication>"));
    }
}
