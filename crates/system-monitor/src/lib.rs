//! System resource monitoring for termide.
//!
//! Provides CPU, memory, and network usage information.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Networks, ProcessRefreshKind, RefreshKind, System,
    UpdateKind,
};

/// System resource statistics.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemStats {
    /// CPU usage percentage (0-100).
    pub cpu_usage: f32,
    /// Memory usage in bytes.
    pub memory_used: u64,
    /// Total memory in bytes.
    pub memory_total: u64,
}

impl SystemStats {
    /// Get memory usage as percentage.
    pub fn memory_percent(&self) -> f32 {
        if self.memory_total == 0 {
            0.0
        } else {
            (self.memory_used as f32 / self.memory_total as f32) * 100.0
        }
    }
}

/// RAM unit for formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RamUnit {
    Gigabytes,
    Megabytes,
}

/// Network throughput state.
struct NetworkState {
    networks: Networks,
    download_rate: u64,
    upload_rate: u64,
    last_refresh: Instant,
}

/// System monitor for tracking resource usage.
pub struct SystemMonitor {
    system: Arc<Mutex<System>>,
    net_state: Mutex<NetworkState>,
}

// Manual Debug impl because Networks doesn't implement Debug
impl std::fmt::Debug for SystemMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemMonitor")
            .field("system", &self.system)
            .finish_non_exhaustive()
    }
}

impl Default for SystemMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Bytes per gigabyte.
const BYTES_PER_GB: f64 = 1_073_741_824.0;
/// Bytes per megabyte.
const BYTES_PER_MB: f64 = 1_048_576.0;

impl SystemMonitor {
    /// Create a new system monitor.
    pub fn new() -> Self {
        let refresh_kind = Self::refresh_kind();

        let mut system = System::new_with_specifics(refresh_kind);
        system.refresh_specifics(refresh_kind);

        let networks = Networks::new_with_refreshed_list();

        Self {
            system: Arc::new(Mutex::new(system)),
            net_state: Mutex::new(NetworkState {
                networks,
                download_rate: 0,
                upload_rate: 0,
                last_refresh: Instant::now(),
            }),
        }
    }

    /// Get refresh kind configuration.
    fn refresh_kind() -> RefreshKind {
        RefreshKind::new()
            .with_cpu(CpuRefreshKind::new().with_cpu_usage())
            .with_memory(MemoryRefreshKind::new().with_ram())
    }

    /// Execute a function with locked system, returning default on lock failure.
    fn with_system<T: Default>(&self, f: impl FnOnce(&System) -> T) -> T {
        self.system.lock().map(|sys| f(&sys)).unwrap_or_default()
    }

    /// Refresh system information.
    pub fn refresh(&self) {
        if let Ok(mut sys) = self.system.lock() {
            sys.refresh_specifics(Self::refresh_kind());
        }
        self.refresh_networks();
    }

    /// Refresh network statistics and compute throughput rates.
    fn refresh_networks(&self) {
        if let Ok(mut state) = self.net_state.lock() {
            let elapsed = state.last_refresh.elapsed();
            let elapsed_secs = elapsed.as_secs_f64();

            state.networks.refresh();

            let mut total_rx: u64 = 0;
            let mut total_tx: u64 = 0;
            for (_name, data) in &state.networks {
                total_rx += data.received();
                total_tx += data.transmitted();
            }

            if elapsed_secs > 0.0 {
                state.download_rate = (total_rx as f64 / elapsed_secs) as u64;
                state.upload_rate = (total_tx as f64 / elapsed_secs) as u64;
            }

            state.last_refresh = Instant::now();
        }
    }

    /// Alias for refresh() - backward compatibility.
    #[inline]
    pub fn update(&mut self) {
        self.refresh();
    }

    /// Get current system stats.
    pub fn stats(&self) -> SystemStats {
        self.with_system(|sys| SystemStats {
            cpu_usage: sys.global_cpu_usage(),
            memory_used: sys.used_memory(),
            memory_total: sys.total_memory(),
        })
    }

    /// Get CPU usage as integer percentage (0-100).
    pub fn cpu_usage(&self) -> u8 {
        self.with_system(|sys| sys.global_cpu_usage().round() as u8)
    }

