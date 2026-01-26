//! Local file operation handlers.
//!
//! Contains handlers for local file system operations:
//! - File copy progress
//! - Directory copy progress
//! - Directory scan progress
//! - Delete progress
//! - Batch operation scheduling

#![allow(deprecated)]

use std::sync::atomic::Ordering;

use termide_modal::{ActiveModal, ChoiceModal};

use crate::state::PendingAction;
use crate::PanelExt;

use super::App;

impl App {
    /// Check and update progress for ongoing local file copy operation
    pub(super) fn check_local_copy_progress(&mut self) {
        let copy_op = match self.state.local_copy_operation.take() {
            Some(op) => op,
            None => return,
        };

        // Sync pause state: BatchOperation -> CopyOperation
        if let Some(PendingAction::ContinueBatchOperation { ref operation }) =
            self.state.pending_action
        {
            let should_pause = operation.pause_state == termide_state::PauseState::Paused;
            copy_op.pause_flag.store(should_pause, Ordering::Relaxed);
        }

        // Poll progress updates (drain all available progress messages)
        while let Ok(progress) = copy_op.progress.try_recv() {
            if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                self.state.active_modal
            {
                modal.update_file_progress(progress.bytes_copied, progress.total_bytes);
                self.state.needs_redraw = true;
            }
        }

        // Poll completion status
        match copy_op.completion.try_recv() {
            Ok(Ok(_)) => {
                // Copy complete - for Move, delete the source file
                if copy_op.is_move {
                    if let Err(e) = std::fs::remove_file(&copy_op.source_path) {
                        log::error!(
                            "Failed to delete source after move: {}: {}",
                            copy_op.source_path.display(),
                            e
                        );
                    }
                }

                // Continue batch operation
                if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                    self.state.pending_action.take()
                {
                    // Track completed destination for cleanup if operation is cancelled later
                    operation.add_completed_destination(copy_op.dest_path.clone());
                    operation.increment_success();
                    operation.advance();
                    self.process_batch_operation(operation);
                }
            }
            Ok(Err(e)) => {
                // Check if this is a cancellation error
                let error_msg = e.to_string();
                let is_cancellation = error_msg.contains("cancelled by user");

                if is_cancellation {
                    // User cancelled - show modal with cleanup options

                    // Extract batch operation info before setting new pending action
                    let (all_dest_paths, batch_operation) = self
                        .state
                        .pending_action
                        .take()
                        .and_then(|action| {
                            if let PendingAction::ContinueBatchOperation { operation } = action {
                                Some((
                                    operation.completed_destinations.clone(),
                                    Some(Box::new(operation)),
                                ))
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();

                    // Show different options based on whether there are completed files
                    let buttons = if all_dest_paths.is_empty() {
                        // Single file - only two options
                        vec!["Delete".to_string(), "Keep".to_string()]
                    } else {
                        // Multiple files - three options
                        vec![
                            "Delete partial".to_string(),
                            "Delete all".to_string(),
                            "Keep all".to_string(),
                        ]
                    };
                    let modal = ChoiceModal::buttons_only("Operation Cancelled", buttons);
                    self.state.active_modal = Some(ActiveModal::Choice(Box::new(modal)));
                    self.state.pending_action = Some(PendingAction::CancelCopyCleanup {
                        partial_path: copy_op.dest_path.clone(),
                        all_dest_paths,
                        is_directory: false,
                        batch_operation,
                    });
                } else {
                    // Other error - record and continue
                    if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                        self.state.pending_action.take()
                    {
                        operation.increment_error();
                        log::error!(
                            "File copy failed for {}: {}",
                            copy_op.dest_path.display(),
                            e
                        );
                        operation.advance();
                        self.process_batch_operation(operation);
                    }
                }
            }
            Err(_) => {
                // Still copying - put back for next tick
                self.state.local_copy_operation = Some(copy_op);
            }
        }
    }

    /// Check and update progress for ongoing local directory copy operation
    pub(super) fn check_local_directory_copy_progress(&mut self) {
        let mut copy_op = match self.state.local_directory_copy_operation.take() {
            Some(op) => op,
            None => return,
        };

        // Sync pause state: BatchOperation -> DirectoryCopyOperation
        if let Some(PendingAction::ContinueBatchOperation { ref operation }) =
            self.state.pending_action
        {
            let should_pause = operation.pause_state == termide_state::PauseState::Paused;
            copy_op.pause_flag.store(should_pause, Ordering::Relaxed);
        }

        // Poll progress updates (drain all available progress messages)
        while let Ok(progress) = copy_op.progress.try_recv() {
            // Track current file being copied (for cleanup on cancel)
            copy_op.current_file = Some(progress.current_file.clone());

            if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                self.state.active_modal
            {
                modal.update_directory_copy_progress(
                    progress.files_completed,
                    progress.total_files,
                    progress.bytes_copied,
                    progress.total_bytes,
                );
                self.state.needs_redraw = true;
            }
        }

        // Poll completion status
        match copy_op.completion.try_recv() {
            Ok(Ok(_)) => {
                // Copy complete - for Move, delete the source directory
                if copy_op.is_move {
                    if let Err(e) = std::fs::remove_dir_all(&copy_op.source_path) {
                        log::error!(
                            "Failed to delete source directory after move: {}: {}",
                            copy_op.source_path.display(),
                            e
                        );
                    }
                }

                // Continue batch operation
                if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                    self.state.pending_action.take()
                {
                    // Track completed destination for cleanup if operation is cancelled later
                    operation.add_completed_destination(copy_op.dest_path.clone());
                    operation.increment_success();
                    operation.advance();
                    self.process_batch_operation(operation);
                }
            }
            Ok(Err(e)) => {
                // Check if this is a cancellation error
                let error_msg = e.to_string();
                let is_cancellation = error_msg.contains("cancelled by user");

                if is_cancellation {
                    // User cancelled directory copy - show 3 cleanup options

                    // Extract batch operation info
                    let batch_operation = self.state.pending_action.take().and_then(|action| {
                        if let PendingAction::ContinueBatchOperation { operation } = action {
                            Some(Box::new(operation))
                        } else {
                            None
                        }
                    });

                    // For directory copy: always show 3 options
                    // 0 = Keep all (keep everything as is)
                    // 1 = Delete partial (only the interrupted file)
                    // 2 = Delete all (entire destination directory)
                    let buttons = vec![
                        "Keep all".to_string(),
                        "Delete partial".to_string(),
                        "Delete all".to_string(),
                    ];
                    let modal = ChoiceModal::buttons_only("Operation Cancelled", buttons);
                    self.state.active_modal = Some(ActiveModal::Choice(Box::new(modal)));
                    self.state.pending_action = Some(PendingAction::CancelCopyCleanup {
                        partial_path: copy_op.current_file.unwrap_or_default(), // The file being copied
                        all_dest_paths: vec![copy_op.dest_path.clone()], // The destination directory
                        is_directory: true,
                        batch_operation,
                    });
                } else {
                    // Other error - record and continue
                    if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                        self.state.pending_action.take()
                    {
                        operation.increment_error();
                        log::error!(
                            "Directory copy failed for {}: {}",
                            copy_op.dest_path.display(),
                            e
                        );
                        operation.advance();
                        self.process_batch_operation(operation);
                    }
                }
            }
            Err(_) => {
                // Still copying - put back for next tick
                self.state.local_directory_copy_operation = Some(copy_op);
            }
        }
    }

    /// Check and update progress for ongoing directory scan operation
    pub(super) fn check_local_scan_progress(&mut self) {
        let scan_op = match self.state.local_scan_operation.take() {
            Some(op) => op,
            None => return,
        };

        // Poll progress updates (drain all available progress messages)
        while let Ok(progress) = scan_op.progress.try_recv() {
            if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                self.state.active_modal
            {
                let current_dir = if !progress.current_dir.as_os_str().is_empty() {
                    Some(progress.current_dir.display().to_string())
                } else {
                    None
                };
                modal.update_scan_progress(progress.files_count, progress.total_bytes, current_dir);
                self.state.needs_redraw = true;
            }
        }

        // Poll completion status
        match scan_op.completion.try_recv() {
            Ok(Ok(scan_result)) => {
                // Scan complete - start the actual directory copy
                log::info!(
                    "Directory scan complete: {} files, {} bytes",
                    scan_result.files.len(),
                    scan_result.total_bytes
                );

                // Check if this is a move operation
                let is_move = scan_op
                    .batch_operation
                    .as_ref()
                    .map(|op| op.operation_type == termide_state::BatchOperationType::Move)
                    .unwrap_or(false);

                // Transition modal from scanning to copying mode
                let title = if is_move { "Move" } else { "Copy" };
                if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                    self.state.active_modal
                {
                    modal.finish_scanning(
                        scan_result.files.len(),
                        scan_result.total_bytes,
                        scan_op.dest_path.display().to_string(),
                        title,
                    );
                }

                // Start the actual directory copy with scan results
                match termide_panel_file_manager::copy_directory_with_progress(
                    &scan_op.source_path,
                    &scan_op.dest_path,
                ) {
                    Ok(copy_op) => {
                        // Store copy operation for async handling
                        self.state.local_directory_copy_operation =
                            Some(crate::state::LocalDirectoryCopyOperation {
                                completion: copy_op.completion,
                                progress: copy_op.progress,
                                source_path: scan_op.source_path.clone(),
                                dest_path: scan_op.dest_path.clone(),
                                is_move,
                                pause_flag: copy_op.pause_flag,
                                cancel_flag: copy_op.cancel_flag,
                                current_file: None,
                            });

                        // Restore batch operation as pending action
                        if let Some(operation) = scan_op.batch_operation {
                            self.state.pending_action =
                                Some(crate::state::PendingAction::ContinueBatchOperation {
                                    operation: *operation,
                                });
                        }
                    }
                    Err(e) => {
                        // Copy failed to start - show error and continue batch
                        log::error!("Failed to start directory copy: {}", e);
                        if let Some(mut operation) = scan_op.batch_operation {
                            operation.increment_error();
                            operation.advance();
                            self.state.close_modal();
                            self.process_batch_operation(*operation);
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                // Scan failed or was cancelled
                let error_msg = e.to_string();
                let is_cancellation = error_msg.contains("cancelled");

                if is_cancellation {
                    // User cancelled - close modal and show status
                    self.state.close_modal();
                    self.state.set_info("Directory scan cancelled".to_string());

                    // Continue batch operation without this directory
                    if let Some(mut operation) = scan_op.batch_operation {
                        operation.increment_skipped();
                        operation.advance();
                        self.process_batch_operation(*operation);
                    }
                } else {
                    // Other error - record and continue
                    log::error!("Directory scan failed: {}", e);
                    if let Some(mut operation) = scan_op.batch_operation {
                        operation.increment_error();
                        operation.advance();
                        self.state.close_modal();
                        self.process_batch_operation(*operation);
                    }
                }
            }
            Err(_) => {
                // Still scanning - put back for next tick
                self.state.local_scan_operation = Some(scan_op);
            }
        }
    }

    /// Check if there's a pending local batch operation that needs to start
    /// (after progress modal has been rendered)
    pub(super) fn check_pending_batch_operation(&mut self) {
        use crate::state::ActiveModal;

        // Don't start new operation if background copy/download/scan is already in progress
        if self.state.local_copy_operation.is_some()
            || self.state.local_directory_copy_operation.is_some()
            || self.state.local_scan_operation.is_some()
            || self.state.batch_download_operation.is_some()
        {
            return;
        }

        // Check if we have a pending batch operation with progress modal open
        if let Some(ActiveModal::Progress(_)) = &self.state.active_modal {
            if let Some(PendingAction::ContinueBatchOperation { operation }) =
                self.state.pending_action.take()
            {
                // Modal has been rendered, now start the actual batch operation
                self.process_batch_operation(operation);
            }
        }
    }

    /// Check and update progress for ongoing local delete operation
    pub(super) fn check_delete_progress(&mut self) {
        let delete_op = match self.state.local_delete_operation.take() {
            Some(op) => op,
            None => return,
        };

        // Poll progress updates (drain all available progress messages)
        while let Ok(progress) = delete_op.progress.try_recv() {
            if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                self.state.active_modal
            {
                modal.update_delete_progress(progress.files_deleted, progress.total_files);
                self.state.needs_redraw = true;
            }
        }

        // Poll completion status
        match delete_op.completion.try_recv() {
            Ok(Ok(_)) => {
                // Delete complete - close modal and refresh FileManager
                self.state.close_modal();

                // Refresh FileManager and clear selection
                if let Some(panel) = self.layout_manager.active_panel_mut() {
                    if let Some(fm) = panel.as_file_manager_mut() {
                        fm.clear_selection();
                        let _ = fm.load_directory();
                    }
                }

                let t = termide_i18n::t();
                self.state.set_info(t.status_item_deleted().to_string());
                log::info!("Delete operation completed successfully");
            }
            Ok(Err(e)) => {
                // Delete failed or cancelled
                self.state.close_modal();

                // Refresh FileManager anyway (partial deletion may have occurred)
                if let Some(panel) = self.layout_manager.active_panel_mut() {
                    if let Some(fm) = panel.as_file_manager_mut() {
                        fm.clear_selection();
                        let _ = fm.load_directory();
                    }
                }

                let error_msg = e.to_string();
                if error_msg.contains("cancelled") {
                    self.state.set_info("Delete cancelled".to_string());
                    log::info!("Delete operation cancelled by user");
                } else {
                    self.state.set_error(format!("Delete failed: {}", e));
                    log::error!("Delete operation failed: {}", e);
                }
            }
            Err(_) => {
                // Still deleting - put back for next tick
                self.state.local_delete_operation = Some(delete_op);
            }
        }
    }
}
