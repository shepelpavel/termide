//! Worker trait and implementations for file operations.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Instant, UNIX_EPOCH};

use termide_vfs::{VfsManager, VfsPath};

use crate::types::{
    ConflictInfo, ConflictMode, ConflictResolution, OperationControl, OperationError,
    OperationEvent, OperationId, OperationPath, OperationPhase, OperationProgress, OperationResult,
};

/// Chunk size for file operations (1MB).
const CHUNK_SIZE: usize = 1024 * 1024;

/// Context for conflict handling in workers.
pub struct ConflictContext {
    /// Operation ID for events.
    pub operation_id: OperationId,
    /// Current conflict handling mode.
    pub conflict_mode: ConflictMode,
    /// Channel to send events (including ConflictDetected).
    pub event_tx: mpsc::Sender<OperationEvent>,
    /// Channel to receive conflict resolutions.
    pub resolution_rx: mpsc::Receiver<ConflictResolution>,
}

impl ConflictContext {
    /// Check for conflict and handle according to mode.
    /// Returns: Ok(true) = proceed with copy, Ok(false) = skip, Err = cancel
    pub fn check_conflict(
        &mut self,
        source: &Path,
        dest: &Path,
        remaining_items: usize,
    ) -> Result<bool, OperationError> {
        // Check if destination exists
        if !dest.exists() {
            return Ok(true); // No conflict
        }

        // Handle based on current mode
        match self.conflict_mode {
            ConflictMode::OverwriteAll => Ok(true),
            ConflictMode::SkipAll => Ok(false),
            ConflictMode::Ask => {
                // Gather file info for the conflict
                let source_meta = fs::metadata(source).ok();
                let dest_meta = fs::metadata(dest).ok();

                let conflict_info = ConflictInfo {
                    source: OperationPath::Local(source.to_path_buf()),
                    destination: OperationPath::Local(dest.to_path_buf()),
                    source_size: source_meta.as_ref().map(|m| m.len()).unwrap_or(0),
                    dest_size: dest_meta.as_ref().map(|m| m.len()).unwrap_or(0),
                    source_modified: source_meta.as_ref().and_then(|m| {
                        m.modified()
                            .ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs()))
                    }),
                    dest_modified: dest_meta.as_ref().and_then(|m| {
                        m.modified()
                            .ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs()))
                    }),
                    remaining_items,
                };

                // Send conflict event
                let _ = self.event_tx.send(OperationEvent::ConflictDetected(
                    self.operation_id,
                    conflict_info,
                ));

                // Wait for resolution (blocking)
                match self.resolution_rx.recv() {
                    Ok(resolution) => match resolution {
                        ConflictResolution::Overwrite => Ok(true),
                        ConflictResolution::Skip => Ok(false),
                        ConflictResolution::OverwriteAll => {
                            self.conflict_mode = ConflictMode::OverwriteAll;
                            Ok(true)
                        }
                        ConflictResolution::SkipAll => {
                            self.conflict_mode = ConflictMode::SkipAll;
                            Ok(false)
                        }
                        ConflictResolution::Cancel => Err(OperationError::Cancelled),
                    },
                    Err(_) => {
                        // Channel closed - operation cancelled
                        Err(OperationError::Cancelled)
                    }
                }
            }
        }
    }
}

/// Trait for operation workers.
pub trait OperationWorker: Send {
    /// Execute the operation.
    fn execute(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult;

    /// Execute with conflict handling support.
    fn execute_with_conflicts(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
        _conflict_ctx: Option<&mut ConflictContext>,
    ) -> OperationResult {
        // Default implementation ignores conflict context
        self.execute(control, progress_tx)
    }
}

/// Worker for local file/directory copy operations.
pub struct LocalCopyWorker {
    /// Source paths.
    sources: Vec<PathBuf>,
    /// Destination path.
    destination: PathBuf,
    /// Whether to delete source after copy (move).
    is_move: bool,
}

impl LocalCopyWorker {
    /// Create a new local copy worker.
    pub fn new(sources: Vec<PathBuf>, destination: PathBuf, is_move: bool) -> Self {
        Self {
            sources,
            destination,
            is_move,
        }
    }

