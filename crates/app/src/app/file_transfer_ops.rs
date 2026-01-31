//! Remote file transfer operation handlers.
//!
//! Contains handlers for remote file system operations:
//! - Upload operations (local → remote)

use std::path::PathBuf;

use crate::PanelExt;

use super::App;

impl App {
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
            "Upload",
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
                // Skip file manager refresh - file already exists, we're just overwriting
                self.state.skip_refresh_after_upload = true;
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
}
