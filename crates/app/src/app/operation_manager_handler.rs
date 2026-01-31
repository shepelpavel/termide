//! Operation manager event handler.
//!
//! Handles events from the unified operation manager for file operations.

use termide_file_ops::{OperationEvent, OperationPath, OperationPhase, OperationResult};

use crate::state::{ActiveModal, PendingAction};
use crate::PanelExt;

use super::App;

impl App {
    /// Poll the unified operation manager for events (new file-ops system).
    /// This handles events from the centralized operation manager which will
    /// eventually replace the individual operation handles.
    pub(super) fn poll_operation_manager(&mut self) {
        let events = self.state.poll_operations();
        let mut any_completed = false;
        let mut should_refresh_file_managers = false;

        for event in events {
            match event {
                OperationEvent::Started(id) => {
                    log::info!("Operation {} started", id);
                    self.state.needs_redraw = true;
                }
                OperationEvent::Progress(_id, progress) => {
                    // Update progress modal if active
                    if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                        self.state.active_modal
                    {
                        // Update byte-level progress
                        modal
                            .update_file_progress(progress.bytes_transferred, progress.total_bytes);

                        // Update file count and current item
                        if progress.total_files > 0 {
                            modal.update_progress(
                                progress.files_completed,
                                progress.current_item.clone(),
                            );
                        } else if let Some(ref current) = progress.current_item {
                            modal.update_progress(0, Some(current.clone()));
                        }

                        self.state.needs_redraw = true;
                    }

                    // Check if operation completed via progress phase
                    if matches!(
                        progress.phase,
                        OperationPhase::Completed
                            | OperationPhase::Failed
                            | OperationPhase::Cancelled
                    ) {
                        any_completed = true;
                    }
                }
                OperationEvent::Completed(id, result) => {
                    any_completed = true;

                    // Check if this operation is part of a BatchOperation
                    let has_batch = matches!(
                        self.state.pending_action,
                        Some(PendingAction::ContinueBatchOperation { .. })
                    );

                    match result {
                        OperationResult::Success | OperationResult::SuccessWithPath(_) => {
                            log::info!("Operation {} completed successfully", id);
                            should_refresh_file_managers = true;

                            // Handle remote delete for move operations (delete source after download)
                            if let Some(pending_delete) = self.state.pending_remote_delete.take() {
                                // Start async delete operation (fire and forget)
                                let delete_op = pending_delete
                                    .vfs_manager
                                    .delete(&pending_delete.vfs_source);
                                std::thread::spawn(move || {
                                    if let Err(e) = delete_op.recv() {
                                        log::error!(
                                            "Failed to delete remote source after move: {}",
                                            e
                                        );
                                    }
                                });
                            }

                            // Handle batch upload continuation
                            if let Some(mut batch_upload) = self.state.pending_batch_upload.take() {
                                // Delete local source if this was a move operation
                                if batch_upload.is_move {
                                    if let Err(e) =
                                        std::fs::remove_file(&batch_upload.current_source)
                                    {
                                        log::warn!("Failed to delete source after move: {}", e);
                                    }
                                }

                                // Check if there are more files to upload
                                batch_upload.current_index += 1;
                                if batch_upload.current_index < batch_upload.all_sources.len() {
                                    // Start next file upload
                                    let next_source = batch_upload.all_sources
                                        [batch_upload.current_index]
                                        .clone();
                                    let source_name = next_source
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_else(|| "file".to_string());

                                    // Parse remote base path and join with filename
                                    if let Ok(remote_base) =
                                        termide_vfs::parse_vfs_url(&batch_upload.dest_base_url)
                                    {
                                        let final_remote = remote_base.join(&source_name);

                                        // Update modal progress
                                        if let Some(crate::state::ActiveModal::Progress(
                                            ref mut modal,
                                        )) = self.state.active_modal
                                        {
                                            modal.update_progress(
                                                batch_upload.current_index + 1,
                                                Some(next_source.display().to_string()),
                                            );
                                            modal.update_source_dest(
                                                next_source.display().to_string(),
                                                final_remote.to_url_string(),
                                            );
                                        }

                                        // Create upload request for next file
                                        let request = termide_file_ops::OperationRequest::upload(
                                            next_source.clone(),
                                            final_remote,
                                        );

                                        // Update batch state
                                        batch_upload.current_source = next_source;

                                        // Start upload for next file
                                        match self.state.start_operation_now(
                                            request,
                                            batch_upload.vfs_manager.clone(),
                                        ) {
                                            Ok(_) => {
                                                // Put back for next tick
                                                self.state.pending_batch_upload =
                                                    Some(batch_upload);
                                            }
                                            Err(e) => {
                                                log::error!("Failed to start next upload: {}", e);
                                                self.state.close_modal();
                                                self.state
                                                    .set_error(format!("Upload failed: {}", e));
                                            }
                                        }
                                    } else {
                                        // Failed to parse URL - abort
                                        self.state.close_modal();
                                        self.state
                                            .set_error("Failed to parse remote URL".to_string());
                                    }
                                } else {
                                    // All files uploaded!
                                    self.state.close_modal();
                                    let total = batch_upload.all_sources.len();
                                    if total == 1 {
                                        self.state.set_info("File uploaded".to_string());
                                    } else {
                                        self.state.set_info(format!("{} files uploaded", total));
                                    }
                                }
                            }

                            // Continue batch operation if pending
                            if has_batch {
                                if let Some(PendingAction::ContinueBatchOperation {
                                    mut operation,
                                }) = self.state.pending_action.take()
                                {
                                    operation.increment_success();
                                    operation.advance();
                                    self.process_batch_operation(operation);
                                }
                            }

                            // Skip file manager refresh for editor uploads (file already existed)
                            if self.state.skip_refresh_after_upload {
                                self.state.skip_refresh_after_upload = false;
                                should_refresh_file_managers = false;
                            }

                            // Handle close editor after upload (for "save and close" flow)
                            if self.state.close_editor_after_upload.is_some() {
                                self.state.close_editor_after_upload = None;
                                // Clear uploading flag on editor
                                if let Some(panel) = self.layout_manager.active_panel_mut() {
                                    if let Some(editor) = panel.as_editor_mut() {
                                        editor.set_uploading(false);
                                    }
                                }
                                // Close the editor panel
                                self.close_panel_at_index(0);
                            }
                        }
                        OperationResult::PartialSuccess {
                            completed,
                            skipped,
                            failed,
                            ..
                        } => {
                            log::info!(
                                "Operation {} partially completed: {} done, {} skipped, {} failed",
                                id,
                                completed,
                                skipped,
                                failed
                            );
                            should_refresh_file_managers = true;

                            // Continue batch operation if pending
                            if has_batch {
                                if let Some(PendingAction::ContinueBatchOperation {
                                    mut operation,
                                }) = self.state.pending_action.take()
                                {
                                    // Add completed count to batch
                                    for _ in 0..completed {
                                        operation.increment_success();
                                    }
                                    for _ in 0..skipped {
                                        operation.increment_skipped();
                                    }
                                    for _ in 0..failed {
                                        operation.increment_error();
                                    }
                                    operation.advance();
                                    self.process_batch_operation(operation);
                                }
                            } else if skipped > 0 || failed > 0 {
                                self.state.set_info(format!(
                                    "Operation completed: {} done, {} skipped, {} failed",
                                    completed, skipped, failed
                                ));
                            }
                        }
                        OperationResult::Failed(err) => {
                            log::error!("Operation {} failed: {}", id, err);

                            // Clear pending remote delete (don't delete source if download failed)
                            self.state.pending_remote_delete = None;

                            // Clear editor upload flags on failure
                            self.state.skip_refresh_after_upload = false;
                            if self.state.close_editor_after_upload.is_some() {
                                self.state.close_editor_after_upload = None;
                                // Clear uploading flag on editor
                                if let Some(panel) = self.layout_manager.active_panel_mut() {
                                    if let Some(editor) = panel.as_editor_mut() {
                                        editor.set_uploading(false);
                                    }
                                }
                            }

                            // Clear pending batch upload (don't continue if upload failed)
                            if self.state.pending_batch_upload.take().is_some() {
                                self.state.close_modal();
                            }

                            // Continue batch operation if pending
                            if has_batch {
                                if let Some(PendingAction::ContinueBatchOperation {
                                    mut operation,
                                }) = self.state.pending_action.take()
                                {
                                    operation.increment_error();
                                    operation.advance();
                                    self.process_batch_operation(operation);
                                }
                            } else {
                                self.state.set_error(format!("Operation failed: {}", err));
                            }
                        }
                        OperationResult::Cancelled => {
                            log::info!("Operation {} cancelled", id);

                            // Clear pending remote delete (don't delete source if download cancelled)
                            self.state.pending_remote_delete = None;

                            // Clear editor upload flags on cancel
                            self.state.skip_refresh_after_upload = false;
                            if self.state.close_editor_after_upload.is_some() {
                                self.state.close_editor_after_upload = None;
                                // Clear uploading flag on editor
                                if let Some(panel) = self.layout_manager.active_panel_mut() {
                                    if let Some(editor) = panel.as_editor_mut() {
                                        editor.set_uploading(false);
                                    }
                                }
                            }

                            // Clear pending batch upload (don't continue if upload cancelled)
                            if self.state.pending_batch_upload.take().is_some() {
                                self.state.close_modal();
                                self.state.set_info("Upload cancelled".to_string());
                            }

                            // For batch operations, show cleanup modal
                            if has_batch {
                                if let Some(PendingAction::ContinueBatchOperation { operation }) =
                                    self.state.pending_action.take()
                                {
                                    // Show cleanup modal similar to check_local_copy_progress
                                    let all_dest_paths = operation.completed_destinations.clone();
                                    let buttons = if all_dest_paths.is_empty() {
                                        vec!["OK".to_string()]
                                    } else {
                                        vec!["Delete copied".to_string(), "Keep copied".to_string()]
                                    };
                                    let modal = termide_modal::ChoiceModal::buttons_only(
                                        "Operation Cancelled",
                                        buttons,
                                    );
                                    self.state.active_modal =
                                        Some(ActiveModal::Choice(Box::new(modal)));
                                    self.state.pending_action =
                                        Some(PendingAction::CancelCopyCleanup {
                                            partial_path: std::path::PathBuf::new(),
                                            all_dest_paths,
                                            is_directory: false,
                                            batch_operation: Some(Box::new(operation)),
                                        });
                                }
                            } else {
                                self.state.set_info("Operation cancelled".to_string());
                            }
                        }
                    }
                }
                OperationEvent::Paused(id) => {
                    log::info!("Operation {} paused", id);
                    // Sync pause state with modal
                    if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                        self.state.active_modal
                    {
                        modal.set_paused(true);
                        self.state.needs_redraw = true;
                    }
                }
                OperationEvent::Resumed(id) => {
                    log::info!("Operation {} resumed", id);
                    // Sync resume state with modal
                    if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                        self.state.active_modal
                    {
                        modal.set_paused(false);
                        self.state.needs_redraw = true;
                    }
                }
                OperationEvent::ConflictDetected(id, conflict_info) => {
                    log::info!(
                        "Operation {} conflict: {} -> {}",
                        id,
                        conflict_info.source.display(),
                        conflict_info.destination.display()
                    );

                    // Convert OperationPath to PathBuf for ConflictModal
                    let source_path = match &conflict_info.source {
                        OperationPath::Local(p) => p.clone(),
                        OperationPath::Remote(vfs_path) => vfs_path.path.clone(),
                    };
                    let dest_path = match &conflict_info.destination {
                        OperationPath::Local(p) => p.clone(),
                        OperationPath::Remote(vfs_path) => vfs_path.path.clone(),
                    };

                    // Show ConflictModal
                    let modal = termide_modal::ConflictModal::new(
                        &source_path,
                        &dest_path,
                        conflict_info.remaining_items,
                    );
                    self.state.set_pending_action(
                        PendingAction::ResolveOperationConflict { operation_id: id },
                        ActiveModal::Conflict(Box::new(modal)),
                    );
                    self.state.needs_redraw = true;
                }
            }
        }

        // Close progress modal if all operations are complete
        if any_completed && !self.state.has_pending_operations() {
            // Only close if we have a progress modal open for OperationManager operations
            // (not for legacy operations which have their own check_* methods)
            if matches!(
                self.state.active_modal,
                Some(crate::state::ActiveModal::Progress(_))
            ) {
                // Check if this modal is associated with a pending batch operation
                // If so, don't close it here - let the batch handler manage it
                let has_batch_pending = matches!(
                    self.state.pending_action,
                    Some(termide_state::PendingAction::ContinueBatchOperation { .. })
                        | Some(termide_state::PendingAction::BatchFileOperation { .. })
                );

                if !has_batch_pending {
                    self.state.close_modal();
                }
            }
        }

        // Refresh file managers after successful operations
        if should_refresh_file_managers {
            for panel in self.layout_manager.iter_all_panels_mut() {
                if let Some(fm) = panel.as_file_manager_mut() {
                    fm.clear_selection();
                    let _ = fm.load_directory();
                }
            }
        }
    }
}
