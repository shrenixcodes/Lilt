//! CPU, memory and network readings backed by one persistent `sysinfo` instance.
//!
//! Each metric is refreshed on its own, so only the data that is about to be shown
//! is collected. Scheduling lives in [`SamplingPolicy`]; the caller owns the timers.

use std::time::{Duration, Instant};

use sysinfo::{MemoryRefreshKind, Networks, System};

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

pub struct SystemMonitor {
    system: System,
    /// `None` if the network counters could not be opened.
    networks: Option<Networks>,
    last_network_sample: Option<Instant>,
}

impl SystemMonitor {
    pub fn new() -> Result<Self, sysinfo::Error> {
        let mut system = System::new()?;
        // Establish the CPU baseline so the first real sample has something to diff against.
        system.refresh_cpu_usage();
        let networks = Networks::new_with_refreshed_list()
            .map_err(|err| tracing::warn!(%err, "network counters unavailable"))
            .ok();
        Ok(Self { system, networks, last_network_sample: Some(Instant::now()) })
    }

    /// Average CPU usage since the previous call, 0..=100.
    pub fn cpu(&mut self) -> f32 {
        self.system.refresh_cpu_usage();
        self.system.global_cpu_usage().clamp(0.0, 100.0)
    }

    pub fn memory(&mut self) -> Memory {
        self.system.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        Memory { total: self.system.total_memory(), available: self.system.available_memory() }
    }

    /// Average transfer rates since the previous call across physical-looking interfaces,
    /// or `None` if network counters are unavailable.
    pub fn network(&mut self) -> Option<NetworkRates> {
        let networks = self.networks.as_mut()?;
        networks.refresh(true);
        let now = Instant::now();
        let elapsed = self.last_network_sample.replace(now).map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        if elapsed < 0.05 {
            return Some(NetworkRates::default());
        }
        let (mut down, mut up) = (0u64, 0u64);
        for (name, data) in networks.iter() {
            if is_counted_interface(&name.to_string_lossy()) {
                down += data.received();
                up += data.transmitted();
            }
        }
        Some(NetworkRates { down: down as f64 / elapsed, up: up as f64 / elapsed })
    }
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
    fn live_readings_are_plausible() {
        let mut monitor = SystemMonitor::new().expect("system counters");
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        let cpu = monitor.cpu();
        assert!((0.0..=100.0).contains(&cpu));
        let memory = monitor.memory();
        assert!(memory.total > 0 && memory.available <= memory.total);
        let rates = monitor.network().expect("network counters");
        assert!(rates.down >= 0.0 && rates.up >= 0.0);
    }
}
