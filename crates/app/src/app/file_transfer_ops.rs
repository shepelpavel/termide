//! Remote file transfer operation handlers.
//!
//! Contains handlers for remote file system operations:
//! - Download operations (remote → local)
//! - Upload operations (local → remote)
//! - Batch upload/download operations

#![allow(deprecated)]

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use termide_panel_editor::{Editor, FileState};

use crate::state::PendingAction;
use crate::PanelExt;

use super::App;

impl App {
    /// Check download operation result (remote file download)
    pub(super) fn check_download_operation_result(&mut self) {
        let download = match self.state.download_operation.take() {
            Some(d) => d,
            None => return,
        };

        match download.operation.try_recv() {
            Some(Ok(_)) => {
                // Download complete!
                self.state.close_modal();

                // Get metadata from downloaded temp file
                let (size, mtime) = match std::fs::metadata(&download.temp_path) {
                    Ok(meta) => (meta.len(), meta.modified().ok()),
                    Err(_) => (0, None),
                };

                // Open editor with temp file and mark as remote
                match Editor::open_file_with_config(download.temp_path.clone(), download.config) {
                    Ok(mut editor) => {
                        // Set remote file state
                        editor.set_file_state(FileState::from_remote(
                            download.remote_path.clone(),
                            download.temp_path,
                            mtime,
                            size,
                        ));

                        // Store VfsManager for saves
                        editor.set_vfs_manager(download.vfs_manager);

                        // Initialize LSP
                        if let Some(lsp) = &mut self.state.lsp_manager {
                            editor.init_lsp(lsp);
                        }

                        self.add_panel(Box::new(editor));
                        self.auto_save_session();

                        let filename = download
                            .remote_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("remote file");
                        log::info!("Remote file '{}' opened in editor", filename);
                        self.state.set_info(format!("File {} opened", filename));
                    }
                    Err(e) => {
                        let error_msg = format!("Failed to open downloaded file: {}", e);
                        log::error!("{}", error_msg);
                        self.state.set_error(error_msg);
                        // Clean up temp file
                        let _ = std::fs::remove_file(&download.temp_path);
                    }
                }
            }
            Some(Err(e)) => {
                // Download failed
                self.state.close_modal();
                let error_msg = format!("Download failed: {}", e);
                log::error!("{}", error_msg);
                self.state.set_error(error_msg);
                // Clean up temp file
                let _ = std::fs::remove_file(&download.temp_path);
            }
            None => {
                // Still downloading - check timeout
                if download.started.elapsed().as_secs() > 120 {
                    self.state.close_modal();
                    log::error!("Download timeout (120s)");
                    self.state.set_error("Download timeout (120s)".to_string());
                    // Clean up temp file
                    let _ = std::fs::remove_file(&download.temp_path);
                } else {
                    // Put back for next tick
                    self.state.download_operation = Some(download);
                }
            }
        }
    }

    /// Handle pending upload operation from Editor (regular Ctrl+S save of remote file)
    pub(super) fn handle_pending_upload(
        &mut self,
        temp_path: PathBuf,
        remote_path: termide_vfs::VfsPath,
        vfs_manager: std::sync::Arc<termide_vfs::VfsManager>,
    ) {
        // Show progress modal
        let filename = remote_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());

        let modal = termide_modal::ProgressModal::indeterminate(
            "Uploading File",
            format!("Uploading {}...", filename),
        );
        self.state.active_modal = Some(crate::state::ActiveModal::Progress(Box::new(modal)));

        // Set uploading flag on active editor
        if let Some(panel) = self.layout_manager.active_panel_mut() {
            if let Some(editor) = panel.as_editor_mut() {
                editor.set_uploading(true);
            }
        }

        // Create upload request via OperationManager
        let request = termide_file_ops::OperationRequest::upload(temp_path, remote_path);

