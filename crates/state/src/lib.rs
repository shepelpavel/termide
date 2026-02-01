//! State types and data structures for termide.
//!
//! This crate contains pure data types used throughout the application,
//! without dependencies on specific implementations.

use chrono::{DateTime, Local};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use termide_vfs::{VfsManager, VfsPath};

/// Message about background directory size calculation result
#[derive(Debug)]
pub struct DirSizeResult {
    pub size: u64,
}

/// Batch operation type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchOperationType {
    Copy,
    Move,
}

/// Automatic conflict resolution mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictMode {
    /// Ask each time
    Ask,
    /// Automatically overwrite all
    OverwriteAll,
    /// Automatically skip all
    SkipAll,
}

/// Layout mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// Single panel mode (width 1-80)
    Single,
    /// Multi-panel mode (width > 100)
    MultiPanel,
}

/// Layout information
#[derive(Debug, Clone)]
pub struct LayoutInfo {
    /// Layout mode
    pub mode: LayoutMode,
    /// Number of main panels
    pub main_panels_count: usize,
}

impl LayoutInfo {
    /// Calculate layout based on terminal width
    pub fn calculate(width: u16) -> Self {
        use termide_config::constants::*;

        if width <= MIN_WIDTH_MULTI_PANEL {
            // Single panel mode for narrow terminals
            Self {
                mode: LayoutMode::Single,
                main_panels_count: 1,
            }
        } else {
            // Multi-panel mode
            let main_panels_count = (width / MIN_MAIN_PANEL_WIDTH).max(1) as usize;
            Self {
                mode: LayoutMode::MultiPanel,
                main_panels_count,
            }
        }
    }

    /// Get recommended layout description
    pub fn recommended_layout_str(&self) -> &'static str {
        match self.mode {
            LayoutMode::Single => "Single panel",
            LayoutMode::MultiPanel => match self.main_panels_count {
                1 => "1 panel",
                2 => "2 panels",
                3 => "3 panels",
                4 => "4 panels",
                5 => "5 panels",
                6 => "6 panels",
                7 => "7 panels",
                8 => "8 panels",
                9 => "9 panels",
                _ => "Many panels",
            },
        }
    }
}

/// State for a submenu (open/closed + selected item index).
///
/// This struct provides a consistent pattern for submenu state management.
/// Instead of having separate `*_open: bool` and `selected_*_item: usize` fields,
/// use this struct to group related state together.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SubmenuState {
    /// Whether the submenu is open
    pub open: bool,
    /// Selected item index within the submenu
    pub selected: usize,
}

impl SubmenuState {
    /// Create a new closed submenu state
    pub const fn new() -> Self {
        Self {
            open: false,
            selected: 0,
        }
    }

    /// Open the submenu and reset selection to first item
    pub fn open(&mut self) {
        self.open = true;
        self.selected = 0;
    }

    /// Open the submenu with a specific initial selection
    pub fn open_at(&mut self, index: usize) {
        self.open = true;
        self.selected = index;
    }

    /// Close the submenu and reset selection
    pub fn close(&mut self) {
        self.open = false;
        self.selected = 0;
    }

    /// Move selection up (wrapping to last item if at first)
    pub fn select_prev(&mut self, item_count: usize) {
        if item_count == 0 {
            return;
        }
        if self.selected > 0 {
            self.selected -= 1;
        } else {
            self.selected = item_count.saturating_sub(1);
        }
    }

    /// Move selection down (wrapping to first item if at last)
    pub fn select_next(&mut self, item_count: usize) {
        if item_count == 0 {
            return;
        }
        self.selected = (self.selected + 1) % item_count;
    }
}

/// State for divider drag resize operation
#[derive(Debug, Default, Clone)]
pub struct DragState {
    /// Index of divider being dragged (between groups idx and idx+1)
    pub active_divider: Option<usize>,
    /// Initial X position when drag started
    pub start_x: u16,
    /// Initial widths of left and right groups
    pub start_widths: (u16, u16),
}

impl DragState {
    /// Start dragging a divider
    pub fn start(&mut self, divider_idx: usize, x: u16, left_width: u16, right_width: u16) {
        self.active_divider = Some(divider_idx);
        self.start_x = x;
        self.start_widths = (left_width, right_width);
    }

