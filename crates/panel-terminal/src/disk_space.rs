//! Disk space utilities for the terminal.
//!
//! This module provides functions for querying disk space information,
//! resolving device mapper devices to physical partitions, and
//! determining which device a path resides on.

use termide_ui::system_monitor::DiskSpaceInfo;

/// Get disk space information for specified path
#[cfg(unix)]
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

/// Get disk space information for specified path (Windows implementation)
#[cfg(windows)]
pub fn get_disk_space_for_path(path: &str) -> Option<DiskSpaceInfo> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    // Get the root of the path (e.g., "C:\")
    let root = std::path::Path::new(path).components().next()?;
    let root_str = format!("{}\\", root.as_os_str().to_string_lossy());

    let wide_path: Vec<u16> = OsStr::new(&root_str)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut free_bytes_available: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut _total_free_bytes: u64 = 0;

    // SAFETY: GetDiskFreeSpaceExW is a Windows API function that writes disk
    // space info into the provided pointers. wide_path is a valid null-terminated
    // wide string. The function returns non-zero on success.
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

/// Resolve dm-X device to physical partition (Unix only)
/// e.g., /dev/dm-0 -> /dev/nvme0n1p2
#[cfg(unix)]
pub fn resolve_dm_device(device: &str) -> Option<String> {
    let dm_name = device.strip_prefix("/dev/")?;
    if !dm_name.starts_with("dm-") {
        return None;
    }

    let slaves_path = format!("/sys/block/{}/slaves", dm_name);
    let slaves_dir = std::fs::read_dir(&slaves_path).ok()?;

    for entry in slaves_dir.flatten() {
        if let Ok(name) = entry.file_name().into_string() {
            return Some(format!("/dev/{}", name));
        }
    }

    None
}

/// Get device name from /proc/mounts for a given path (Unix only)
#[cfg(unix)]
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

        if let Ok(canonical_path) = std::path::Path::new(path).canonicalize() {
            if let Ok(canonical_mount) = std::path::Path::new(mount_point).canonicalize() {
                if canonical_path.starts_with(&canonical_mount) {
                    let mount_len = canonical_mount.as_os_str().len();
                    if best_match.as_ref().is_none_or(|b| mount_len > b.1) {
                        best_match = Some((device.to_string(), mount_len));
                    }
                }
            }
        }
    }

    best_match.and_then(|(device, _)| {
        let resolved = std::path::Path::new(&device)
            .canonicalize()
            .ok()
            .and_then(|p| p.to_str().map(|s| s.to_string()))
            .unwrap_or_else(|| device.clone());

        if resolved.contains("/dm-") {
            resolve_dm_device(&resolved).or(Some(resolved))
        } else {
            Some(resolved)
        }
    })
}
