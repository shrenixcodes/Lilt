//! CPU, memory and network readings, read straight from Windows.
//!
//! Each metric is read on its own, so only the data that is about to be shown is
//! collected. Scheduling lives in [`SamplingPolicy`]; the caller owns the timers.
//!
//! CPU usage comes from `GetSystemTimes` rather than the Performance Data Helper, whose
//! counter tables would cost several megabytes of memory for one number.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::FILETIME;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::NetworkManagement::Ndis::MediaConnectStateDisconnected;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
}

impl Memory {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    pub fn used_fraction(&self) -> f32 {
        if self.total == 0 { 0.0 } else { self.used() as f32 / self.total as f32 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetworkRates {
    /// Bytes per second received.
    pub down: f64,
    /// Bytes per second sent.
    pub up: f64,
}

/// Processor time since boot, in 100 ns units, summed over all processors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub idle: u64,
    /// Kernel and user time; kernel time includes idle time.
    pub total: u64,
}

impl CpuTimes {
    fn now() -> Option<Self> {
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
        let value = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
        Some(Self { idle: value(idle), total: value(kernel) + value(user) })
    }
}

/// Share of processor time spent working between two readings, 0..=100.
pub fn cpu_usage(before: CpuTimes, after: CpuTimes) -> f32 {
    let total = after.total.saturating_sub(before.total);
    let idle = after.idle.saturating_sub(before.idle).min(total);
    if total == 0 { 0.0 } else { ((total - idle) as f64 / total as f64 * 100.0) as f32 }
}

pub struct SystemMonitor {
    last_cpu: Option<CpuTimes>,
    /// Received and sent byte counters per interface at the previous network reading.
    last_octets: HashMap<u64, (u64, u64)>,
    last_network_sample: Option<Instant>,
}

impl SystemMonitor {
    pub fn new() -> Self {
        // Baselines, so that the first real readings have something to compare with.
        let mut monitor = Self { last_cpu: CpuTimes::now(), last_octets: HashMap::new(), last_network_sample: None };
        monitor.network();
        monitor
    }

    /// Average CPU usage since the previous call, 0..=100.
    pub fn cpu(&mut self) -> f32 {
        let now = CpuTimes::now();
        let usage = match (self.last_cpu, now) {
            (Some(before), Some(after)) => cpu_usage(before, after),
            _ => 0.0,
        };
        self.last_cpu = now.or(self.last_cpu);
        usage
    }

    pub fn memory(&mut self) -> Memory {
        let mut status =
            MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        match unsafe { GlobalMemoryStatusEx(&mut status) } {
            Ok(()) => Memory { total: status.ullTotalPhys, available: status.ullAvailPhys },
            Err(_) => Memory::default(),
        }
    }

    /// Average transfer rates since the previous call across physical-looking interfaces,
    /// or `None` if network counters are unavailable.
    pub fn network(&mut self) -> Option<NetworkRates> {
        let counters = interface_octets()?;
        let now = Instant::now();
        let elapsed = self.last_network_sample.replace(now).map(|t| now.duration_since(t).as_secs_f64());
        let (mut down, mut up) = (0u64, 0u64);
        for (luid, (received, sent)) in &counters {
            if let Some((was_received, was_sent)) = self.last_octets.get(luid) {
                down += received.saturating_sub(*was_received);
                up += sent.saturating_sub(*was_sent);
            }
        }
        self.last_octets = counters;
        match elapsed {
            Some(elapsed) if elapsed >= 0.05 => {
                Some(NetworkRates { down: down as f64 / elapsed, up: up as f64 / elapsed })
            }
            _ => Some(NetworkRates::default()),
        }
    }
}

impl Default for SystemMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Byte counters of the counted interfaces, by interface LUID.
///
/// Software layers are left out as `sysinfo` did before: interfaces without link speed,
/// disconnected ones, ones without a hardware address, and every interface that shares
/// its GUID with another (filter layers stacked on an adapter); then the name filter
/// below.
fn interface_octets() -> Option<HashMap<u64, (u64, u64)>> {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    unsafe { GetIfTable2(&mut table) }.ok().ok()?;
    if table.is_null() {
        return None;
    }
    // SAFETY: GetIfTable2 succeeded; the table holds NumEntries rows until it is freed.
    let rows = unsafe { std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize) };
    let eligible: Vec<_> = rows
        .iter()
        .filter(|r| {
            !(r.TransmitLinkSpeed == 0 && r.ReceiveLinkSpeed == 0)
                && r.MediaConnectState != MediaConnectStateDisconnected
                && r.PhysicalAddressLength != 0
        })
        .collect();
    let mut groups: HashMap<windows::core::GUID, usize> = HashMap::new();
    for row in &eligible {
        *groups.entry(row.InterfaceGuid).or_default() += 1;
    }
    let mut counters = HashMap::new();
    for row in eligible {
        if groups.get(&row.InterfaceGuid).copied().unwrap_or(0) > 1 {
            continue;
        }
        let length = row.Alias.iter().position(|&c| c == 0).unwrap_or(row.Alias.len());
        let name = String::from_utf16_lossy(&row.Alias[..length]);
        if is_counted_interface(&name) {
            // SAFETY: NET_LUID_LH is a union whose `Value` covers all of it.
            let luid = unsafe { row.InterfaceLuid.Value };
            counters.insert(luid, (row.InOctets, row.OutOctets));
        }
    }
    unsafe { FreeMibTable(table as *const _) };
    Some(counters)
}

/// Excludes loopback, virtual adapters and driver filter layers. Windows lists filter
/// layers (e.g. "Wi-Fi-... LightWeight Filter-0000") as separate interfaces carrying the
/// same traffic as the adapter beneath them, so counting them would double the rates.
fn is_counted_interface(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    const SKIP: &[&str] = &[
        "loopback",
        "filter",
        "packet scheduler",
        "-wfp",
        "vethernet",
        "hyper-v",
        "virtualbox",
        "vmware",
        "teredo",
        "isatap",
        "6to4",
        "wsl",
        "docker",
    ];
    !SKIP.iter().any(|s| lower.contains(s))
}

/// How often each metric is refreshed. `None` means the metric is not sampled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SamplingPolicy {
    pub cpu: Option<Duration>,
    pub memory: Option<Duration>,
    pub network: Option<Duration>,
}

