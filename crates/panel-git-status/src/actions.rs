//! Git operations and button actions for Git Status Panel.

use std::path::{Path, PathBuf};

use termide_core::{GitOperationType, PanelEvent};
use termide_git as git;
use termide_modal::{ActionButton, ActiveModal, InfoActionModal};
use termide_state::PendingAction;
use termide_system_monitor::format_bytes;

use crate::types::{Button, Selection};
use crate::GitStatusPanel;

impl GitStatusPanel {
    /// Execute a git file operation with common error handling
    pub(crate) fn execute_git_op<F>(&mut self, files: Vec<PathBuf>, op: F, action: &str)
    where
        F: FnOnce(&Path, &[PathBuf]) -> Result<(), String>,
    {
        if files.is_empty() {
            return;
        }
        if let Some(repo) = self.repo_manager.current() {
            match op(repo, &files) {
                Ok(()) => {
                    let t = termide_i18n::t();
                    self.status_message = Some(t.git_action_files_fmt(action, files.len()));
                    self.refresh();
                }
                Err(e) => {
                    let t = termide_i18n::t();
                    self.status_message = Some(t.git_action_error_fmt(action, &e.to_string()));
                }
            }
        }
    }

    /// Execute stage action
    pub(crate) fn do_stage(&mut self) {
        let files = self.get_selected_unstaged();
        let t = termide_i18n::t();
        self.execute_git_op(files, git::stage_files, t.git_staged_header());
    }

    /// Execute unstage action
    pub(crate) fn do_unstage(&mut self) {
        let files = self.get_selected_staged();
        let t = termide_i18n::t();
        self.execute_git_op(files, git::unstage_files, t.git_unstaged_header());
    }

    /// Stage all unstaged files
    pub(crate) fn do_stage_all(&mut self) {
        let files: Vec<PathBuf> = self.unstaged_files.iter().map(|f| f.path.clone()).collect();
        let t = termide_i18n::t();
        self.execute_git_op(files, git::stage_files, t.git_staged_header());
    }

    /// Unstage all staged files
    pub(crate) fn do_unstage_all(&mut self) {
        let files: Vec<PathBuf> = self.staged_files.iter().map(|f| f.path.clone()).collect();
        let t = termide_i18n::t();
        self.execute_git_op(files, git::unstage_files, t.git_unstaged_header());
    }

    /// Show file properties modal with Edit/Diff/Revert actions
    pub(crate) fn show_file_properties(&mut self) -> Vec<PanelEvent> {
        let t = termide_i18n::t();

        let (is_staged, idx) = match self.get_selection() {
            Some(Selection::UnstagedFile(idx)) => (false, idx),
            Some(Selection::StagedFile(idx)) => (true, idx),
            _ => return vec![], // Headers, directories, or nothing selected
        };

        let (file_path, status_str) = if is_staged {
            if let Some(file) = self.staged_files.get(idx) {
                let path = PathBuf::from(&file.path);
                let status = match file.status {
                    'A' => t.git_status_added(),
                    'M' => t.git_status_modified(),
                    'D' => t.git_status_deleted(),
                    'R' => t.git_status_renamed(),
                    c => c.to_string(),
                };
                (path, status)
            } else {
                return vec![];
            }
        } else if let Some(file) = self.unstaged_files.get(idx) {
            let path = PathBuf::from(&file.path);
            let status = if file.untracked {
                t.git_status_untracked()
            } else {
                match file.status {
                    'M' => t.git_status_modified(),
                    'D' => t.git_status_deleted(),
                    c => c.to_string(),
                }
            };
            (path, status)
        } else {
            return vec![];
        };

        let Some(repo_path) = self.repo_manager.current().map(|p| p.to_path_buf()) else {
            return vec![];
        };

        // Get full path for file stats
        let full_path = repo_path.join(&file_path);

        // Get file metadata (size + line count combined)
        let size_info = if full_path.exists() {
            let size = std::fs::metadata(&full_path).map(|m| m.len()).unwrap_or(0);
            let lines = std::fs::read_to_string(&full_path)
                .map(|s| s.lines().count())
                .unwrap_or(0);
            format!("{} ({} LOC)", format_bytes(size), lines)
        } else {
            t.git_props_deleted().to_string()
        };

        // Get diff stats
        let diff_stats = git::get_file_diff_stats(&repo_path, &file_path, is_staged);
        let diff_info = format!("+{} -{}", diff_stats.additions, diff_stats.deletions);

        // Build data for modal
        let data = vec![
            (
                t.git_props_path().to_string(),
                file_path.display().to_string(),
            ),
            (t.git_props_size().to_string(), size_info),
            (t.git_props_status().to_string(), status_str),
            (t.git_props_diff().to_string(), diff_info),
        ];

        // Build action buttons (Revert shown for all files - staged files will be unstaged first)
        let buttons = vec![
            ActionButton::new(t.git_action_edit(), "edit"),
            ActionButton::new(t.git_action_revert(), "revert"),
            ActionButton::new(t.git_action_close(), "close"),
        ];

        // Select Close button by default
        let selected_button = buttons.len().saturating_sub(1);

        let modal_title = t.git_file_properties_title().to_string();
        let modal =
            InfoActionModal::new(modal_title, data, buttons).with_selected_button(selected_button);

        // Store modal request
        self.modal_request = Some((
            PendingAction::GitFileAction {
                file_path,
                repo_path,
                is_staged,
            },
            ActiveModal::InfoAction(Box::new(modal)),
        ));

        vec![]
    }

