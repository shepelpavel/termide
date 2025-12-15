//! Unified filesystem and git watcher for termide.
//!
//! Provides filesystem change notifications with git awareness.
//! - Watches files/directories with reference counting
//! - Filters .git/ events to only commit-related changes
//! - Separate debounce: 300ms for files, 1000ms for git

use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebouncedEvent, Debouncer};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

/// Debounce duration for filesystem events.
pub const FS_DEBOUNCE_MS: u64 = 300;
/// Debounce duration for git events.
pub const GIT_DEBOUNCE_MS: u64 = 1000;

/// Watch event types emitted by UnifiedWatcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// File changed (for Editor: external modification check)
    FileChanged(PathBuf),
    /// Directory changed (for FileManager: refresh listing)
    DirectoryChanged {
        /// Root path being watched
        root: PathBuf,
        /// Actual path that changed
        changed: PathBuf,
    },
    /// Git commit occurred (for all: update git status/diff)
    GitCommit(PathBuf),
}

/// Internal event from debouncer callback.
#[derive(Debug, Clone)]
enum InternalEvent {
    /// Regular filesystem change
    FsChange { changed_path: PathBuf },
    /// Git-related change (needs additional debounce)
    GitChange { repo_root: PathBuf },
}

/// Unified watcher for filesystem and git changes.
///
/// Combines functionality of FileSystemWatcher and GitWatcher:
/// - Reference counting for all watches
/// - Filters .git/ to only index/HEAD/refs changes -> GitCommit events
/// - Base debounce 300ms for files, manual 1000ms debounce for git
#[derive(Debug)]
pub struct UnifiedWatcher {
    debouncer: Debouncer<RecommendedWatcher>,
    /// Git repos: repo_root -> reference count (Recursive mode)
    watched_repos: HashMap<PathBuf, usize>,
    /// Non-git dirs: dir_path -> reference count (NonRecursive mode)
    watched_dirs: HashMap<PathBuf, usize>,
    /// Receiver for internal events from debouncer callback
    internal_rx: Receiver<InternalEvent>,
    /// Pending git events waiting for 1000ms debounce
    pending_git: HashMap<PathBuf, Instant>,
    /// Pending fs changes to emit
    pending_fs: HashSet<PathBuf>,
}

impl UnifiedWatcher {
    /// Create a new UnifiedWatcher.
    pub fn new() -> Result<Self> {
        let (internal_tx, internal_rx) = channel();

        let debouncer = new_debouncer(
            Duration::from_millis(FS_DEBOUNCE_MS),
            move |result: notify_debouncer_mini::DebounceEventResult| {
                if let Ok(events) = result {
                    for event in events {
                        Self::process_raw_event(&event, &internal_tx);
                    }
                }
            },
        )
        .context("Failed to create filesystem watcher")?;

        Ok(Self {
            debouncer,
            watched_repos: HashMap::new(),
            watched_dirs: HashMap::new(),
            internal_rx,
            pending_git: HashMap::new(),
            pending_fs: HashSet::new(),
        })
    }

    /// Process raw event from debouncer, classify and send to internal channel.
    fn process_raw_event(event: &DebouncedEvent, tx: &Sender<InternalEvent>) {
        let path = &event.path;

        // Check if this is a .git/ event
        if Self::is_git_path(path) {
            // Filter to only commit-related files
            if Self::is_commit_related(path) {
                if let Some(repo_root) = Self::find_repo_root_from_git_path(path) {
                    let _ = tx.send(InternalEvent::GitChange { repo_root });
                }
            }
            // Ignore other .git/* events
        } else {
            // Regular filesystem change
            let _ = tx.send(InternalEvent::FsChange {
                changed_path: path.clone(),
            });
        }
    }

    /// Check if path is inside .git directory.
    fn is_git_path(path: &Path) -> bool {
        path.components().any(|c| c.as_os_str() == ".git")
    }