    /// Scan directory to count files and total size.
    #[allow(clippy::only_used_in_recursion)]
    fn scan_directory(
        &self,
        path: &Path,
        control: &OperationControl,
    ) -> Result<(usize, u64), OperationError> {
        control.check_cancelled()?;

        let mut file_count = 0;
        let mut total_bytes = 0u64;

        for entry in fs::read_dir(path)? {
            control.check_cancelled()?;
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;

            if metadata.is_dir() && !metadata.is_symlink() {
                let (count, bytes) = self.scan_directory(&entry.path(), control)?;
                file_count += count;
                total_bytes += bytes;
            } else {
                file_count += 1;
                if !metadata.is_symlink() {
                    total_bytes += metadata.len();
                }
            }
        }

        Ok((file_count, total_bytes))
    }

    /// Copy a single file with progress.
    #[allow(clippy::too_many_arguments)]
    fn copy_file(
        &self,
        source: &Path,
        dest: &Path,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
        bytes_copied: &mut u64,
        total_bytes: u64,
        files_copied: &mut usize,
        total_files: usize,
        start_time: Instant,
    ) -> Result<(), OperationError> {
        control.check_cancelled()?;
        control.wait_if_paused()?;

        let metadata = fs::symlink_metadata(source)?;

        if metadata.is_symlink() {
            // Copy symlink
            #[cfg(unix)]
            {
                let link_target = fs::read_link(source)?;
                std::os::unix::fs::symlink(&link_target, dest)?;
            }
            #[cfg(not(unix))]
            {
                fs::copy(source, dest)?;
            }
            *files_copied += 1;
        } else {
            // Copy regular file with chunked reading
            let file_size = metadata.len();
            let mut source_file = File::open(source)?;
            let mut dest_file = File::create(dest)?;

            let mut buffer = vec![0u8; CHUNK_SIZE];
            let mut file_bytes_copied = 0u64;

            loop {
                control.check_cancelled()?;
                control.wait_if_paused()?;

                let bytes_read = source_file.read(&mut buffer)?;
                if bytes_read == 0 {
                    break;
                }

                dest_file.write_all(&buffer[..bytes_read])?;
                file_bytes_copied += bytes_read as u64;
                *bytes_copied += bytes_read as u64;

                // Calculate speed and ETA
                let elapsed = start_time.elapsed().as_secs_f64();
                let speed_bps = if elapsed > 0.0 {
                    *bytes_copied as f64 / elapsed
                } else {
                    0.0
                };
                let remaining_bytes = total_bytes.saturating_sub(*bytes_copied);
                let eta_seconds = if speed_bps > 0.0 {
                    Some((remaining_bytes as f64 / speed_bps) as u64)
                } else {
                    None
                };

                // Send progress
                let _ = progress_tx.send(OperationProgress {
                    phase: OperationPhase::Transferring,
                    bytes_transferred: *bytes_copied,
                    total_bytes,
                    files_completed: *files_copied,
                    total_files,
                    current_item: source
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(String::from),
                    speed_bps,
                    eta_seconds,
                });
            }

            *files_copied += 1;

            // Verify file size
            if file_bytes_copied != file_size {
                return Err(OperationError::Io(format!(
                    "File size mismatch for {}: expected {}, got {}",
                    source.display(),
                    file_size,
                    file_bytes_copied
                )));
            }
        }

        Ok(())
    }

    /// Copy a directory recursively.
    #[allow(clippy::too_many_arguments)]
    fn copy_directory(
        &self,
        source: &Path,
        dest: &Path,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
        bytes_copied: &mut u64,
        total_bytes: u64,
        files_copied: &mut usize,
        total_files: usize,
        start_time: Instant,
        depth: usize,
    ) -> Result<(), OperationError> {
        const MAX_DEPTH: usize = 100;
        if depth > MAX_DEPTH {
            return Err(OperationError::Invalid(format!(
                "Directory nesting too deep (> {})",
                MAX_DEPTH
            )));
        }

        control.check_cancelled()?;
        control.wait_if_paused()?;

        // Create destination directory
        fs::create_dir_all(dest)?;

        for entry in fs::read_dir(source)? {
            control.check_cancelled()?;
            let entry = entry?;
            let entry_path = entry.path();
            let dest_path = dest.join(entry.file_name());
            let metadata = fs::symlink_metadata(&entry_path)?;

            if metadata.is_dir() && !metadata.is_symlink() {
                self.copy_directory(
                    &entry_path,
                    &dest_path,
                    control,
                    progress_tx,
                    bytes_copied,
                    total_bytes,
                    files_copied,
                    total_files,
                    start_time,
                    depth + 1,
                )?;
            } else {
                self.copy_file(
                    &entry_path,
                    &dest_path,
                    control,
                    progress_tx,
                    bytes_copied,
                    total_bytes,
                    files_copied,
                    total_files,
                    start_time,
                )?;
            }
        }

        Ok(())
    }
}

impl OperationWorker for LocalCopyWorker {
    fn execute(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        // Default implementation without conflict checking
        self.execute_with_conflicts(control, progress_tx, None)
    }

