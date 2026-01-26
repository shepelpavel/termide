//! Worker trait and implementations for file operations.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Instant, UNIX_EPOCH};

use termide_vfs::{VfsManager, VfsPath};

use crate::retry::{is_retryable_error, RetryPolicy, RetryState};
use crate::types::{
    ConflictInfo, ConflictMode, ConflictResolution, OperationControl, OperationError,
    OperationEvent, OperationId, OperationPath, OperationPhase, OperationProgress, OperationResult,
};

/// Chunk size for file operations (1MB).
const CHUNK_SIZE: usize = 1024 * 1024;

/// Result of conflict check - determines how to handle the file.
#[derive(Debug)]
pub enum ConflictAction {
    /// Proceed with copy/move to original destination.
    Proceed,
    /// Skip this file.
    Skip,
    /// Rename: copy/move to new destination path.
    RenameAs(PathBuf),
}

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
    /// Returns: ConflictAction indicating how to proceed.
    pub fn check_conflict(
        &mut self,
        source: &Path,
        dest: &Path,
        remaining_items: usize,
    ) -> Result<ConflictAction, OperationError> {
        // Check if destination exists
        if !dest.exists() {
            return Ok(ConflictAction::Proceed); // No conflict
        }

        // Handle based on current mode
        match self.conflict_mode {
            ConflictMode::OverwriteAll => Ok(ConflictAction::Proceed),
            ConflictMode::SkipAll => Ok(ConflictAction::Skip),
            ConflictMode::RenameAll => {
                // Auto-generate a unique name
                let new_dest = generate_unique_path(dest);
                Ok(ConflictAction::RenameAs(new_dest))
            }
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
                        ConflictResolution::Overwrite => Ok(ConflictAction::Proceed),
                        ConflictResolution::Skip => Ok(ConflictAction::Skip),
                        ConflictResolution::Rename(new_name) => {
                            // Use the user-provided new name
                            let new_dest = dest.parent().unwrap_or(Path::new("")).join(&new_name);
                            Ok(ConflictAction::RenameAs(new_dest))
                        }
                        ConflictResolution::OverwriteAll => {
                            self.conflict_mode = ConflictMode::OverwriteAll;
                            Ok(ConflictAction::Proceed)
                        }
                        ConflictResolution::SkipAll => {
                            self.conflict_mode = ConflictMode::SkipAll;
                            Ok(ConflictAction::Skip)
                        }
                        ConflictResolution::RenameAll => {
                            self.conflict_mode = ConflictMode::RenameAll;
                            let new_dest = generate_unique_path(dest);
                            Ok(ConflictAction::RenameAs(new_dest))
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

/// Generate a unique path by appending a number suffix.
/// Example: "file.txt" -> "file (1).txt", "file (1).txt" -> "file (2).txt"
fn generate_unique_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(Path::new(""));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let extension = path.extension().and_then(|e| e.to_str());

    for i in 1..1000 {
        let new_name = if let Some(ext) = extension {
            format!("{} ({}).{}", stem, i, ext)
        } else {
            format!("{} ({})", stem, i)
        };
        let new_path = parent.join(&new_name);
        if !new_path.exists() {
            return new_path;
        }
    }

    // Fallback: use timestamp
    let timestamp = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let new_name = if let Some(ext) = extension {
        format!("{}_{}.{}", stem, timestamp, ext)
    } else {
        format!("{}_{}", stem, timestamp)
    };
    parent.join(&new_name)
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
                    individual_file_bytes: 0,
                    individual_file_total: 0,
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
        let _ = progress_tx.send(OperationProgress::scanning());

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
            // Determine final destination (may be renamed)
            let final_dest = if let Some(ref mut ctx) = conflict_ctx {
                let remaining = total_files.saturating_sub(files_copied + skipped_files);
                match ctx.check_conflict(source, &dest, remaining) {
                    Ok(ConflictAction::Proceed) => dest.clone(),
                    Ok(ConflictAction::Skip) => {
                        // Skip this item
                        skipped_files += 1;
                        continue;
                    }
                    Ok(ConflictAction::RenameAs(new_dest)) => new_dest,
                    Err(OperationError::Cancelled) => return OperationResult::Cancelled,
                    Err(e) => return OperationResult::Failed(e.to_string()),
                }
            } else {
                dest.clone()
            };

            let result = if metadata.is_dir() && !metadata.is_symlink() {
                self.copy_directory(
                    source,
                    &final_dest,
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
                    &final_dest,
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
                individual_file_bytes: 0,
                individual_file_total: 0,
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
        let _ = progress_tx.send(OperationProgress::completed(
            bytes_copied,
            files_copied,
            total_files,
        ));

        if skipped_files > 0 {
            OperationResult::PartialSuccess {
                completed: files_copied,
                skipped: skipped_files,
                failed: 0,
                failed_files: Vec::new(),
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
                individual_file_bytes: 0,
                individual_file_total: 0,
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
        let _ = progress_tx.send(OperationProgress::scanning());

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
                    individual_file_bytes: 0,
                    individual_file_total: 0,
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
        let _ = progress_tx.send(OperationProgress::completed(0, files_deleted, total_files));

        OperationResult::Success
    }
}

/// Worker for downloading files from remote to local.
///
/// Supports both single and multiple file downloads. For single file:
/// use `sources` with one element. For batch: use multiple sources.
pub struct DownloadWorker {
    /// VFS manager for remote operations.
    vfs_manager: std::sync::Arc<VfsManager>,
    /// Remote source paths.
    sources: Vec<VfsPath>,
    /// Local destination directory (or file path for single download).
    dest_dir: PathBuf,
    /// Whether to delete sources after download (move).
    is_move: bool,
    /// Retry policy for network errors.
    retry_policy: RetryPolicy,
}

impl DownloadWorker {
    /// Create a new download worker.
    pub fn new(
        vfs_manager: std::sync::Arc<VfsManager>,
        sources: Vec<VfsPath>,
        dest_dir: PathBuf,
        is_move: bool,
    ) -> Self {
        Self {
            vfs_manager,
            sources,
            dest_dir,
            is_move,
            retry_policy: RetryPolicy::network(),
        }
    }

    /// Create a new download worker with custom retry policy.
    pub fn with_retry_policy(
        vfs_manager: std::sync::Arc<VfsManager>,
        sources: Vec<VfsPath>,
        dest_dir: PathBuf,
        is_move: bool,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            vfs_manager,
            sources,
            dest_dir,
            is_move,
            retry_policy,
        }
    }
}

impl OperationWorker for DownloadWorker {
    fn execute(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        let total_files = self.sources.len();

        // Single file case: simpler path
        if total_files == 1 {
            return self.execute_single_file(control, progress_tx);
        }

        // Multiple files: batch mode
        let mut files_completed = 0;
        let mut files_skipped = 0;
        let mut failed_files: Vec<String> = Vec::new();

        // We don't know total bytes upfront for remote files
        // Use file count for progress
        let mut bytes_transferred = 0u64;
        let mut total_bytes_known = 0u64;

        for (idx, source) in self.sources.clone().iter().enumerate() {
            if control.is_cancelled() {
                return OperationResult::Cancelled;
            }

            let file_name = source
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file");
            let dest_path = self.dest_dir.join(file_name);

            // Report progress
            let _ = progress_tx.send(OperationProgress {
                phase: OperationPhase::Transferring,
                bytes_transferred,
                total_bytes: total_bytes_known,
                files_completed,
                total_files,
                current_item: Some(format!("{} ({}/{})", file_name, idx + 1, total_files)),
                speed_bps: 0.0,
                eta_seconds: None,
                individual_file_bytes: 0,
                individual_file_total: 0, // Unknown until download starts
            });

            // Track last total_bytes from this file's progress
            let mut last_file_total = 0u64;

            // Download this file with retry
            let result = self.download_single_with_retry(
                source,
                &dest_path,
                control,
                progress_tx,
                |sub_progress, file_total_ref| {
                    // Update total bytes as we learn it
                    *file_total_ref = sub_progress.total_bytes;
                    let new_total =
                        total_bytes_known.max(bytes_transferred + sub_progress.total_bytes);

                    let _ = progress_tx.send(OperationProgress {
                        phase: OperationPhase::Transferring,
                        bytes_transferred: bytes_transferred + sub_progress.bytes_transferred,
                        total_bytes: new_total,
                        files_completed,
                        total_files,
                        current_item: Some(format!("{} ({}/{})", file_name, idx + 1, total_files)),
                        speed_bps: sub_progress.speed_bps,
                        eta_seconds: sub_progress.eta_seconds,
                        individual_file_bytes: sub_progress.bytes_transferred,
                        individual_file_total: sub_progress.total_bytes,
                    });
                },
                &mut last_file_total,
            );

            match result {
                OperationResult::Success | OperationResult::SuccessWithPath(_) => {
                    files_completed += 1;
                    bytes_transferred += last_file_total;
                    total_bytes_known = total_bytes_known.max(bytes_transferred);

                    // If move, delete remote source
                    if self.is_move {
                        if let Err(e) = self.delete_remote_source(source, control) {
                            failed_files
                                .push(format!("{}: failed to delete source: {}", file_name, e));
                        }
                    }
                }
                OperationResult::Cancelled => {
                    return OperationResult::Cancelled;
                }
                OperationResult::PartialSuccess { skipped, .. } => {
                    files_completed += 1;
                    files_skipped += skipped;
                }
                OperationResult::Failed(err) => {
                    failed_files.push(format!("{}: {}", file_name, err));
                }
            }
        }

        // Final progress
        let _ = progress_tx.send(OperationProgress::completed(
            bytes_transferred,
            files_completed,
            total_files,
        ));

        if failed_files.is_empty() {
            OperationResult::Success
        } else if files_completed > 0 {
            OperationResult::PartialSuccess {
                completed: files_completed,
                skipped: files_skipped,
                failed: failed_files.len(),
                failed_files,
            }
        } else {
            OperationResult::Failed(failed_files.join("; "))
        }
    }
}

impl DownloadWorker {
    /// Execute download for a single file (optimized path).
    fn execute_single_file(
        &self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        let source = &self.sources[0];
        let dest_path = self.dest_dir.clone();
        let mut _file_total = 0u64;

        let result = self.download_single_with_retry(
            source,
            &dest_path,
            control,
            progress_tx,
            |progress, _| {
                let _ = progress_tx.send(progress);
            },
            &mut _file_total,
        );

        // Handle move cleanup for single file
        if self.is_move && result.is_success() {
            if let Err(e) = self.delete_remote_source(source, control) {
                return OperationResult::Failed(format!(
                    "Download succeeded but failed to delete source: {}",
                    e
                ));
            }
        }

        result
    }

    /// Download a single file with retry logic.
    fn download_single_with_retry<F>(
        &self,
        source: &VfsPath,
        dest: &Path,
        control: &OperationControl,
        _progress_tx: &mpsc::Sender<OperationProgress>,
        on_progress: F,
        file_total: &mut u64,
    ) -> OperationResult
    where
        F: Fn(OperationProgress, &mut u64),
    {
        let mut retry_state = RetryState::new(self.retry_policy.clone());

        loop {
            if control.is_cancelled() {
                return OperationResult::Cancelled;
            }

            // Wait for retry delay if this is a retry attempt
            if retry_state.attempt() > 0 {
                let delay = retry_state.next_delay();
                std::thread::sleep(delay);
            }

            let result =
                self.execute_download_attempt(source, dest, control, &on_progress, file_total);

            match &result {
                OperationResult::Failed(error_msg) => {
                    if is_retryable_error(error_msg) && retry_state.record_failure(error_msg) {
                        continue; // Retry
                    }
                    return result;
                }
                _ => return result,
            }
        }
    }

    /// Execute a single download attempt.
    fn execute_download_attempt<F>(
        &self,
        source: &VfsPath,
        dest: &Path,
        control: &OperationControl,
        on_progress: &F,
        file_total: &mut u64,
    ) -> OperationResult
    where
        F: Fn(OperationProgress, &mut u64),
    {
        // Use VFS download_with_progress
        let operation = self.vfs_manager.download_with_progress(source, dest);

        // Speed/ETA tracking
        let start_time = Instant::now();
        let mut last_bytes = 0u64;
        let mut last_time = start_time;
        let mut current_speed = 0.0f64;

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

            // Forward progress with speed/ETA calculation
            if let Some(progress) = operation.drain_progress() {
                let now = Instant::now();
                let bytes_transferred = progress.bytes_downloaded;
                let total_bytes = progress.total_bytes;
                *file_total = total_bytes;

                // Calculate speed using delta over interval (smoother than total average)
                let elapsed_since_last = now.duration_since(last_time).as_secs_f64();
                if elapsed_since_last >= 0.2 {
                    // Update speed every 200ms
                    let delta_bytes = bytes_transferred.saturating_sub(last_bytes);
                    if elapsed_since_last > 0.0 {
                        // Smooth speed using exponential moving average
                        let instant_speed = delta_bytes as f64 / elapsed_since_last;
                        current_speed = if current_speed > 0.0 {
                            current_speed * 0.7 + instant_speed * 0.3
                        } else {
                            instant_speed
                        };
                    }
                    last_bytes = bytes_transferred;
                    last_time = now;
                }

                // Calculate ETA
                let eta_seconds = if current_speed > 0.0 && total_bytes > bytes_transferred {
                    let remaining_bytes = total_bytes - bytes_transferred;
                    Some((remaining_bytes as f64 / current_speed) as u64)
                } else {
                    None
                };

                on_progress(
                    OperationProgress {
                        phase: OperationPhase::Transferring,
                        bytes_transferred,
                        total_bytes,
                        files_completed: progress.files_downloaded,
                        total_files: progress.total_files,
                        current_item: progress.current_file,
                        speed_bps: current_speed,
                        eta_seconds,
                        individual_file_bytes: bytes_transferred,
                        individual_file_total: total_bytes,
                    },
                    file_total,
                );
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// Delete remote source after successful download.
    fn delete_remote_source(
        &self,
        path: &VfsPath,
        control: &OperationControl,
    ) -> Result<(), OperationError> {
        let operation = self.vfs_manager.delete(path);

        loop {
            if control.is_cancelled() {
                return Err(OperationError::Cancelled);
            }

            if let Some(result) = operation.try_recv() {
                return match result {
                    Ok(()) => Ok(()),
                    Err(e) => Err(OperationError::Vfs(e.to_string())),
                };
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

/// Worker for uploading files from local to remote.
///
/// Supports both single and multiple file uploads. For single file:
/// use `sources` with one element. For batch: use multiple sources.
pub struct UploadWorker {
    /// VFS manager for remote operations.
    vfs_manager: std::sync::Arc<VfsManager>,
    /// Local source paths.
    sources: Vec<PathBuf>,
    /// Remote destination base path (directory for batch, file for single).
    dest_base: VfsPath,
    /// Whether to delete sources after upload (move).
    is_move: bool,
    /// Retry policy for network errors.
    retry_policy: RetryPolicy,
}

impl UploadWorker {
    /// Create a new upload worker.
    pub fn new(
        vfs_manager: std::sync::Arc<VfsManager>,
        sources: Vec<PathBuf>,
        dest_base: VfsPath,
        is_move: bool,
    ) -> Self {
        Self {
            vfs_manager,
            sources,
            dest_base,
            is_move,
            retry_policy: RetryPolicy::network(),
        }
    }

    /// Create a new upload worker with custom retry policy.
    pub fn with_retry_policy(
        vfs_manager: std::sync::Arc<VfsManager>,
        sources: Vec<PathBuf>,
        dest_base: VfsPath,
        is_move: bool,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            vfs_manager,
            sources,
            dest_base,
            is_move,
            retry_policy,
        }
    }
}

impl OperationWorker for UploadWorker {
    fn execute(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        let total_files = self.sources.len();

        // Single file case: simpler path
        if total_files == 1 {
            return self.execute_single_file(control, progress_tx);
        }

        // Multiple files: batch mode
        let mut files_completed = 0;
        let mut files_skipped = 0;
        let mut failed_files: Vec<String> = Vec::new();

        // Calculate total size for overall progress
        let total_bytes: u64 = self
            .sources
            .iter()
            .filter_map(|p| fs::metadata(p).ok().map(|m| m.len()))
            .sum();
        let mut bytes_transferred = 0u64;

        for (idx, source) in self.sources.clone().iter().enumerate() {
            if control.is_cancelled() {
                return OperationResult::Cancelled;
            }

            // Build destination path
            let file_name = source
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file");
            let dest_path = self.dest_base.join(file_name);
            let file_size = fs::metadata(source).map(|m| m.len()).unwrap_or(0);

            // Report progress before starting this file
            let _ = progress_tx.send(OperationProgress {
                phase: OperationPhase::Transferring,
                bytes_transferred,
                total_bytes,
                files_completed,
                total_files,
                current_item: Some(format!("{} ({}/{})", file_name, idx + 1, total_files)),
                speed_bps: 0.0,
                eta_seconds: None,
                individual_file_bytes: 0,
                individual_file_total: file_size,
            });

            // Upload this file with retry
            let result = self.upload_single_with_retry(
                source,
                &dest_path,
                control,
                progress_tx,
                |sub_progress| {
                    // Send aggregated progress
                    let _ = progress_tx.send(OperationProgress {
                        phase: OperationPhase::Transferring,
                        bytes_transferred: bytes_transferred + sub_progress.bytes_transferred,
                        total_bytes,
                        files_completed,
                        total_files,
                        current_item: Some(format!("{} ({}/{})", file_name, idx + 1, total_files)),
                        speed_bps: sub_progress.speed_bps,
                        eta_seconds: if sub_progress.speed_bps > 0.0 {
                            let remaining = total_bytes
                                .saturating_sub(bytes_transferred + sub_progress.bytes_transferred);
                            Some((remaining as f64 / sub_progress.speed_bps) as u64)
                        } else {
                            None
                        },
                        individual_file_bytes: sub_progress.bytes_transferred,
                        individual_file_total: file_size,
                    });
                },
            );

            match result {
                OperationResult::Success | OperationResult::SuccessWithPath(_) => {
                    files_completed += 1;
                    bytes_transferred += file_size;

                    // If move, delete source
                    if self.is_move {
                        if let Err(e) = fs::remove_file(source) {
                            // Log but don't fail the batch
                            failed_files
                                .push(format!("{}: failed to delete source: {}", file_name, e));
                        }
                    }
                }
                OperationResult::Cancelled => {
                    return OperationResult::Cancelled;
                }
                OperationResult::PartialSuccess { skipped, .. } => {
                    files_completed += 1;
                    files_skipped += skipped;
                    bytes_transferred += file_size;
                }
                OperationResult::Failed(err) => {
                    failed_files.push(format!("{}: {}", file_name, err));
                }
            }
        }

        // Final progress
        let _ = progress_tx.send(OperationProgress::completed(
            total_bytes,
            files_completed,
            total_files,
        ));

        if failed_files.is_empty() {
            OperationResult::Success
        } else if files_completed > 0 {
            OperationResult::PartialSuccess {
                completed: files_completed,
                skipped: files_skipped,
                failed: failed_files.len(),
                failed_files,
            }
        } else {
            OperationResult::Failed(failed_files.join("; "))
        }
    }
}

impl UploadWorker {
    /// Execute upload for a single file (optimized path).
    fn execute_single_file(
        &self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        let source = &self.sources[0];
        let dest_path = if self.sources.len() == 1 {
            // For single file, dest_base could be full path or directory
            // Check if it looks like a directory by trying to use the filename
            self.dest_base.clone()
        } else {
            let file_name = source
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file");
            self.dest_base.join(file_name)
        };

        let result =
            self.upload_single_with_retry(source, &dest_path, control, progress_tx, |progress| {
                let _ = progress_tx.send(progress);
            });

        // Handle move cleanup for single file
        if self.is_move && result.is_success() {
            if let Err(e) = fs::remove_file(source) {
                return OperationResult::Failed(format!(
                    "Upload succeeded but failed to delete source: {}",
                    e
                ));
            }
        }

        result
    }

    /// Upload a single file with retry logic.
    fn upload_single_with_retry<F>(
        &self,
        source: &Path,
        dest: &VfsPath,
        control: &OperationControl,
        _progress_tx: &mpsc::Sender<OperationProgress>,
        on_progress: F,
    ) -> OperationResult
    where
        F: Fn(OperationProgress),
    {
        let mut retry_state = RetryState::new(self.retry_policy.clone());

        loop {
            if control.is_cancelled() {
                return OperationResult::Cancelled;
            }

            // Wait for retry delay if this is a retry attempt
            if retry_state.attempt() > 0 {
                let delay = retry_state.next_delay();
                std::thread::sleep(delay);
            }

            let result = self.execute_upload_attempt(source, dest, control, &on_progress);

            match &result {
                OperationResult::Failed(error_msg) => {
                    if is_retryable_error(error_msg) && retry_state.record_failure(error_msg) {
                        continue; // Retry
                    }
                    return result;
                }
                _ => return result,
            }
        }
    }

    /// Execute a single upload attempt.
    fn execute_upload_attempt<F>(
        &self,
        source: &Path,
        dest: &VfsPath,
        control: &OperationControl,
        on_progress: &F,
    ) -> OperationResult
    where
        F: Fn(OperationProgress),
    {
        // Use VFS upload_with_progress
        let operation = self.vfs_manager.upload_with_progress(source, dest);

        // Speed/ETA tracking
        let start_time = Instant::now();
        let mut last_bytes = 0u64;
        let mut last_time = start_time;
        let mut current_speed = 0.0f64;

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

            // Forward progress with speed/ETA calculation
            if let Some(progress) = operation.drain_progress() {
                let now = Instant::now();
                let bytes_transferred = progress.bytes_uploaded;
                let total_bytes = progress.total_bytes;

                // Calculate speed using delta over interval (smoother than total average)
                let elapsed_since_last = now.duration_since(last_time).as_secs_f64();
                if elapsed_since_last >= 0.2 {
                    // Update speed every 200ms
                    let delta_bytes = bytes_transferred.saturating_sub(last_bytes);
                    if elapsed_since_last > 0.0 {
                        // Smooth speed using exponential moving average
                        let instant_speed = delta_bytes as f64 / elapsed_since_last;
                        current_speed = if current_speed > 0.0 {
                            current_speed * 0.7 + instant_speed * 0.3
                        } else {
                            instant_speed
                        };
                    }
                    last_bytes = bytes_transferred;
                    last_time = now;
                }

                // Calculate ETA
                let eta_seconds = if current_speed > 0.0 && total_bytes > bytes_transferred {
                    let remaining_bytes = total_bytes - bytes_transferred;
                    Some((remaining_bytes as f64 / current_speed) as u64)
                } else {
                    None
                };

                on_progress(OperationProgress {
                    phase: OperationPhase::Transferring,
                    bytes_transferred,
                    total_bytes,
                    files_completed: 0,
                    total_files: 1,
                    current_item: source
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(String::from),
                    speed_bps: current_speed,
                    eta_seconds,
                    individual_file_bytes: bytes_transferred,
                    individual_file_total: total_bytes,
                });
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

/// Worker for deleting remote files/directories.
pub struct RemoteDeleteWorker {
    /// VFS manager for remote operations.
    vfs_manager: std::sync::Arc<VfsManager>,
    /// Remote paths to delete.
    paths: Vec<VfsPath>,
    /// Retry policy for network errors.
    retry_policy: RetryPolicy,
}

impl RemoteDeleteWorker {
    /// Create a new remote delete worker.
    pub fn new(vfs_manager: std::sync::Arc<VfsManager>, paths: Vec<VfsPath>) -> Self {
        Self {
            vfs_manager,
            paths,
            retry_policy: RetryPolicy::network(),
        }
    }

    /// Create a new remote delete worker with custom retry policy.
    pub fn with_retry_policy(
        vfs_manager: std::sync::Arc<VfsManager>,
        paths: Vec<VfsPath>,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            vfs_manager,
            paths,
            retry_policy,
        }
    }
}

impl OperationWorker for RemoteDeleteWorker {
    fn execute(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        let total_files = self.paths.len();
        let mut files_deleted = 0;
        let mut failed_files: Vec<String> = Vec::new();

        for path in &self.paths {
            if control.is_cancelled() {
                return OperationResult::Cancelled;
            }

            // Send progress
            let _ = progress_tx.send(OperationProgress {
                phase: OperationPhase::Cleaning,
                bytes_transferred: 0,
                total_bytes: 0,
                files_completed: files_deleted,
                total_files,
                current_item: path.file_name().and_then(|n| n.to_str()).map(String::from),
                speed_bps: 0.0,
                eta_seconds: None,
                individual_file_bytes: 0,
                individual_file_total: 0,
            });

            // Attempt delete with retry
            let result = self.delete_with_retry(path, control);

            match result {
                Ok(()) => {
                    files_deleted += 1;
                }
                Err(OperationError::Cancelled) => {
                    return OperationResult::Cancelled;
                }
                Err(e) => {
                    failed_files.push(format!("{}: {}", path.to_url_string(), e));
                }
            }
        }

        // Send final progress
        let _ = progress_tx.send(OperationProgress::completed(0, files_deleted, total_files));

        if failed_files.is_empty() {
            OperationResult::Success
        } else if files_deleted > 0 {
            OperationResult::PartialSuccess {
                completed: files_deleted,
                skipped: 0,
                failed: failed_files.len(),
                failed_files,
            }
        } else {
            OperationResult::Failed(failed_files.join("; "))
        }
    }
}

impl RemoteDeleteWorker {
    /// Delete a single path with retry logic.
    fn delete_with_retry(
        &self,
        path: &VfsPath,
        control: &OperationControl,
    ) -> Result<(), OperationError> {
        let mut retry_state = RetryState::new(self.retry_policy.clone());

        loop {
            if control.is_cancelled() {
                return Err(OperationError::Cancelled);
            }

            // Wait for retry delay if this is a retry attempt
            if retry_state.attempt() > 0 {
                let delay = retry_state.next_delay();
                std::thread::sleep(delay);
            }

            // Attempt the delete
            let result = self.execute_delete_attempt(path, control);

            match &result {
                Err(OperationError::Vfs(error_msg)) | Err(OperationError::Io(error_msg)) => {
                    // Check if error is retryable and we have retries left
                    if is_retryable_error(error_msg) && retry_state.record_failure(error_msg) {
                        continue; // Retry
                    }
                    return result;
                }
                _ => return result,
            }
        }
    }

    /// Execute a single delete attempt.
    fn execute_delete_attempt(
        &self,
        path: &VfsPath,
        control: &OperationControl,
    ) -> Result<(), OperationError> {
        // Use VFS delete (which handles both files and directories)
        let operation = self.vfs_manager.delete(path);

        loop {
            if control.is_cancelled() {
                return Err(OperationError::Cancelled);
            }

            // Check for completion
            if let Some(result) = operation.try_recv() {
                return match result {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        if matches!(e, termide_vfs::VfsError::Cancelled) {
                            Err(OperationError::Cancelled)
                        } else {
                            Err(OperationError::Vfs(e.to_string()))
                        }
                    }
                };
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

/// Direction of cross-protocol transfer.
#[derive(Debug, Clone, Copy)]
pub enum CrossProtocolDirection {
    /// Download: Remote → Local
    Download,
    /// Upload: Local → Remote
    Upload,
    /// Remote to Remote (via temp file)
    RemoteToRemote,
}

/// Worker for cross-protocol copy/move operations.
///
/// Handles transfers between local and remote filesystems:
/// - Remote → Local: uses VFS download
/// - Local → Remote: uses VFS upload
/// - Remote → Remote: download to temp, then upload
pub struct CrossProtocolWorker {
    /// VFS manager for remote operations.
    vfs_manager: std::sync::Arc<VfsManager>,
    /// Source path.
    source: OperationPath,
    /// Destination path.
    destination: OperationPath,
    /// Whether to delete source after copy (move).
    is_move: bool,
    /// Transfer direction.
    direction: CrossProtocolDirection,
    /// Retry policy for network errors.
    retry_policy: RetryPolicy,
}

impl CrossProtocolWorker {
    /// Create a new cross-protocol worker.
    ///
    /// Automatically determines the transfer direction based on source/destination types.
    pub fn new(
        vfs_manager: std::sync::Arc<VfsManager>,
        source: OperationPath,
        destination: OperationPath,
        is_move: bool,
    ) -> Result<Self, OperationError> {
        let direction = match (&source, &destination) {
            (OperationPath::Remote(_), OperationPath::Local(_)) => CrossProtocolDirection::Download,
            (OperationPath::Local(_), OperationPath::Remote(_)) => CrossProtocolDirection::Upload,
            (OperationPath::Remote(_), OperationPath::Remote(_)) => {
                CrossProtocolDirection::RemoteToRemote
            }
            (OperationPath::Local(_), OperationPath::Local(_)) => {
                return Err(OperationError::Invalid(
                    "Use LocalCopyWorker for local-to-local transfers".to_string(),
                ));
            }
        };

        Ok(Self {
            vfs_manager,
            source,
            destination,
            is_move,
            direction,
            retry_policy: RetryPolicy::network(),
        })
    }

    /// Create with custom retry policy.
    pub fn with_retry_policy(
        vfs_manager: std::sync::Arc<VfsManager>,
        source: OperationPath,
        destination: OperationPath,
        is_move: bool,
        retry_policy: RetryPolicy,
    ) -> Result<Self, OperationError> {
        let mut worker = Self::new(vfs_manager, source, destination, is_move)?;
        worker.retry_policy = retry_policy;
        Ok(worker)
    }
}

impl OperationWorker for CrossProtocolWorker {
    fn execute(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        match self.direction {
            CrossProtocolDirection::Download => self.execute_download(control, progress_tx),
            CrossProtocolDirection::Upload => self.execute_upload(control, progress_tx),
            CrossProtocolDirection::RemoteToRemote => {
                self.execute_remote_to_remote(control, progress_tx)
            }
        }
    }
}

impl CrossProtocolWorker {
    /// Execute download (remote → local).
    fn execute_download(
        &self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        let remote_path = match &self.source {
            OperationPath::Remote(p) => p.clone(),
            _ => return OperationResult::Failed("Source must be remote for download".to_string()),
        };

        let local_path = match &self.destination {
            OperationPath::Local(p) => p.clone(),
            _ => {
                return OperationResult::Failed(
                    "Destination must be local for download".to_string(),
                )
            }
        };

        // Use DownloadWorker (pass false for is_move - we handle move ourselves)
        let mut worker = DownloadWorker::with_retry_policy(
            std::sync::Arc::clone(&self.vfs_manager),
            vec![remote_path.clone()],
            local_path,
            false, // is_move handled by CrossProtocolWorker
            self.retry_policy.clone(),
        );

        let result = worker.execute(control, progress_tx);

        // If move and successful, delete source
        if self.is_move && result.is_success() {
            if let Err(e) = self.delete_remote_source(&remote_path, control) {
                return OperationResult::Failed(format!(
                    "Copy succeeded but failed to delete source: {}",
                    e
                ));
            }
        }

        result
    }

    /// Execute upload (local → remote).
    fn execute_upload(
        &self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        let local_path = match &self.source {
            OperationPath::Local(p) => p.clone(),
            _ => return OperationResult::Failed("Source must be local for upload".to_string()),
        };

        let remote_path = match &self.destination {
            OperationPath::Remote(p) => p.clone(),
            _ => {
                return OperationResult::Failed("Destination must be remote for upload".to_string())
            }
        };

        // Use UploadWorker (pass false for is_move - we handle move ourselves)
        let mut worker = UploadWorker::with_retry_policy(
            std::sync::Arc::clone(&self.vfs_manager),
            vec![local_path.clone()],
            remote_path,
            false, // is_move handled by CrossProtocolWorker
            self.retry_policy.clone(),
        );

        let result = worker.execute(control, progress_tx);

        // If move and successful, delete local source
        if self.is_move && result.is_success() {
            if let Err(e) = self.delete_local_source(&local_path) {
                return OperationResult::Failed(format!(
                    "Copy succeeded but failed to delete source: {}",
                    e
                ));
            }
        }

        result
    }

    /// Execute remote-to-remote transfer via temp file.
    fn execute_remote_to_remote(
        &self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        let source_path = match &self.source {
            OperationPath::Remote(p) => p.clone(),
            _ => return OperationResult::Failed("Source must be remote".to_string()),
        };

        let dest_path = match &self.destination {
            OperationPath::Remote(p) => p.clone(),
            _ => return OperationResult::Failed("Destination must be remote".to_string()),
        };

        // Create temp directory for intermediate file
        let temp_dir = std::env::temp_dir().join(format!("termide_xfer_{}", std::process::id()));
        if let Err(e) = std::fs::create_dir_all(&temp_dir) {
            return OperationResult::Failed(format!("Failed to create temp directory: {}", e));
        }

        let file_name = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("transfer_file");
        let temp_file = temp_dir.join(file_name);

        // Phase 1: Download to temp
        let _ = progress_tx.send(OperationProgress {
            phase: OperationPhase::Transferring,
            current_item: Some(format!("Downloading {} (1/2)", file_name)),
            ..Default::default()
        });

        let mut download_worker = DownloadWorker::with_retry_policy(
            std::sync::Arc::clone(&self.vfs_manager),
            vec![source_path.clone()],
            temp_file.clone(),
            false, // is_move handled by CrossProtocolWorker
            self.retry_policy.clone(),
        );

        let download_result = download_worker.execute(control, progress_tx);
        if !download_result.is_success() {
            let _ = std::fs::remove_dir_all(&temp_dir);
            return download_result;
        }

        if control.is_cancelled() {
            let _ = std::fs::remove_dir_all(&temp_dir);
            return OperationResult::Cancelled;
        }

        // Phase 2: Upload from temp
        let _ = progress_tx.send(OperationProgress {
            phase: OperationPhase::Transferring,
            current_item: Some(format!("Uploading {} (2/2)", file_name)),
            ..Default::default()
        });

        let mut upload_worker = UploadWorker::with_retry_policy(
            std::sync::Arc::clone(&self.vfs_manager),
            vec![temp_file.clone()],
            dest_path,
            false, // is_move handled by CrossProtocolWorker (temp files deleted separately)
            self.retry_policy.clone(),
        );

        let upload_result = upload_worker.execute(control, progress_tx);

        // Clean up temp file
        let _ = std::fs::remove_dir_all(&temp_dir);

        if !upload_result.is_success() {
            return upload_result;
        }

        // If move and successful, delete remote source
        if self.is_move {
            if let Err(e) = self.delete_remote_source(&source_path, control) {
                return OperationResult::Failed(format!(
                    "Copy succeeded but failed to delete source: {}",
                    e
                ));
            }
        }

        OperationResult::Success
    }

    /// Delete remote source file after successful move.
    fn delete_remote_source(
        &self,
        path: &VfsPath,
        control: &OperationControl,
    ) -> Result<(), OperationError> {
        let operation = self.vfs_manager.delete(path);

        loop {
            if control.is_cancelled() {
                return Err(OperationError::Cancelled);
            }

            if let Some(result) = operation.try_recv() {
                return match result {
                    Ok(()) => Ok(()),
                    Err(e) => Err(OperationError::Vfs(e.to_string())),
                };
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// Delete local source file after successful move.
    fn delete_local_source(&self, path: &PathBuf) -> Result<(), OperationError> {
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}
