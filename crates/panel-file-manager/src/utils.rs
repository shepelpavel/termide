use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use termide_git::truncate_right;
use termide_ui::constants::{GIGABYTE, KILOBYTE, MEGABYTE};

use super::FileEntry;

/// Get attribute character (selection checkmark or directory flags)
/// Returns 1 character
pub fn get_attribute(entry: &FileEntry, is_selected: bool) -> &'static str {
    if is_selected {
        return "+";
    }

    // For directories: show R if read-only
    if entry.is_dir && entry.is_readonly {
        return "R";
    }

    " "
}

/// Truncate file name to specified display width
pub fn truncate_name(name: &str, max_len: usize) -> String {
    truncate_right(name, max_len)
}

/// Format file size in human-readable format (compact, whole numbers only).
/// Used in file panel columns where space is limited.
pub fn format_size_compact(bytes: u64) -> String {
    let t = termide_i18n::t();
    if bytes >= GIGABYTE {
        format!(
            "{:.0} {}",
            bytes as f64 / GIGABYTE as f64,
            t.size_gigabytes()
        )
    } else if bytes >= MEGABYTE {
        format!(
            "{:.0} {}",
            bytes as f64 / MEGABYTE as f64,
            t.size_megabytes()
        )
    } else if bytes >= KILOBYTE {
        format!(
            "{:.0} {}",
            bytes as f64 / KILOBYTE as f64,
            t.size_kilobytes()
        )
    } else {
        format!("{} {}", bytes, t.size_bytes())
    }
}

/// Format file size in human-readable format (detailed).
/// B, KB — whole numbers; MB — one decimal; GB+ — two decimals.
/// Used in file info modal where precision matters.
pub fn format_size(bytes: u64) -> String {
    let t = termide_i18n::t();
    if bytes >= GIGABYTE {
        format!(
            "{:.2} {}",
            bytes as f64 / GIGABYTE as f64,
            t.size_gigabytes()
        )
    } else if bytes >= MEGABYTE {
        format!(
            "{:.1} {}",
            bytes as f64 / MEGABYTE as f64,
            t.size_megabytes()
        )
    } else if bytes >= KILOBYTE {
        format!(
            "{:.0} {}",
            bytes as f64 / KILOBYTE as f64,
            t.size_kilobytes()
        )
    } else {
        format!("{} {}", bytes, t.size_bytes())
    }
}

/// Result of a time-bounded directory size walk.
///
/// `overflowed == true` means the walk was cut short because `budget`
/// elapsed before the tree was fully traversed; in that case `size`
/// holds the partial total accumulated so far (never a final number).
#[derive(Debug, Clone, Copy)]
pub struct DirSizeOutcome {
    pub size: u64,
    pub overflowed: bool,
}

/// Iteratively walk `path` and sum file sizes, stopping at `budget`.
///
/// Mirrors [`calculate_dir_size`] (same symlink policy — `entry.metadata()`
/// follows symlinks, breadth-first traversal with a queue) but returns
/// an overflow flag so callers can render a marker instead of a stale
/// partial number when the walk didn't finish in time.
pub fn calculate_dir_size_bounded(path: &Path, budget: Duration) -> DirSizeOutcome {
    use std::collections::VecDeque;

    let start = Instant::now();
    let mut total: u64 = 0;
    let mut queue: VecDeque<std::path::PathBuf> = VecDeque::new();
    queue.push_back(path.to_path_buf());

    while let Some(dir) = queue.pop_front() {
        if start.elapsed() >= budget {
            return DirSizeOutcome {
                size: total,
                overflowed: true,
            };
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if start.elapsed() >= budget {
                return DirSizeOutcome {
                    size: total,
                    overflowed: true,
                };
            }
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total = total.saturating_add(meta.len());
                } else if meta.is_dir() {
                    queue.push_back(entry.path());
                }
            }
        }
    }

    DirSizeOutcome {
        size: total,
        overflowed: false,
    }
}

