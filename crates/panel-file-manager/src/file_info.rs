use std::fs;
use std::sync::mpsc;

use super::{utils, FileManager};
use termide_modal::{ActionButton, ActiveModal};
use termide_state::{DirSizeResult, PendingAction};
use termide_ui::system_monitor::DiskSpaceInfo;

/// File information for display
#[derive(Clone, Debug)]
pub struct FileInfo {
    pub name: String,
    pub file_type: String,
    pub size: String,
    pub owner: String,
    pub group: String,
    #[allow(dead_code)]
    pub modified: String,
    pub mode: String, // Access permissions in format "0755"
}

impl FileManager {
    /// Get information about the currently selected file
    pub fn get_current_file_info(&self) -> Option<FileInfo> {
        use std::os::unix::fs::MetadataExt;
        use std::time::SystemTime;

        let entry = self.entries.get(self.selected)?;

        // Handle ".." directory for remote paths
        if entry.name == ".." && self.is_remote() {
            return Some(FileInfo {
                name: "..".to_string(),
                file_type: "Directory".to_string(),
                size: "DIR".to_string(),
                owner: "remote".to_string(),
                group: "remote".to_string(),
                modified: "Unknown".to_string(),
                mode: "????".to_string(),
            });
        }

        // For remote files, use FileEntry metadata directly (from VfsEntry)
        if self.is_remote() {
            let file_type = if entry.is_dir {
                "Directory"
            } else if entry.is_symlink {
                "Symlink"
            } else {
                "File"
            };

            let size = if entry.is_dir {
                "DIR".to_string()
            } else {
                entry
                    .size
                    .map(utils::format_size)
                    .unwrap_or_else(|| "Unknown".to_string())
            };

            let modified = entry
                .modified
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| {
                    chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                        .map(|dt| {
                            dt.with_timezone(&chrono::Local)
                                .format("%Y-%m-%d %H:%M:%S")
                                .to_string()
                        })
                        .unwrap_or_else(|| "Unknown".to_string())
                })
                .unwrap_or_else(|| "Unknown".to_string());

            let mode = if entry.is_executable {
                "0755".to_string()
            } else if entry.is_readonly {
                "0444".to_string()
            } else {
                "0644".to_string()
            };

            return Some(FileInfo {
                name: entry.name.clone(),
                file_type: file_type.to_string(),
                size,
                owner: "remote".to_string(),
                group: "remote".to_string(),
                modified,
                mode,
            });
        }

        // Local file handling
        let file_path = if entry.name == ".." {
            self.current_path
                .parent()
                .unwrap_or(&self.current_path)
                .to_path_buf()
        } else {
            self.current_path.join(&entry.name)
        };

        let metadata = fs::metadata(&file_path).ok()?;

        let file_type = if metadata.is_dir() {
            "Directory"
        } else if metadata.is_symlink() {
            "Symlink"
        } else {
            "File"
        };

        let size = if metadata.is_dir() {
            "DIR".to_string()
        } else {
            utils::format_size(metadata.len())
        };

