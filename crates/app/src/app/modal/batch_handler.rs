//! Batch file operation handling.

// Note: PanelExt is used for FileManager batch operations (copy/move/delete/rename).
#![allow(deprecated)]

use anyhow::Result;
use std::path::{Path, PathBuf};

use super::super::App;
use crate::state::{
    ActiveModal, BatchOperation, BatchOperationType, ConflictMode, PendingAction,
    PendingRemoteDelete,
};
use crate::PanelExt;
use termide_i18n as i18n;
use termide_modal::ConflictModal;
use termide_ui::path_utils;
use termide_vfs::VfsPath;

/// Format VfsPath for display in progress modal
fn format_vfs_path_for_display(vfs_path: &VfsPath, file_path: &Path) -> String {
    if vfs_path.is_local() {
        file_path.display().to_string()
    } else {
        let mut result = String::new();

        // Add username@host
        if let Some(ref user) = vfs_path.username {
            result.push_str(user);
            result.push('@');
        }

        if let Some(ref host) = vfs_path.host {
            result.push_str(host);
        }

        // Add port if non-standard
        if let Some(port) = vfs_path.port {
            let default_port = vfs_path.default_port();
            if Some(port) != default_port {
                result.push(':');
                result.push_str(&port.to_string());
            }
        }

        // Add path (no colon separator)
        let full_path = vfs_path
            .path
            .join(file_path.file_name().unwrap_or_default());
        result.push_str(&full_path.display().to_string());

        result
    }
}

impl App {
    /// Common method for handling file operations (Copy/Move)
    fn handle_file_operation(
        &mut self,
        operation_type: BatchOperationType,
        panel_index: usize,
        sources: Vec<PathBuf>,
        target_directory: Option<PathBuf>,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        // Extract destination string first to check if it's a remote URL
        let destination_str: Option<String> = if let Some(confirmed) = value.downcast_ref::<bool>()
        {
            if !confirmed {
                return Ok(()); // Operation cancelled by user
            }
            // Use target_directory as string for Ctrl+V confirmation
            target_directory.as_ref().map(|p| p.display().to_string())
        } else if let Some(s) = value.downcast_ref::<String>() {
            Some(s.clone())
        } else {
            return Ok(()); // Invalid response type
        };

        let Some(dest_str) = destination_str else {
            return Ok(());
        };

        // Check if destination is a remote VFS URL (e.g., sftp://user@host/path)
        if termide_vfs::is_vfs_url(&dest_str) {
            // Local-to-remote upload
            return self.start_upload_operation(operation_type, sources, &dest_str);
        }

        // Local destination - determine absolute path
        let absolute_destination = if let Some(target_dir) = target_directory {
            let destination = PathBuf::from(&dest_str);
            if destination.is_absolute() {
                destination
            } else {
                target_dir.join(&destination)
            }
        } else {
            // Get active FileManager panel to determine base path
            if let Some(panel) = self.layout_manager.active_panel_mut() {
                if let Some(fm) = panel.as_file_manager_mut() {
                    let destination = PathBuf::from(&dest_str);
                    if destination.is_absolute() {
                        destination
                    } else {
                        fm.get_current_directory().join(&destination)
                    }
                } else {
                    log::error!("Panel {} is not FileManager", panel_index);
                    return Ok(());
                }
            } else {
                log::error!("Panel with index {} not found", panel_index);
                return Ok(());
            }
        };

        // Create and start batch operation
        let batch_op = BatchOperation::new(operation_type, sources, absolute_destination);

        self.process_batch_operation(batch_op);
        Ok(())
    }