    fn execute_with_conflicts(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
        mut conflict_ctx: Option<&mut ConflictContext>,
    ) -> OperationResult {
        let start_time = Instant::now();

        // Phase 1: Scan to get totals
        let _ = progress_tx.send(OperationProgress {
            phase: OperationPhase::Scanning,
            ..Default::default()
        });

        let mut total_files = 0;
        let mut total_bytes = 0u64;

        for source in &self.sources {
            if control.is_cancelled() {
                return OperationResult::Cancelled;
            }

            let metadata = match fs::symlink_metadata(source) {
                Ok(m) => m,
                Err(e) => return OperationResult::Failed(e.to_string()),
            };

            if metadata.is_dir() {
                match self.scan_directory(source, control) {
                    Ok((count, bytes)) => {
                        total_files += count;
                        total_bytes += bytes;
                    }
                    Err(OperationError::Cancelled) => return OperationResult::Cancelled,
                    Err(e) => return OperationResult::Failed(e.to_string()),
                }
            } else {
                total_files += 1;
                if !metadata.is_symlink() {
                    total_bytes += metadata.len();
                }
            }
        }

        // Phase 2: Copy files
        let mut bytes_copied = 0u64;
        let mut files_copied = 0usize;
        let mut skipped_files = 0usize;
        // Track sources that were successfully copied (for move cleanup)
        let mut copied_sources: Vec<PathBuf> = Vec::new();

        for source in &self.sources {
            if control.is_cancelled() {
                return OperationResult::Cancelled;
            }

            let metadata = match fs::symlink_metadata(source) {
                Ok(m) => m,
                Err(e) => return OperationResult::Failed(e.to_string()),
            };

            let dest = if self.destination.is_dir() || self.sources.len() > 1 {
                self.destination
                    .join(source.file_name().unwrap_or_default())
            } else {
                self.destination.clone()
            };

            // Check for conflict at top level before copying
            if let Some(ref mut ctx) = conflict_ctx {
                let remaining = total_files.saturating_sub(files_copied + skipped_files);
                match ctx.check_conflict(source, &dest, remaining) {
                    Ok(true) => {
                        // Proceed with copy
                    }
                    Ok(false) => {
                        // Skip this item
                        skipped_files += 1;
                        continue;
                    }
                    Err(OperationError::Cancelled) => return OperationResult::Cancelled,
                    Err(e) => return OperationResult::Failed(e.to_string()),
                }
            }

            let result = if metadata.is_dir() && !metadata.is_symlink() {
                self.copy_directory(
                    source,
                    &dest,
                    control,
                    progress_tx,
                    &mut bytes_copied,
                    total_bytes,
                    &mut files_copied,
                    total_files,
                    start_time,
                    0,
                )
            } else {
                self.copy_file(
                    source,
                    &dest,
                    control,
                    progress_tx,
                    &mut bytes_copied,
                    total_bytes,
                    &mut files_copied,
                    total_files,
                    start_time,
                )
            };

            match result {
                Ok(()) => {
                    copied_sources.push(source.clone());
                }
                Err(OperationError::Cancelled) => return OperationResult::Cancelled,
                Err(e) => return OperationResult::Failed(e.to_string()),
            }
        }

        // Phase 3: Delete source if move (only for successfully copied sources)
        if self.is_move && !copied_sources.is_empty() {
            let _ = progress_tx.send(OperationProgress {
                phase: OperationPhase::Cleaning,
                bytes_transferred: bytes_copied,
                total_bytes,
                files_completed: files_copied,
                total_files,
                current_item: None,
                speed_bps: 0.0,
                eta_seconds: None,
            });

            for source in &copied_sources {
                if control.is_cancelled() {
                    return OperationResult::Cancelled;
                }

                let result = if source.is_dir() {
                    fs::remove_dir_all(source)
                } else {
                    fs::remove_file(source)
                };

                if let Err(e) = result {
                    return OperationResult::Failed(format!(
                        "Failed to delete source {}: {}",
                        source.display(),
                        e
                    ));
                }
            }
        }

        // Complete
        let _ = progress_tx.send(OperationProgress {
            phase: OperationPhase::Completed,
            bytes_transferred: bytes_copied,
            total_bytes,
            files_completed: files_copied,
            total_files,
            current_item: None,
            speed_bps: 0.0,
            eta_seconds: None,
        });

        if skipped_files > 0 {
            OperationResult::PartialSuccess {
                completed: files_copied,
                skipped: skipped_files,
                failed: 0,
            }
        } else {
            OperationResult::SuccessWithPath(self.destination.clone())
        }
    }
}

/// Worker for local file/directory delete operations.
pub struct LocalDeleteWorker {
    /// Paths to delete.
    paths: Vec<PathBuf>,
}

impl LocalDeleteWorker {
    /// Create a new local delete worker.
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }

