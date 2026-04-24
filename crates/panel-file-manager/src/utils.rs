use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
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

/// Outcome of trying to claim a size walk through the shared cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// Result is already in the cache — no work needed.
    AlreadyCached,
    /// Another panel is currently walking this path — wait for it.
    InProgress,
    /// Caller has exclusive ownership and must compute.
    Claimed,
}

/// Process-wide shared cache for directory sizes shown in FM wide view.
///
/// Multiple FM panels open on overlapping trees share results through
/// this cache. The `inflight` set ensures that only one panel walks any
/// given path at a time; other panels observing the path see `InProgress`
/// and wait for the completion to land in `entries`. A monotonic
/// `generation` counter ticks on every mutation so panels can cheaply
/// detect "something changed" and trigger a redraw.
///
/// Invalidation is per-subtree: when an FM panel reloads its current
/// directory, only entries and claims rooted at that directory are
/// dropped. Results for other panels' directories stay intact.
#[derive(Default)]
pub struct DirSizeCache {
    entries: Mutex<HashMap<PathBuf, DirSizeOutcome>>,
    inflight: Mutex<HashSet<PathBuf>>,
    generation: AtomicU64,
}

impl DirSizeCache {
    /// Monotonic counter — increments on any mutation (insert/invalidate).
    /// Panels compare the last-seen value to decide when to redraw.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn get(&self, path: &Path) -> Option<DirSizeOutcome> {
        self.entries.lock().ok()?.get(path).copied()
    }

    /// Try to acquire exclusive ownership of a walk for `path`.
    pub fn claim(&self, path: &Path) -> ClaimOutcome {
        if let Ok(entries) = self.entries.lock() {
            if entries.contains_key(path) {
                return ClaimOutcome::AlreadyCached;
            }
        }
        let Ok(mut inflight) = self.inflight.lock() else {
            return ClaimOutcome::InProgress;
        };
        if inflight.contains(path) {
            ClaimOutcome::InProgress
        } else {
            inflight.insert(path.to_path_buf());
            ClaimOutcome::Claimed
        }
    }

    /// Deposit a completed result. Silently dropped if the claim was
    /// revoked by `invalidate_subtree` while the worker was running,
    /// preventing a stale number from replacing fresh tree state.
    pub fn complete(&self, path: PathBuf, outcome: DirSizeOutcome) {
        let Ok(mut inflight) = self.inflight.lock() else {
            return;
        };
        if !inflight.remove(&path) {
            return;
        }
        drop(inflight);
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(path, outcome);
            self.generation.fetch_add(1, Ordering::Release);
        }
    }

    /// Drop entries and claims rooted at `root`. Any worker currently
    /// walking an invalidated path will have its result discarded on
    /// `complete` because its claim is gone.
    pub fn invalidate_subtree(&self, root: &Path) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|p, _| !p.starts_with(root));
        }
        if let Ok(mut inflight) = self.inflight.lock() {
            inflight.retain(|p| !p.starts_with(root));
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// Drop entries and claims whose path is an **ancestor** of `changed`
    /// (including `changed` itself). Intended for FS-watcher events:
    /// a file under `/a/b/c` mutating invalidates the cached sizes of
    /// `/a/b/c`, `/a/b`, `/a` — their totals are now stale.
    pub fn invalidate_ancestors(&self, changed: &Path) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|p, _| !changed.starts_with(p));
        }
        if let Ok(mut inflight) = self.inflight.lock() {
            inflight.retain(|p| !changed.starts_with(p));
        }
        self.generation.fetch_add(1, Ordering::Release);
    }
}

/// Accessor for the process-wide shared directory-size cache.
pub fn shared_dir_size_cache() -> &'static DirSizeCache {
    static CACHE: OnceLock<DirSizeCache> = OnceLock::new();
    CACHE.get_or_init(DirSizeCache::default)
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
    fn shared_cache_claim_deduplicates_walks() {
        // Using a unique path so the global cache doesn't collide with
        // other tests in this process.
        let path = PathBuf::from("/__dir_size_cache_test__/dedup_A");

        let cache = DirSizeCache::default();
        assert_eq!(cache.claim(&path), ClaimOutcome::Claimed);
        // Second claim while first is in-flight must not race.
        assert_eq!(cache.claim(&path), ClaimOutcome::InProgress);

        cache.complete(
            path.clone(),
            DirSizeOutcome {
                size: 42,
                overflowed: false,
            },
        );
        // Now the value is cached and further claims short-circuit.
        assert_eq!(cache.claim(&path), ClaimOutcome::AlreadyCached);
        assert_eq!(cache.get(&path).map(|o| o.size), Some(42));
    }

    #[test]
    fn shared_cache_invalidate_discards_stale_completion() {
        let root = PathBuf::from("/__dir_size_cache_test__/invalidate");
        let child = root.join("child");

        let cache = DirSizeCache::default();
        assert_eq!(cache.claim(&child), ClaimOutcome::Claimed);

        // Subtree invalidation before the worker reports in — its result
        // must be silently dropped so we don't poison a fresh tree state.
        cache.invalidate_subtree(&root);
        cache.complete(
            child.clone(),
            DirSizeOutcome {
                size: 999,
                overflowed: false,
            },
        );
        assert!(cache.get(&child).is_none());
    }

    #[test]
    fn shared_cache_invalidate_ancestors_targets_parents_only() {
        let parent = PathBuf::from("/__dir_size_cache_test__/ancestors");
        let child_file = parent.join("sub/file.txt");
        let sibling = PathBuf::from("/__dir_size_cache_test__/other");

        let cache = DirSizeCache::default();
        let outcome = DirSizeOutcome {
            size: 1,
            overflowed: false,
        };
        // Seed three entries: ancestor of the changed file, the
        // changed file itself (unlikely to be in cache but valid), and
        // an unrelated sibling that must survive.
        cache.claim(&parent);
        cache.complete(parent.clone(), outcome);
        cache.claim(&sibling);
        cache.complete(sibling.clone(), outcome);

        cache.invalidate_ancestors(&child_file);

        assert!(
            cache.get(&parent).is_none(),
            "ancestor of changed path must be invalidated"
        );
        assert!(
            cache.get(&sibling).is_some(),
            "unrelated entry must survive ancestor invalidation"
        );
    }

    #[test]
    fn shared_cache_generation_ticks_on_insert_and_invalidate() {
        let path = PathBuf::from("/__dir_size_cache_test__/generation");
        let cache = DirSizeCache::default();
        let g0 = cache.generation();

        assert_eq!(cache.claim(&path), ClaimOutcome::Claimed);
        // claim alone does not bump generation (no observable change).
        assert_eq!(cache.generation(), g0);

        cache.complete(
            path.clone(),
            DirSizeOutcome {
                size: 1,
                overflowed: false,
            },
        );
        let g1 = cache.generation();
        assert!(g1 > g0, "complete must bump generation");

        cache.invalidate_subtree(&path);
        assert!(cache.generation() > g1, "invalidate must bump generation");
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