    /// Get memory usage percentage.
    pub fn memory_percent(&self) -> f32 {
        self.stats().memory_percent()
    }

    /// Get RAM info in specified unit: (used, total).
    fn ram_info(&self, divisor: f64) -> (u64, u64) {
        self.with_system(|sys| {
            let used = (sys.used_memory() as f64 / divisor).round() as u64;
            let total = (sys.total_memory() as f64 / divisor).round() as u64;
            (used, total)
        })
    }

    /// Get RAM info: (used_gb, total_gb).
    pub fn ram_info_gb(&self) -> (u64, u64) {
        self.ram_info(BYTES_PER_GB)
    }

    /// Get RAM info: (used_mb, total_mb).
    pub fn ram_info_mb(&self) -> (u64, u64) {
        self.ram_info(BYTES_PER_MB)
    }

    /// Get RAM usage as integer percentage (0-100).
    pub fn ram_usage_percent(&self) -> u8 {
        self.with_system(|sys| {
            let used = sys.used_memory();
            let total = sys.total_memory();
            if total > 0 {
                ((used as f64 / total as f64) * 100.0).round() as u8
            } else {
                0
            }
        })
    }

    /// Format RAM info with automatic unit selection.
    pub fn format_ram(&self) -> (String, RamUnit) {
        let (used_gb, total_gb) = self.ram_info_gb();
        if total_gb >= 1 {
            (format!("{}/{}", used_gb, total_gb), RamUnit::Gigabytes)
        } else {
            let (used_mb, total_mb) = self.ram_info_mb();
            (format!("{}/{}", used_mb, total_mb), RamUnit::Megabytes)
        }
    }

    /// Get network download rate in bytes per second.
    pub fn net_download_rate(&self) -> u64 {
        self.net_state.lock().map(|s| s.download_rate).unwrap_or(0)
    }

    /// Get network upload rate in bytes per second.
    pub fn net_upload_rate(&self) -> u64 {
        self.net_state.lock().map(|s| s.upload_rate).unwrap_or(0)
    }

    /// Get top N processes by CPU usage, grouped by binary name.
    ///
    /// Performs an on-demand process refresh (not part of periodic refresh).
    pub fn top_cpu_processes(&self, n: usize) -> Vec<ProcessInfo> {
        self.grouped_processes(n, |p| p.cpu_percent, true)
    }

    /// Get top N processes by memory usage, grouped by binary name.
    ///
    /// Performs an on-demand process refresh (not part of periodic refresh).
    pub fn top_memory_processes(&self, n: usize) -> Vec<ProcessInfo> {
        self.grouped_processes(n, |p| p.memory_bytes as f32, false)
    }

    /// Refresh processes and return top N grouped by name, sorted by the given key.
    fn grouped_processes(
        &self,
        n: usize,
        sort_key: impl Fn(&ProcessInfo) -> f32,
        is_cpu: bool,
    ) -> Vec<ProcessInfo> {
        let Ok(mut sys) = self.system.lock() else {
            return Vec::new();
        };

        // One-shot process refresh with CPU, memory, and exe path (to filter kernel threads)
        let process_refresh = ProcessRefreshKind::new()
            .with_cpu()
            .with_memory()
            .with_exe(UpdateKind::OnlyIfNotSet);
        sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, process_refresh);

        // Normalize CPU usage to total system capacity (0-100%)
        let num_cpus = sys.cpus().len().max(1) as f32;

        // Group by process name
        let mut grouped: HashMap<String, ProcessInfo> = HashMap::new();
        for process in sys.processes().values() {
            // Skip threads (they share memory with the main process) and
            // kernel threads (no executable on disk)
            if process.thread_kind().is_some() {
                continue;
            }
            if process.exe().is_none_or(|p| p.as_os_str().is_empty()) {
                continue;
            }
            let name = process.name().to_string_lossy().to_string();
            if name.is_empty() {
                continue;
            }
            let cpu = process.cpu_usage() / num_cpus;
            let mem = process.memory();
            let entry = grouped.entry(name.clone()).or_insert_with(|| ProcessInfo {
                name,
                cpu_percent: 0.0,
                memory_bytes: 0,
                count: 0,
            });
            if is_cpu {
                entry.cpu_percent += cpu;
            } else {
                entry.cpu_percent = entry.cpu_percent.max(cpu);
            }
            entry.memory_bytes += mem;
            entry.count += 1;
        }