    /// End dragging
    pub fn end(&mut self) {
        self.active_divider = None;
    }

    /// Check if currently dragging
    pub fn is_dragging(&self) -> bool {
        self.active_divider.is_some()
    }
}

/// UI components state
#[derive(Debug, Default)]
pub struct UiState {
    /// Is menu open
    pub menu_open: bool,
    /// Selected menu item (None if menu closed)
    pub selected_menu_item: Option<usize>,
    /// Selected item in dropdown list
    pub selected_dropdown_item: usize,
    /// Status line message (for displaying errors and notifications)
    pub status_message: Option<(String, bool)>, // (message, is_error)
    /// Options submenu state (e.g., Preferences dropdown)
    pub options_submenu: SubmenuState,
    /// Nested submenu state (e.g., Themes list inside Options)
    pub nested_submenu: SubmenuState,
    /// Original theme name before preview (for restoring on cancel)
    pub theme_preview_original: Option<String>,
    /// Original language code before preview (for restoring on cancel)
    pub language_preview_original: Option<String>,
    /// Divider drag state for panel resize
    pub drag: DragState,
    /// Sessions submenu state
    pub sessions_submenu: SubmenuState,
    /// Tools submenu state
    pub tools_submenu: SubmenuState,
    /// Scripts submenu state
    pub scripts_submenu: SubmenuState,
    /// Scripts nested submenu state (for subdirectory groups)
    pub scripts_nested: SubmenuState,
    /// Current script group name (for nested submenu)
    pub current_scripts_group: Option<String>,
    /// Bookmarks submenu state
    pub bookmarks_submenu: SubmenuState,
    /// Bookmarks nested submenu state (for groups)
    pub bookmarks_nested: SubmenuState,
    /// Current bookmarks group name (for nested submenu)
    pub current_bookmarks_group: Option<String>,
    /// Is git operation (push/pull) in progress
    pub git_operation_in_progress: bool,
    /// Spinner frame for animated loading indicators
    pub spinner_frame: usize,
}

impl UiState {
    /// Close all main-level submenus (sessions, tools, options, scripts, bookmarks)
    /// and their nested submenus. Use before opening a specific submenu.
    pub fn close_all_submenus(&mut self) {
        self.sessions_submenu.close();
        self.tools_submenu.close();
        self.options_submenu.close();
        self.nested_submenu.close();
        self.scripts_submenu.close();
        self.scripts_nested.close();
        self.current_scripts_group = None;
        self.bookmarks_submenu.close();
        self.bookmarks_nested.close();
        self.current_bookmarks_group = None;
    }
}

/// Terminal state (dimensions)
#[derive(Debug, Clone, Copy)]
pub struct TerminalState {
    /// Terminal width
    pub width: u16,
    /// Terminal height
    pub height: u16,
}

impl Default for TerminalState {
    fn default() -> Self {
        Self {
            width: 80,
            height: 24,
        }
    }
}

/// File rename pattern
#[derive(Debug, Clone)]
pub struct RenamePattern {
    template: String,
}

impl RenamePattern {
    /// Create new rename pattern
    pub fn new(template: String) -> Self {
        Self { template }
    }

    /// Apply pattern to filename
    pub fn apply(
        &self,
        original_name: &str,
        counter: usize,
        created: Option<SystemTime>,
        modified: Option<SystemTime>,
    ) -> String {
        // Collect parts without allocating Strings - just &str slices
        let parts: Vec<&str> = original_name.split('.').collect();
        let mut result = self.template.clone();

        // Replace $0 (full name)
        result = result.replace("$0", original_name);

        // Replace $1-9 (parts from left)
        for i in 1..=9 {
            let placeholder = format!("${}", i);
            let value = parts.get(i - 1).copied().unwrap_or("");
            result = result.replace(&placeholder, value);
        }

        // Replace $-1 to $-9 (parts from right)
        for i in 1..=9 {
            let placeholder = format!("$-{}", i);
            let idx = parts.len().saturating_sub(i);
            let value = parts.get(idx).copied().unwrap_or("");
            result = result.replace(&placeholder, value);
        }

        // Replace $I (counter)
        result = result.replace("$I", &counter.to_string());

        // Replace $C (creation time)
        if let Some(time) = created {
            result = result.replace("$C", &Self::format_time(time));
        } else {
            result = result.replace("$C", "");
        }

        // Replace $M (modification time)
        if let Some(time) = modified {
            result = result.replace("$M", &Self::format_time(time));
        } else {
            result = result.replace("$M", "");
        }

        result
    }

