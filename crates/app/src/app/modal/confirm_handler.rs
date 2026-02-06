//! Confirm modal result handling.

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

use super::super::App;
use crate::state::OperationType;
use termide_file_ops::{OperationPath, OperationRequest};
use termide_ui::path_utils;
use termide_vfs::{VfsManager, VfsPath};

impl App {
    /// Handle deletion of files/directories
    pub(in crate::app) fn handle_delete_path(
        &mut self,
        paths: Vec<PathBuf>,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        if let Some(confirmed) = value.downcast_ref::<bool>() {
            if *confirmed && !paths.is_empty() {
                // Build source display string
                let source_display = if paths.len() == 1 {
                    path_utils::get_file_name_str(&paths[0]).to_string()
                } else {
                    format!("{} items", paths.len())
                };

                log::info!("Starting async delete of {}", source_display);

                let paths_count = paths.len();

                // Create delete operation request
                let sources: Vec<OperationPath> =
                    paths.into_iter().map(OperationPath::Local).collect();
                let request = OperationRequest::delete(sources);

                // Get or create VFS manager for operation manager
                let vfs_manager = std::sync::Arc::new(termide_vfs::VfsManager::new());

                // Start delete operation via OperationManager
                match self.start_tracked_operation(
                    request,
                    vfs_manager,
                    OperationType::Delete,
                    source_display,
                    String::new(),
                    paths_count,
                    0,
                ) {
                    Ok(_operation_id) => {}
                    Err(e) => {
                        log::error!("Failed to start delete operation: {}", e);
                        self.state
                            .set_error(termide_i18n::t().status_delete_failed(&e.to_string()));
                    }
                }
            }
        }
        Ok(())
    }

    /// Handle deletion of remote files/directories
    pub(in crate::app) fn handle_delete_remote_path(
        &mut self,
        paths: Vec<VfsPath>,
        vfs_manager: Arc<VfsManager>,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        if let Some(confirmed) = value.downcast_ref::<bool>() {
            if *confirmed && !paths.is_empty() {
                // Build source display string
                let source_display = if paths.len() == 1 {
                    paths[0]
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "file".to_string())
                } else {
                    format!("{} items", paths.len())
                };

                log::info!("Starting async remote delete of {}", source_display);

                let paths_count = paths.len();

                // Create delete operation request with VFS paths
                let sources: Vec<OperationPath> =
                    paths.into_iter().map(OperationPath::Remote).collect();
                let request = OperationRequest::delete(sources);

                // Start delete operation via OperationManager with tracking
                match self.start_tracked_operation(
                    request,
                    vfs_manager,
                    OperationType::Delete,
                    source_display,
                    String::new(),
                    paths_count,
                    0,
                ) {
                    Ok(_operation_id) => {}
                    Err(e) => {
                        log::error!("Failed to start remote delete operation: {}", e);
                        self.state
                            .set_error(termide_i18n::t().status_delete_failed(&e.to_string()));
                    }
                }
            }
        }
        Ok(())
    }

    /// Handle panel closure
    pub(in crate::app) fn handle_close_panel(
        &mut self,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        if let Some(confirmed) = value.downcast_ref::<bool>() {
            if *confirmed {
                // Terminate processes in active panel (for terminal)
                if let Some(panel) = self.layout_manager.active_panel_mut() {
                    panel.kill_processes();
                }
                // Close active panel
                self.close_panel_at_index();
            }
        }
        Ok(())
    }
}