    /// Count files in directory.
    #[allow(clippy::only_used_in_recursion)]
    fn count_files(
        &self,
        path: &Path,
        control: &OperationControl,
    ) -> Result<usize, OperationError> {
        control.check_cancelled()?;

        let mut count = 0;
        for entry in fs::read_dir(path)? {
            control.check_cancelled()?;
            let entry = entry?;
            if entry.path().is_dir() {
                count += self.count_files(&entry.path(), control)?;
            } else {
                count += 1;
            }
        }
        // Count directory itself
        count += 1;
        Ok(count)
    }

    /// Delete directory recursively with progress.
    #[allow(clippy::only_used_in_recursion)]
    fn delete_directory(
        &self,
        path: &Path,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
        files_deleted: &mut usize,
        total_files: usize,
    ) -> Result<(), OperationError> {
        control.check_cancelled()?;
        control.wait_if_paused()?;

        for entry in fs::read_dir(path)? {
            control.check_cancelled()?;
            let entry = entry?;
            let entry_path = entry.path();

            // Send progress
            let _ = progress_tx.send(OperationProgress {
                phase: OperationPhase::Cleaning,
                bytes_transferred: 0,
                total_bytes: 0,
                files_completed: *files_deleted,
                total_files,
                current_item: Some(entry_path.display().to_string()),
                speed_bps: 0.0,
                eta_seconds: None,
            });

            if entry_path.is_dir() {
                self.delete_directory(
                    &entry_path,
                    control,
                    progress_tx,
                    files_deleted,
                    total_files,
                )?;
            } else {
                fs::remove_file(&entry_path)?;
                *files_deleted += 1;
            }
        }

        fs::remove_dir(path)?;
        *files_deleted += 1;

        Ok(())
    }
}

impl OperationWorker for LocalDeleteWorker {
    fn execute(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        // Phase 1: Count files
        let _ = progress_tx.send(OperationProgress {
            phase: OperationPhase::Scanning,
            ..Default::default()
        });

        let mut total_files = 0;
        for path in &self.paths {
            if control.is_cancelled() {
                return OperationResult::Cancelled;
            }

            if path.is_dir() {
                match self.count_files(path, control) {
                    Ok(count) => total_files += count,
                    Err(OperationError::Cancelled) => return OperationResult::Cancelled,
                    Err(e) => return OperationResult::Failed(e.to_string()),
                }
            } else {
                total_files += 1;
            }
        }

        // Phase 2: Delete files
        let mut files_deleted = 0;
        for path in &self.paths {
            if control.is_cancelled() {
                return OperationResult::Cancelled;
            }

            let result = if path.is_dir() {
                self.delete_directory(path, control, progress_tx, &mut files_deleted, total_files)
            } else {
                let _ = progress_tx.send(OperationProgress {
                    phase: OperationPhase::Cleaning,
                    bytes_transferred: 0,
                    total_bytes: 0,
                    files_completed: files_deleted,
                    total_files,
                    current_item: Some(path.display().to_string()),
                    speed_bps: 0.0,
                    eta_seconds: None,
                });

                match fs::remove_file(path) {
                    Ok(()) => {
                        files_deleted += 1;
                        Ok(())
                    }
                    Err(e) => Err(OperationError::Io(e.to_string())),
                }
            };

            match result {
                Ok(()) => {}
                Err(OperationError::Cancelled) => return OperationResult::Cancelled,
                Err(e) => return OperationResult::Failed(e.to_string()),
            }
        }

        // Complete
        let _ = progress_tx.send(OperationProgress {
            phase: OperationPhase::Completed,
            bytes_transferred: 0,
            total_bytes: 0,
            files_completed: files_deleted,
            total_files,
            current_item: None,
            speed_bps: 0.0,
            eta_seconds: None,
        });

        OperationResult::Success
    }
}