    /// Format time to YYYYMMDD_HHMMSS string
    fn format_time(time: SystemTime) -> String {
        let datetime: DateTime<Local> = time.into();
        datetime.format("%Y%m%d_%H%M%S").to_string()
    }

    /// Get preview result for example
    pub fn preview(&self, example_name: &str) -> String {
        self.apply(example_name, 1, None, None)
    }

    /// Check if result contains forbidden characters
    pub fn is_valid_result(&self, result: &str) -> bool {
        // Forbidden characters in filenames
        let forbidden = ['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\0'];
        !result.is_empty() && !result.chars().any(|c| forbidden.contains(&c))
    }
}

/// Pause state for batch operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseState {
    /// Operation is running
    Running,
    /// Operation is paused by user
    Paused,
}

/// Batch file operation with conflict support
#[derive(Debug, Clone)]
pub struct BatchOperation {
    /// Operation type
    pub operation_type: BatchOperationType,
    /// List of files to process
    pub sources: Vec<PathBuf>,
    /// Target directory
    pub destination: PathBuf,
    /// Current index of file being processed
    pub current_index: usize,
    /// Conflict resolution mode
    pub conflict_mode: ConflictMode,
    /// Rename pattern for RenameAll
    pub rename_pattern: Option<RenamePattern>,
    /// Counter for $I variable in pattern
    pub rename_counter: usize,
    /// Statistics: successfully processed
    pub success_count: usize,
    /// Statistics: errors
    pub error_count: usize,
    /// Statistics: skipped
    pub skipped_count: usize,
    /// Pause state for batch operation
    pub pause_state: PauseState,
    /// Paths of successfully copied/moved destinations (for cleanup on cancel)
    pub completed_destinations: Vec<PathBuf>,
    /// Cumulative files completed from previous batch items (for multi-folder downloads)
    pub cumulative_files_completed: usize,
    /// Cumulative bytes completed from previous batch items
    pub cumulative_bytes_completed: u64,
    /// Total files across all batch items (when known)
    pub cumulative_total_files: usize,
    /// Total bytes across all batch items (when known)
    pub cumulative_total_bytes: u64,
}

impl BatchOperation {
    /// Create new batch operation
    pub fn new(
        operation_type: BatchOperationType,
        sources: Vec<PathBuf>,
        destination: PathBuf,
    ) -> Self {
        Self {
            operation_type,
            sources,
            destination,
            current_index: 0,
            conflict_mode: ConflictMode::Ask,
            rename_pattern: None,
            rename_counter: 1,
            success_count: 0,
            error_count: 0,
            skipped_count: 0,
            pause_state: PauseState::Running,
            completed_destinations: Vec::new(),
            cumulative_files_completed: 0,
            cumulative_bytes_completed: 0,
            cumulative_total_files: 0,
            cumulative_total_bytes: 0,
        }
    }

    /// Add a successfully completed destination path
    pub fn add_completed_destination(&mut self, path: PathBuf) {
        self.completed_destinations.push(path);
    }

    /// Set rename pattern
    pub fn set_rename_pattern(&mut self, pattern: RenamePattern) {
        self.rename_pattern = Some(pattern);
    }

    /// Get and increment rename counter
    pub fn get_and_increment_rename_counter(&mut self) -> usize {
        let counter = self.rename_counter;
        self.rename_counter += 1;
        counter
    }

    /// Get current file being processed
    pub fn current_source(&self) -> Option<&PathBuf> {
        self.sources.get(self.current_index)
    }

    /// Check if operation is complete
    pub fn is_complete(&self) -> bool {
        self.current_index >= self.sources.len()
    }

    /// Advance to next file
    pub fn advance(&mut self) {
        self.current_index += 1;
    }

    /// Total number of files
    pub fn total_count(&self) -> usize {
        self.sources.len()
    }

    /// Set conflict resolution mode
    pub fn set_conflict_mode(&mut self, mode: ConflictMode) {
        self.conflict_mode = mode;
    }