    /// Start upload operation for local-to-remote file transfer
    fn start_upload_operation(
        &mut self,
        operation_type: BatchOperationType,
        sources: Vec<PathBuf>,
        remote_url: &str,
    ) -> Result<()> {
        use termide_modal::ProgressModal;

        if sources.is_empty() {
            return Ok(());
        }

        // Parse the remote URL
        let remote_path = match termide_vfs::parse_vfs_url(remote_url) {
            Ok(path) => path,
            Err(e) => {
                log::error!("Invalid remote URL '{}': {}", remote_url, e);
                self.state.set_error(format!("Invalid remote URL: {}", e));
                return Ok(());
            }
        };

        // Get VFS manager from an existing remote panel or create a new one
        let vfs_manager = self.get_or_create_vfs_manager(&remote_path);

        let is_move = operation_type == BatchOperationType::Move;
        let total_files = sources.len();

        // Start with the first file (clone to avoid borrow issues)
        let source = sources[0].clone();
        let source_name = source
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());

        // Determine final remote destination path
        let final_remote = remote_path.join(&source_name);

        // Show progress modal with source/destination
        let title = match operation_type {
            BatchOperationType::Copy => "Upload",
            BatchOperationType::Move => "Upload (Move)",
        };
        let modal = ProgressModal::new_copy_progress(
            title,
            total_files,
            source.display().to_string(),
            final_remote.to_url_string(),
            true, // Support pause via OperationManager
        );
        self.state.active_modal = Some(ActiveModal::Progress(Box::new(modal)));

        // Create upload operation request
        use termide_file_ops::OperationRequest;
        let request = OperationRequest::upload(source.clone(), final_remote);

        // Store batch upload state for continuation
        self.state.pending_batch_upload = Some(crate::state::PendingBatchUpload {
            all_sources: sources,
            current_index: 0,
            dest_base_url: remote_url.to_string(),
            vfs_manager: vfs_manager.clone(),
            is_move,
            current_source: source.clone(),
        });

        // Start upload via OperationManager
        match self.state.start_operation_now(request, vfs_manager) {
            Ok(_operation_id) => {
                // Operation started, will be polled in poll_operation_manager
            }
            Err(e) => {
                log::error!("Failed to start upload operation: {}", e);
                self.state.pending_batch_upload = None;
                self.state.close_modal();
                self.state.set_error(format!("Upload failed: {}", e));
            }
        }