/// Worker for downloading files from remote to local.
pub struct DownloadWorker {
    /// VFS manager for remote operations.
    vfs_manager: std::sync::Arc<VfsManager>,
    /// Remote source path.
    remote_path: VfsPath,
    /// Local destination path.
    local_path: PathBuf,
}

impl DownloadWorker {
    /// Create a new download worker.
    pub fn new(
        vfs_manager: std::sync::Arc<VfsManager>,
        remote_path: VfsPath,
        local_path: PathBuf,
    ) -> Self {
        Self {
            vfs_manager,
            remote_path,
            local_path,
        }
    }
}

impl OperationWorker for DownloadWorker {
    fn execute(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        // Use VFS download_with_progress
        let operation = self
            .vfs_manager
            .download_with_progress(&self.remote_path, &self.local_path);

        loop {
            // Check for cancellation
            if control.is_cancelled() {
                operation.cancel();
                return OperationResult::Cancelled;
            }

            // Handle pause
            if control.is_paused() {
                operation.set_paused(true);
            } else {
                operation.set_paused(false);
            }

            // Check for completion
            if let Some(result) = operation.try_recv() {
                return match result {
                    Ok(path) => OperationResult::SuccessWithPath(path),
                    Err(e) => {
                        if matches!(e, termide_vfs::VfsError::Cancelled) {
                            OperationResult::Cancelled
                        } else {
                            OperationResult::Failed(e.to_string())
                        }
                    }
                };
            }

            // Forward progress
            if let Some(progress) = operation.drain_progress() {
                let _ = progress_tx.send(OperationProgress {
                    phase: OperationPhase::Transferring,
                    bytes_transferred: progress.bytes_downloaded,
                    total_bytes: progress.total_bytes,
                    files_completed: progress.files_downloaded,
                    total_files: progress.total_files,
                    current_item: progress.current_file,
                    speed_bps: 0.0, // VFS doesn't provide speed, could calculate
                    eta_seconds: None,
                });
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

/// Worker for uploading files from local to remote.
pub struct UploadWorker {
    /// VFS manager for remote operations.
    vfs_manager: std::sync::Arc<VfsManager>,
    /// Local source path.
    local_path: PathBuf,
    /// Remote destination path.
    remote_path: VfsPath,
}

impl UploadWorker {
    /// Create a new upload worker.
    pub fn new(
        vfs_manager: std::sync::Arc<VfsManager>,
        local_path: PathBuf,
        remote_path: VfsPath,
    ) -> Self {
        Self {
            vfs_manager,
            local_path,
            remote_path,
        }
    }
}

impl OperationWorker for UploadWorker {
    fn execute(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        // Use VFS upload_with_progress
        let operation = self
            .vfs_manager
            .upload_with_progress(&self.local_path, &self.remote_path);

        loop {
            // Check for cancellation
            if control.is_cancelled() {
                operation.cancel();
                return OperationResult::Cancelled;
            }

            // Handle pause
            if control.is_paused() {
                operation.set_paused(true);
            } else {
                operation.set_paused(false);
            }

            // Check for completion
            if let Some(result) = operation.try_recv() {
                return match result {
                    Ok(()) => OperationResult::Success,
                    Err(e) => {
                        if matches!(e, termide_vfs::VfsError::Cancelled) {
                            OperationResult::Cancelled
                        } else {
                            OperationResult::Failed(e.to_string())
                        }
                    }
                };
            }

            // Forward progress
            if let Some(progress) = operation.drain_progress() {
                let _ = progress_tx.send(OperationProgress {
                    phase: OperationPhase::Transferring,
                    bytes_transferred: progress.bytes_uploaded,
                    total_bytes: progress.total_bytes,
                    files_completed: 0,
                    total_files: 1,
                    current_item: self
                        .local_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(String::from),
                    speed_bps: 0.0,
                    eta_seconds: None,
                });
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}