    /// Increment success counter
    pub fn increment_success(&mut self) {
        self.success_count += 1;
    }

    /// Increment error counter
    pub fn increment_error(&mut self) {
        self.error_count += 1;
    }

    /// Increment skipped counter
    pub fn increment_skipped(&mut self) {
        self.skipped_count += 1;
    }

    /// Get the last successfully processed source filename
    /// Returns the filename of the file at current_index - 1 if available
    pub fn last_successful_filename(&self) -> Option<String> {
        if self.current_index == 0 || self.success_count == 0 {
            return None;
        }

        // Get the file that was just processed (current_index - 1)
        self.sources
            .get(self.current_index.saturating_sub(1))
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .map(|s| s.to_string())
    }

    /// Get destination path reference
    pub fn destination_path(&self) -> &PathBuf {
        &self.destination
    }
}

/// Action pending modal result
#[derive(Debug, Clone)]
pub enum PendingAction {
    /// Create new file in specified directory
    CreateFile {
        panel_index: usize,
        directory: PathBuf,
    },
    /// Create new directory in specified directory
    CreateDirectory {
        panel_index: usize,
        directory: PathBuf,
    },
    /// Delete files/directories (one or multiple)
    DeletePath {
        panel_index: usize,
        paths: Vec<PathBuf>,
    },
    /// Delete remote files/directories (one or multiple)
    DeleteRemotePath {
        panel_index: usize,
        paths: Vec<VfsPath>,
        vfs_manager: Arc<VfsManager>,
    },
    /// Copy files/directories (one or multiple)
    CopyPath {
        panel_index: usize,
        sources: Vec<PathBuf>,
        target_directory: Option<PathBuf>,
    },
    /// Move files/directories (one or multiple)
    MovePath {
        panel_index: usize,
        sources: Vec<PathBuf>,
        target_directory: Option<PathBuf>,
    },
    /// Save unnamed file (Save As)
    SaveFileAs {
        panel_index: usize,
        directory: PathBuf,
    },
    /// Close panel (with confirmation if there are unsaved changes)
    ClosePanel { panel_index: usize },
    /// Close editor with choice: save, don't save, cancel
    CloseEditorWithSave { panel_index: usize },
    /// Close editor with external changes (file changed on disk)
    CloseEditorExternal { panel_index: usize },
    /// Close editor with conflict (local changes + external changes)
    CloseEditorConflict { panel_index: usize },
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
    FileSearch { panel_index: usize },
    /// Content search in file manager
    ContentSearch { panel_index: usize },
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
    GoToPath {
        panel_index: usize,
        current_directory: PathBuf,
    },
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
    /// Resolve a file conflict for an OperationManager operation
    ResolveOperationConflict {
        /// The operation ID waiting for resolution
        operation_id: termide_file_ops::OperationId,
    },
}

// ============================================================================
// Operations Panel Types
// ============================================================================

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
}

impl OperationType {
    /// Returns true if this operation involves data transfer (not delete/rename)
    pub fn has_data_progress(&self) -> bool {
        !matches!(self, Self::Delete | Self::Rename)
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

/// Tracker for calculating transfer speed
#[derive(Debug)]
pub struct SpeedTracker {
    /// Last known bytes transferred
    last_bytes: u64,
    /// Last update time
    last_time: Instant,
    /// Current speed in bytes per second
    current_speed: f64,
}

impl Default for SpeedTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeedTracker {
    /// Create new speed tracker
    pub fn new() -> Self {
        Self {
            last_bytes: 0,
            last_time: Instant::now(),
            current_speed: 0.0,
        }
    }

    /// Update speed calculation with new bytes transferred
    pub fn update(&mut self, bytes_transferred: u64) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_time).as_secs_f64();

        // Update speed every 0.5 seconds for smoother display
        if elapsed >= 0.5 {
            let bytes_delta = bytes_transferred.saturating_sub(self.last_bytes);
            let instant_speed = bytes_delta as f64 / elapsed;

            // Exponential moving average for smoother values
            if self.current_speed > 0.0 {
                self.current_speed = 0.3 * instant_speed + 0.7 * self.current_speed;
            } else {
                self.current_speed = instant_speed;
            }

            self.last_bytes = bytes_transferred;
            self.last_time = now;
        }
    }