        let owner = utils::get_user_name(metadata.uid());
        let group = utils::get_group_name(metadata.gid());

        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| {
                chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                    .map(|dt| {
                        dt.with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M:%S")
                            .to_string()
                    })
                    .unwrap_or_else(|| "Unknown".to_string())
            })
            .unwrap_or_else(|| "Unknown".to_string());

        // Format access permissions in octal format (e.g. "0755")
        let mode = format!("{:04o}", metadata.mode() & 0o7777);

        Some(FileInfo {
            name: entry.name.clone(),
            file_type: file_type.to_string(),
            size,
            owner,
            group,
            modified,
            mode,
        })
    }

    /// Show file/directory information (Space)
    pub(crate) fn show_file_info(&mut self) {
        use std::os::unix::fs::MetadataExt;
        use std::time::SystemTime;

        if let Some(entry) = self.entries.get(self.selected) {
            // Handle remote file info display
            if self.is_remote() {
                let t = termide_i18n::t();

                // Determine type and title
                let (modal_title, is_dir) = if entry.is_dir {
                    (t.file_info_title_directory(&entry.name), true)
                } else if entry.is_symlink {
                    (t.file_info_title_symlink(&entry.name), false)
                } else {
                    (t.file_info_title_file(&entry.name), false)
                };

                let size = if is_dir {
                    "DIR".to_string()
                } else {
                    entry
                        .size
                        .map(utils::format_size)
                        .unwrap_or_else(|| "Unknown".to_string())
                };

                let modified = entry
                    .modified
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| {
                        chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                            .map(|dt| {
                                dt.with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M:%S")
                                    .to_string()
                            })
                            .unwrap_or_else(|| "Unknown".to_string())
                    })
                    .unwrap_or_else(|| "Unknown".to_string());

                let mode = if entry.is_executable {
                    "0755"
                } else if entry.is_readonly {
                    "0444"
                } else {
                    "0644"
                };

                // Collect data for remote file (no git status)
                let data = vec![
                    (t.file_info_path().to_string(), self.display_path()),
                    (t.file_info_size().to_string(), size),
                    (t.file_info_owner().to_string(), "remote".to_string()),
                    (t.file_info_group().to_string(), "remote".to_string()),
                    (t.file_info_modified().to_string(), modified),
                    ("Mode".to_string(), mode.to_string()),
                ];

                let modal = termide_modal::InfoModal::new(modal_title, data);
                self.modal_request = Some((
                    PendingAction::ClosePanel { panel_index: 0 },
                    ActiveModal::Info(Box::new(modal)),
                ));

                return;
            }

            // Local file handling
            let file_path = if entry.name == ".." {
                self.current_path
                    .parent()
                    .unwrap_or(&self.current_path)
                    .to_path_buf()
            } else {
                self.current_path.join(&entry.name)
            };

            if let Ok(metadata) = fs::metadata(&file_path) {
                let t = termide_i18n::t();

                // Determine type and title
                let (modal_title, is_dir) = if metadata.is_dir() {
                    (t.file_info_title_directory(&entry.name), true)
                } else if metadata.is_symlink() {
                    (t.file_info_title_symlink(&entry.name), false)
                } else {
                    (t.file_info_title_file(&entry.name), false)
                };

                let size = if is_dir {
                    format!("{}...", t.file_info_calculating())
                } else {
                    utils::format_size(metadata.len())
                };

                let owner = utils::get_user_name(metadata.uid());
                let group = utils::get_group_name(metadata.gid());

                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| {
                        chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                            .map(|dt| {
                                dt.with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M:%S")
                                    .to_string()
                            })
                            .unwrap_or_else(|| "Unknown".to_string())
                    })
                    .unwrap_or_else(|| "Unknown".to_string());

                let created = metadata
                    .created()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| {
                        chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                            .map(|dt| {
                                dt.with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M:%S")
                                    .to_string()
                            })
                            .unwrap_or_else(|| "Unknown".to_string())
                    })
                    .unwrap_or_else(|| "Unknown".to_string());

                // Collect data without Name and Type
                let mut data = vec![
                    (
                        t.file_info_path().to_string(),
                        file_path.display().to_string(),
                    ),
                    (t.file_info_size().to_string(), size),
                    (t.file_info_owner().to_string(), owner),
                    (t.file_info_group().to_string(), group),
                    (t.file_info_created().to_string(), created),
                    (t.file_info_modified().to_string(), modified),
                ];

                // Add git status if in repository (filtered by specific file/directory)
                // Special case: if directory is itself a git repo root, show its git info
                let (git_status, repo_path) = if is_dir && file_path.join(".git").exists() {
                    // Directory is a git repo root - get its own status
                    (
                        termide_git::get_repo_status(&file_path, &file_path),
                        Some(file_path.clone()),
                    )
                } else {
                    // Regular file/directory - check status in parent repo
                    let repo = termide_git::find_repo_root(&self.current_path);
                    (
                        termide_git::get_repo_status(&self.current_path, &file_path),
                        repo,
                    )
                };

                // Track whether file has actionable git status (any git actions available)
                let has_git_actions = git_status
                    .as_ref()
                    .map(|s| {
                        !s.is_ignored && (s.uncommitted_changes > 0 || s.ahead > 0 || s.behind > 0)
                    })
                    .unwrap_or(false);

                if let Some(ref git_status) = git_status {
                    if git_status.is_ignored {
                        // If file is ignored, show only one line
                        data.push((
                            t.file_info_git().to_string(),
                            t.file_info_git_ignored().to_string(),
                        ));
                    } else {
                        // Otherwise show three lines for uncommitted, ahead, behind
                        data.push((
                            t.file_info_git().to_string(),
                            t.file_info_git_uncommitted(git_status.uncommitted_changes),
                        ));
                        data.push((
                            String::new(), // Empty key - aligns with first line's value
                            t.file_info_git_ahead(git_status.ahead),
                        ));
                        data.push((
                            String::new(), // Empty key
                            t.file_info_git_behind(git_status.behind),
                        ));
                    }
                }

                // If file has git actions, use InfoActionModal with smart buttons
                if let (true, Some(ref status), Some(repo)) =
                    (has_git_actions, &git_status, repo_path)
                {
                    let buttons = Self::build_git_action_buttons(status);
                    let selected_button = buttons.len().saturating_sub(1); // Select [Close]
                    let modal =
                        termide_modal::InfoActionModal::new(modal_title, data.clone(), buttons)
                            .with_selected_button(selected_button);
                    self.modal_request = Some((
                        PendingAction::GitFileAction {
                            file_path: file_path.clone(),
                            repo_path: repo,
                            is_staged: false, // File manager shows unstaged files
                        },
                        ActiveModal::InfoAction(Box::new(modal)),
                    ));
                } else {
                    let modal = termide_modal::InfoModal::new(modal_title, data);
                    self.modal_request = Some((
                        PendingAction::ClosePanel { panel_index: 0 },
                        ActiveModal::Info(Box::new(modal)),
                    ));
                }

                if is_dir {
                    let (tx, rx) = mpsc::channel();

                    std::thread::spawn(move || {
                        let size = utils::calculate_dir_size(&file_path);
                        let _ = tx.send(DirSizeResult { size });
                    });

                    self.dir_size_receiver = Some(rx);
                }
            }
        }
    }

    /// Build action buttons for git info modal
    fn build_git_action_buttons(_git_status: &termide_git::GitRepoStatus) -> Vec<ActionButton> {
        let t = termide_i18n::t();

        // Show Git Status button to navigate to Git Status panel
        // where user can perform all git operations (commit, push, pull, etc.)
        vec![
            ActionButton::new(t.git_action_git_status(), "git_status"),
            ActionButton::new(t.git_action_close(), "close"),
        ]
    }

    /// Get disk space information for the current directory.
    pub fn get_disk_space_info(&self) -> Option<DiskSpaceInfo> {
        // Don't show disk info during VFS connection (status bar should show connection status)
        if self.vfs.has_pending_operation() {
            return None;
        }
        termide_system_monitor::get_disk_space_info(&self.current_path)
    }
}
