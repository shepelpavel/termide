//! System resource monitoring for termide.
//!
//! Provides CPU and memory usage information.

use std::sync::{Arc, Mutex};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

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

/// System monitor for tracking resource usage.
#[derive(Debug)]
pub struct SystemMonitor {
    system: Arc<Mutex<System>>,
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

        Self {
            system: Arc::new(Mutex::new(system)),
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
    #[cfg(test)]
    pub fn used_gb(&self) -> u64 {
        (self.used() as f64 / BYTES_PER_GB).round() as u64
    }

    /// Get total space in GB.
    #[cfg(test)]
    pub fn total_gb(&self) -> u64 {
        (self.total as f64 / BYTES_PER_GB).round() as u64
    }

    /// Get device name (extracted from path).
    #[cfg(test)]
    pub fn device_name(&self) -> Option<String> {
        self.device
            .as_ref()
            .map(|d| d.strip_prefix("/dev/").unwrap_or(d).to_uppercase())
    }
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

use std::path::Path;

/// Resolve dm-X device to physical partition.
/// e.g., /dev/dm-0 -> /dev/nvme0n1p2
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
                    if best_match.is_none() || mount_len > best_match.as_ref().unwrap().1 {
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

/// Get disk space information for a given path.
///
/// Returns `DiskSpaceInfo` with device name, available and total space.
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
