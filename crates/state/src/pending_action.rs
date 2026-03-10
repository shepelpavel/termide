//! Pending action types for modal result handling.

use std::path::PathBuf;
use std::sync::Arc;

use termide_vfs::{VfsManager, VfsPath};

use crate::batch::BatchOperation;

/// Action pending modal result
#[derive(Debug, Clone)]
pub enum PendingAction {
    /// Create new file in specified directory
    CreateFile { directory: PathBuf },
    /// Create new directory in specified directory
    CreateDirectory { directory: PathBuf },
    /// Delete files/directories (one or multiple)
    DeletePath { paths: Vec<PathBuf> },
    /// Delete remote files/directories (one or multiple)
    DeleteRemotePath {
        paths: Vec<VfsPath>,
        vfs_manager: Arc<VfsManager>,
    },
    /// Copy files/directories (one or multiple)
    CopyPath {
        sources: Vec<PathBuf>,
        target_directory: Option<PathBuf>,
        create_symlink: bool,
    },
    /// Move files/directories (one or multiple)
    MovePath {
        sources: Vec<PathBuf>,
        target_directory: Option<PathBuf>,
    },
    /// Save unnamed file (Save As)
    SaveFileAs { directory: PathBuf },
    /// Close panel (with confirmation if there are unsaved changes)
    ClosePanel,
    /// Close editor with choice: save, don't save, cancel
    CloseEditorWithSave,
    /// Close editor with external changes (file changed on disk)
    CloseEditorExternal,
    /// Close editor with conflict (local changes + external changes)
    CloseEditorConflict,
    /// Batch file operation (copy/move)
    BatchFileOperation { operation: BatchOperation },
    /// Continue batch operation after conflict resolution
    ContinueBatchOperation { operation: BatchOperation },
    /// Request rename pattern and apply to file
    RenameWithPattern {
        operation: BatchOperation,
        original_name: String,
    },
    /// Text search in editor
    Search,
    /// Text replace in editor
    Replace,
    /// Switch to next panel
    NextPanel,
    /// Switch to previous panel
    PrevPanel,
    /// Quit application (with confirmation if there are unsaved changes)
    QuitApplication,
    /// Switch to another session
    SwitchSession,
    /// Create new session in specified directory
    NewSession,
    /// Change root path of current session
    ChangeRootPath,
    /// File search in file manager
    FileSearch,
    /// Content search in file manager
    ContentSearch,
    /// Open Git Status panel
    OpenGitStatus,
    /// Open Git Log panel
    OpenGitLog,
    /// Git file action from File Info modal
    GitFileAction {
        /// The file path to operate on
        file_path: PathBuf,
        /// Repository root path
        repo_path: PathBuf,
        /// Whether the file is staged
        is_staged: bool,
    },
    /// Git commit action
    GitCommit {
        /// Repository root path
        repo_path: PathBuf,
    },
    /// Git revert file action (with confirmation)
    GitRevertFile {
        /// The file path to revert
        file_path: PathBuf,
        /// Repository root path
        repo_path: PathBuf,
        /// Whether the file is staged
        is_staged: bool,
    },
    /// Switch active panel's working directory
    SwitchDirectory,
    /// Add a bookmark
    AddBookmark,
    /// Go to path/URL (supports local paths and remote URLs like sftp://)
    GoToPath { current_directory: PathBuf },
    /// VFS information message (connection cancelled, error, etc.)
    VfsMessage,
    /// Handle cancelled copy/move operation cleanup
    CancelCopyCleanup {
        /// Path to the partial file/directory being copied
        partial_path: PathBuf,
        /// All destination paths created during this batch operation
        all_dest_paths: Vec<PathBuf>,
        /// Whether this is a directory (true) or file (false)
        is_directory: bool,
        /// Optional batch operation to continue after handling
        batch_operation: Option<Box<BatchOperation>>,
    },
    /// Follow symlink — navigate to symlink target
    FollowSymlink { target_path: PathBuf },
    /// Resolve a file conflict for an OperationManager operation
    ResolveOperationConflict {
        /// The operation ID waiting for resolution
        operation_id: termide_file_ops::OperationId,
    },
}