/// Iteratively calculate directory size (without recursion, protected from stack overflow)
pub fn calculate_dir_size(path: &Path) -> u64 {
    use std::collections::VecDeque;

    let mut total_size = 0u64;
    let mut dirs_to_process = VecDeque::new();
    dirs_to_process.push_back(path.to_path_buf());

    // Iterative traversal with explicit stack
    while let Some(current_dir) = dirs_to_process.pop_front() {
        if let Ok(entries) = fs::read_dir(&current_dir) {
            for entry in entries.flatten() {
                // Use symlink_metadata to not follow symlinks
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() {
                        total_size += metadata.len();
                    } else if metadata.is_dir() {
                        // Add directory to queue for processing
                        dirs_to_process.push_back(entry.path());
                    }
                    // Ignore symlinks to avoid cycles
                }
            }
        }
    }

    total_size
}

/// Get user name by UID
/// Returns symbolic name if available, otherwise numeric ID
#[cfg(unix)]
pub fn get_user_name(uid: u32) -> String {
    // SAFETY: getpwuid is a POSIX function that returns a pointer to a static
    // passwd struct or NULL. We check for NULL before dereferencing. The returned
    // pointer is valid until the next call to getpwuid/getpwnam, but we immediately
    // copy the string data so this is safe. The pw_name field is a null-terminated
    // C string that we convert safely via CStr::from_ptr after NULL check.
    unsafe {
        let pwd = libc::getpwuid(uid);
        if !pwd.is_null() {
            let name_ptr = (*pwd).pw_name;
            if !name_ptr.is_null() {
                if let Ok(name) = std::ffi::CStr::from_ptr(name_ptr).to_str() {
                    return name.to_string();
                }
            }
        }
    }
    uid.to_string()
}

/// Get group name by GID
/// Returns symbolic name if available, otherwise numeric ID
#[cfg(unix)]
pub fn get_group_name(gid: u32) -> String {
    // SAFETY: getgrgid is a POSIX function that returns a pointer to a static
    // group struct or NULL. We check for NULL before dereferencing. The returned
    // pointer is valid until the next call to getgrgid/getgrnam, but we immediately
    // copy the string data so this is safe. The gr_name field is a null-terminated
    // C string that we convert safely via CStr::from_ptr after NULL check.
    unsafe {
        let grp = libc::getgrgid(gid);
        if !grp.is_null() {
            let name_ptr = (*grp).gr_name;
            if !name_ptr.is_null() {
                if let Ok(name) = std::ffi::CStr::from_ptr(name_ptr).to_str() {
                    return name.to_string();
                }
            }
        }
    }
    gid.to_string()
}

/// Format modification time in YYYY-MM-DD HH:MM:SS format
/// Returns 19 characters (time string or spaces)
pub fn format_modified_time(time: Option<SystemTime>) -> String {
    time.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .and_then(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0))
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "                   ".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(path: &Path, bytes: usize) {
        let mut f = fs::File::create(path).expect("create file");
        f.write_all(&vec![0u8; bytes]).expect("write file");
    }

    #[test]
    fn bounded_completes_small_tree() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Flat tree with predictable total: 3 * 100 = 300 bytes.
        for i in 0..3 {
            write_file(&tmp.path().join(format!("f{i}.bin")), 100);
        }
        let sub = tmp.path().join("nested");
        fs::create_dir(&sub).unwrap();
        write_file(&sub.join("x.bin"), 50);

        let outcome = calculate_dir_size_bounded(tmp.path(), Duration::from_secs(60));
        assert!(!outcome.overflowed, "small tree must finish well under 60s");
        assert_eq!(outcome.size, 350);
    }

    #[test]
    fn bounded_stops_when_budget_exhausted() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Enough entries that a zero-duration budget trips the deadline
        // on the very first loop iteration. We don't care exactly how much
        // was accumulated, only that overflowed is reported.
        for i in 0..32 {
            write_file(&tmp.path().join(format!("f{i}.bin")), 1024);
        }

        let outcome = calculate_dir_size_bounded(tmp.path(), Duration::from_nanos(0));
        assert!(outcome.overflowed, "zero-budget walk must overflow");
        // Partial total must never exceed reality.
        assert!(outcome.size <= 32 * 1024);
    }
}
