//! Disk space utilities for the terminal.
//!
//! This module provides functions for querying disk space information,
//! resolving device mapper devices to physical partitions, and
//! determining which device a path resides on.

use termide_ui::system_monitor::DiskSpaceInfo;

/// Resolve dm-X device to physical partition
/// e.g., /dev/dm-0 -> /dev/nvme0n1p2
pub fn resolve_dm_device(device: &str) -> Option<String> {
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

/// Get device name from /proc/mounts for a given path
pub fn get_device_for_path(path: &str) -> Option<String> {
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
        if let Ok(canonical_path) = std::path::Path::new(path).canonicalize() {
            if let Ok(canonical_mount) = std::path::Path::new(mount_point).canonicalize() {
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
        let resolved = std::path::Path::new(&device)
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

/// Get disk space information for specified path
pub fn get_disk_space_for_path(path: &str) -> Option<DiskSpaceInfo> {
    use std::ffi::CString;

    let path_cstr = CString::new(path).ok()?;

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
            // On macOS, f_bavail and f_blocks are u32, f_bsize is u64
            // On Linux, all are u64
            #[cfg(target_os = "macos")]
            let available = (stat.f_bavail as u64).saturating_mul(stat.f_bsize);
            #[cfg(not(target_os = "macos"))]
            let available = stat.f_bavail.saturating_mul(stat.f_bsize);

            #[cfg(target_os = "macos")]
            let total = (stat.f_blocks as u64).saturating_mul(stat.f_bsize);
            #[cfg(not(target_os = "macos"))]
            let total = stat.f_blocks.saturating_mul(stat.f_bsize);

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
