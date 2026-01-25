//! Confirm modal result handling.

use anyhow::Result;
use std::path::PathBuf;

use super::super::App;
use crate::state::ActiveModal;
use termide_file_ops::{OperationPath, OperationRequest};
use termide_modal::ProgressModal;
use termide_ui::path_utils;

impl App {
    /// Handle deletion of files/directories
    pub(in crate::app) fn handle_delete_path(
        &mut self,
        _panel_index: usize, // obsolete with LayoutManager
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

                termide_logger::info(format!("Starting async delete of {}", source_display));

                // Create delete operation request
                let sources: Vec<OperationPath> =
                    paths.into_iter().map(OperationPath::Local).collect();
                let request = OperationRequest::delete(sources);

                // Get or create VFS manager for operation manager
                let vfs_manager = std::sync::Arc::new(termide_vfs::VfsManager::new());

                // Start delete operation via OperationManager
                match self.state.start_operation_now(request, vfs_manager) {
                    Ok(_operation_id) => {
                        // Show progress modal
                        let modal = ProgressModal::new_delete_progress(0, source_display);
                        self.state.active_modal = Some(ActiveModal::Progress(Box::new(modal)));
                    }
                    Err(e) => {
                        termide_logger::error(format!("Failed to start delete operation: {}", e));
                        self.state.set_error(format!("Delete failed: {}", e));
                    }
                }
            }
        }
        Ok(())
    }

    /// Handle panel closure
    pub(in crate::app) fn handle_close_panel(
        &mut self,
        _panel_index: usize, // obsolete with LayoutManager
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        if let Some(confirmed) = value.downcast_ref::<bool>() {
            if *confirmed {
                // Terminate processes in active panel (for terminal)
                if let Some(panel) = self.layout_manager.active_panel_mut() {
                    panel.kill_processes();
                }
                // Close active panel
                self.close_panel_at_index(0); // panel_index is obsolete
            }
        }
        Ok(())
    }
}
