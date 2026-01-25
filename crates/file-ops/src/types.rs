//! Unified type definitions for file operations.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use termide_vfs::VfsPath;

/// Unique identifier for an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OperationId(pub u64);

impl OperationId {
    /// Create a new operation ID.
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for OperationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "op-{}", self.0)
    }
}

/// Phase of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationPhase {
    /// Scanning/counting files and calculating total size.
    Scanning,
    /// Actively transferring data.
    Transferring,
    /// Cleaning up (e.g., deleting source for move operations).
    Cleaning,
    /// Operation completed successfully.
    Completed,
    /// Operation failed.
    Failed,
    /// Operation was cancelled.
    Cancelled,
}

/// Type of file operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationType {
    /// Copy file(s).
    Copy,
    /// Move file(s) (copy + delete source).
    Move,
    /// Delete file(s).
    Delete,
    /// Download from remote to local.
    Download,
    /// Upload from local to remote.
    Upload,
}

/// Priority level for operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum OperationPriority {
    /// Low priority - background operations.
    Low = 0,
    /// Normal priority - user-initiated operations.
    #[default]
    Normal = 1,
    /// High priority - urgent operations.
    High = 2,
    /// Immediate - bypass queue.
    Immediate = 3,
}

/// Unified path that can be local or remote.
#[derive(Debug, Clone)]
pub enum OperationPath {
    /// Local filesystem path.
    Local(PathBuf),
    /// Remote VFS path.
    Remote(VfsPath),
}

impl OperationPath {
    /// Create a local path.
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self::Local(path.into())
    }

    /// Create a remote path.
    pub fn remote(path: VfsPath) -> Self {
        Self::Remote(path)
    }

    /// Check if this is a local path.
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local(_))
    }

    /// Check if this is a remote path.
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote(_))
    }

    /// Get the file name.
    pub fn file_name(&self) -> Option<String> {
        match self {
            Self::Local(p) => p.file_name().and_then(|n| n.to_str()).map(String::from),
            Self::Remote(p) => p.file_name().and_then(|n| n.to_str()).map(String::from),
        }
    }

    /// Convert to display string.
    pub fn display(&self) -> String {
        match self {
            Self::Local(p) => p.display().to_string(),
            Self::Remote(p) => p.to_string(),
        }
    }
}

impl From<PathBuf> for OperationPath {
    fn from(path: PathBuf) -> Self {
        Self::Local(path)
    }
}

impl From<VfsPath> for OperationPath {
    fn from(path: VfsPath) -> Self {
        if path.is_local() {
            Self::Local(path.path.clone())
        } else {
            Self::Remote(path)
        }
    }
}

/// Unified progress information for all operations.
#[derive(Debug, Clone)]
pub struct OperationProgress {
    /// Current phase of the operation.
    pub phase: OperationPhase,
    /// Bytes transferred so far.
    pub bytes_transferred: u64,
    /// Total bytes to transfer (0 if unknown).
    pub total_bytes: u64,
    /// Files completed so far.
    pub files_completed: usize,
    /// Total files to process (0 if unknown).
    pub total_files: usize,
    /// Current item being processed.
    pub current_item: Option<String>,
    /// Current transfer speed in bytes per second.
    pub speed_bps: f64,
    /// Estimated time remaining in seconds.
    pub eta_seconds: Option<u64>,
}

impl Default for OperationProgress {
    fn default() -> Self {
        Self {
            phase: OperationPhase::Scanning,
            bytes_transferred: 0,
            total_bytes: 0,
            files_completed: 0,
            total_files: 0,
            current_item: None,
            speed_bps: 0.0,
            eta_seconds: None,
        }
    }
}

impl OperationProgress {
    /// Create a new progress instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate completion percentage (0.0 - 1.0).
    pub fn percentage(&self) -> f64 {
        if self.total_bytes > 0 {
            self.bytes_transferred as f64 / self.total_bytes as f64
        } else if self.total_files > 0 {
            self.files_completed as f64 / self.total_files as f64
        } else {
            0.0
        }
    }