        Ok(())
    }

    /// Get VFS manager from an existing remote FM panel or create a new one
    fn get_or_create_vfs_manager(
        &self,
        remote_path: &termide_vfs::VfsPath,
    ) -> std::sync::Arc<termide_vfs::VfsManager> {
        // Try to find an existing FM panel connected to the same host
        for group in &self.layout_manager.panel_groups {
            for panel in group.panels() {
                if let Some(fm) = panel
                    .as_any()
                    .downcast_ref::<termide_panel_file_manager::FileManager>()
                {
                    if fm.is_remote() {
                        let fm_path = fm.vfs_state().current_path();
                        // Check if same connection (protocol + host + username)
                        if fm_path.connection_key() == remote_path.connection_key() {
                            return fm.vfs_state().manager_arc();
                        }
                    }
                }
            }
        }

        // No existing connection found - create a new manager
        std::sync::Arc::new(termide_vfs::VfsManager::new())
    }

    /// Handle file copying
    pub(in crate::app) fn handle_copy_path(
        &mut self,
        panel_index: usize,
        sources: Vec<PathBuf>,
        target_directory: Option<PathBuf>,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        self.handle_file_operation(
            BatchOperationType::Copy,
            panel_index,
            sources,
            target_directory,
            value,
        )
    }

    /// Handle file moving
    pub(in crate::app) fn handle_move_path(
        &mut self,
        panel_index: usize,
        sources: Vec<PathBuf>,
        target_directory: Option<PathBuf>,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        self.handle_file_operation(
            BatchOperationType::Move,
            panel_index,
            sources,
            target_directory,
            value,
        )
    }

    /// Handle continuation of batch operation after conflict resolution
    pub(in crate::app) fn handle_continue_batch_operation(
        &mut self,
        mut operation: BatchOperation,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        use termide_modal::ConflictResolution;

        if let Some(resolution) = value.downcast_ref::<ConflictResolution>() {
            match resolution {
                ConflictResolution::Overwrite => {
                    // Overwrite this file - execute operation directly
                    if let Some(source) = operation.current_source().cloned() {
                        let final_dest = path_utils::resolve_batch_destination_path(
                            &source,
                            &operation.destination,
                            operation.sources.len() == 1,
                        );

                        // Execute operation
                        if let Some(panel) = self.layout_manager.active_panel_mut() {
                            if let Some(fm) = panel.as_file_manager_mut() {
                                // Show progress modal for the operation
                                use termide_modal::ProgressModal;
                                let title = match operation.operation_type {
                                    BatchOperationType::Copy => "Copy",
                                    BatchOperationType::Move => "Move",
                                };
                                let source_display = source.display().to_string();
                                let dest_display = final_dest.display().to_string();
                                let modal = ProgressModal::new_copy_progress(
                                    title,
                                    operation.total_count(),
                                    source_display,
                                    dest_display,
                                    true,
                                );
                                self.state.active_modal =
                                    Some(ActiveModal::Progress(Box::new(modal)));

                                // Check if source is remote - use OperationManager download
                                if fm.is_remote() {
                                    use termide_file_ops::OperationRequest;

                                    // For remote-to-local copy/move, use VFS download via OperationManager
                                    let source_name = source
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    let vfs_source =
                                        fm.vfs_state().current_path().join(&source_name);
                                    let vfs_manager = fm.vfs_state().manager_arc();

                                    let is_move =
                                        operation.operation_type == BatchOperationType::Move;

                                    // Create download request
                                    let request =
                                        OperationRequest::download(vfs_source.clone(), final_dest);

                                    // Store VFS info for move operation (delete source after download)
                                    if is_move {
                                        self.state.pending_remote_delete =
                                            Some(PendingRemoteDelete {
                                                vfs_source,
                                                vfs_manager: vfs_manager.clone(),
                                            });
                                    }

                                    match self.state.start_operation_now(request, vfs_manager) {
                                        Ok(_operation_id) => {
                                            self.state.pending_action =
                                                Some(PendingAction::ContinueBatchOperation {
                                                    operation,
                                                });
                                        }
                                        Err(e) => {
                                            log::error!(
                                                "Failed to start download operation: {}",
                                                e
                                            );
                                            operation.increment_error();
                                            operation.advance();
                                            self.state.pending_action =
                                                Some(PendingAction::ContinueBatchOperation {
                                                    operation,
                                                });
                                        }
                                    }
                                    return Ok(());
                                }

                                // Local file - use OperationManager for async copy
                                if source.is_file() {
                                    use termide_file_ops::{OperationPath, OperationRequest};

                                    let is_move =
                                        operation.operation_type == BatchOperationType::Move;
                                    let request = if is_move {
                                        OperationRequest::r#move(
                                            vec![OperationPath::Local(source.clone())],
                                            OperationPath::Local(final_dest.clone()),
                                        )
                                    } else {
                                        OperationRequest::copy(
                                            vec![OperationPath::Local(source.clone())],
                                            OperationPath::Local(final_dest.clone()),
                                        )
                                    };

                                    let vfs_manager =
                                        std::sync::Arc::new(termide_vfs::VfsManager::new());

                                    match self.state.start_operation_now(request, vfs_manager) {
                                        Ok(_operation_id) => {
                                            self.state.pending_action =
                                                Some(PendingAction::ContinueBatchOperation {
                                                    operation,
                                                });
                                        }
                                        Err(e) => {
                                            log::error!("Failed to start copy operation: {}", e);
                                            operation.increment_error();
                                            operation.advance();
                                            self.state.pending_action =
                                                Some(PendingAction::ContinueBatchOperation {
                                                    operation,
                                                });
                                        }
                                    }
                                    return Ok(());
                                }

                                // Local directory - use OperationManager (handles scan + copy)
                                if source.is_dir() {
                                    use termide_file_ops::{OperationPath, OperationRequest};

                                    let is_move =
                                        operation.operation_type == BatchOperationType::Move;
                                    let request = if is_move {
                                        OperationRequest::r#move(
                                            vec![OperationPath::Local(source.clone())],
                                            OperationPath::Local(final_dest.clone()),
                                        )
                                    } else {
                                        OperationRequest::copy(
                                            vec![OperationPath::Local(source.clone())],
                                            OperationPath::Local(final_dest.clone()),
                                        )
                                    };

                                    let vfs_manager =
                                        std::sync::Arc::new(termide_vfs::VfsManager::new());

                                    match self.state.start_operation_now(request, vfs_manager) {
                                        Ok(_operation_id) => {
                                            self.state.pending_action =
                                                Some(PendingAction::ContinueBatchOperation {
                                                    operation,
                                                });
                                        }
                                        Err(e) => {
                                            log::error!(
                                                "Failed to start directory copy operation: {}",
                                                e
                                            );
                                            operation.increment_error();
                                            operation.advance();
                                            self.state.pending_action =
                                                Some(PendingAction::ContinueBatchOperation {
                                                    operation,
                                                });
                                        }
                                    }
                                    return Ok(());
                                }

                                // Unknown source type - skip with error
                                log::error!("Unsupported source type: {}", source.display());
                                operation.increment_error();
                            }
                        }
                    }

                    // Re-show progress modal BEFORE advancing (check with current index)
                    // (conflict modal was shown and closed, now we need progress modal back)
                    if operation.total_count() > 1 {
                        use termide_modal::ProgressModal;
                        let title = match operation.operation_type {
                            BatchOperationType::Copy => "Copying Files",
                            BatchOperationType::Move => "Moving Files",
                        };
                        let modal =
                            ProgressModal::new_with_controls(title, operation.total_count(), true);
                        self.state.active_modal = Some(ActiveModal::Progress(Box::new(modal)));
                    }

                    // Move to next file
                    operation.advance();

                    // Store and return to allow UI update
                    self.state.pending_action =
                        Some(PendingAction::ContinueBatchOperation { operation });
                }
                ConflictResolution::Skip => {
                    // Skip this file
                    operation.increment_skipped();

                    // Re-show progress modal BEFORE advancing (check with current index)
                    if operation.total_count() > 1 {
                        use termide_modal::ProgressModal;
                        let title = match operation.operation_type {
                            BatchOperationType::Copy => "Copying Files",
                            BatchOperationType::Move => "Moving Files",
                        };
                        let modal =
                            ProgressModal::new_with_controls(title, operation.total_count(), true);
                        self.state.active_modal = Some(ActiveModal::Progress(Box::new(modal)));
                    }

                    operation.advance();

                    // Store and return to allow UI update
                    self.state.pending_action =
                        Some(PendingAction::ContinueBatchOperation { operation });
                }
                ConflictResolution::OverwriteAll => {
                    // Set "overwrite all" mode
                    operation.set_conflict_mode(ConflictMode::OverwriteAll);

                    // Re-show progress modal BEFORE processing (check with current index)
                    if operation.total_count() > 1 {
                        use termide_modal::ProgressModal;
                        let title = match operation.operation_type {
                            BatchOperationType::Copy => "Copying Files",
                            BatchOperationType::Move => "Moving Files",
                        };
                        let modal =
                            ProgressModal::new_with_controls(title, operation.total_count(), true);
                        self.state.active_modal = Some(ActiveModal::Progress(Box::new(modal)));
                    }

                    // Store and return to allow UI update
                    self.state.pending_action =
                        Some(PendingAction::ContinueBatchOperation { operation });
                }
                ConflictResolution::SkipAll => {
                    // Set "skip all" mode
                    operation.set_conflict_mode(ConflictMode::SkipAll);
                    operation.increment_skipped();

                    // Re-show progress modal BEFORE advancing (check with current index)
                    if operation.total_count() > 1 {
                        use termide_modal::ProgressModal;
                        let title = match operation.operation_type {
                            BatchOperationType::Copy => "Copying Files",
                            BatchOperationType::Move => "Moving Files",
                        };
                        let modal =
                            ProgressModal::new_with_controls(title, operation.total_count(), true);
                        self.state.active_modal = Some(ActiveModal::Progress(Box::new(modal)));
                    }

                    operation.advance();

                    // Store and return to allow UI update
                    self.state.pending_action =
                        Some(PendingAction::ContinueBatchOperation { operation });
                }
                ConflictResolution::Rename => {
                    // Request rename pattern for single file
                    if let Some(source) = operation.current_source() {
                        let original_name = path_utils::get_file_name_string(source);

                        // Get file metadata for preview
                        let metadata = source.metadata().ok();
                        let created = metadata.as_ref().and_then(|m| m.created().ok());
                        let modified = metadata.as_ref().and_then(|m| m.modified().ok());

                        use termide_modal::RenamePatternModal;

                        let modal = RenamePatternModal::new(
                            &format!("Rename {}", original_name),
                            &original_name,
                            "$0", // Default pattern
                            created,
                            modified,
                        );

                        self.state.pending_action = Some(PendingAction::RenameWithPattern {
                            operation,
                            original_name,
                        });
                        self.state.active_modal = Some(ActiveModal::RenamePattern(Box::new(modal)));
                    }
                }
                ConflictResolution::RenameAll => {
                    // Request rename pattern for all files
                    if let Some(source) = operation.current_source() {
                        let original_name = path_utils::get_file_name_string(source);

                        // Get file metadata for preview
                        let metadata = source.metadata().ok();
                        let created = metadata.as_ref().and_then(|m| m.created().ok());
                        let modified = metadata.as_ref().and_then(|m| m.modified().ok());

                        use termide_modal::RenamePatternModal;

                        let modal = RenamePatternModal::new(
                            &format!("Rename all ({})", original_name),
                            &original_name,
                            "$0", // Default pattern
                            created,
                            modified,
                        );

                        // Set flag that this is RenameAll
                        operation.set_conflict_mode(ConflictMode::Ask); // Reset to Ask to apply pattern

                        self.state.pending_action = Some(PendingAction::RenameWithPattern {
                            operation,
                            original_name,
                        });
                        self.state.active_modal = Some(ActiveModal::RenamePattern(Box::new(modal)));
                    }
                }
            }
        }
        Ok(())
    }

    /// Handle batch file operation (copy/move)
    pub(in crate::app) fn process_batch_operation(&mut self, mut operation: BatchOperation) {
        // Show progress modal for:
        // 1. Multi-file operations (total_count > 1), OR
        // 2. Single remote file operations (need network transfer feedback), OR
        // 3. Single directory operations (recursive copy/move can take time), OR
        // 4. Single file > 1MB (large file transfer needs progress feedback)

        // Check if this is a remote operation by examining active panel
        let is_remote_operation = {
            if let Some(panel) = self.layout_manager.active_panel_mut() {
                panel
                    .as_file_manager_mut()
                    .map(|fm| fm.is_remote())
                    .unwrap_or(false)
            } else {
                false
            }
        };

        // Check if source is a directory or large file (>1MB)
        let needs_progress = operation
            .current_source()
            .and_then(|p| {
                p.metadata().ok().map(|meta| {
                    meta.is_dir() || meta.len() > 1_048_576 // 1MB
                })
            })
            .unwrap_or(false);

        // Show enhanced progress modal when starting operation
        // Only create modal if not already shown (check that no progress modal is active)
        let should_show_modal = operation.current_index == 0
            && (operation.total_count() > 1 || is_remote_operation || needs_progress)
            && !matches!(self.state.active_modal, Some(ActiveModal::Progress(_)));

        if should_show_modal {
            use termide_modal::ProgressModal;

            // Close any existing modal (e.g., destination selection) before showing progress
            self.state.close_modal();

            let title = match operation.operation_type {
                BatchOperationType::Copy => "Copy",
                BatchOperationType::Move => "Move",
            };

            // Get source and destination paths for display
            let source_display = if let Some(panel) = self.layout_manager.active_panel_mut() {
                if let Some(fm) = panel.as_file_manager_mut() {
                    let vfs_path = fm.vfs_state().current_path();
                    if let Some(source_file) = operation.current_source() {
                        format_vfs_path_for_display(vfs_path, source_file)
                    } else {
                        String::new()
                    }
                } else {
                    operation
                        .current_source()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                }
            } else {
                operation
                    .current_source()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            };

            let dest_display = operation.destination.display().to_string();

            let modal = ProgressModal::new_copy_progress(
                title,
                operation.total_count(),
                source_display,
                dest_display,
                true, // pause_enabled
            );
            self.state.active_modal = Some(ActiveModal::Progress(Box::new(modal)));

            // Store operation as pending action to allow UI to render modal
            // before starting actual file operations
            self.state.pending_action = Some(PendingAction::ContinueBatchOperation { operation });
            return;
        }

        // Check if operation is paused - keep modal open and don't process next file
        if operation.pause_state == termide_state::PauseState::Paused {
            // Store operation and return - will resume when user unpauses
            self.state.pending_action = Some(PendingAction::ContinueBatchOperation { operation });
            return;
        }

        // Check if operation is complete
        if operation.is_complete() {
            // Close progress modal if open
            if matches!(self.state.active_modal, Some(ActiveModal::Progress(_))) {
                self.state.close_modal();
            }

            // Show final results
            self.show_batch_results(&operation);

            // Refresh ALL file manager panels after batch operation
            // (both source and destination might need refresh)
            if operation.success_count > 0 {
                // Get last successful filename for cursor positioning
                let last_filename = operation.last_successful_filename();
                let dest_path = operation.destination_path();

                for group in &mut self.layout_manager.panel_groups {
                    for panel in group.panels_mut() {
                        if let Some(fm) = panel.as_file_manager_mut() {
                            fm.clear_selection();

                            // Set cursor target BEFORE reload for destination panel
                            if fm.current_path() == dest_path {
                                if let Some(ref name) = last_filename {
                                    fm.set_newly_created(name.clone());
                                }
                            }

                            // Force reload by bypassing debounce to ensure file list updates
                            let _ = fm.force_reload_directory();
                        }
                    }
                }
            }
            return;
        }

        // Get current file
        let Some(source) = operation.current_source().cloned() else {
            return;
        };

        let item_name = path_utils::get_file_name_string(&source);

        // Determine target path (considering rename pattern if set)
        let final_dest = if operation.rename_pattern.is_some() {
            // Apply rename pattern
            let counter = operation.get_and_increment_rename_counter();
            let metadata = source.metadata().ok();
            let created = metadata.as_ref().and_then(|m| m.created().ok());
            let modified = metadata.as_ref().and_then(|m| m.modified().ok());

            let pattern = operation.rename_pattern.as_ref().unwrap();
            let new_name = pattern.apply(&item_name, counter, created, modified);

            path_utils::resolve_rename_destination_path(&operation.destination, &new_name)
        } else {
            // Standard logic without renaming
            path_utils::resolve_batch_destination_path(
                &source,
                &operation.destination,
                operation.sources.len() == 1,
            )
        };

        // Update progress modal with current file paths
        if let Some(ActiveModal::Progress(ref mut modal)) = self.state.active_modal {
            // Get source display path
            let source_display = if let Some(panel) = self.layout_manager.active_panel_mut() {
                if let Some(fm) = panel.as_file_manager_mut() {
                    let vfs_path = fm.vfs_state().current_path();
                    format_vfs_path_for_display(vfs_path, &source)
                } else {
                    item_name.clone()
                }
            } else {
                item_name.clone()
            };

            let dest_file = final_dest
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let dest_display = operation.destination.join(dest_file).display().to_string();

            modal.update_progress_with_paths(
                operation.current_index + 1, // 1-based for display
                source_display,
                dest_display,
            );
        }

        // Check conflict
        if final_dest.exists() {
            match operation.conflict_mode {
                ConflictMode::Ask => {
                    // Show conflict resolution modal window
                    let remaining_items = operation
                        .sources
                        .len()
                        .saturating_sub(operation.current_index + 1);
                    let modal = ConflictModal::new(&source, &final_dest, remaining_items);
                    self.state.pending_action =
                        Some(PendingAction::ContinueBatchOperation { operation });
                    self.state.active_modal = Some(ActiveModal::Conflict(Box::new(modal)));
                    return;
                }
                ConflictMode::SkipAll => {
                    // Skip file
                    log::info!("'{}' пропущен (файл существует)", item_name);
                    operation.increment_skipped();
                    operation.advance();
                    // Store and return to allow UI update
                    self.state.pending_action =
                        Some(PendingAction::ContinueBatchOperation { operation });
                    return;
                }
                ConflictMode::OverwriteAll => {
                    // Continue with overwrite (processing below)
                }
            }
        }

        // Execute operation
        if let Some(panel) = self.layout_manager.active_panel_mut() {
            if let Some(fm) = panel.as_file_manager_mut() {
                // Check if source is remote - use OperationManager download
                if fm.is_remote() {
                    use termide_file_ops::OperationRequest;

                    // For remote-to-local copy/move, use VFS download via OperationManager
                    let source_name = source
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let vfs_source = fm.vfs_state().current_path().join(&source_name);
                    let vfs_manager = fm.vfs_state().manager_arc();

                    let is_move = operation.operation_type == BatchOperationType::Move;

                    // Create download request
                    let request = OperationRequest::download(vfs_source.clone(), final_dest);

                    // Store VFS info for move operation (delete source after download)
                    if is_move {
                        self.state.pending_remote_delete = Some(PendingRemoteDelete {
                            vfs_source,
                            vfs_manager: vfs_manager.clone(),
                        });
                    }

                    match self.state.start_operation_now(request, vfs_manager) {
                        Ok(_operation_id) => {
                            self.state.pending_action =
                                Some(PendingAction::ContinueBatchOperation { operation });
                        }
                        Err(e) => {
                            log::error!("Failed to start download operation: {}", e);
                            operation.increment_error();
                            operation.advance();
                            self.state.pending_action =
                                Some(PendingAction::ContinueBatchOperation { operation });
                        }
                    }
                    return;
                }

                // Local file - use OperationManager for async copy with progress
                // Applies to both Copy and Move (move may need copy+delete for cross-filesystem)
                if source.is_file() {
                    use termide_file_ops::{OperationPath, OperationRequest};

                    let is_move = operation.operation_type == BatchOperationType::Move;
                    let request = if is_move {
                        OperationRequest::r#move(
                            vec![OperationPath::Local(source.clone())],
                            OperationPath::Local(final_dest.clone()),
                        )
                    } else {
                        OperationRequest::copy(
                            vec![OperationPath::Local(source.clone())],
                            OperationPath::Local(final_dest.clone()),
                        )
                    };

                    // Get or create VFS manager for operation manager
                    let vfs_manager = std::sync::Arc::new(termide_vfs::VfsManager::new());

                    // Start operation immediately via OperationManager
                    match self.state.start_operation_now(request, vfs_manager) {
                        Ok(_operation_id) => {
                            // Store batch operation as pending action for continuation after copy
                            self.state.pending_action =
                                Some(PendingAction::ContinueBatchOperation { operation });
                        }
                        Err(e) => {
                            log::error!("Failed to start copy operation: {}", e);
                            operation.increment_error();
                            operation.advance();
                            self.state.pending_action =
                                Some(PendingAction::ContinueBatchOperation { operation });
                        }
                    }
                    return;
                }

                // Local directory - use OperationManager (handles scan + copy)
                if source.is_dir() {
                    use termide_file_ops::{OperationPath, OperationRequest};

                    let is_move = operation.operation_type == BatchOperationType::Move;
                    let request = if is_move {
                        OperationRequest::r#move(
                            vec![OperationPath::Local(source.clone())],
                            OperationPath::Local(final_dest),
                        )
                    } else {
                        OperationRequest::copy(
                            vec![OperationPath::Local(source.clone())],
                            OperationPath::Local(final_dest),
                        )
                    };

                    let vfs_manager = std::sync::Arc::new(termide_vfs::VfsManager::new());

                    match self.state.start_operation_now(request, vfs_manager) {
                        Ok(_operation_id) => {
                            self.state.pending_action =
                                Some(PendingAction::ContinueBatchOperation { operation });
                        }
                        Err(e) => {
                            log::error!("Failed to start directory copy operation: {}", e);
                            operation.increment_error();
                            operation.advance();
                            self.state.pending_action =
                                Some(PendingAction::ContinueBatchOperation { operation });
                        }
                    }
                    return;
                }

                // Unknown source type (symlink?) - skip with error
                log::error!("Unsupported source type: {}", source.display());
                operation.increment_error();
            }
        }

        // Move to next file
        operation.advance();

        // Store operation and return to allow UI update between files
        // This enables:
        // 1. Progress bar to update visually
        // 2. User to pause/cancel between files
        // 3. Spinner animation to work
        self.state.pending_action = Some(PendingAction::ContinueBatchOperation { operation });
    }

    /// Show batch operation final results
    pub(in crate::app) fn show_batch_results(&mut self, operation: &BatchOperation) {
        let total = operation.total_count();
        let success = operation.success_count;
        let errors = operation.error_count;
        let skipped = operation.skipped_count;
        let t = i18n::t();

        let action_name = match operation.operation_type {
            BatchOperationType::Copy => (t.batch_result_file_copied(), t.batch_result_copied()),
            BatchOperationType::Move => (t.batch_result_file_moved(), t.batch_result_moved()),
        };

        if total == 1 {
            if success == 1 {
                self.state.set_info(format!("Файл {}", action_name.0));
            } else {
                let error_msg = match operation.operation_type {
                    BatchOperationType::Copy => t.batch_result_error_copy(),
                    BatchOperationType::Move => t.batch_result_error_move(),
                };
                self.state.set_error(error_msg.to_string());
            }
        } else {
            let mut parts = vec![];
            if success > 0 {
                parts.push(format!("{}: {}", action_name.1, success));
            }
            if skipped > 0 {
                parts.push(t.batch_result_skipped_fmt(skipped));
            }
            if errors > 0 {
                parts.push(t.batch_result_errors_fmt(errors));
            }

            self.state.set_info(parts.join(", "));
        }
    }

    /// Handle rename pattern input result
    pub(in crate::app) fn handle_rename_with_pattern(
        &mut self,
        mut operation: BatchOperation,
        original_name: String,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        if let Some(pattern_str) = value.downcast_ref::<String>() {
            use termide_state::RenamePattern;

            let pattern = RenamePattern::new(pattern_str.clone());

            // Check that for single file (Rename)
            // need to get counter and apply pattern once
            if operation.rename_pattern.is_none() {
                // This is Rename (single rename)
                if let Some(source) = operation.current_source().cloned() {
                    let counter = operation.get_and_increment_rename_counter();
                    let metadata = source.metadata().ok();
                    let created = metadata.as_ref().and_then(|m| m.created().ok());
                    let modified = metadata.as_ref().and_then(|m| m.modified().ok());

                    let new_name = pattern.apply(&original_name, counter, created, modified);

                    // Create new destination path with new name
                    let new_dest = path_utils::resolve_rename_destination_path(
                        &operation.destination,
                        &new_name,
                    );

                    // Check that new path doesn't conflict
                    if new_dest.exists() {
                        // Show ConflictModal again
                        let remaining_items = operation
                            .sources
                            .len()
                            .saturating_sub(operation.current_index + 1);
                        let modal = ConflictModal::new(&source, &new_dest, remaining_items);
                        self.state.pending_action =
                            Some(PendingAction::ContinueBatchOperation { operation });
                        self.state.active_modal = Some(ActiveModal::Conflict(Box::new(modal)));
                        return Ok(());
                    }

                    // Execute operation with new name
                    operation.destination = new_dest;
                    // Continue processing the batch operation
                    self.process_batch_operation(operation);
                }
            } else {
                // This is RenameAll - pattern already set in operation,
                // just continue processing
                operation.set_rename_pattern(pattern);
                // Continue processing the batch operation
                self.process_batch_operation(operation);
            }
        }
        Ok(())
    }
}
