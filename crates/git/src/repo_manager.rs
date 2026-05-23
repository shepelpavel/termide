//! Repository selection manager for git panels.
//!
//! Provides common logic for managing repository selection across git panels.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use crate::{find_all_repos, find_toplevel_repo, find_toplevel_repos};

/// Default depth git panels recurse into a repo root to discover submodules.
/// `0` would skip the submodule walk entirely.
const SUBMODULE_DEPTH: usize = 2;

/// Manages repository selection for git panels.
///
/// The constructor finds top-level repo roots synchronously (cheap: just
/// walks up from each path until it sees a `.git` directory) and kicks
/// off the recursive submodule walk on a background thread. The panel
/// is usable immediately with the root repos; submodules join the list
/// the first time [`Self::poll`] is called after the walk completes,
/// without blocking the UI.
pub struct RepoManager {
    repos: Vec<PathBuf>,
    selected: usize,
    /// Background submodule walk in flight — `None` once it has landed
    /// (or if it was never started, e.g. on an empty repo list).
    submodule_rx: Option<mpsc::Receiver<Vec<PathBuf>>>,
}

impl RepoManager {
    /// Create a new repo manager from a list of paths.
    ///
    /// Discovers top-level repo roots immediately and spawns the
    /// submodule walk in the background. Use [`Self::poll`] from the
    /// panel's update tick to fold the submodule results in when ready.
    pub fn new(paths: &[PathBuf]) -> Self {
        let roots = find_toplevel_repos(paths);
        let submodule_rx = spawn_submodule_walk(&roots);
        Self {
            repos: roots,
            selected: 0,
            submodule_rx,
        }
    }

    /// Create a repo manager for a specific repository.
    ///
    /// Walks up from `repo_path` to find the top-level repo root the
    /// same way [`Self::new`] does, then spawns the submodule walk in
    /// the background. Returns an empty manager if `repo_path` does
    /// not live under a git repository.
    pub fn for_repo(repo_path: PathBuf) -> Self {
        let roots = match find_toplevel_repo(&repo_path) {
            Some(root) => vec![root],
            None => Vec::new(),
        };
        let submodule_rx = spawn_submodule_walk(&roots);
        Self {
            repos: roots,
            selected: 0,
            submodule_rx,
        }
    }

    /// Drain the background submodule walk if it has finished.
    ///
    /// Returns `true` once the list changed so the caller can trigger a
    /// redraw. Subsequent calls are no-ops until a new walk is spawned
    /// by [`Self::update`].
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.submodule_rx else {
            return false;
        };
        match rx.try_recv() {
            Ok(full) => {
                let current = self.current().map(|p| p.to_path_buf());
                self.repos = full;
                if let Some(c) = current {
                    self.selected = self.repos.iter().position(|r| r == &c).unwrap_or(0);
                }
                self.submodule_rx = None;
                true
            }
            // Walk hasn't completed yet — keep the receiver around. A
            // disconnected channel (worker thread panicked) is treated
            // the same as "nothing yet"; the next `update()` will reset.
            Err(_) => false,
        }
    }

    /// Get the currently selected repository path.
    pub fn current(&self) -> Option<&Path> {
        self.repos.get(self.selected).map(|p| p.as_path())
    }

    /// Get all discovered repositories.
    pub fn repos(&self) -> &[PathBuf] {
        &self.repos
    }

    /// Get the index of the selected repository.
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Select a repository by index.
    pub fn select(&mut self, index: usize) {
        if index < self.repos.len() {
            self.selected = index;
        }
    }

    /// Select the next repository (wrapping to first).
    pub fn select_next(&mut self) {
        if !self.repos.is_empty() {
            self.selected = (self.selected + 1) % self.repos.len();
        }
    }

    /// Select the previous repository (wrapping to last).
    pub fn select_prev(&mut self) {
        if !self.repos.is_empty() {
            self.selected = self.selected.checked_sub(1).unwrap_or(self.repos.len() - 1);
        }
    }

    /// Update repositories from new paths.
    ///
    /// Replaces the top-level set immediately and re-spawns the
    /// submodule walk in the background. Preserves the current
    /// selection if it still exists.
    /// Returns true if the immediate top-level list changed — note that
    /// submodules joining later via [`Self::poll`] also return true
    /// from that call.
    pub fn update(&mut self, paths: &[PathBuf]) -> bool {
        let current = self.current().map(|p| p.to_path_buf());
        let new_roots = find_toplevel_repos(paths);

        let changed = new_roots != self.repos;
        if changed {
            self.repos = new_roots.clone();
            if let Some(current) = current {
                self.selected = self.repos.iter().position(|r| r == &current).unwrap_or(0);
            } else {
                self.selected = 0;
            }
        }
        // Always restart the submodule walk so a later poll() picks up
        // any new/removed submodules even when the top-level set was
        // unchanged.
        self.submodule_rx = spawn_submodule_walk(&new_roots);
        changed
    }

    /// Check if there are multiple repositories.
    pub fn has_multiple(&self) -> bool {
        self.repos.len() > 1
    }

    /// Check if there are no repositories.
    pub fn is_empty(&self) -> bool {
        self.repos.is_empty()
    }

    /// Get the number of repositories.
    pub fn len(&self) -> usize {
        self.repos.len()
    }
}

/// Spawn a background submodule walk for `roots` and return its receiver.
/// Returns `None` when there is nothing to walk so callers can skip the
/// `poll()` round-trip entirely on empty repo sets.
fn spawn_submodule_walk(roots: &[PathBuf]) -> Option<mpsc::Receiver<Vec<PathBuf>>> {
    if roots.is_empty() {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let roots: Vec<PathBuf> = roots.to_vec();
    std::thread::spawn(move || {
        use std::collections::HashSet;
        let mut all: HashSet<PathBuf> = roots.iter().cloned().collect();
        for root in &roots {
            for submodule in find_all_repos(root, SUBMODULE_DEPTH) {
                all.insert(submodule);
            }
        }
        let mut full: Vec<PathBuf> = all.into_iter().collect();
        full.sort();
        let _ = tx.send(full);
    });
    Some(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let manager = RepoManager::new(&[]);
        assert!(manager.is_empty());
        assert!(manager.current().is_none());
        assert!(!manager.has_multiple());
    }

    #[test]
    fn test_for_repo() {
        // for_repo now searches for submodules, so with a non-existent path it returns empty
        let manager = RepoManager::for_repo(PathBuf::from("/test/repo"));
        assert!(manager.is_empty()); // No actual git repo at this path
    }

    #[test]
    fn test_select_bounds() {
        let mut manager = RepoManager::new(&[]);
        manager.select(10); // Out of bounds on empty
        assert_eq!(manager.selected_index(), 0); // Unchanged
    }
}