        let mut processes: Vec<ProcessInfo> = grouped.into_values().collect();
        processes.sort_by(|a, b| {
            sort_key(b)
                .partial_cmp(&sort_key(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        processes.truncate(n);
        processes
    }
}

/// Information about a process (or group of processes with the same name).
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    /// Process binary name.
    pub name: String,
    /// CPU usage percentage (summed across all processes with this name).
    pub cpu_percent: f32,
    /// Memory usage in bytes (summed across all processes with this name).
    pub memory_bytes: u64,
    /// Number of processes with this name.
    pub count: usize,
}

/// Disk space information.
#[derive(Clone, Debug)]
pub struct DiskSpaceInfo {
    /// Device name (e.g., "NVME0N1", "SDA1").
    pub device: Option<String>,
    /// Available space in bytes.
    pub available: u64,
    /// Total space in bytes.
    pub total: u64,
}

impl DiskSpaceInfo {
    /// Get disk usage percentage (0-100).
    pub fn usage_percent(&self) -> u8 {
        if self.total > 0 {
            let used = self.total.saturating_sub(self.available);
            ((used * 100) / self.total).min(100) as u8
        } else {
            0
        }
    }

    /// Get used space in bytes.
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    /// Get used space in GB.
    pub fn used_gb(&self) -> u64 {
        (self.used() as f64 / BYTES_PER_GB).round() as u64
    }

    /// Get total space in GB.
    pub fn total_gb(&self) -> u64 {
        (self.total as f64 / BYTES_PER_GB).round() as u64
    }

    /// Get device name (extracted from path).
    pub fn device_name(&self) -> Option<String> {
        self.device
            .as_ref()
            .map(|d| d.strip_prefix("/dev/").unwrap_or(d).to_uppercase())
    }
}

/// Format network speed as compact human-readable string.
///
/// Returns strings like "0B/s", "4kB/s", "1.2MB/s", "2.5GB/s".
pub fn format_net_speed(bytes_per_sec: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    let s = if bytes_per_sec >= 1000 * MB {
        format!("{}GB/s", bytes_per_sec.div_ceil(GB))
    } else if bytes_per_sec >= 1000 * KB {
        format!("{}MB/s", bytes_per_sec.div_ceil(MB))
    } else {
        format!("{}kB/s", bytes_per_sec.div_ceil(KB))
    };

    format!("{s:<7}")
}

/// Format bytes as human-readable string.
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

/// Extension trait for DiskSpaceInfo with i18n support.
pub trait DiskSpaceInfoExt {
    /// Format disk space with device name and usage.
    fn format_space(&self) -> String;
}

impl DiskSpaceInfoExt for DiskSpaceInfo {
    fn format_space(&self) -> String {
        let t = termide_i18n::t();

        // Calculate used space and percentage
        let used = self.total.saturating_sub(self.available);
        let percent = if self.total > 0 {
            ((used * 100) / self.total).min(100)
        } else {
            0
        };

        // Convert to GB (rounded to nearest integer)
        let used_gb = (used as f64 / BYTES_PER_GB).round() as u64;
        let total_gb = (self.total as f64 / BYTES_PER_GB).round() as u64;

        if let Some(device) = &self.device {
            // Extract device name from path like "/dev/nvme0n1p2" -> "NVME0N1P2"
            let device_name = device
                .strip_prefix("/dev/")
                .unwrap_or(device)
                .to_uppercase();
            format!(
                "{}: {}/{}{} ({}%)",
                device_name,
                used_gb,
                total_gb,
                t.size_gigabytes(),
                percent
            )
        } else {
            format!(
                "{}/{}{} ({}%)",
                used_gb,
                total_gb,
                t.size_gigabytes(),
                percent
            )
        }
    }
}

// ============================================================================
// Disk space utility functions
// ============================================================================

#[cfg(unix)]
use std::path::Path;

/// Resolve dm-X device to physical partition.
/// e.g., /dev/dm-0 -> /dev/nvme0n1p2
#[cfg(unix)]
fn resolve_dm_device(device: &str) -> Option<String> {
    // Extract dm number (e.g., "dm-0" from "/dev/dm-0")
    let dm_name = device.strip_prefix("/dev/")?;
    if !dm_name.starts_with("dm-") {
        return None;
    }

    // Read /sys/block/dm-X/slaves/ to find physical partition
    let slaves_path = format!("/sys/block/{}/slaves", dm_name);
    let slaves_dir = std::fs::read_dir(&slaves_path).ok()?;

    // Get first slave (physical partition)
    for entry in slaves_dir.flatten() {
        if let Ok(name) = entry.file_name().into_string() {
            return Some(format!("/dev/{}", name));
        }
    }

    None
}

/// Get device name from /proc/mounts for a given path.
#[cfg(unix)]
fn get_device_for_path(path: &Path) -> Option<String> {
    let mounts_content = std::fs::read_to_string("/proc/mounts").ok()?;
    let mut best_match: Option<(String, usize)> = None;

    for line in mounts_content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        let device = parts[0];
        let mount_point = parts[1];

        // Check if this mount point is a prefix of our path
        if let Ok(canonical_path) = path.canonicalize() {
            if let Ok(canonical_mount) = Path::new(mount_point).canonicalize() {
                if canonical_path.starts_with(&canonical_mount) {
                    let mount_len = canonical_mount.as_os_str().len();
                    // Keep track of the longest matching mount point
                    if best_match.as_ref().is_none_or(|b| mount_len > b.1) {
                        best_match = Some((device.to_string(), mount_len));
                    }
                }
            }
        }
    }

    best_match.and_then(|(device, _)| {
        // First try to resolve symlink (e.g., /dev/disk/by-uuid/... -> /dev/nvme0n1p2)
        let resolved = Path::new(&device)
            .canonicalize()
            .ok()
            .and_then(|p| p.to_str().map(|s| s.to_string()))
            .unwrap_or_else(|| device.clone());

        // If it's a dm device, resolve to physical partition
        if resolved.contains("/dm-") {
            resolve_dm_device(&resolved).or(Some(resolved))
        } else {
            Some(resolved)
        }
    })
}

/// Get disk space information for a given path.
///
/// Returns `DiskSpaceInfo` with device name, available and total space.
#[cfg(unix)]
pub fn get_disk_space_info(path: &Path) -> Option<DiskSpaceInfo> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    // Convert path to CString for passing to statvfs
    let path_cstr = CString::new(path.as_os_str().as_bytes()).ok()?;

