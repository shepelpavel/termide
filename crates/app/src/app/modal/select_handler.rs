//! Select modal result handling.

// Note: PanelExt is used for editor-specific operations (go_to_line, save, replace).
#![allow(deprecated)]

use anyhow::Result;
use std::path::PathBuf;

use super::super::App;
use crate::state::{ActiveModal, PendingAction};
use crate::PanelExt;
use termide_i18n as i18n;
use termide_ui::path_utils;

impl App {
    /// Handle editor closure with saving
    pub(in crate::app) fn handle_close_editor_with_save(
        &mut self,
        _panel_index: usize, // obsolete with LayoutManager
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        // Store info needed for LSP notification (before mutable borrow)
        let mut lsp_info: Option<(String, PathBuf)> = None;

        if let Some(selected) = value.downcast_ref::<Vec<usize>>() {
            if selected.is_empty() {
                // Cancel or Esc - do nothing
                return Ok(());
            }

            match selected[0] {
                0 => {
                    // Save and close
                    log::info!("Selected: Save and close editor");
                    if let Some(panel) = self.layout_manager.active_panel_mut() {
                        if let Some(editor) = panel.as_editor_mut() {
                            // Try to save
                            if editor.has_file_path() {
                                // File already has path - just save
                                let t = i18n::t();

                                match editor.save() {
                                    Err(e) => {
                                        log::error!("Save error: {}", e);
                                        self.state.set_error(t.status_error_save(&e.to_string()));
                                        return Ok(());
                                    }
                                    Ok(Some((temp_path, remote_path, vfs_manager))) => {
                                        // Remote file - start async upload with progress modal
                                        log::info!("Starting remote file upload before closing");

                                        let filename = remote_path
                                            .file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_else(|| "file".to_string());
                                        let modal = termide_modal::ProgressModal::indeterminate(
                                            "Upload",
                                            format!("Uploading {}...", filename),
                                        );
                                        self.state.active_modal =
                                            Some(ActiveModal::Progress(Box::new(modal)));

                                        // Set uploading flag to show spinner in editor header
                                        if let Some(panel) = self.layout_manager.active_panel_mut()
                                        {
                                            if let Some(editor) = panel.as_editor_mut() {
                                                editor.set_uploading(true);
                                            }
                                        }

                                        // Create upload request via OperationManager
                                        let request = termide_file_ops::OperationRequest::upload(
                                            temp_path,
                                            remote_path,
                                        );

                                        // Start upload via OperationManager
                                        match self.state.start_operation_now(request, vfs_manager) {
                                            Ok(_operation_id) => {
                                                log::info!("Started save-before-close upload");
                                            }
                                            Err(e) => {
                                                log::error!("Failed to start upload: {}", e);
                                                self.state.close_modal();
                                                self.state
                                                    .set_error(format!("Upload failed: {}", e));
                                                // Clear uploading flag
                                                if let Some(panel) =
                                                    self.layout_manager.active_panel_mut()
                                                {
                                                    if let Some(editor) = panel.as_editor_mut() {
                                                        editor.set_uploading(false);
                                                    }
                                                }
                                            }
                                        }

                                        // Don't close panel yet - wait for upload to complete
                                        return Ok(());
                                    }
                                    Ok(None) => {
                                        // Local file - saved synchronously
                                        log::info!("File saved before closing");
                                    }
                                }

                                // Collect LSP info for didSave notification
                                if let Some(lang) = editor.lsp_language() {
                                    if let Some(path) = editor.file_path() {
                                        lsp_info = Some((lang.to_string(), path.to_path_buf()));
                                    }
                                }
                            } else {
                                // Unnamed file - need to request name
                                let t = i18n::t();
                                let modal =
                                    termide_modal::InputModal::new(t.modal_save_as_title(), "");
                                let current_dir =
                                    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
                                let action = PendingAction::SaveFileAs {
                                    panel_index: 0, // placeholder, obsolete
                                    directory: current_dir,
                                };
                                self.state.set_pending_action(
                                    action,
                                    ActiveModal::Input(Box::new(modal)),
                                );
                                // After saving file will remain open, need to close separately
                                // This is simplification, but for full implementation need more complex PendingAction
                                return Ok(());
                            }
                        }
                    }
                    // Close panel after saving
                    self.close_panel_at_index(0); // panel_index is obsolete
                }
                1 => {
                    // Close without saving
                    log::info!("Selected: Close without saving");
                    self.close_panel_at_index(0); // panel_index is obsolete
                }
                _ => {
                    // Cancel - do nothing
                    log::info!("Selected: Cancel closing");
                }
            }
        }

        // Send LSP didSave notification (triggers full analysis for semantic errors)
        if let Some((lang, file_path)) = lsp_info {
            if let Some(ref lsp_manager) = self.state.lsp_manager {
                lsp_manager.did_save(&lang, &file_path, None);
            }
        }

        Ok(())
    }

