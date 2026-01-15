//! Repository selection manager for git panels.
//!
//! Provides common logic for managing repository selection across git panels.

use std::path::{Path, PathBuf};

use crate::find_repos_from_paths;

/// Manages repository selection for git panels.
///
/// Consolidates common logic for tracking and switching between repositories.
pub struct RepoManager {
    repos: Vec<PathBuf>,
    selected: usize,
}

impl RepoManager {
    /// Create a new repo manager from a list of paths.
    ///
    /// Automatically discovers git repositories in the given paths.
    pub fn new(paths: &[PathBuf]) -> Self {
        Self {
            repos: find_repos_from_paths(paths, 2),
            selected: 0,
        }
    }

    /// Create a repo manager for a specific repository (including its submodules).
    pub fn for_repo(repo_path: PathBuf) -> Self {
        Self {
            repos: find_repos_from_paths(&[repo_path], 2),
            selected: 0,
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
    /// Attempts to preserve the current selection if it still exists.
    /// Returns true if the repository list changed.
    pub fn update(&mut self, paths: &[PathBuf]) -> bool {
        let current = self.current().map(|p| p.to_path_buf());
        let new_repos = find_repos_from_paths(paths, 2);

        if new_repos != self.repos {
            self.repos = new_repos;
            if let Some(current) = current {
                self.selected = self.repos.iter().position(|r| r == &current).unwrap_or(0);
            } else {
                self.selected = 0;
            }
            true
        } else {
            false
        }
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
