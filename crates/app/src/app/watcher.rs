//! Filesystem and git watcher event handling.
//!
//! Handles unified watcher events for filesystem changes and git operations.

use std::collections::HashSet;

use termide_core::{CommandResult, PanelCommand};
use termide_git::find_repo_root;
use termide_watcher::WatchEvent;

use super::App;

impl App {
    /// Check unified watcher for git and filesystem events
    pub(super) fn check_watcher_events(&mut self) {
        let Some(watcher) = &mut self.state.watcher else {
            return;
        };

        // Lazy registration: register panel directories with watcher
        for panel in self.layout_manager.iter_all_panels_mut() {
            // Use GetFsWatchInfo to check watch state
            if let CommandResult::FsWatchInfo {
                watched_root,
                current_path,
                is_git_repo: _,
            } = panel.handle_command(PanelCommand::GetFsWatchInfo)
            {
                if watched_root.is_none() {
                    // Determine the new watched root
                    let repo_root = find_repo_root(&current_path);
                    let is_git_repo = repo_root.is_some();
                    let new_root = repo_root.unwrap_or_else(|| current_path.clone());

                    // Watch new root (now fast - respects .gitignore)
                    if is_git_repo {
                        if !watcher.is_watching_repo(&new_root) {
                            let _ = watcher.watch_repository(new_root.clone());
                        }
                    } else if !watcher.is_watching_dir(&new_root) {
                        let _ = watcher.watch_directory(new_root.clone());
                    }

                    // Update panel's watched root
                    panel.handle_command(PanelCommand::SetFsWatchRoot {
                        root: Some(new_root),
                        is_git_repo,
                    });
                }
            }

            // Also handle Editor panels via GetRepoRoot
            if let CommandResult::RepoRoot(Some(repo_root)) =
                panel.handle_command(PanelCommand::GetRepoRoot)
            {
                if !watcher.is_watching_repo(&repo_root) {
                    let _ = watcher.watch_repository(repo_root);
                }
            }
        }

        // Poll events from unified watcher
        let events = watcher.poll_events();
        if events.is_empty() {
            return;
        }

        // Separate git and fs events
        let mut git_repos: HashSet<std::path::PathBuf> = HashSet::new();
        let mut fs_paths: HashSet<std::path::PathBuf> = HashSet::new();

        let mut gitignore_changed_repos: Vec<std::path::PathBuf> = Vec::new();

        for event in events {
            match event {
                WatchEvent::GitCommit(repo_root) => {
                    git_repos.insert(repo_root);
                }
                WatchEvent::DirectoryChanged { changed, .. } => {
                    fs_paths.insert(changed);
                }
                WatchEvent::FileChanged(path) => {
                    fs_paths.insert(path);
                }
                WatchEvent::GitignoreChanged(repo_root) => {
                    gitignore_changed_repos.push(repo_root);
                }
            }
        }

        // Handle .gitignore changes - reinitialize watcher
        for repo_root in gitignore_changed_repos {
            watcher.unwatch_repository(&repo_root);
            let _ = watcher.watch_repository(repo_root);
        }

        // Process git events
        if !git_repos.is_empty() {
            let repo_paths: Vec<&std::path::Path> = git_repos.iter().map(|p| p.as_path()).collect();

            for panel in self.layout_manager.iter_all_panels_mut() {
                if panel
                    .handle_command(PanelCommand::OnGitUpdate {
                        repo_paths: &repo_paths,
                    })
                    .needs_redraw()
                {
                    self.state.needs_redraw = true;
                }
            }
        }

        // Process filesystem events
        for panel in self.layout_manager.iter_all_panels_mut() {
            for path in &fs_paths {
                if panel
                    .handle_command(PanelCommand::OnFsUpdate { changed_path: path })
                    .needs_redraw()
                {
                    self.state.needs_redraw = true;
                    break;
                }
            }
        }
    }
}