    /// Handle editor closure with external changes (file changed on disk)
    pub(in crate::app) fn handle_close_editor_external(
        &mut self,
        _panel_index: usize, // obsolete with LayoutManager
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        if let Some(selected) = value.downcast_ref::<Vec<usize>>() {
            if selected.is_empty() {
                // Cancel or Esc - do nothing
                return Ok(());
            }

            match selected[0] {
                0 => {
                    // Overwrite disk with current content
                    log::info!("Selected: Overwrite disk with current content");
                    if let Some(panel) = self.layout_manager.active_panel_mut() {
                        if let Some(editor) = panel.as_editor_mut() {
                            let t = i18n::t();
                            match editor.force_save() {
                                Err(e) => {
                                    log::error!("Force save error: {}", e);
                                    self.state.set_error(t.status_error_save(&e.to_string()));
                                    return Ok(());
                                }
                                Ok(_upload_op) => {
                                    // TODO: Handle async upload operation for remote files
                                }
                            }
                        }
                    }
                    self.close_panel_at_index(0);
                }
                1 => {
                    // Keep disk version (just close)
                    log::info!("Selected: Keep disk version, close editor");
                    self.close_panel_at_index(0);
                }
                2 => {
                    // Reload into editor (don't close)
                    log::info!("Selected: Reload file into editor");
                    if let Some(panel) = self.layout_manager.active_panel_mut() {
                        if let Some(editor) = panel.as_editor_mut() {
                            let t = i18n::t();
                            if let Err(e) = editor.reload_from_disk() {
                                log::error!("Reload error: {}", e);
                                self.state.set_error(t.status_error_reload(&e.to_string()));
                            } else {
                                self.state.set_info(t.status_file_reloaded().to_string());
                            }
                        }
                    }
                    // Don't close - user wants to continue editing
                }
                _ => {
                    // Cancel - do nothing
                    log::info!("Selected: Cancel closing");
                }
            }
        }
        Ok(())
    }

    /// Handle editor closure with conflict (local + external changes)
    pub(in crate::app) fn handle_close_editor_conflict(
        &mut self,
        _panel_index: usize, // obsolete with LayoutManager
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        if let Some(selected) = value.downcast_ref::<Vec<usize>>() {
            if selected.is_empty() {
                // Cancel or Esc - do nothing
                return Ok(());
            }

            match selected[0] {
                0 => {
                    // Overwrite disk with my changes
                    log::info!("Selected: Overwrite disk with local changes");
                    if let Some(panel) = self.layout_manager.active_panel_mut() {
                        if let Some(editor) = panel.as_editor_mut() {
                            let t = i18n::t();
                            match editor.force_save() {
                                Err(e) => {
                                    log::error!("Force save error: {}", e);
                                    self.state.set_error(t.status_error_save(&e.to_string()));
                                    return Ok(());
                                }
                                Ok(_upload_op) => {
                                    // TODO: Handle async upload operation for remote files
                                }
                            }
                        }
                    }
                    self.close_panel_at_index(0);
                }
                1 => {
                    // Reload from disk (discard local changes)
                    log::info!("Selected: Reload from disk, discard local changes");
                    if let Some(panel) = self.layout_manager.active_panel_mut() {
                        if let Some(editor) = panel.as_editor_mut() {
                            let t = i18n::t();
                            if let Err(e) = editor.reload_from_disk() {
                                log::error!("Reload error: {}", e);
                                self.state.set_error(t.status_error_reload(&e.to_string()));
                                return Ok(());
                            }
                        }
                    }
                    self.close_panel_at_index(0);
                }
                _ => {
                    // Cancel - do nothing
                    log::info!("Selected: Cancel closing");
                }
            }
        }
        Ok(())
    }

