//! Operations panel types for tracking active file operations.

use std::time::Instant;

pub use termide_file_ops::SpeedTracker;

/// Type of file operation (for display in Operations panel)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationType {
    /// Local copy
    Copy,
    /// Local move
    Move,
    /// Rename (move within the same directory)
    Rename,
    /// Copy from local to remote (upload)
    CopyUpload,
    /// Copy from remote to local (download)
    CopyDownload,
    /// Move from local to remote (upload + delete source)
    MoveUpload,
    /// Move from remote to local (download + delete source)
    MoveDownload,
    /// Delete file(s)
    Delete,
    /// Script execution (report script)
    Script,
}

impl OperationType {
    /// Returns true if this operation involves data transfer (not delete/rename/script)
    pub fn has_data_progress(&self) -> bool {
        !matches!(self, Self::Delete | Self::Rename | Self::Script)
    }
}

/// Progress information for an active operation
#[derive(Debug, Clone, Default)]
pub struct OperationProgress {
    /// Number of files completed
    pub files_completed: usize,
    /// Total number of files
    pub total_files: usize,
    /// Bytes transferred so far
    pub bytes_transferred: u64,
    /// Total bytes to transfer
    pub total_bytes: u64,
}

impl OperationProgress {
    /// Create new empty progress
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate completion percentage (0-100)
    pub fn percent(&self) -> u8 {
        if self.total_bytes > 0 {
            (((self.bytes_transferred * 100) / self.total_bytes) as u8).min(100)
        } else if self.total_files > 0 {
            (((self.files_completed * 100) / self.total_files) as u8).min(100)
        } else {
            0
        }
    }
}

/// An active file operation being tracked in the Operations panel
#[derive(Debug)]
pub struct ActiveOperation {
    /// Unique operation ID
    pub id: termide_file_ops::OperationId,
    /// Type of operation
    pub op_type: OperationType,
    /// Source path/URL display string
    pub source: String,
    /// Destination path/URL display string
    pub dest: String,
    /// Current progress
    pub progress: OperationProgress,
    /// Whether operation is paused
    pub is_paused: bool,
    /// When the operation started
    pub started_at: Instant,
    /// Speed tracker for calculating transfer rate
    pub speed_tracker: SpeedTracker,
    /// (Batch only) Cumulative bytes from already-completed individual files.
    pub batch_bytes_offset: u64,
    /// (Batch only) Current individual file's total size (for offset shift on completion).
    pub batch_current_file_total: u64,
    /// Whether the operation is currently in scanning phase (e.g., counting files before delete).
    pub is_scanning: bool,
}

impl ActiveOperation {
    /// Create new active operation
    pub fn new(
        id: termide_file_ops::OperationId,
        op_type: OperationType,
        source: String,
        dest: String,
        total_files: usize,
        total_bytes: u64,
    ) -> Self {
        Self {
            id,
            op_type,
            source,
            dest,
            progress: OperationProgress {
                files_completed: 0,
                total_files,
                bytes_transferred: 0,
                total_bytes,
            },
            is_paused: false,
            started_at: Instant::now(),
            speed_tracker: SpeedTracker::new(),
            batch_bytes_offset: 0,
            batch_current_file_total: 0,
            is_scanning: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // OperationProgress tests
    // =========================================================================

    #[test]
    fn test_operation_progress_percent_by_bytes() {
        let p = OperationProgress {
            bytes_transferred: 50,
            total_bytes: 100,
            ..Default::default()
        };
        assert_eq!(p.percent(), 50);
    }

    #[test]
    fn test_operation_progress_percent_by_files() {
        let p = OperationProgress {
            files_completed: 3,
            total_files: 10,
            total_bytes: 0,
            bytes_transferred: 0,
        };
        assert_eq!(p.percent(), 30);
    }

    #[test]
    fn test_operation_progress_percent_zero_total() {
        let p = OperationProgress::default();
        assert_eq!(p.percent(), 0);
    }

    #[test]
    fn test_operation_progress_percent_capped_at_100() {
        let p = OperationProgress {
            bytes_transferred: 200,
            total_bytes: 100,
            ..Default::default()
        };
        assert_eq!(p.percent(), 100);
    }

    // =========================================================================
    // OperationType tests
    // =========================================================================

    #[test]
    fn test_operation_type_has_data_progress() {
        assert!(OperationType::Copy.has_data_progress());
        assert!(OperationType::Move.has_data_progress());
        assert!(OperationType::CopyUpload.has_data_progress());
        assert!(OperationType::CopyDownload.has_data_progress());
        assert!(!OperationType::Delete.has_data_progress());
        assert!(!OperationType::Rename.has_data_progress());
    }
}
