//! Remote file download worker.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use termide_vfs::{VfsManager, VfsPath};

use super::OperationWorker;
use crate::retry::{is_retryable_error, RetryPolicy, RetryState};
use crate::types::{
    OperationControl, OperationError, OperationPhase, OperationProgress, OperationResult,
};
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

    /// Check if a remote path is a directory by trying to list it.
    /// Returns true if listing succeeds (meaning it's a directory).
    /// Returns false if cancelled or not a directory.
    fn is_remote_directory(&self, path: &VfsPath, control: &OperationControl) -> bool {
        // Try to list the path - if it succeeds, it's a directory
        let operation = self.vfs_manager.list_dir(path);

        loop {
            if control.is_cancelled() {
                return false;
            }
            if let Some(result) = operation.try_recv() {
                return result.is_ok();
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

impl OperationWorker for DownloadWorker {
    fn execute(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult {
        // For single source (file or directory), use optimized single-file path
        // which properly proxies VFS progress (real file counts, pause, cancel)
        if self.sources.len() == 1 {
            let is_dir = self.is_remote_directory(&self.sources[0], control);
            if is_dir {
                // Notify UI about scanning phase before VFS starts internal scan
                let _ = progress_tx.send(OperationProgress::scanning());
            }
            return self.execute_single_file(control, progress_tx);
        }

        let total_files = self.sources.len();

        // Multiple files/directories batch mode
        let mut files_completed = 0;
        let mut files_skipped = 0;
        let mut failed_files: Vec<String> = Vec::new();

        // We don't know total bytes upfront for remote files
        // Use file count for progress
        let mut bytes_transferred = 0u64;
        let mut total_bytes_known = 0u64;

        for (idx, source) in self.sources.iter().enumerate() {
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
        let mut speed_tracker = super::SpeedTracker::new();

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
                let bytes_transferred = progress.bytes_downloaded;
                let total_bytes = progress.total_bytes;
                *file_total = total_bytes;

                let current_speed = speed_tracker.update(bytes_transferred);
                let eta_seconds = speed_tracker.eta(bytes_transferred, total_bytes);

                // Detect scanning phase: VFS sends progress with 0 bytes
                // downloaded and 0 files downloaded during file counting
                let phase = if progress.bytes_downloaded == 0
                    && progress.files_downloaded == 0
                    && progress.current_file_total == 0
                {
                    OperationPhase::Scanning
                } else {
                    OperationPhase::Transferring
                };

                on_progress(
                    OperationProgress {
                        phase,
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
        super::poll_vfs_delete(self.vfs_manager.delete(path), control)
    }
}