impl SamplingPolicy {
    pub const OFF: Self = Self { cpu: None, memory: None, network: None };

    /// Metrics are only shown in the expanded island, so nothing is sampled while it is
    /// compact or hidden. Low-power mode stretches every interval.
    pub fn for_state(visible: bool, expanded: bool, low_power: bool) -> Self {
        if !visible || !expanded {
            return Self::OFF;
        }
        let secs = |normal: u64, low: u64| Some(Duration::from_secs(if low_power { low } else { normal }));
        Self { cpu: secs(2, 5), memory: secs(5, 10), network: secs(1, 5) }
    }
}

/// Formats a byte rate compactly, e.g. `840 KB/s`, `12.4 MB/s`.
pub fn format_rate(bytes_per_second: f64) -> String {
    let (value, unit) = scale(bytes_per_second.max(0.0));
    if unit == "B" || value >= 100.0 { format!("{value:.0} {unit}/s") } else { format!("{value:.1} {unit}/s") }
}

/// Formats a byte count in GB with one decimal, e.g. `6.1 GB`.
pub fn format_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1024f64.powi(3))
}

fn scale(bytes: f64) -> (f64, &'static str) {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    (value, UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_formatting() {
        assert_eq!(format_rate(0.0), "0 B/s");
        assert_eq!(format_rate(512.0), "512 B/s");
        assert_eq!(format_rate(1500.0), "1.5 KB/s");
        assert_eq!(format_rate(860_000.0), "840 KB/s");
        assert_eq!(format_rate(13_000_000.0), "12.4 MB/s");
        assert_eq!(format_rate(-5.0), "0 B/s");
    }

    #[test]
    fn gb_formatting() {
        assert_eq!(format_gb(8 * 1024 * 1024 * 1024), "8.0 GB");
    }

    #[test]
    fn memory_fraction() {
        let m = Memory { total: 100, available: 25 };
        assert_eq!(m.used(), 75);
        assert!((m.used_fraction() - 0.75).abs() < f32::EPSILON);
        assert_eq!(Memory::default().used_fraction(), 0.0);
    }

    #[test]
    fn policy_is_off_unless_expanded_and_visible() {
        assert_eq!(SamplingPolicy::for_state(true, false, false), SamplingPolicy::OFF);
        assert_eq!(SamplingPolicy::for_state(false, true, false), SamplingPolicy::OFF);
        let normal = SamplingPolicy::for_state(true, true, false);
        let low = SamplingPolicy::for_state(true, true, true);
        assert_eq!(normal.network, Some(Duration::from_secs(1)));
        assert!(low.cpu > normal.cpu && low.memory > normal.memory && low.network > normal.network);
    }

    #[test]
    fn interface_filter() {
        assert!(is_counted_interface("Wi-Fi"));
        assert!(is_counted_interface("Ethernet 2"));
        assert!(!is_counted_interface("Loopback Pseudo-Interface 1"));
        assert!(!is_counted_interface("vEthernet (WSL)"));
        assert!(!is_counted_interface("Wi-Fi-Realtek NDIS LightWeight Filter (NDIS 6.40)-0000"));
        assert!(!is_counted_interface("Ethernet-WFP Native MAC Layer LightWeight Filter-0000"));
        assert!(!is_counted_interface("Ethernet-QoS Packet Scheduler-0000"));
    }

    #[test]
    fn cpu_usage_is_the_busy_share() {
        let before = CpuTimes { idle: 1_000, total: 4_000 };
        assert_eq!(cpu_usage(before, CpuTimes { idle: 1_750, total: 5_000 }), 25.0);
        assert_eq!(cpu_usage(before, before), 0.0);
        // Counters never run backwards; if they appear to, nothing is reported.
        assert_eq!(cpu_usage(before, CpuTimes { idle: 0, total: 0 }), 0.0);
        assert_eq!(cpu_usage(before, CpuTimes { idle: 9_000, total: 5_000 }), 0.0);
    }

    #[test]
    fn live_readings_are_plausible() {
        let mut monitor = SystemMonitor::new();
        std::thread::sleep(Duration::from_millis(250));
        let cpu = monitor.cpu();
        assert!((0.0..=100.0).contains(&cpu));
        let memory = monitor.memory();
        assert!(memory.total > 0 && memory.available <= memory.total);
        let rates = monitor.network().expect("network counters");
        assert!(rates.down >= 0.0 && rates.up >= 0.0);
    }
}