    /// Check if the operation is complete.
    pub fn is_complete(&self) -> bool {
        matches!(
            self.phase,
            OperationPhase::Completed | OperationPhase::Failed | OperationPhase::Cancelled
        )
    }
}

/// Control flags for an operation.
#[derive(Debug, Clone)]
pub struct OperationControl {
    /// Flag to pause the operation.
    pub pause_flag: Arc<AtomicBool>,
    /// Flag to cancel the operation.
    pub cancel_flag: Arc<AtomicBool>,
}

impl Default for OperationControl {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationControl {
    /// Create new operation control flags.
    pub fn new() -> Self {
        Self {
            pause_flag: Arc::new(AtomicBool::new(false)),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Set paused state.
    pub fn set_paused(&self, paused: bool) {
        self.pause_flag.store(paused, Ordering::Relaxed);
    }

    /// Check if paused.
    pub fn is_paused(&self) -> bool {
        self.pause_flag.load(Ordering::Relaxed)
    }

    /// Cancel the operation.
    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    /// Check if cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::Relaxed)
    }

    /// Check for cancellation, returning error if cancelled.
    pub fn check_cancelled(&self) -> Result<(), OperationError> {
        if self.is_cancelled() {
            Err(OperationError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Wait while paused, checking for cancellation.
    pub fn wait_if_paused(&self) -> Result<(), OperationError> {
        while self.is_paused() {
            if self.is_cancelled() {
                return Err(OperationError::Cancelled);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Ok(())
    }
}

/// Result of an operation.
#[derive(Debug, Clone)]
pub enum OperationResult {
    /// Operation completed successfully.
    Success,
    /// Operation completed with a result path.
    SuccessWithPath(PathBuf),
    /// Operation partially succeeded (some items skipped or failed).
    PartialSuccess {
        /// Number of items completed successfully.
        completed: usize,
        /// Number of items skipped.
        skipped: usize,
        /// Number of items that failed.
        failed: usize,
    },
    /// Operation failed.
    Failed(String),
    /// Operation was cancelled.
    Cancelled,
}

impl OperationResult {
    /// Check if the result is success (full or partial).
    pub fn is_success(&self) -> bool {
        matches!(
            self,
            Self::Success | Self::SuccessWithPath(_) | Self::PartialSuccess { .. }
        )
    }
}

/// Error type for file operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum OperationError {
    /// Operation was cancelled.
    #[error("Operation cancelled")]
    Cancelled,

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(String),

    /// VFS error.
    #[error("VFS error: {0}")]
    Vfs(String),

    /// Operation not found.
    #[error("Operation not found: {0}")]
    NotFound(OperationId),

    /// Queue is full.
    #[error("Operation queue is full")]
    QueueFull,

    /// Invalid operation.
    #[error("Invalid operation: {0}")]
    Invalid(String),
}

impl From<std::io::Error> for OperationError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<termide_vfs::VfsError> for OperationError {
    fn from(e: termide_vfs::VfsError) -> Self {
        Self::Vfs(e.to_string())
    }
}

/// Events emitted by the operation system.
#[derive(Debug, Clone)]
pub enum OperationEvent {
    /// Operation started.
    Started(OperationId),
    /// Operation progress updated.
    Progress(OperationId, OperationProgress),
    /// Operation completed.
    Completed(OperationId, OperationResult),
    /// Operation paused.
    Paused(OperationId),
    /// Operation resumed.
    Resumed(OperationId),
    /// Conflict detected - operation waiting for user decision.
    ConflictDetected(OperationId, ConflictInfo),
}

/// Information about a file conflict.
#[derive(Debug, Clone)]
pub struct ConflictInfo {
    /// Source file path.
    pub source: OperationPath,
    /// Destination file path (existing file).
    pub destination: OperationPath,
    /// Source file size in bytes.
    pub source_size: u64,
    /// Destination file size in bytes.
    pub dest_size: u64,
    /// Source file modification time (Unix timestamp).
    pub source_modified: Option<u64>,
    /// Destination file modification time (Unix timestamp).
    pub dest_modified: Option<u64>,
    /// Number of remaining items in the batch.
    pub remaining_items: usize,
}

/// User's decision for handling a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolution {
    /// Overwrite the destination file.
    Overwrite,
    /// Skip this file.
    Skip,
    /// Overwrite all remaining conflicts.
    OverwriteAll,
    /// Skip all remaining conflicts.
    SkipAll,
    /// Cancel the entire operation.
    Cancel,
}

/// Conflict handling mode for batch operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictMode {
    /// Ask the user for each conflict.
    #[default]
    Ask,
    /// Automatically overwrite all conflicts.
    OverwriteAll,
    /// Automatically skip all conflicts.
    SkipAll,
}

/// Request to create a new operation.
#[derive(Debug, Clone)]
pub struct OperationRequest {
    /// Type of operation.
    pub op_type: OperationType,
    /// Source path(s).
    pub sources: Vec<OperationPath>,
    /// Destination path (for copy/move/download/upload).
    pub destination: Option<OperationPath>,
    /// Priority level.
    pub priority: OperationPriority,
    /// Whether this is a move operation (delete source after copy).
    pub is_move: bool,
    /// How to handle file conflicts.
    pub conflict_mode: ConflictMode,
}

impl OperationRequest {
    /// Create a copy operation request.
    pub fn copy(sources: Vec<OperationPath>, destination: OperationPath) -> Self {
        Self {
            op_type: OperationType::Copy,
            sources,
            destination: Some(destination),
            priority: OperationPriority::Normal,
            is_move: false,
            conflict_mode: ConflictMode::Ask,
        }
    }

    /// Create a move operation request.
    pub fn r#move(sources: Vec<OperationPath>, destination: OperationPath) -> Self {
        Self {
            op_type: OperationType::Move,
            sources,
            destination: Some(destination),
            priority: OperationPriority::Normal,
            is_move: true,
            conflict_mode: ConflictMode::Ask,
        }
    }

    /// Create a delete operation request.
    pub fn delete(sources: Vec<OperationPath>) -> Self {
        Self {
            op_type: OperationType::Delete,
            sources,
            destination: None,
            priority: OperationPriority::Normal,
            is_move: false,
            conflict_mode: ConflictMode::Ask,
        }
    }

    /// Create a download operation request (remote to local).
    pub fn download(remote: VfsPath, local: PathBuf) -> Self {
        Self {
            op_type: OperationType::Download,
            sources: vec![OperationPath::Remote(remote)],
            destination: Some(OperationPath::Local(local)),
            priority: OperationPriority::Normal,
            is_move: false,
            conflict_mode: ConflictMode::Ask,
        }
    }

    /// Create an upload operation request (local to remote).
    pub fn upload(local: PathBuf, remote: VfsPath) -> Self {
        Self {
            op_type: OperationType::Upload,
            sources: vec![OperationPath::Local(local)],
            destination: Some(OperationPath::Remote(remote)),
            priority: OperationPriority::Normal,
            is_move: false,
            conflict_mode: ConflictMode::Ask,
        }
    }

    /// Set the priority.
    pub fn with_priority(mut self, priority: OperationPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Set the conflict handling mode.
    pub fn with_conflict_mode(mut self, mode: ConflictMode) -> Self {
        self.conflict_mode = mode;
        self
    }
}

/// Information about a queued or running operation.
#[derive(Debug)]
pub struct OperationInfo {
    /// Operation ID.
    pub id: OperationId,
    /// Operation type.
    pub op_type: OperationType,
    /// Source paths.
    pub sources: Vec<OperationPath>,
    /// Destination path.
    pub destination: Option<OperationPath>,
    /// Current progress.
    pub progress: OperationProgress,
    /// Control flags.
    pub control: OperationControl,
    /// Whether the operation is currently active.
    pub is_active: bool,
}

/// Handle to an active file operation.
pub struct FileOperation {
    /// Operation ID.
    pub id: OperationId,
    /// Operation type.
    pub op_type: OperationType,
    /// Source path(s).
    pub sources: Vec<OperationPath>,
    /// Destination path.
    pub destination: Option<OperationPath>,
    /// Control flags.
    pub control: OperationControl,
    /// Progress receiver.
    pub(crate) progress_rx: mpsc::Receiver<OperationProgress>,
    /// Completion receiver.
    pub(crate) completion_rx: mpsc::Receiver<OperationResult>,
}

impl FileOperation {
    /// Try to receive progress update without blocking.
    pub fn try_recv_progress(&self) -> Option<OperationProgress> {
        self.progress_rx.try_recv().ok()
    }

    /// Drain all pending progress updates and return the latest.
    pub fn drain_progress(&self) -> Option<OperationProgress> {
        let mut latest = None;
        while let Ok(p) = self.progress_rx.try_recv() {
            latest = Some(p);
        }
        latest
    }

    /// Try to receive completion result without blocking.
    pub fn try_recv_completion(&self) -> Option<OperationResult> {
        self.completion_rx.try_recv().ok()
    }

    /// Pause the operation.
    pub fn pause(&self) {
        self.control.set_paused(true);
    }

    /// Resume the operation.
    pub fn resume(&self) {
        self.control.set_paused(false);
    }

    /// Cancel the operation.
    pub fn cancel(&self) {
        self.control.cancel();
    }

    /// Check if paused.
    pub fn is_paused(&self) -> bool {
        self.control.is_paused()
    }

    /// Check if cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.control.is_cancelled()
    }
}

impl std::fmt::Debug for FileOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileOperation")
            .field("id", &self.id)
            .field("op_type", &self.op_type)
            .field("paused", &self.is_paused())
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

/// Summary of background operations for status bar display.
#[derive(Debug, Clone, Default)]
pub struct BackgroundOperationSummary {
    /// Number of active operations running in background.
    pub active_count: usize,
    /// Number of queued operations waiting.
    pub queued_count: usize,
    /// Total bytes transferred across all background operations.
    pub total_bytes_transferred: u64,
    /// Total bytes to transfer across all background operations.
    pub total_bytes: u64,
    /// Number of files completed across all background operations.
    pub files_completed: usize,
    /// Total files to process across all background operations.
    pub total_files: usize,
    /// Overall transfer speed in bytes per second.
    pub speed_bps: f64,
    /// Whether any operation is paused.
    pub any_paused: bool,
    /// Short description of current activity.
    pub current_activity: Option<String>,
}

impl BackgroundOperationSummary {
    /// Calculate overall completion percentage (0.0 - 1.0).
    pub fn percentage(&self) -> f64 {
        if self.total_bytes > 0 {
            self.total_bytes_transferred as f64 / self.total_bytes as f64
        } else if self.total_files > 0 {
            self.files_completed as f64 / self.total_files as f64
        } else {
            0.0
        }
    }

    /// Check if there are any operations (active or queued).
    pub fn has_operations(&self) -> bool {
        self.active_count > 0 || self.queued_count > 0
    }

    /// Format for status bar display.
    pub fn status_text(&self) -> String {
        if !self.has_operations() {
            return String::new();
        }

        let percent = (self.percentage() * 100.0) as u8;
        let count = self.active_count + self.queued_count;

        if count == 1 {
            if let Some(ref activity) = self.current_activity {
                format!("{} {}%", activity, percent)
            } else {
                format!("Operation {}%", percent)
            }
        } else {
            format!("{} ops {}%", count, percent)
        }
    }

    /// Format speed for display.
    pub fn speed_text(&self) -> String {
        if self.speed_bps < 1024.0 {
            format!("{:.0} B/s", self.speed_bps)
        } else if self.speed_bps < 1024.0 * 1024.0 {
            format!("{:.1} KB/s", self.speed_bps / 1024.0)
        } else {
            format!("{:.1} MB/s", self.speed_bps / (1024.0 * 1024.0))
        }
    }
}