    /// Check if git path is commit-related (index, HEAD, refs).
    fn is_commit_related(path: &Path) -> bool {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name == "index" || name == "HEAD" {
                return true;
            }
        }
        // Also check for refs/* and logs/* changes
        let path_str = path.to_string_lossy();
        path_str.contains("/refs/") || path_str.contains("/logs/")
    }

    /// Find repository root from a path inside .git directory.
    fn find_repo_root_from_git_path(path: &Path) -> Option<PathBuf> {
        let mut current = path;
        while let Some(parent) = current.parent() {
            if parent.file_name().and_then(|n| n.to_str()) == Some(".git") {
                return parent.parent().map(|p| p.to_path_buf());
            }
            current = parent;
        }
        None
    }

    /// Start watching a git repository root recursively.
    /// Increments reference count if already watching.
    pub fn watch_repository(&mut self, repo_root: PathBuf) -> Result<()> {
        // Increment reference count if already watching
        if let Some(count) = self.watched_repos.get_mut(&repo_root) {
            *count += 1;
            return Ok(());
        }

        let watcher = self.debouncer.watcher();
        watcher.watch(&repo_root, RecursiveMode::Recursive)?;

        self.watched_repos.insert(repo_root, 1);
        Ok(())
    }

    /// Stop watching a git repository (decrement reference count).
    /// Only unwatches when count reaches 0.
    pub fn unwatch_repository(&mut self, repo_root: &Path) {
        if let Some(count) = self.watched_repos.get_mut(repo_root) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.watched_repos.remove(repo_root);
                let watcher = self.debouncer.watcher();
                let _ = watcher.unwatch(repo_root);
            }
        }
    }

    /// Check if repository root is being watched.
    pub fn is_watching_repo(&self, repo_root: &Path) -> bool {
        self.watched_repos.contains_key(repo_root)
    }

    /// Start watching a non-git directory (non-recursive, direct children only).
    /// Increments reference count if already watching.
    pub fn watch_directory(&mut self, dir_path: PathBuf) -> Result<()> {
        // Increment reference count if already watching
        if let Some(count) = self.watched_dirs.get_mut(&dir_path) {
            *count += 1;
            return Ok(());
        }

        let watcher = self.debouncer.watcher();
        watcher.watch(&dir_path, RecursiveMode::NonRecursive)?;

        self.watched_dirs.insert(dir_path, 1);
        Ok(())
    }

    /// Stop watching a non-git directory (decrement reference count).
    /// Only unwatches when count reaches 0.
    pub fn unwatch_directory(&mut self, dir_path: &Path) {
        if let Some(count) = self.watched_dirs.get_mut(dir_path) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.watched_dirs.remove(dir_path);
                let watcher = self.debouncer.watcher();
                let _ = watcher.unwatch(dir_path);
            }
        }
    }

    /// Check if non-git directory is being watched.
    pub fn is_watching_dir(&self, dir_path: &Path) -> bool {
        self.watched_dirs.contains_key(dir_path)
    }

    /// Poll for pending events.
    /// Call this periodically (e.g., on tick) to get accumulated events.
    pub fn poll_events(&mut self) -> Vec<WatchEvent> {
        // Collect all internal events
        while let Ok(event) = self.internal_rx.try_recv() {
            match event {
                InternalEvent::FsChange { changed_path } => {
                    self.pending_fs.insert(changed_path);
                }
                InternalEvent::GitChange { repo_root } => {
                    // Update timestamp for git debounce
                    self.pending_git.insert(repo_root, Instant::now());
                }
            }
        }

        let mut events = Vec::new();
        let now = Instant::now();

        // Emit git events that have been debounced for 1000ms
        let git_debounce = Duration::from_millis(GIT_DEBOUNCE_MS);
        let ready_git: Vec<PathBuf> = self
            .pending_git
            .iter()
            .filter(|(_, timestamp)| now.duration_since(**timestamp) >= git_debounce)
            .map(|(path, _)| path.clone())
            .collect();

        for repo_root in ready_git {
            self.pending_git.remove(&repo_root);
            events.push(WatchEvent::GitCommit(repo_root));
        }

        // Emit filesystem events (already debounced by notify at 300ms)
        // Collect first to avoid borrow conflict
        let pending_fs: Vec<PathBuf> = self.pending_fs.drain().collect();
        for changed_path in pending_fs {
            // Find the watched root for this path
            let root = self.find_watched_root(&changed_path);
            events.push(WatchEvent::DirectoryChanged {
                root,
                changed: changed_path,
            });
        }

        events
    }

    /// Find the watched root that contains this path.
    fn find_watched_root(&self, path: &Path) -> PathBuf {
        // First check watched repos
        for repo_root in self.watched_repos.keys() {
            if path.starts_with(repo_root) {
                return repo_root.clone();
            }
        }

        // Then check watched directories
        for dir_path in self.watched_dirs.keys() {
            if path.starts_with(dir_path) {
                return dir_path.clone();
            }
        }

        // Fallback to parent directory
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| path.to_path_buf())
    }
}

/// Create a unified watcher instance.
pub fn create_watcher() -> Result<UnifiedWatcher> {
    UnifiedWatcher::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_git_path() {
        assert!(UnifiedWatcher::is_git_path(Path::new(
            "/home/user/project/.git/index"
        )));
        assert!(UnifiedWatcher::is_git_path(Path::new(
            "/home/user/project/.git/refs/heads/main"
        )));
        assert!(!UnifiedWatcher::is_git_path(Path::new(
            "/home/user/project/src/main.rs"
        )));
    }

    #[test]
    fn test_is_commit_related() {
        assert!(UnifiedWatcher::is_commit_related(Path::new(
            "/project/.git/index"
        )));
        assert!(UnifiedWatcher::is_commit_related(Path::new(
            "/project/.git/HEAD"
        )));
        assert!(UnifiedWatcher::is_commit_related(Path::new(
            "/project/.git/refs/heads/main"
        )));
        assert!(UnifiedWatcher::is_commit_related(Path::new(
            "/project/.git/logs/HEAD"
        )));
        assert!(!UnifiedWatcher::is_commit_related(Path::new(
            "/project/.git/objects/ab/cdef"
        )));
        assert!(!UnifiedWatcher::is_commit_related(Path::new(
            "/project/.git/config"
        )));
    }

    #[test]
    fn test_find_repo_root() {
        let path = PathBuf::from("/home/user/project/.git/refs/heads/main");
        let root = UnifiedWatcher::find_repo_root_from_git_path(&path);
        assert_eq!(root, Some(PathBuf::from("/home/user/project")));

        let path = PathBuf::from("/home/user/project/.git/index");
        let root = UnifiedWatcher::find_repo_root_from_git_path(&path);
        assert_eq!(root, Some(PathBuf::from("/home/user/project")));

        let path = PathBuf::from("/home/user/project/src/main.rs");
        let root = UnifiedWatcher::find_repo_root_from_git_path(&path);
        assert_eq!(root, None);
    }
}
