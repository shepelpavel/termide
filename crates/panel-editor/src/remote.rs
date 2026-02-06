//! Remote file editing support via VFS.
//!
//! This module provides functionality for editing remote files through the VFS abstraction.
//! Remote files are downloaded to a temporary location for editing, then uploaded on save.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use termide_vfs::{VfsManager, VfsOperation, VfsPath};

use crate::config::EditorConfig;
use crate::state::FileState;
use crate::Editor;

/// Pending remote file open operation.
pub struct PendingRemoteOpen {
    pub operation: VfsOperation<PathBuf>,
    pub temp_path: PathBuf,
    pub remote_path: VfsPath,
    pub config: EditorConfig,
    pub vfs_manager: Arc<VfsManager>,
}

impl Editor {
    /// Open a remote file for editing.
    ///
    /// Downloads the file to a temporary location and opens it in the editor.
    /// The remote path is tracked so changes can be uploaded on save.
    pub fn open_remote_file(
        vfs_manager: Arc<VfsManager>,
        remote_path: VfsPath,
        config: EditorConfig,
    ) -> Result<Self> {
        // Create temp directory for remote files
        let temp_dir = std::env::temp_dir().join("termide-remote-edit");
        std::fs::create_dir_all(&temp_dir)?;

        // Generate unique temp file name
        let file_name = remote_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("remote_file");
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let temp_file_name = format!("{}_{}", timestamp, file_name);
        let temp_path = temp_dir.join(&temp_file_name);

        // Download file to temp location using VfsManager (which handles provider lookup internally)
        match vfs_manager.download(&remote_path, &temp_path).recv() {
            Ok(_) => {}
            Err(e) => return Err(anyhow::anyhow!("Failed to download file: {}", e)),
        }

        // Get metadata by reading file info from local file (already downloaded)
        let (size, mtime) = match std::fs::metadata(&temp_path) {
            Ok(meta) => {
                let size = meta.len();
                let mtime = meta.modified().ok();
                (size, mtime)
            }
            Err(_) => (0, None),
        };

        // Now open the temp file with the editor
        let mut editor = Self::open_file_with_config(temp_path.clone(), config)?;

        // Track remote file info
        editor.file_state = FileState::from_remote(remote_path, temp_path, mtime, size);

        // Store VfsManager for remote saves
        editor.vfs_manager = Some(vfs_manager);

        Ok(editor)
    }

    /// Start downloading a remote file for editing (async).
    ///
    /// Returns a PendingRemoteOpen that tracks the download operation.
    /// The caller should poll this operation and call `complete_remote_open()` when done.
    pub fn start_remote_open(
        vfs_manager: Arc<VfsManager>,
        remote_path: VfsPath,
        config: EditorConfig,
    ) -> Result<PendingRemoteOpen> {
        // Create temp directory for remote files
        let temp_dir = std::env::temp_dir().join("termide-remote-edit");
        std::fs::create_dir_all(&temp_dir)?;

        // Generate unique temp file name
        let file_name = remote_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("remote_file");
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let temp_file_name = format!("{}_{}", timestamp, file_name);
        let temp_path = temp_dir.join(&temp_file_name);

        // Start download (non-blocking)
        let operation = vfs_manager.download(&remote_path, &temp_path);

        Ok(PendingRemoteOpen {
            operation,
            temp_path,
            remote_path,
            config,
            vfs_manager,
        })
    }

    /// Complete remote file open after download finishes.
    ///
    /// Creates an editor with the downloaded temp file and tracks remote path.
    pub fn complete_remote_open(pending: PendingRemoteOpen) -> Result<Self> {
        let PendingRemoteOpen {
            temp_path,
            remote_path,
            config,
            vfs_manager,
            ..
        } = pending;

        // Get metadata from downloaded file
        let (size, mtime) = match std::fs::metadata(&temp_path) {
            Ok(meta) => {
                let size = meta.len();
                let mtime = meta.modified().ok();
                (size, mtime)
            }
            Err(_) => (0, None),
        };

        // Open the temp file with the editor
        let mut editor = Self::open_file_with_config(temp_path.clone(), config)?;

        // Track remote file info
        editor.file_state = FileState::from_remote(remote_path, temp_path, mtime, size);

        // Store VfsManager for remote saves
        editor.vfs_manager = Some(vfs_manager);

        Ok(editor)
    }

    /// Save a remote file.
    ///
    /// Uploads the local temp file back to the remote location.
    pub fn save_remote(&mut self, vfs_manager: &VfsManager) -> Result<()> {
        let remote_path = self
            .file_state
            .remote_path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No remote path associated with this file"))?
            .clone();

        let temp_path = self
            .file_state
            .temp_local_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No temp file path"))?;

        // First save to local temp file
        self.buffer_mut().save()?;

        // Upload using VfsManager (which handles provider lookup internally)
        match vfs_manager.upload(&temp_path, &remote_path).recv() {
            Ok(_) => {
                log::info!("Remote file uploaded: {}", remote_path.to_url_string());

                // Update remote mtime after successful upload
                // We'll use the local file's mtime since we just wrote it
                if let Ok(meta) = std::fs::metadata(&temp_path) {
                    if let Ok(mtime) = meta.modified() {
                        self.file_state.update_remote_mtime(Some(mtime));
                    }
                }

                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("Failed to upload file: {}", e)),
        }
    }

    /// Check if this editor is editing a remote file.
    pub fn is_remote_file(&self) -> bool {
        self.file_state.is_remote()
    }

    /// Get the remote path if editing a remote file.
    pub fn remote_path(&self) -> Option<&VfsPath> {
        self.file_state.remote_path()
    }

    /// Get the display path (remote URL or local path).
    pub fn display_path(&self) -> String {
        if let Some(remote) = &self.file_state.remote_path {
            remote.to_url_string()
        } else if let Some(local) = self.file_path() {
            local.display().to_string()
        } else {
            "Untitled".to_string()
        }
    }
}