    /// Handle cancelled copy/move operation cleanup
    ///
    /// For directory copy (is_directory=true):
    /// - partial_path = the file that was being copied when cancelled
    /// - all_dest_paths[0] = the destination directory
    /// - Options: Delete partial (file only), Delete all (entire dir), Keep all
    ///
    /// For file copy (is_directory=false):
    /// - partial_path = the partial file
    /// - all_dest_paths = previously completed files (empty for single file)
    /// - Options: Delete (partial), [Delete all], Keep
    pub(in crate::app) fn handle_cancel_copy_cleanup(
        &mut self,
        partial_path: PathBuf,
        all_dest_paths: Vec<PathBuf>,
        is_directory: bool,
        _batch_operation: Option<Box<termide_state::BatchOperation>>,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        // ChoiceModal returns usize (button index)
        if let Some(&selected) = value.downcast_ref::<usize>() {
            if is_directory {
                // Directory copy cancellation - always 3 options:
                // 0 = Keep all (keep everything as is)
                // 1 = Delete partial (only the interrupted file)
                // 2 = Delete all (entire destination directory)
                match selected {
                    1 => {
                        // Delete only the interrupted file
                        log::info!(
                            "Cancel cleanup: delete partial file {}",
                            partial_path.display()
                        );
                        if partial_path.exists() {
                            if let Err(e) = std::fs::remove_file(&partial_path) {
                                self.state.set_error(format!("Failed to delete: {}", e));
                            } else {
                                self.state.set_info("Partial file deleted".to_string());
                            }
                        }
                    }
                    2 => {
                        // Delete entire destination directory
                        if let Some(dest_dir) = all_dest_paths.first() {
                            log::info!("Cancel cleanup: delete all {}", dest_dir.display());
                            if dest_dir.exists() {
                                if let Err(e) = std::fs::remove_dir_all(dest_dir) {
                                    self.state.set_error(format!("Failed to delete: {}", e));
                                } else {
                                    self.state.set_info("Directory deleted".to_string());
                                }
                            }
                        }
                    }
                    _ => {
                        // Keep all (0 or any other)
                        log::info!("Cancel cleanup: keep all");
                    }
                }
            } else {
                // File copy cancellation
                let has_multiple = !all_dest_paths.is_empty();

                match (selected, has_multiple) {
                    (0, _) => {
                        // Delete partial file only
                        log::info!("Cancel cleanup: delete partial {}", partial_path.display());
                        if partial_path.exists() {
                            if let Err(e) = std::fs::remove_file(&partial_path) {
                                self.state.set_error(format!("Failed to delete: {}", e));
                            } else {
                                self.state.set_info("File deleted".to_string());
                            }
                        }
                    }
                    (1, true) => {
                        // Delete all copied files
                        log::info!(
                            "Cancel cleanup: delete all {} items",
                            all_dest_paths.len() + 1
                        );
                        let mut deleted = 0;
                        let mut errors = 0;

                        // Delete partial file first
                        if partial_path.exists() {
                            if std::fs::remove_file(&partial_path).is_ok() {
                                deleted += 1;
                            } else {
                                errors += 1;
                            }
                        }

                        // Delete all completed files
                        for dest in &all_dest_paths {
                            if dest.exists() {
                                let result = if dest.is_dir() {
                                    std::fs::remove_dir_all(dest)
                                } else {
                                    std::fs::remove_file(dest)
                                };
                                if result.is_ok() {
                                    deleted += 1;
                                } else {
                                    errors += 1;
                                }
                            }
                        }

                        if errors > 0 {
                            self.state
                                .set_error(format!("Deleted {} items, {} errors", deleted, errors));
                        } else {
                            self.state.set_info(format!("Deleted {} items", deleted));
                        }
                    }
                    _ => {
                        // Keep everything
                        log::info!("Cancel cleanup: keep everything");
                    }
                }
            }
        }

        // Refresh file manager panels to show changes
        if let Some(parent) = partial_path.parent() {
            self.refresh_fm_panels(parent);
        }
        for dest in &all_dest_paths {
            if let Some(parent) = dest.parent() {
                self.refresh_fm_panels(parent);
            }
        }

        Ok(())
    }

    /// Handle file overwrite decision
    pub(in crate::app) fn handle_overwrite_decision(
        &mut self,
        _panel_index: usize, // obsolete with LayoutManager
        source: PathBuf,
        destination: PathBuf,
        is_move: bool,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        if let Some(choice) = value.downcast_ref::<termide_modal::OverwriteChoice>() {
            use termide_modal::OverwriteChoice;

            let item_name = path_utils::get_file_name_str(&source);

            let final_dest = path_utils::resolve_destination_path(&source, &destination);

            // Check overwrite conditions
            let should_proceed = match choice {
                OverwriteChoice::Replace => true,
                OverwriteChoice::ReplaceIfNewer => {
                    // Compare modification time
                    if let (Ok(src_meta), Ok(dst_meta)) = (source.metadata(), final_dest.metadata())
                    {
                        if let (Ok(src_time), Ok(dst_time)) =
                            (src_meta.modified(), dst_meta.modified())
                        {
                            src_time > dst_time
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
                OverwriteChoice::ReplaceIfLarger => {
                    // Compare file sizes
                    if let (Ok(src_meta), Ok(dst_meta)) = (source.metadata(), final_dest.metadata())
                    {
                        src_meta.len() > dst_meta.len()
                    } else {
                        false
                    }
                }
                OverwriteChoice::Skip => false,
            };

            if should_proceed {
                // Use batch operation system for async handling (all operations)
                use termide_state::{BatchOperation, BatchOperationType};

                let operation_type = if is_move {
                    BatchOperationType::Move
                } else {
                    BatchOperationType::Copy
                };

                let batch_op = BatchOperation::new(
                    operation_type,
                    vec![source.clone()],
                    final_dest.parent().unwrap_or(&final_dest).to_path_buf(),
                );

                self.process_batch_operation(batch_op);
            } else {
                let t = i18n::t();
                log::info!("Operation '{}' skipped", item_name);
                self.state.set_info(t.status_operation_skipped(item_name));
            }
        }
        Ok(())
    }
}