    /// Get current speed in bytes per second
    pub fn speed(&self) -> f64 {
        self.current_speed
    }

    /// Reset speed tracker (e.g., when operation is paused)
    pub fn reset(&mut self) {
        self.current_speed = 0.0;
        self.last_bytes = 0;
        self.last_time = Instant::now();
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

    #[test]
    fn test_basic_replacement() {
        let pattern = RenamePattern::new("$0".to_string());
        assert_eq!(pattern.preview("file.txt"), "file.txt");
    }

    #[test]
    fn test_parts_from_left() {
        let pattern = RenamePattern::new("$1_copy.$2".to_string());
        assert_eq!(pattern.preview("document.txt"), "document_copy.txt");
    }

    #[test]
    fn test_parts_from_right() {
        let pattern = RenamePattern::new("$1_backup.$-1".to_string());
        assert_eq!(pattern.preview("archive.tar.gz"), "archive_backup.gz");
    }

    #[test]
    fn test_counter() {
        let pattern = RenamePattern::new("$1_$I.$-1".to_string());
        assert_eq!(pattern.apply("file.txt", 5, None, None), "file_5.txt");
    }

    #[test]
    fn test_complex_pattern() {
        let pattern = RenamePattern::new("$1_$I.$2.$3".to_string());
        assert_eq!(pattern.preview("document.tar.gz"), "document_1.tar.gz");
    }

    #[test]
    fn test_missing_parts() {
        let pattern = RenamePattern::new("$1.$5".to_string());
        assert_eq!(pattern.preview("file.txt"), "file.");
    }

    #[test]
    fn test_validation() {
        let pattern = RenamePattern::new("$1_copy.$-1".to_string());
        assert!(pattern.is_valid_result("file_copy.txt"));
        assert!(!pattern.is_valid_result("file/copy.txt"));
        assert!(!pattern.is_valid_result("file:copy.txt"));
        assert!(!pattern.is_valid_result(""));
    }

    #[test]
    fn test_batch_operation_new() {
        let op = BatchOperation::new(
            BatchOperationType::Copy,
            vec![PathBuf::from("/a"), PathBuf::from("/b")],
            PathBuf::from("/dest"),
        );
        assert_eq!(op.total_count(), 2);
        assert!(!op.is_complete());
    }

    // =========================================================================
    // SubmenuState tests
    // =========================================================================

    #[test]
    fn test_submenu_state_new() {
        let state = SubmenuState::new();
        assert!(!state.open);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_submenu_state_open() {
        let mut state = SubmenuState::new();
        state.selected = 5; // Set some value
        state.open();
        assert!(state.open);
        assert_eq!(state.selected, 0); // Reset to 0
    }

    #[test]
    fn test_submenu_state_open_at() {
        let mut state = SubmenuState::new();
        state.open_at(3);
        assert!(state.open);
        assert_eq!(state.selected, 3);
    }

    #[test]
    fn test_submenu_state_close() {
        let mut state = SubmenuState::new();
        state.open = true;
        state.selected = 5;
        state.close();
        assert!(!state.open);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_submenu_state_select_prev() {
        let mut state = SubmenuState::new();
        state.selected = 2;

        state.select_prev(5);
        assert_eq!(state.selected, 1);

        state.select_prev(5);
        assert_eq!(state.selected, 0);

        // Wrap to last
        state.select_prev(5);
        assert_eq!(state.selected, 4);
    }

    #[test]
    fn test_submenu_state_select_next() {
        let mut state = SubmenuState::new();
        state.selected = 3;

        state.select_next(5);
        assert_eq!(state.selected, 4);

        // Wrap to first
        state.select_next(5);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_submenu_state_empty_list() {
        let mut state = SubmenuState::new();
        state.selected = 0;

        // Should not panic with empty list
        state.select_prev(0);
        assert_eq!(state.selected, 0);

        state.select_next(0);
        assert_eq!(state.selected, 0);
    }

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
            ..Default::default()
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
    // BatchOperation tests
    // =========================================================================

    #[test]
    fn test_batch_operation_lifecycle() {
        let mut op = BatchOperation::new(
            BatchOperationType::Copy,
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c"),
            ],
            PathBuf::from("/dest"),
        );

        assert_eq!(op.total_count(), 3);
        assert!(!op.is_complete());
        assert_eq!(op.current_source(), Some(&PathBuf::from("/a")));

        op.advance();
        op.increment_success();
        assert_eq!(op.current_index, 1);
        assert_eq!(op.success_count, 1);

        op.advance();
        op.increment_error();

        op.advance();
        op.increment_skipped();

        assert!(op.is_complete());
        assert_eq!(op.success_count, 1);
        assert_eq!(op.error_count, 1);
        assert_eq!(op.skipped_count, 1);
    }

    #[test]
    fn test_batch_operation_completed_destinations() {
        let mut op = BatchOperation::new(
            BatchOperationType::Move,
            vec![PathBuf::from("/a")],
            PathBuf::from("/dest"),
        );

        op.add_completed_destination(PathBuf::from("/dest/a"));
        assert_eq!(op.completed_destinations.len(), 1);
    }

    #[test]
    fn test_batch_operation_rename_counter() {
        let mut op = BatchOperation::new(
            BatchOperationType::Copy,
            vec![PathBuf::from("/a")],
            PathBuf::from("/dest"),
        );

        assert_eq!(op.get_and_increment_rename_counter(), 1);
        assert_eq!(op.get_and_increment_rename_counter(), 2);
        assert_eq!(op.get_and_increment_rename_counter(), 3);
    }

    #[test]
    fn test_batch_operation_last_successful_filename() {
        let mut op = BatchOperation::new(
            BatchOperationType::Copy,
            vec![PathBuf::from("/a/file.txt"), PathBuf::from("/b/other.rs")],
            PathBuf::from("/dest"),
        );

        // Before any processing
        assert_eq!(op.last_successful_filename(), None);

        // After processing first file
        op.advance();
        op.increment_success();
        assert_eq!(op.last_successful_filename(), Some("file.txt".to_string()));
    }

    #[test]
    fn test_batch_operation_pause_state() {
        let op = BatchOperation::new(
            BatchOperationType::Copy,
            vec![PathBuf::from("/a")],
            PathBuf::from("/dest"),
        );
        assert_eq!(op.pause_state, PauseState::Running);
    }

    // =========================================================================
    // DragState tests
    // =========================================================================

    #[test]
    fn test_drag_state_lifecycle() {
        let mut drag = DragState::default();
        assert!(!drag.is_dragging());

        drag.start(1, 100, 50, 50);
        assert!(drag.is_dragging());
        assert_eq!(drag.active_divider, Some(1));
        assert_eq!(drag.start_x, 100);
        assert_eq!(drag.start_widths, (50, 50));

        drag.end();
        assert!(!drag.is_dragging());
    }

    // =========================================================================
    // UiState tests
    // =========================================================================

    #[test]
    fn test_ui_state_close_all_submenus() {
        let mut ui = UiState::default();
        ui.sessions_submenu.open();
        ui.tools_submenu.open();
        ui.options_submenu.open();
        ui.scripts_submenu.open();
        ui.bookmarks_submenu.open();
        ui.current_scripts_group = Some("test".to_string());
        ui.current_bookmarks_group = Some("test".to_string());

        ui.close_all_submenus();

        assert!(!ui.sessions_submenu.open);
        assert!(!ui.tools_submenu.open);
        assert!(!ui.options_submenu.open);
        assert!(!ui.scripts_submenu.open);
        assert!(!ui.bookmarks_submenu.open);
        assert!(ui.current_scripts_group.is_none());
        assert!(ui.current_bookmarks_group.is_none());
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

    // =========================================================================
    // RenamePattern additional tests
    // =========================================================================

    #[test]
    fn test_rename_pattern_no_extension() {
        let pattern = RenamePattern::new("$1_backup".to_string());
        assert_eq!(pattern.preview("README"), "README_backup");
    }

    #[test]
    fn test_rename_pattern_multiple_extensions() {
        let pattern = RenamePattern::new("$1.$2.$-1".to_string());
        assert_eq!(pattern.preview("file.tar.gz"), "file.tar.gz");
    }

    // =========================================================================
    // TerminalState tests
    // =========================================================================

    #[test]
    fn test_terminal_state_default() {
        let ts = TerminalState::default();
        assert_eq!(ts.width, 80);
        assert_eq!(ts.height, 24);
    }
}
