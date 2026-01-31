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

/// Calculate total size of all sources (files and directories recursively).
fn scan_sources_total_bytes(sources: &[PathBuf]) -> u64 {
    fn dir_size(path: &Path) -> u64 {
        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if meta.is_dir() {
                    total += dir_size(&entry.path());
                } else {
                    total += meta.len();
                }
            }
        }
        total
    }

    let mut total = 0u64;
    for source in sources {
        match std::fs::metadata(source) {
            Ok(meta) if meta.is_dir() => total += dir_size(source),
            Ok(meta) => total += meta.len(),
            Err(_) => {}
        }
    }
    total
}

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
    /// Start a sub-operation within a batch and store continuation state.
    ///
    /// On success, stores the operation ID and sets pending action for continuation.
    /// On failure, logs the error, increments error count, advances to next file,
    /// and still stores the pending action for continuation.
    fn start_batch_sub_operation(
        &mut self,
        request: termide_file_ops::OperationRequest,
        vfs_manager: std::sync::Arc<termide_vfs::VfsManager>,
        mut operation: BatchOperation,
    ) {
        // Extract tracking info before starting (request is consumed)
        let source_display = request
            .sources
            .first()
            .map(|s| s.display())
            .unwrap_or_default();
        let dest_display = request
            .destination
            .as_ref()
            .map(|d| d.display())
            .unwrap_or_default();
        let op_type = Self::tracking_op_type(&request);

        match self.state.start_operation_now(request, vfs_manager) {
            Ok(op_id) => {
                self.state.batch_sub_operation_id = Some(op_id);

                // If no batch tracking card exists, create one and open the panel.
                // Use start_batch_tracking() to get a synthetic ID so that
                // untrack_operation(real_id) on sub-op completion won't remove it.
                if self.state.batch_tracking_id.is_none() {
                    let batch_id = self.state.start_batch_tracking(
                        op_type,
                        source_display,
                        dest_display,
                        1,
                        0,
                    );
                    let _ = self.open_operations_panel_with_focus(batch_id);
                }

                self.state.pending_action =
                    Some(PendingAction::ContinueBatchOperation { operation });
            }
            Err(e) => {
                log::error!("Failed to start operation: {}", e);
                operation.increment_error();
                operation.advance();
                self.state.pending_action =
                    Some(PendingAction::ContinueBatchOperation { operation });
            }
        }
    }

    /// Map an OperationRequest to a tracking OperationType.
    fn tracking_op_type(
        request: &termide_file_ops::OperationRequest,
    ) -> crate::state::OperationType {
        use termide_file_ops::OperationType as FO;
        let is_remote_src = request
            .sources
            .first()
            .map(|s| s.is_remote())
            .unwrap_or(false);
        let is_remote_dst = request
            .destination
            .as_ref()
            .map(|d| d.is_remote())
            .unwrap_or(false);

        match request.op_type {
            FO::Copy | FO::Move if is_remote_src && !is_remote_dst => {
                if request.is_move {
                    crate::state::OperationType::MoveDownload
                } else {
                    crate::state::OperationType::CopyDownload
                }
            }
            FO::Copy | FO::Move if !is_remote_src && is_remote_dst => {
                if request.is_move {
                    crate::state::OperationType::MoveUpload
                } else {
                    crate::state::OperationType::CopyUpload
                }
            }
            FO::Copy | FO::Move if is_remote_src && is_remote_dst => {
                if request.is_move {
                    crate::state::OperationType::MoveUpload
                } else {
                    crate::state::OperationType::CopyUpload
                }
            }
            FO::Delete => crate::state::OperationType::Delete,
            _ => {
                if request.is_move {
                    crate::state::OperationType::Move
                } else {
                    crate::state::OperationType::Copy
                }
            }
        }
    }

    /// Build a VfsPath using connection info from another VfsPath but with a different path.
    fn vfs_path_with_connection(base: &VfsPath, path: PathBuf) -> VfsPath {
        VfsPath {
            protocol: base.protocol,
            host: base.host.clone(),
            port: base.port,
            username: base.username.clone(),
            path,
        }
    }

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
            // Check if the ACTIVE panel (source) is remote — that means same-server copy.
            // Use active panel, not find_remote_file_manager_info() which searches ALL panels
            // and would incorrectly match the destination panel for local→remote uploads.
            let active_is_remote = self
                .layout_manager
                .active_panel()
                .and_then(|p| {
                    p.as_any()
                        .downcast_ref::<termide_panel_file_manager::FileManager>()
                })
                .map(|fm| fm.is_remote())
                .unwrap_or(false);

            if active_is_remote {
                if let Some((vfs_manager, vfs_current_path)) = self.find_remote_file_manager_info()
                {
                    return self.start_remote_to_remote_operation(
                        operation_type,
                        sources,
                        &dest_str,
                        vfs_manager,
                        vfs_current_path,
                    );
                }
            }
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
        use crate::state::OperationType;

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

        // Find the destination panel that is connected to this remote
        let (vfs_manager, connected_path) = match self.find_connected_vfs_manager(&remote_path) {
            Some(result) => result,
            None => {
                log::error!(
                    "No active connection to remote host: {}",
                    remote_path.connection_key()
                );
                self.state
                    .set_error("No active connection to remote host".to_string());
                return Ok(());
            }
        };

        // Normalize remote_path to use connection info from the connected panel
        // (user URL may omit username/port that were resolved from SSH config)
        let remote_path = Self::vfs_path_with_connection(&connected_path, remote_path.path);

        let is_move = operation_type == BatchOperationType::Move;
        let total_files = sources.len();

        // Start with the first file (clone to avoid borrow issues)
        let source = sources[0].clone();
        let source_name = source
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());

        // Determine final remote destination path using VFS stat
        let (final_remote, dest_exists) = self.resolve_remote_dest(
            &remote_path,
            &source_name,
            remote_url,
            sources.len() > 1,
            &vfs_manager,
        );

        if dest_exists {
            // Show conflict modal
            let remaining = sources.len().saturating_sub(1);
            let modal = ConflictModal::new(
                &source,
                &PathBuf::from(final_remote.to_url_string()),
                remaining,
            );
            self.state.pending_action = Some(PendingAction::ContinueBatchOperation {
                operation: BatchOperation::new(operation_type, sources, PathBuf::from(remote_url)),
            });
            self.state.active_modal = Some(ActiveModal::Conflict(Box::new(modal)));
            return Ok(());
        }

        // Create upload operation request
        use termide_file_ops::OperationRequest;
        let request = OperationRequest::upload(source.clone(), final_remote.clone());

        // Store batch upload state for continuation
        self.state.pending_batch_upload = Some(crate::state::PendingBatchUpload {
            all_sources: sources,
            current_index: 0,
            dest_base_url: remote_url.to_string(),
            vfs_manager: vfs_manager.clone(),
            is_move,
            current_source: source.clone(),
        });

        // Determine operation type for display
        let op_type = if is_move {
            OperationType::MoveUpload
        } else {
            OperationType::CopyUpload
        };

        // Start tracked upload operation (opens Operations panel)
        match self.start_tracked_operation(
            request,
            vfs_manager,
            op_type,
            source.display().to_string(),
            final_remote.to_url_string(),
            total_files,
            0, // bytes will be updated during progress
        ) {
            Ok(operation_id) => {
                // Store operation ID for pause/resume
                self.state.active_operation_id = Some(operation_id);
            }
            Err(e) => {
                log::error!("Failed to start upload operation: {}", e);
                self.state.pending_batch_upload = None;
                self.state.set_error(format!("Upload failed: {}", e));
            }
        }

        Ok(())
    }

    /// Start a remote-to-remote copy/move on the same server.
    ///
    /// Sources are server-side absolute paths (from `get_selected_paths()` where
    /// `current_path` is `/`). The destination is a VFS URL like `sftp://user@host/path/`.
    /// We construct proper VFS paths for both source and destination and use
    /// `CrossProtocolWorker::RemoteToRemote` which downloads to temp then uploads.
    fn start_remote_to_remote_operation(
        &mut self,
        operation_type: BatchOperationType,
        sources: Vec<PathBuf>,
        remote_url: &str,
        vfs_manager: std::sync::Arc<termide_vfs::VfsManager>,
        vfs_current_path: termide_vfs::VfsPath,
    ) -> Result<()> {
        use crate::state::OperationType;

        if sources.is_empty() {
            return Ok(());
        }

        let parsed_dest = match termide_vfs::parse_vfs_url(remote_url) {
            Ok(path) => path,
            Err(e) => {
                log::error!("Invalid remote URL '{}': {}", remote_url, e);
                self.state.set_error(format!("Invalid remote URL: {}", e));
                return Ok(());
            }
        };

        // Normalize destination to use connection info from the connected panel
        // (user URL may omit username/port that were resolved from SSH config)
        let remote_dest = Self::vfs_path_with_connection(&vfs_current_path, parsed_dest.path);

        let is_move = operation_type == BatchOperationType::Move;
        let total_files = sources.len();

        // Build VFS source path from the server-side PathBuf
        let source = &sources[0];
        let source_name = source
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let vfs_source = vfs_current_path.join(&source_name);

        // Resolve destination using VFS stat
        let (vfs_dest, dest_exists) = self.resolve_remote_dest(
            &remote_dest,
            &source_name,
            remote_url,
            sources.len() > 1,
            &vfs_manager,
        );

        if dest_exists {
            // Show conflict modal
            let remaining = sources.len().saturating_sub(1);
            let modal =
                ConflictModal::new(source, &PathBuf::from(vfs_dest.to_url_string()), remaining);
            self.state.pending_action = Some(PendingAction::ContinueBatchOperation {
                operation: BatchOperation::new(operation_type, sources, PathBuf::from(remote_url)),
            });
            self.state.active_modal = Some(ActiveModal::Conflict(Box::new(modal)));
            return Ok(());
        }

        // Create remote-to-remote copy/move request
        use termide_file_ops::{OperationPath, OperationRequest};
        let request = if is_move {
            OperationRequest::r#move(
                vec![OperationPath::Remote(vfs_source.clone())],
                OperationPath::Remote(vfs_dest.clone()),
            )
        } else {
            OperationRequest::copy(
                vec![OperationPath::Remote(vfs_source.clone())],
                OperationPath::Remote(vfs_dest.clone()),
            )
        };

        let op_type = if is_move {
            OperationType::Move
        } else {
            OperationType::Copy
        };

        match self.start_tracked_operation(
            request,
            vfs_manager,
            op_type,
            vfs_source.to_url_string(),
            vfs_dest.to_url_string(),
            total_files,
            0,
        ) {
            Ok(operation_id) => {
                self.state.active_operation_id = Some(operation_id);
            }
            Err(e) => {
                log::error!("Failed to start remote copy operation: {}", e);
                self.state.set_error(format!("Remote copy failed: {}", e));
            }
        }

        Ok(())
    }

    /// Resolve remote destination path using VFS stat.
    ///
    /// Logic (mirrors local `resolve_batch_destination_path`):
    /// 1. If URL ends with '/' or multiple sources — treat as directory, append filename
    /// 2. Otherwise, stat the path on server:
    ///    - If it's a directory — append filename (copy INTO directory)
    ///    - If it exists as a file — conflict (return dest path + exists=true)
    ///    - If doesn't exist — use as-is (rename)
    fn resolve_remote_dest(
        &self,
        remote_dest: &VfsPath,
        source_name: &str,
        remote_url: &str,
        is_multi_source: bool,
        vfs_manager: &std::sync::Arc<termide_vfs::VfsManager>,
    ) -> (VfsPath, bool) {
        log::debug!(
            "resolve_remote_dest: dest={}, source_name={}, url={}, multi={}",
            remote_dest.to_url_string(),
            source_name,
            remote_url,
            is_multi_source,
        );

        // Multiple sources or trailing slash — always directory
        if is_multi_source || remote_url.ends_with('/') {
            let final_path = remote_dest.join(source_name);
            let exists = vfs_manager.exists(&final_path).recv().unwrap_or(false);
            log::debug!(
                "resolve_remote_dest: trailing slash/multi → final={}, exists={}",
                final_path.to_url_string(),
                exists,
            );
            return (final_path, exists);
        }

        // Single source, no trailing slash — check what dest is on server
        match vfs_manager.metadata(remote_dest).recv() {
            Ok(meta) if meta.file_type.is_dir() => {
                // Dest is an existing directory — copy INTO it
                let final_path = remote_dest.join(source_name);
                let exists = vfs_manager.exists(&final_path).recv().unwrap_or(false);
                log::debug!(
                    "resolve_remote_dest: dest is dir → final={}, exists={}",
                    final_path.to_url_string(),
                    exists,
                );
                (final_path, exists)
            }
            Ok(_) => {
                // Dest exists and is a file — conflict (overwrite)
                log::debug!("resolve_remote_dest: dest is file → conflict");
                (remote_dest.clone(), true)
            }
            Err(e) => {
                // Dest doesn't exist — use as-is (rename)
                log::debug!("resolve_remote_dest: dest not found ({}) → rename", e);
                (remote_dest.clone(), false)
            }
        }
    }

    /// Find VFS manager from an existing connected FileManager panel.
    /// Returns None if no panel is connected to the target remote.
    /// Also returns the panel's VfsPath for connection info normalization.
    fn find_connected_vfs_manager(
        &self,
        remote_path: &termide_vfs::VfsPath,
    ) -> Option<(
        std::sync::Arc<termide_vfs::VfsManager>,
        termide_vfs::VfsPath,
    )> {
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
                            return Some((fm.vfs_state().manager_arc(), fm_path.clone()));
                        }
                    }
                }
            }
        }
        None
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
                        let dest_str_ow = operation.destination.to_string_lossy().to_string();
                        let is_remote_dest_ow = termide_vfs::is_vfs_url(&dest_str_ow);
                        let item_name_ow = path_utils::get_file_name_string(&source);

                        let final_dest = if is_remote_dest_ow {
                            let base = termide_vfs::parse_vfs_url(&dest_str_ow)
                                .map(|p| p.path)
                                .unwrap_or_else(|_| operation.destination.clone());
                            base.join(&item_name_ow)
                        } else {
                            path_utils::resolve_batch_destination_path(
                                &source,
                                &operation.destination,
                                operation.sources.len() == 1,
                            )
                        };

                        // Execute operation - only use remote path when source or dest is remote
                        let needs_remote_ow = is_remote_dest_ow || !source.exists();
                        if needs_remote_ow {
                            if let Some((vfs_manager, vfs_current_path)) =
                                self.find_remote_file_manager_info()
                            {
                                use termide_file_ops::{OperationPath, OperationRequest};

                                let source_name = source
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                let vfs_source = vfs_current_path.join(&source_name);

                                let is_move = operation.operation_type == BatchOperationType::Move;

                                let request = if is_remote_dest_ow {
                                    let vfs_dest = Self::vfs_path_with_connection(
                                        &vfs_current_path,
                                        final_dest,
                                    );
                                    if is_move {
                                        OperationRequest::r#move(
                                            vec![OperationPath::Remote(vfs_source.clone())],
                                            OperationPath::Remote(vfs_dest),
                                        )
                                    } else {
                                        OperationRequest::copy(
                                            vec![OperationPath::Remote(vfs_source.clone())],
                                            OperationPath::Remote(vfs_dest),
                                        )
                                    }
                                } else {
                                    let r =
                                        OperationRequest::download(vfs_source.clone(), final_dest);
                                    if is_move {
                                        self.state.pending_remote_delete =
                                            Some(PendingRemoteDelete {
                                                vfs_source,
                                                vfs_manager: vfs_manager.clone(),
                                            });
                                    }
                                    r
                                };

                                self.start_batch_sub_operation(request, vfs_manager, operation);
                                return Ok(());
                            }
                        }

                        // Local file or directory - use OperationManager for async copy
                        if source.is_file() || source.is_dir() {
                            use termide_file_ops::{
                                ConflictMode as FileOpsConflictMode, OperationPath,
                                OperationRequest,
                            };

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
                            }
                            .with_conflict_mode(FileOpsConflictMode::OverwriteAll);

                            let vfs_manager = std::sync::Arc::new(termide_vfs::VfsManager::new());

                            self.start_batch_sub_operation(request, vfs_manager, operation);
                            return Ok(());
                        }

                        // Unknown source type - skip with error
                        log::error!("Unsupported source type: {}", source.display());
                        operation.increment_error();
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
                    operation.advance();

                    // Store and return to allow UI update
                    self.state.pending_action =
                        Some(PendingAction::ContinueBatchOperation { operation });
                }
                ConflictResolution::OverwriteAll => {
                    // Set "overwrite all" mode
                    operation.set_conflict_mode(ConflictMode::OverwriteAll);

                    // Store and return to allow UI update
                    self.state.pending_action =
                        Some(PendingAction::ContinueBatchOperation { operation });
                }
                ConflictResolution::SkipAll => {
                    // Set "skip all" mode
                    operation.set_conflict_mode(ConflictMode::SkipAll);
                    operation.increment_skipped();
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

    /// Find a remote file manager panel (searches all panels, not just active).
    /// Returns (vfs_manager, vfs_current_path) if found.
    fn find_remote_file_manager_info(
        &self,
    ) -> Option<(
        std::sync::Arc<termide_vfs::VfsManager>,
        termide_vfs::VfsPath,
    )> {
        for group in &self.layout_manager.panel_groups {
            for panel in group.panels() {
                if let Some(fm) = panel
                    .as_any()
                    .downcast_ref::<termide_panel_file_manager::FileManager>()
                {
                    if fm.is_remote() {
                        return Some((
                            fm.vfs_state().manager_arc(),
                            fm.vfs_state().current_path().clone(),
                        ));
                    }
                }
            }
        }
        None
    }

    /// Handle batch file operation (copy/move)
    pub(in crate::app) fn process_batch_operation(&mut self, mut operation: BatchOperation) {
        // Show progress modal for:
        // 1. Multi-file operations (total_count > 1), OR
        // 2. Single remote file operations (need network transfer feedback), OR
        // 3. Single directory operations (recursive copy/move can take time), OR
        // 4. Single file > 1MB (large file transfer needs progress feedback)

        // Check if this is a remote operation based on actual operation data:
        // - destination is a VFS URL (e.g., sftp://...), OR
        // - source doesn't exist locally (server-side path)
        let dest_str_check = operation.destination.to_string_lossy();
        let is_remote_dest_check = termide_vfs::is_vfs_url(&dest_str_check);
        let source_is_local = operation
            .current_source()
            .map(|p| p.exists())
            .unwrap_or(false);
        let is_remote_operation = is_remote_dest_check || !source_is_local;

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
        // Only create if not already shown (no progress modal and no batch tracking active)
        let should_show_modal = operation.current_index == 0
            && (operation.total_count() > 1 || is_remote_operation || needs_progress)
            && !matches!(self.state.active_modal, Some(ActiveModal::Progress(_)))
            && self.state.batch_tracking_id.is_none();

        if should_show_modal {
            use crate::state::OperationType;

            // Close any existing modal (e.g., destination selection) before showing progress
            self.state.close_modal();

            // Get source display for tracking
            let source_display = if is_remote_operation {
                // For remote operations, find the VFS path for nice display
                if let Some((_, vfs_path)) = self.find_remote_file_manager_info() {
                    if let Some(source_file) = operation.current_source() {
                        format_vfs_path_for_display(&vfs_path, source_file)
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

            let op_type = match (operation.operation_type, is_remote_operation) {
                (BatchOperationType::Copy, true) => OperationType::CopyDownload,
                (BatchOperationType::Move, true) => OperationType::MoveDownload,
                (BatchOperationType::Copy, false) => OperationType::Copy,
                (BatchOperationType::Move, false) => OperationType::Move,
            };

            // Pre-scan all sources to get total bytes for progress display
            let total_bytes = if !is_remote_operation {
                scan_sources_total_bytes(&operation.sources)
            } else {
                0 // Remote sources: byte progress comes from individual operations
            };

            // Start batch tracking in Operations panel
            let batch_id = self.state.start_batch_tracking(
                op_type,
                source_display,
                dest_display,
                operation.total_count(),
                total_bytes,
            );

            // Open Operations panel with focus on the new batch operation
            let _ = self.open_operations_panel_with_focus(batch_id);

            // Store operation as pending action to allow UI to render panel
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
            // Close progress modal if open (legacy)
            if matches!(self.state.active_modal, Some(ActiveModal::Progress(_))) {
                self.state.close_modal();
            }

            // Finish batch tracking in Operations panel
            self.state.finish_batch_tracking();

            // Bell signal on completion
            self.state.bell();

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

        let dest_str = operation.destination.to_string_lossy().to_string();
        let is_remote_dest = termide_vfs::is_vfs_url(&dest_str);

        // Determine target path (considering rename pattern if set).
        // For remote URLs, PathBuf::is_dir() returns false, so standard
        // resolve functions don't work; compute the path component directly.
        let final_dest = if operation.rename_pattern.is_some() {
            // Apply rename pattern
            let counter = operation.get_and_increment_rename_counter();
            let metadata = source.metadata().ok();
            let created = metadata.as_ref().and_then(|m| m.created().ok());
            let modified = metadata.as_ref().and_then(|m| m.modified().ok());

            let pattern = operation.rename_pattern.as_ref().unwrap();
            let new_name = pattern.apply(&item_name, counter, created, modified);

            if is_remote_dest {
                let base = termide_vfs::parse_vfs_url(&dest_str)
                    .map(|p| p.path)
                    .unwrap_or_else(|_| operation.destination.clone());
                base.join(&new_name)
            } else {
                path_utils::resolve_rename_destination_path(&operation.destination, &new_name)
            }
        } else if is_remote_dest {
            // Remote destination: parse URL path and join filename
            let base = termide_vfs::parse_vfs_url(&dest_str)
                .map(|p| p.path)
                .unwrap_or_else(|_| operation.destination.clone());
            base.join(&item_name)
        } else {
            // Standard logic without renaming
            path_utils::resolve_batch_destination_path(
                &source,
                &operation.destination,
                operation.sources.len() == 1,
            )
        };

        // Update batch tracking file-level progress in Operations panel
        if self.state.batch_tracking_id.is_some() {
            self.state
                .update_batch_progress(operation.current_index, operation.total_count());
            self.state.needs_redraw = true;
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

        // Execute operation - use remote path only when source or destination is actually remote.
        // source.exists() is false for server-side paths (they don't exist locally).
        let needs_remote = is_remote_dest || !source.exists();
        if needs_remote {
            if let Some((vfs_manager, vfs_current_path)) = self.find_remote_file_manager_info() {
                use termide_file_ops::{OperationPath, OperationRequest};

                let source_name = source
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let vfs_source = vfs_current_path.join(&source_name);

                let is_move = operation.operation_type == BatchOperationType::Move;

                let request = if is_remote_dest {
                    let vfs_dest = Self::vfs_path_with_connection(&vfs_current_path, final_dest);
                    if is_move {
                        OperationRequest::r#move(
                            vec![OperationPath::Remote(vfs_source.clone())],
                            OperationPath::Remote(vfs_dest),
                        )
                    } else {
                        OperationRequest::copy(
                            vec![OperationPath::Remote(vfs_source.clone())],
                            OperationPath::Remote(vfs_dest),
                        )
                    }
                } else {
                    let r = OperationRequest::download(vfs_source.clone(), final_dest);
                    if is_move {
                        self.state.pending_remote_delete = Some(PendingRemoteDelete {
                            vfs_source,
                            vfs_manager: vfs_manager.clone(),
                        });
                    }
                    r
                };

                self.start_batch_sub_operation(request, vfs_manager, operation);
                return;
            }
        }

        // Local file or directory - use OperationManager for async copy with progress
        // Applies to both Copy and Move (move may need copy+delete for cross-filesystem)
        if source.is_file() || source.is_dir() {
            use termide_file_ops::{
                ConflictMode as FileOpsConflictMode, OperationPath, OperationRequest,
            };

            let is_move = operation.operation_type == BatchOperationType::Move;
            let worker_conflict_mode = match operation.conflict_mode {
                ConflictMode::OverwriteAll => FileOpsConflictMode::OverwriteAll,
                ConflictMode::SkipAll => FileOpsConflictMode::SkipAll,
                _ => FileOpsConflictMode::Ask,
            };
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
            }
            .with_conflict_mode(worker_conflict_mode);

            let vfs_manager = std::sync::Arc::new(termide_vfs::VfsManager::new());

            self.start_batch_sub_operation(request, vfs_manager, operation);
            return;
        }

        // Unknown source type (symlink?) - skip with error
        log::error!("Unsupported source type: {}", source.display());
        operation.increment_error();

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
                // Capitalize the localized action word (e.g., "copied" → "Copied")
                let word = action_name.0;
                let capitalized: String = word
                    .chars()
                    .take(1)
                    .flat_map(|c| c.to_uppercase())
                    .chain(word.chars().skip(1))
                    .collect();
                self.state.set_info(capitalized);
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

                    let dest_str = operation.destination.to_string_lossy();
                    let is_remote_dest = termide_vfs::is_vfs_url(&dest_str);

                    // Create new destination path with new name
                    let new_dest = if is_remote_dest {
                        // For remote URLs, is_dir() always returns false, so
                        // resolve_rename_destination_path would incorrectly
                        // use with_file_name(). Instead, parse URL and join.
                        let base = termide_vfs::parse_vfs_url(&dest_str)
                            .map(|p| p.path)
                            .unwrap_or_else(|_| operation.destination.clone());
                        base.join(&new_name)
                    } else {
                        path_utils::resolve_rename_destination_path(
                            &operation.destination,
                            &new_name,
                        )
                    };

                    // Check that new path doesn't conflict (local only;
                    // remote conflicts were already detected via VFS stat)
                    if !is_remote_dest && new_dest.exists() {
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

                    // Execute operation directly with the renamed destination.
                    // Do NOT modify operation.destination — it must remain the original
                    // directory for subsequent files in the batch. Instead, run the
                    // operation for this single file using new_dest directly
                    // (same approach as the Overwrite handler).
                    let needs_remote_rn = is_remote_dest || !source.exists();
                    if needs_remote_rn {
                        if let Some((vfs_manager, vfs_current_path)) =
                            self.find_remote_file_manager_info()
                        {
                            use termide_file_ops::{OperationPath, OperationRequest};

                            let source_name = source
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let vfs_source = vfs_current_path.join(&source_name);

                            let is_move = operation.operation_type == BatchOperationType::Move;

                            let request = if is_remote_dest {
                                let vfs_dest =
                                    Self::vfs_path_with_connection(&vfs_current_path, new_dest);
                                if is_move {
                                    OperationRequest::r#move(
                                        vec![OperationPath::Remote(vfs_source.clone())],
                                        OperationPath::Remote(vfs_dest),
                                    )
                                } else {
                                    OperationRequest::copy(
                                        vec![OperationPath::Remote(vfs_source.clone())],
                                        OperationPath::Remote(vfs_dest),
                                    )
                                }
                            } else {
                                let r = OperationRequest::download(vfs_source.clone(), new_dest);
                                if is_move {
                                    self.state.pending_remote_delete = Some(PendingRemoteDelete {
                                        vfs_source,
                                        vfs_manager: vfs_manager.clone(),
                                    });
                                }
                                r
                            };

                            self.start_batch_sub_operation(request, vfs_manager, operation);
                        }
                    } else if let Some(panel) = self.layout_manager.active_panel_mut() {
                        if let Some(_fm) = panel.as_file_manager_mut() {
                            use termide_file_ops::{OperationPath, OperationRequest};

                            let is_move = operation.operation_type == BatchOperationType::Move;
                            let request = if is_move {
                                OperationRequest::r#move(
                                    vec![OperationPath::Local(source.clone())],
                                    OperationPath::Local(new_dest),
                                )
                            } else {
                                OperationRequest::copy(
                                    vec![OperationPath::Local(source.clone())],
                                    OperationPath::Local(new_dest),
                                )
                            };

                            let vfs_manager = std::sync::Arc::new(termide_vfs::VfsManager::new());

                            self.start_batch_sub_operation(request, vfs_manager, operation);
                        }
                    }
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