    // Get device name for this path
    let device = get_device_for_path(path);

    // SAFETY: statvfs is a POSIX function that fills a statvfs struct with
    // filesystem statistics. We zero-initialize the struct to ensure all fields
    // have defined values. path_cstr is a valid null-terminated CString created
    // above. statvfs returns 0 on success and writes valid data to the struct.
    // We only read the struct fields after confirming success (return == 0).
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(path_cstr.as_ptr(), &mut stat) == 0 {
            // f_bavail - available blocks for non-privileged users
            // f_blocks - total blocks in the filesystem
            // f_bsize - block size in bytes
            // On macOS, f_bavail and f_blocks are u32, f_bsize is u64
            // On Linux, all are u64
            #[cfg(target_os = "macos")]
            let available = (stat.f_bavail as u64) * stat.f_bsize;
            #[cfg(not(target_os = "macos"))]
            let available = stat.f_bavail * stat.f_bsize;

            #[cfg(target_os = "macos")]
            let total = (stat.f_blocks as u64) * stat.f_bsize;
            #[cfg(not(target_os = "macos"))]
            let total = stat.f_blocks * stat.f_bsize;

            Some(DiskSpaceInfo {
                device,
                available,
                total,
            })
        } else {
            None
        }
    }
}

/// Get disk space information for all real mounted devices.
///
/// Parses `/proc/mounts`, filters for real devices (`/dev/`),
/// deduplicates by device path, and calls `statvfs` for each.
#[cfg(unix)]
pub fn get_all_disk_space_info() -> Vec<DiskSpaceInfo> {
    let Ok(mounts_content) = std::fs::read_to_string("/proc/mounts") else {
        return Vec::new();
    };

    let mut seen_devices: HashMap<String, String> = HashMap::new(); // device -> mount_point

    for line in mounts_content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let device = parts[0];
        let mount_point = parts[1];

        // Only real devices
        if !device.starts_with("/dev/") {
            continue;
        }

        // Skip pseudo-devices
        if device.starts_with("/dev/loop") || device.starts_with("/dev/ram") {
            continue;
        }

        // Resolve device symlinks/dm
        let resolved = Path::new(device)
            .canonicalize()
            .ok()
            .and_then(|p| p.to_str().map(|s| s.to_string()))
            .unwrap_or_else(|| device.to_string());

        let resolved = if resolved.contains("/dm-") {
            resolve_dm_device(&resolved).unwrap_or(resolved)
        } else {
            resolved
        };

        // Keep only first mount point per device (usually the most relevant)
        seen_devices
            .entry(resolved)
            .or_insert_with(|| mount_point.to_string());
    }

    let mut result = Vec::new();
    for (device, mount_point) in &seen_devices {
        if let Some(info) = get_disk_space_info(Path::new(mount_point)) {
            result.push(DiskSpaceInfo {
                device: Some(device.clone()),
                ..info
            });
        }
    }

    // Sort by device name for consistent ordering
    result.sort_by(|a, b| a.device.cmp(&b.device));
    result
}