    /// Switch to a different branch
    pub(crate) fn switch_to_branch(&mut self, branch_idx: usize) {
        if let Some(branch_name) = self.branches.get(branch_idx) {
            if let Some(repo) = self.repo_manager.current() {
                let branch_name = branch_name.clone();
                match git::checkout_branch(repo, &branch_name) {
                    Ok(()) => {
                        let t = termide_i18n::t();
                        self.status_message = Some(t.git_switched_to_fmt(&branch_name));
                        self.refresh();
                    }
                    Err(e) => {
                        let t = termide_i18n::t();
                        self.status_message = Some(t.git_checkout_error_fmt(&e.to_string()));
                    }
                }
            }
        }
    }

    /// Get list of buttons that should be visible based on current state
    pub(crate) fn get_visible_buttons(&self) -> Vec<Button> {
        let mut buttons = Vec::new();

        // If no repos found, show Init button only
        if self.repo_manager.is_empty() {
            if !self.initial_paths.is_empty() {
                buttons.push(Button::Init);
            }
            return buttons;
        }

        // Diff - show if there are any changes (unstaged or staged)
        if !self.unstaged_files.is_empty() || !self.staged_files.is_empty() {
            buttons.push(Button::Diff);
        }

        // Commit - only if there are staged files
        if !self.staged_files.is_empty() {
            buttons.push(Button::Commit);
        }

        // Show spinner button if operation in progress
        if self.git_operation_in_progress {
            match self.current_operation.as_deref() {
                Some("push") => buttons.push(Button::Pushing),
                Some("pull") => buttons.push(Button::Pulling),
                _ => {} // Unknown operation, don't show button
            }
        } else {
            // Push - only if ahead > 0
            if self.ahead > 0 {
                buttons.push(Button::Push);
            }

            // Pull - only if behind > 0
            if self.behind > 0 {
                buttons.push(Button::Pull);
            }
        }

        buttons
    }

    /// Execute button action
    pub(crate) fn execute_button(&mut self) -> Vec<PanelEvent> {
        let buttons = self.get_visible_buttons();
        if self.selected_button >= buttons.len() {
            return vec![];
        }
        let button = buttons[self.selected_button];
        match button {
            Button::Diff => {
                if let Some(repo) = self.repo_manager.current() {
                    vec![PanelEvent::OpenGitDiff {
                        repo_path: repo.to_path_buf(),
                        commit_hash: None,
                    }]
                } else {
                    vec![]
                }
            }
            Button::Commit => {
                if let Some(repo) = self.repo_manager.current() {
                    let staged_count = self.staged_files.len();
                    let repo_name = git::get_repo_name(repo);
                    let branch_name = self
                        .branch
                        .clone()
                        .unwrap_or_else(|| termide_i18n::t().git_branch_detached().to_string());
                    let modal =
                        termide_modal::CommitModal::new(staged_count, repo_name, branch_name);
                    self.modal_request = Some((
                        termide_state::PendingAction::GitCommit {
                            repo_path: repo.to_path_buf(),
                        },
                        termide_modal::ActiveModal::Commit(Box::new(modal)),
                    ));
                }
                vec![]
            }
            Button::Pull => {
                if let Some(repo) = self.repo_manager.current() {
                    vec![PanelEvent::GitOperation {
                        operation: GitOperationType::Pull,
                        repo_path: repo.to_path_buf(),
                    }]
                } else {
                    vec![]
                }
            }
            Button::Push => {
                if let Some(repo) = self.repo_manager.current() {
                    vec![PanelEvent::GitOperation {
                        operation: GitOperationType::Push,
                        repo_path: repo.to_path_buf(),
                    }]
                } else {
                    vec![]
                }
            }
            Button::Pushing | Button::Pulling => {
                // Click on spinner button cancels the operation
                vec![PanelEvent::CancelGitOperation]
            }
            Button::Init => {
                // Initialize a new git repository in the first initial path
                if let Some(path) = self.initial_paths.first().cloned() {
                    match git::init_repo(&path) {
                        Ok(()) => {
                            // Refresh to detect the new repo
                            self.repo_manager = git::RepoManager::new(&self.initial_paths);
                            self.refresh();
                            let t = termide_i18n::t();
                            self.status_message =
                                Some(t.git_init_success(&path.display().to_string()));
                        }
                        Err(e) => {
                            let t = termide_i18n::t();
                            self.status_message = Some(t.git_init_failed_fmt(&e.to_string()));
                        }
                    }
                }
                vec![]
            }
        }
    }
}
