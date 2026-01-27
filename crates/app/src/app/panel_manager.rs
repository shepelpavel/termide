//! Panel management: close, focus, and path collection.

// Note: PanelExt is used for panel-specific path queries that require concrete type access.
#![allow(deprecated)]

use std::path::PathBuf;

use super::App;
use crate::PanelExt;
use termide_panel_misc::HelpPanel as Help;

impl App {
    /// Close panel by index and switch focus to next visible panel
    /// NOTE: panel_index parameter is now obsolete with LayoutManager, kept for compatibility
    pub(super) fn close_panel_at_index(&mut self, _panel_index: usize) {
        // Before closing, cleanup temporary files if this is an unsaved editor
        if let Some(panel) = self.layout_manager.active_panel_mut() {
            if let Some(editor) = panel.as_editor_mut() {
                // Cleanup LSP before closing
                if let Some(ref lsp_manager) = self.state.lsp_manager {
                    editor.cleanup_lsp(lsp_manager);
                }

                // Check if editor has a temporary unsaved buffer file
                if let Some(filename) = editor.unsaved_buffer_file() {
                    // Get session directory and delete the temporary file
                    if let Ok(session_dir) =
                        termide_session::Session::get_session_dir(&self.project_root)
                    {
                        if let Err(e) =
                            termide_session::delete_unsaved_buffer(&session_dir, filename)
                        {
                            log::warn!("Failed to delete unsaved buffer file: {}", e);
                        }
                    }
                }
            }
        }

        // Check if editor was editing bookmarks.toml - reload bookmarks on close
        if let Some(panel) = self.layout_manager.active_panel_mut() {
            if let Some(editor) = panel.as_editor() {
                if let Some(path) = editor.file_path() {
                    if let Ok(bookmarks_path) = termide_config::BookmarksConfig::config_file_path()
                    {
                        if path == bookmarks_path {
                            self.state.bookmarks = termide_config::BookmarksConfig::load();
                        }
                    }
                }
            }
        }

        // Before closing, unwatch filesystem if this is a FileManager panel
        if let Some(panel) = self.layout_manager.active_panel_mut() {
            if let Some(fm) = panel.as_file_manager_mut() {
                // Unwatch the filesystem root for this FileManager
                if let Some(watched_root) = fm.take_watched_root() {
                    if let Some(watcher) = &mut self.state.watcher {
                        if termide_git::find_repo_root(&watched_root).is_some() {
                            watcher.unwatch_repository(&watched_root);
                        } else {
                            watcher.unwatch_directory(&watched_root);
                        }
                    }
                }
            }
        }

        // Calculate available width for panel groups
        let terminal_width = self.state.terminal.width;

        // Close active panel (LayoutManager handles active panel tracking)
        let _ = self.layout_manager.close_active_panel(terminal_width);
        self.auto_save_session();

        // Note: FileManager reload removed - FS watcher handles git status updates
        // when files change. Cascade reload caused O(n*m) delays on panel close.

        // Add Welcome panel if needed
        // Check if no panel groups remain (all panels closed)
        let should_add_welcome = self.layout_manager.panel_groups.is_empty();

        if should_add_welcome {
            let welcome = Help::new(&self.state.config);
            self.add_panel(Box::new(welcome));
        }

        // Active panel tracking is handled by LayoutManager
        // No need to manually update active_panel index
    }

    /// Collect all working directory paths from all panels
    /// Returns deduplicated list of paths from all panel types (FM, Terminal, Editor, etc.)
    pub(super) fn collect_panel_paths(&self) -> Vec<PathBuf> {
        use std::collections::HashSet;

        let mut unique_paths: HashSet<PathBuf> = HashSet::new();

        // Collect all unique paths from all panels in groups
        for group in &self.layout_manager.panel_groups {
            for panel in group.panels() {
                // Get working directory from any panel type
                if let Some(dir) = panel.get_working_directory() {
                    unique_paths.insert(dir);
                }
            }
        }

        unique_paths.into_iter().collect()
    }

    /// Find all panels that have working directories
    /// Returns deduplicated and sorted list of paths from all panel types (FM, Terminal, Editor)
    /// For remote file managers, returns full URLs (e.g., sftp://user@host/path)
    pub(super) fn find_all_other_panel_paths(&self) -> Vec<termide_modal::SelectOption> {
        use std::collections::HashSet;

        let mut unique_paths: HashSet<String> = HashSet::new();

        // Collect all unique paths from all panels in groups
        // Use get_working_directory_display() to get full URLs for remote paths
        for group in &self.layout_manager.panel_groups {
            for panel in group.panels() {
                if let Some(path_str) = panel.get_working_directory_display() {
                    unique_paths.insert(path_str);
                }
            }
        }

        // Convert to SelectOptions (add trailing slash for directories)
        let mut options: Vec<_> = unique_paths
            .into_iter()
            .map(|path_str| {
                let with_slash = if path_str.ends_with('/') {
                    path_str
                } else {
                    format!("{}/", path_str)
                };
                termide_modal::SelectOption {
                    value: with_slash.clone(),
                    display: with_slash,
                }
            })
            .collect();

        // Sort by value for consistent ordering
        options.sort_by(|a, b| a.value.cmp(&b.value));

        options
    }

    /// Refresh all FM panels that show specified directory
    pub(super) fn refresh_fm_panels(&mut self, directory: &std::path::Path) {
        // Refresh all FileManager panels showing this directory
        for group in &mut self.layout_manager.panel_groups {
            for panel in group.panels_mut() {
                if let Some(fm) = panel.as_file_manager_mut() {
                    // Check if FM working directory matches target
                    let fm_dir = fm.get_current_directory();
                    if fm_dir == directory {
                        // Refresh directory contents (preserving selection)
                        let _ = fm.reload_directory();
                    }
                }
            }
        }
    }
}