#[cfg(windows)]
pub fn get_disk_space_info(path: &std::path::Path) -> Option<DiskSpaceInfo> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let root = path.components().next()?;
    let root_str = format!("{}\\", root.as_os_str().to_string_lossy());

    let wide_path: Vec<u16> = OsStr::new(&root_str)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut free_bytes_available: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut _total_free_bytes: u64 = 0;

    let success = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide_path.as_ptr(),
            &mut free_bytes_available,
            &mut total_bytes,
            &mut _total_free_bytes,
        )
    };

    if success != 0 {
        Some(DiskSpaceInfo {
            device: Some(root_str.trim_end_matches('\\').to_string()),
            available: free_bytes_available,
            total: total_bytes,
        })
    } else {
        None
    }
}

#[cfg(windows)]
pub fn get_all_disk_space_info() -> Vec<DiskSpaceInfo> {
    // Query drives A-Z using GetDiskFreeSpaceExW
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let mut result = Vec::new();
    for letter in b'A'..=b'Z' {
        let drive = format!("{}:\\", letter as char);
        let wide_path: Vec<u16> = OsStr::new(&drive)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut free_bytes_available: u64 = 0;
        let mut total_bytes: u64 = 0;
        let mut _total_free_bytes: u64 = 0;

        let success = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide_path.as_ptr(),
                &mut free_bytes_available,
                &mut total_bytes,
                &mut _total_free_bytes,
            )
        };

        if success != 0 && total_bytes > 0 {
            result.push(DiskSpaceInfo {
                device: Some(format!("{}:", letter as char)),
                available: free_bytes_available,
                total: total_bytes,
            });
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // SystemStats::memory_percent()
    // =========================================================================

    #[test]
    fn test_memory_percent_normal() {
        let stats = SystemStats {
            cpu_usage: 50.0,
            memory_used: 4_000_000_000,
            memory_total: 16_000_000_000,
        };
        let percent = stats.memory_percent();
        assert!((percent - 25.0).abs() < 0.01);
    }

    #[test]
    fn test_memory_percent_zero_total() {
        let stats = SystemStats {
            cpu_usage: 0.0,
            memory_used: 0,
            memory_total: 0,
        };
        assert_eq!(stats.memory_percent(), 0.0);
    }

    #[test]
    fn test_memory_percent_full() {
        let stats = SystemStats {
            cpu_usage: 0.0,
            memory_used: 16_000_000_000,
            memory_total: 16_000_000_000,
        };
        assert!((stats.memory_percent() - 100.0).abs() < 0.01);
    }

    // =========================================================================
    // DiskSpaceInfo
    // =========================================================================

    #[test]
    fn test_disk_usage_percent() {
        let info = DiskSpaceInfo {
            device: Some("/dev/sda1".to_string()),
            available: 200_000_000_000,
            total: 1_000_000_000_000,
        };
        // used = 800GB, total = 1TB, percent = 80%
        assert_eq!(info.usage_percent(), 80);
    }

    #[test]
    fn test_disk_usage_percent_zero_total() {
        let info = DiskSpaceInfo {
            device: None,
            available: 0,
            total: 0,
        };
        assert_eq!(info.usage_percent(), 0);
    }

    #[test]
    fn test_disk_used_bytes() {
        let info = DiskSpaceInfo {
            device: None,
            available: 300,
            total: 1000,
        };
        assert_eq!(info.used(), 700);
    }

    #[test]
    fn test_disk_device_name() {
        let info = DiskSpaceInfo {
            device: Some("/dev/nvme0n1p2".to_string()),
            available: 0,
            total: 0,
        };
        assert_eq!(info.device_name(), Some("NVME0N1P2".to_string()));
    }

    #[test]
    fn test_disk_device_name_no_prefix() {
        let info = DiskSpaceInfo {
            device: Some("sda1".to_string()),
            available: 0,
            total: 0,
        };
        assert_eq!(info.device_name(), Some("SDA1".to_string()));
    }

    #[test]
    fn test_disk_device_name_none() {
        let info = DiskSpaceInfo {
            device: None,
            available: 0,
            total: 0,
        };
        assert_eq!(info.device_name(), None);
    }

    // =========================================================================
    // format_bytes
    // =========================================================================

    #[test]
    fn test_format_bytes_bytes() {
        assert_eq!(format_bytes(500), "500B");
    }

    #[test]
    fn test_format_bytes_kb() {
        assert_eq!(format_bytes(2048), "2.0KB");
    }

    #[test]
    fn test_format_bytes_mb() {
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0MB");
    }

    #[test]
    fn test_format_bytes_gb() {
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0GB");
    }

    #[test]
    fn test_format_bytes_zero() {
        assert_eq!(format_bytes(0), "0B");
    }

    // =========================================================================
    // DiskSpaceInfo GB calculations
    // =========================================================================

    // =========================================================================
    // Process info
    // =========================================================================

    #[test]
    fn test_top_cpu_processes_sorted() {
        let monitor = SystemMonitor::new();
        let procs = monitor.top_cpu_processes(10);
        // Verify descending CPU order
        for window in procs.windows(2) {
            assert!(window[0].cpu_percent >= window[1].cpu_percent);
        }
    }

    #[test]
    fn test_top_memory_processes_sorted() {
        let monitor = SystemMonitor::new();
        let procs = monitor.top_memory_processes(10);
        // Verify descending memory order
        for window in procs.windows(2) {
            assert!(window[0].memory_bytes >= window[1].memory_bytes);
        }
    }

    #[test]
    fn test_processes_grouped_by_name() {
        let monitor = SystemMonitor::new();
        let procs = monitor.top_cpu_processes(100);
        // All names should be unique (grouped)
        let mut names: Vec<&str> = procs.iter().map(|p| p.name.as_str()).collect();
        let len_before = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), len_before);
    }

    #[cfg(unix)]
    #[test]
    fn test_get_all_disk_space_info() {
        let disks = get_all_disk_space_info();
        // Should find at least one real disk on any Linux system
        assert!(!disks.is_empty());
        for disk in &disks {
            assert!(disk.device.is_some());
            assert!(disk.total > 0);
            // No virtual filesystems
            let dev = disk.device.as_ref().unwrap();
            assert!(dev.starts_with("/dev/"));
            assert!(!dev.starts_with("/dev/loop"));
        }
    }

    // =========================================================================
    // DiskSpaceInfo GB calculations
    // =========================================================================

    #[test]
    fn test_disk_used_gb() {
        let info = DiskSpaceInfo {
            device: None,
            available: 500 * 1_073_741_824, // 500 GB available
            total: 1000 * 1_073_741_824,    // 1 TB total
        };
        assert_eq!(info.used_gb(), 500);
        assert_eq!(info.total_gb(), 1000);
    }
}