        // Start upload via OperationManager
        match self.state.start_operation_now(request, vfs_manager) {
            Ok(_operation_id) => {
                log::info!("Started editor upload operation");
            }
            Err(e) => {
                log::error!("Failed to start upload operation: {}", e);
                self.state.close_modal();
                self.state.set_error(format!("Upload failed: {}", e));
                // Clear uploading flag
                if let Some(panel) = self.layout_manager.active_panel_mut() {
                    if let Some(editor) = panel.as_editor_mut() {
                        editor.set_uploading(false);
                    }
                }
            }
        }
    }

    /// Check upload operation result (remote file upload)
    pub(super) fn check_upload_operation_result(&mut self) {
        let upload = match self.state.upload_operation.take() {
            Some(u) => u,
            None => return,
        };

        // Non-blocking poll
        match upload.operation.try_recv() {
            Some(Ok(_)) => {
                // Upload complete!
                self.state.close_modal();

                // Update editor mtime to prevent "changed on disk" warning
                // Note: We use active_panel since the editor should still be active
                // (upload happens during save-before-close which doesn't close until upload completes)
                if let Some(panel) = self.layout_manager.active_panel_mut() {
                    if let Some(editor) = panel.as_editor_mut() {
                        // Update editor mtime from temp file
                        if let Ok(meta) = std::fs::metadata(&upload.temp_path) {
                            if let Ok(mtime) = meta.modified() {
                                editor.update_file_mtime(Some(mtime));
                            }
                        }
                        editor.clear_external_change_detected();
                        editor.set_uploading(false);
                    }
                }

                let filename = upload
                    .remote_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "file".to_string());
                log::info!("Remote file '{}' uploaded successfully", filename);
                self.state.set_info(format!("File {} uploaded", filename));

                // Close editor if this was a "save and close" operation
                if upload.close_after_upload {
                    self.close_panel_at_index(0);
                }
            }
            Some(Err(e)) => {
                // Upload failed
                self.state.close_modal();
                // Clear uploading flag on error
                if let Some(panel) = self.layout_manager.active_panel_mut() {
                    if let Some(editor) = panel.as_editor_mut() {
                        editor.set_uploading(false);
                    }
                }
                let error_msg = format!("Upload failed: {}", e);
                log::error!("{}", error_msg);
                self.state.set_error(error_msg);
            }
            None => {
                // Still uploading - check timeout
                if upload.started.elapsed().as_secs() > 120 {
                    self.state.close_modal();
                    // Clear uploading flag on timeout
                    if let Some(panel) = self.layout_manager.active_panel_mut() {
                        if let Some(editor) = panel.as_editor_mut() {
                            editor.set_uploading(false);
                        }
                    }
                    log::error!("Upload timeout (120s)");
                    self.state.set_error("Upload timeout (120s)".to_string());
                } else {
                    // Still uploading - spinner updated by update_modal_spinners()
                    // Put back for next tick
                    self.state.upload_operation = Some(upload);
                }
            }
        }
    }

    /// Check batch upload operation result (local→remote batch copy)
    pub(super) fn check_batch_upload_result(&mut self) {
        let mut upload = match self.state.batch_upload_operation.take() {
            Some(u) => u,
            None => return,
        };

        // Check for progress updates and update modal
        if let Some(progress) = upload.operation.drain_progress() {
            if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                self.state.active_modal
            {
                modal.update_file_progress(progress.bytes_uploaded, progress.total_bytes);
            }
        }

        // Non-blocking poll for completion
        match upload.operation.try_recv() {
            Some(Ok(_)) => {
                // Current file upload complete!
                let filename = std::path::Path::new(&upload.dest_url)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "file".to_string());

                log::info!("File '{}' uploaded successfully", filename);

                // If this was a move operation, delete the local source
                if upload.is_move {
                    if let Err(e) = std::fs::remove_file(&upload.source_path) {
                        log::warn!("Failed to delete source after move: {}", e);
                    }
                }

                // Check if there are more files to upload
                upload.current_index += 1;
                if upload.current_index < upload.all_sources.len() {
                    // Start next file upload
                    let next_source = &upload.all_sources[upload.current_index];
                    let source_name = next_source
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "file".to_string());

                    // Parse remote base path and join with filename
                    if let Ok(remote_base) = termide_vfs::parse_vfs_url(&upload.dest_base_url) {
                        let final_remote = remote_base.join(&source_name);
                        let total_bytes =
                            std::fs::metadata(next_source).map(|m| m.len()).unwrap_or(0);

                        // Update modal progress
                        if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                            self.state.active_modal
                        {
                            modal.update_progress(
                                upload.current_index + 1,
                                Some(next_source.display().to_string()),
                            );
                            modal.update_source_dest(
                                next_source.display().to_string(),
                                final_remote.to_url_string(),
                            );
                            // Reset file progress for new file
                            modal.update_file_progress(0, total_bytes);
                        }

                        // Start upload for next file
                        let upload_op = upload
                            .vfs_manager
                            .upload_with_progress(next_source, &final_remote);

                        // Update upload state
                        upload.operation = upload_op;
                        upload.source_path = next_source.clone();
                        upload.dest_url = final_remote.to_url_string();
                        upload.total_bytes = total_bytes;
                        upload.started = std::time::Instant::now();

                        // Put back for next tick
                        self.state.batch_upload_operation = Some(upload);
                    } else {
                        // Failed to parse URL - abort
                        self.state.close_modal();
                        self.state
                            .set_error("Failed to parse remote URL".to_string());
                    }
                } else {
                    // All files uploaded!
                    self.state.close_modal();
                    let total = upload.all_sources.len();
                    if total == 1 {
                        self.state.set_info(format!("File {} uploaded", filename));
                    } else {
                        self.state.set_info(format!("{} files uploaded", total));
                    }

                    // Refresh file manager panels that show the destination directory
                    if let Ok(dest_path) = termide_vfs::parse_vfs_url(&upload.dest_url) {
                        if let Some(parent) = dest_path.parent() {
                            for group in &mut self.layout_manager.panel_groups {
                                for panel in group.panels_mut() {
                                    if let Some(fm) = panel.as_file_manager_mut() {
                                        if fm.is_remote() {
                                            let fm_path = fm.vfs_state().current_path();
                                            if fm_path.connection_key() == parent.connection_key()
                                                && fm_path.path == parent.path
                                            {
                                                let _ = fm.reload_directory();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Some(Err(e)) => {
                // Upload failed - log error and continue with next file
                log::error!("Upload failed for {}: {}", upload.source_path.display(), e);

                upload.current_index += 1;
                if upload.current_index < upload.all_sources.len() {
                    // Try next file
                    let next_source = &upload.all_sources[upload.current_index];
                    let source_name = next_source
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "file".to_string());

                    if let Ok(remote_base) = termide_vfs::parse_vfs_url(&upload.dest_base_url) {
                        let final_remote = remote_base.join(&source_name);
                        let total_bytes =
                            std::fs::metadata(next_source).map(|m| m.len()).unwrap_or(0);

                        // Update modal
                        if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                            self.state.active_modal
                        {
                            modal.update_progress(
                                upload.current_index + 1,
                                Some(next_source.display().to_string()),
                            );
                            modal.update_source_dest(
                                next_source.display().to_string(),
                                final_remote.to_url_string(),
                            );
                            modal.update_file_progress(0, total_bytes);
                        }

                        let upload_op = upload
                            .vfs_manager
                            .upload_with_progress(next_source, &final_remote);
                        upload.operation = upload_op;
                        upload.source_path = next_source.clone();
                        upload.dest_url = final_remote.to_url_string();
                        upload.total_bytes = total_bytes;
                        upload.started = std::time::Instant::now();
                        self.state.batch_upload_operation = Some(upload);
                    } else {
                        self.state.close_modal();
                        self.state.set_error(format!("Upload failed: {}", e));
                    }
                } else {
                    // No more files - show error
                    self.state.close_modal();
                    self.state.set_error(format!("Upload failed: {}", e));
                }
            }
            None => {
                // Still uploading - check timeout
                if upload.started.elapsed().as_secs() > 300 {
                    // 5 minute timeout for file upload
                    self.state.close_modal();
                    log::error!("Upload timeout (5 min)");
                    self.state.set_error("Upload timeout (5 min)".to_string());
                } else {
                    // Still uploading - put back for next tick
                    self.state.batch_upload_operation = Some(upload);
                }
            }
        }
    }

    /// Check batch download operation result (remote→local file copy/move during batch operations)
    pub(super) fn check_batch_download_result(&mut self) {
        let mut download = match self.state.batch_download_operation.take() {
            Some(d) => d,
            None => return,
        };

        // Sync pause state: BatchOperation -> download operation
        if let Some(PendingAction::ContinueBatchOperation { ref operation }) =
            self.state.pending_action
        {
            let should_pause = operation.pause_state == termide_state::PauseState::Paused;
            download
                .operation
                .pause_flag
                .store(should_pause, Ordering::Relaxed);
        }

        // Poll progress updates (drain all available progress messages)
        if let Some(progress) = download.operation.drain_progress() {
            // Update last known totals for this item
            download.last_total_files = progress.total_files;
            download.last_total_bytes = progress.total_bytes;

            if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                self.state.active_modal
            {
                // Get cumulative values from batch operation
                let (cumulative_files, cumulative_bytes) =
                    if let Some(PendingAction::ContinueBatchOperation { ref operation }) =
                        self.state.pending_action
                    {
                        (
                            operation.cumulative_files_completed,
                            operation.cumulative_bytes_completed,
                        )
                    } else {
                        (0, 0)
                    };

                // Use update_directory_copy_progress with cumulative + current values
                modal.update_directory_copy_progress(
                    cumulative_files + progress.files_downloaded,
                    cumulative_files + progress.total_files, // Approximate total (will grow as we process more items)
                    cumulative_bytes + progress.bytes_downloaded,
                    cumulative_bytes + progress.total_bytes, // Approximate total
                );
                // Update individual file progress for chunked downloads (progress bar)
                modal.update_individual_file_progress(
                    progress.current_file_bytes,
                    progress.current_file_total,
                );
                // Also update current item being downloaded
                if let Some(ref file) = progress.current_file {
                    modal.update_progress(
                        cumulative_files + progress.files_downloaded,
                        Some(file.clone()),
                    );
                }
                self.state.needs_redraw = true;
            }
        }

        match download.operation.try_recv() {
            Some(Ok(_)) => {
                // Download complete - for Move, delete the source file on remote
                if download.is_move {
                    if let (Some(vfs_source), Some(vfs_manager)) =
                        (&download.vfs_source, &download.vfs_manager)
                    {
                        // Start async delete operation (fire and forget for now)
                        let delete_op = vfs_manager.delete(vfs_source);
                        // Spawn thread to wait for delete result and log error if any
                        std::thread::spawn(move || {
                            if let Err(e) = delete_op.recv() {
                                log::error!("Failed to delete remote source after move: {}", e);
                            }
                        });
                    }
                }

                // Continue batch operation
                if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                    self.state.pending_action.take()
                {
                    operation.success_count += 1;

                    // Update cumulative counters with completed item's totals
                    operation.cumulative_files_completed += download.last_total_files;
                    operation.cumulative_bytes_completed += download.last_total_bytes;

                    // Update progress modal
                    if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                        self.state.active_modal
                    {
                        modal.update_progress(
                            operation.current_index + 1,
                            Some(download.dest_path.display().to_string()),
                        );
                    }

                    operation.current_index += 1;
                    self.process_batch_operation(operation);
                }
            }
            Some(Err(e)) => {
                // Download failed - record error and continue
                if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                    self.state.pending_action.take()
                {
                    operation.error_count += 1;
                    log::error!(
                        "Batch download failed for {}: {}",
                        download.dest_path.display(),
                        e
                    );

                    // Still update cumulative counters for the failed item
                    operation.cumulative_files_completed += download.last_total_files;
                    operation.cumulative_bytes_completed += download.last_total_bytes;

                    operation.current_index += 1;
                    self.process_batch_operation(operation);
                }
            }
            None => {
                // Still downloading - check timeout (5 minutes for potentially large directories)
                if download.started.elapsed().as_secs() > 300 {
                    // Timeout - record error and continue
                    if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                        self.state.pending_action.take()
                    {
                        operation.error_count += 1;
                        log::error!(
                            "Batch download timeout for {}",
                            download.dest_path.display()
                        );

                        // Update cumulative counters even for timeout
                        operation.cumulative_files_completed += download.last_total_files;
                        operation.cumulative_bytes_completed += download.last_total_bytes;

                        operation.current_index += 1;
                        self.process_batch_operation(operation);
                    }
                } else {
                    // Put back for next tick
                    self.state.batch_download_operation = Some(download);
                }
            }
        }
    }
}
