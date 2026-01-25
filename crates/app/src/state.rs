//! Application state and types.
//!
//! Re-exports pure types from termide-state crate and defines
//! complex types that depend on other application modules.
//!
//! Implements core traits from termide-app-core for standardized
//! state management and modal handling.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};

use termide_config::constants::DEFAULT_MAIN_PANEL_WIDTH;
use termide_config::{BookmarksConfig, Config};
use termide_file_ops::{
    BackgroundOperationSummary, OperationEvent, OperationManager, OperationRequest,
};
use termide_lsp::{LspConfig, LspManager, LspServerConfig};
use termide_panel_editor::EditorConfig;
use termide_system_monitor::SystemMonitor;
use termide_theme::Theme;
use termide_vfs::{VfsManager, VfsOperation, VfsPath};
use termide_watcher::UnifiedWatcher;

// Import core traits
use termide_app_core::{ModalManager, StateManager};

// Re-export pure types from state crate
pub use termide_state::{
    BatchOperation, BatchOperationType, ConflictMode, DirSizeResult, LayoutInfo, LayoutMode,
    PendingAction, RenamePattern, SubmenuState, TerminalState, UiState,
};

// Re-export ActiveModal from modal crate
pub use termide_modal::ActiveModal;

/// Result of background git operation (push/pull)
#[derive(Debug)]
pub struct GitOperationResult {
    /// Operation type: "push" or "pull"
    pub operation: String,
    /// Whether the operation succeeded
    pub success: bool,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
}

/// Handle for background git operation (allows cancellation)
pub struct GitOperationHandle {
    /// Receiver for operation result
    pub receiver: mpsc::Receiver<GitOperationResult>,
    /// Process ID for cancellation
    pub pid: u32,
    /// Operation type: "push" or "pull"
    pub operation: String,
}

impl std::fmt::Debug for GitOperationHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitOperationHandle")
            .field("pid", &self.pid)
            .field("operation", &self.operation)
            .finish_non_exhaustive()
    }
}

/// Result of background script operation (.report. scripts)
#[derive(Debug)]
pub struct ScriptOperationResult {
    /// Script display name
    pub script_name: String,
    /// Whether the script succeeded (exit code 0)
    pub success: bool,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
}

/// Handle for background script operation
pub struct ScriptOperationHandle {
    /// Receiver for operation result
    pub receiver: mpsc::Receiver<ScriptOperationResult>,
    /// Script display name
    pub script_name: String,
}

impl std::fmt::Debug for ScriptOperationHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptOperationHandle")
            .field("script_name", &self.script_name)
            .finish_non_exhaustive()
    }
}

/// State for async download operation of remote files
pub struct DownloadOperation {
    /// VFS operation handle for the download
    pub operation: VfsOperation<PathBuf>,
    /// Remote path being downloaded
    pub remote_path: VfsPath,
    /// Local temp path where file is being downloaded
    pub temp_path: PathBuf,
    /// Editor config for opening the file
    pub config: EditorConfig,
    /// VFS manager reference for opening the editor
    pub vfs_manager: Arc<VfsManager>,
    /// When the download started (for timeout)
    pub started: std::time::Instant,
}

impl std::fmt::Debug for DownloadOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadOperation")
            .field("remote_path", &self.remote_path.to_url_string())
            .field("temp_path", &self.temp_path)
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

/// State for async upload operation of remote files
pub struct UploadOperation {
    /// VFS operation handle for the upload
    pub operation: VfsOperation<()>,
    /// Remote path being uploaded to
    pub remote_path: VfsPath,
    /// Local temp path being uploaded from
    pub temp_path: PathBuf,
    /// Which editor panel triggered this upload (for updating after completion)
    pub editor_panel_id: usize,
    /// When the upload started (for timeout)
    pub started: std::time::Instant,
    /// Whether to close editor panel after successful upload
    pub close_after_upload: bool,
}

impl std::fmt::Debug for UploadOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadOperation")
            .field("remote_path", &self.remote_path.to_url_string())
            .field("temp_path", &self.temp_path)
            .field("editor_panel_id", &self.editor_panel_id)
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

/// State for async batch copy/move download operation (remote→local) with progress and pause/cancel
pub struct BatchDownloadOperation {
    /// VFS download operation handle with progress and pause/cancel
    pub operation: termide_vfs::VfsDownloadOperation,
    /// Local destination path where file is being downloaded to
    pub dest_path: PathBuf,
    /// When the download started (for timeout)
    pub started: std::time::Instant,
    /// Whether this is a move operation (delete source after download)
    pub is_move: bool,
    /// VFS source path for deletion after move (only set if is_move)
    pub vfs_source: Option<termide_vfs::VfsPath>,
    /// VFS manager for deletion (only set if is_move)
    pub vfs_manager: Option<std::sync::Arc<termide_vfs::VfsManager>>,
    /// Last known total files for this item (for cumulative tracking)
    pub last_total_files: usize,
    /// Last known total bytes for this item (for cumulative tracking)
    pub last_total_bytes: u64,
}

impl std::fmt::Debug for BatchDownloadOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchDownloadOperation")
            .field("dest_path", &self.dest_path)
            .field("started", &self.started)
            .field("is_move", &self.is_move)
            .finish_non_exhaustive()
    }
}

/// State for async batch upload operation (local→remote) with progress
pub struct BatchUploadOperation {
    /// VFS upload operation handle with progress
    pub operation: termide_vfs::VfsUploadOperation,
    /// Local source path being uploaded
    pub source_path: PathBuf,
    /// Remote destination URL
    pub dest_url: String,
    /// Total bytes to upload for current file
    pub total_bytes: u64,
    /// When the upload started (for timeout)
    pub started: std::time::Instant,
    /// Whether this is a move operation (delete source after upload)
    pub is_move: bool,
    /// All source files to upload
    pub all_sources: Vec<PathBuf>,
    /// Remote destination base URL (directory)
    pub dest_base_url: String,
    /// Current file index in the batch
    pub current_index: usize,
    /// VFS manager for the upload
    pub vfs_manager: std::sync::Arc<termide_vfs::VfsManager>,
}

impl std::fmt::Debug for BatchUploadOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchUploadOperation")
            .field("source_path", &self.source_path)
            .field("dest_url", &self.dest_url)
            .field("total_bytes", &self.total_bytes)
            .field("is_move", &self.is_move)
            .field("current_index", &self.current_index)
            .field("total_files", &self.all_sources.len())
            .finish_non_exhaustive()
    }
}

/// State for async local file copy operation with progress
pub struct LocalCopyOperation {
    /// Completion receiver
    pub completion: mpsc::Receiver<anyhow::Result<PathBuf>>,
    /// Progress receiver
    pub progress: mpsc::Receiver<termide_panel_file_manager::CopyProgress>,
    /// Source path (needed for Move to delete after copy)
    pub source_path: PathBuf,
    /// Destination path
    pub dest_path: PathBuf,
    /// Whether this is a move operation (delete source after copy)
    pub is_move: bool,
    /// Pause flag (shared with background thread)
    pub pause_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Cancel flag (shared with background thread)
    pub cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl std::fmt::Debug for LocalCopyOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalCopyOperation")
            .field("source_path", &self.source_path)
            .field("dest_path", &self.dest_path)
            .field("is_move", &self.is_move)
            .field(
                "pause_flag",
                &self.pause_flag.load(std::sync::atomic::Ordering::Relaxed),
            )
            .field(
                "cancel_flag",
                &self.cancel_flag.load(std::sync::atomic::Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

/// State for async local directory copy operation with progress
pub struct LocalDirectoryCopyOperation {
    /// Completion receiver
    pub completion: mpsc::Receiver<anyhow::Result<PathBuf>>,
    /// Progress receiver
    pub progress: mpsc::Receiver<termide_panel_file_manager::DirectoryCopyProgress>,
    /// Source path (needed for Move to delete after copy)
    pub source_path: PathBuf,
    /// Destination path
    pub dest_path: PathBuf,
    /// Whether this is a move operation (delete source after copy)
    pub is_move: bool,
    /// Pause flag (shared with background thread)
    pub pause_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Cancel flag (shared with background thread)
    pub cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Current file being copied (updated from progress, used for partial cleanup on cancel)
    pub current_file: Option<PathBuf>,
}

impl std::fmt::Debug for LocalDirectoryCopyOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalDirectoryCopyOperation")
            .field("source_path", &self.source_path)
            .field("dest_path", &self.dest_path)
            .field("is_move", &self.is_move)
            .field(
                "pause_flag",
                &self.pause_flag.load(std::sync::atomic::Ordering::Relaxed),
            )
            .field(
                "cancel_flag",
                &self.cancel_flag.load(std::sync::atomic::Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

/// State for async directory scan operation before copy
pub struct LocalScanOperation {
    /// Completion receiver (returns DirectoryScanResult)
    pub completion: mpsc::Receiver<anyhow::Result<termide_panel_file_manager::DirectoryScanResult>>,
    /// Progress receiver
    pub progress: mpsc::Receiver<termide_panel_file_manager::ScanProgress>,
    /// Source path being scanned
    pub source_path: PathBuf,
    /// Destination path for copy after scan
    pub dest_path: PathBuf,
    /// Cancel flag (shared with background thread)
    pub cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Reference to the batch operation to continue after scan
    pub batch_operation: Option<Box<BatchOperation>>,
}

impl std::fmt::Debug for LocalScanOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalScanOperation")
            .field("source_path", &self.source_path)
            .field("dest_path", &self.dest_path)
            .field(
                "cancel_flag",
                &self.cancel_flag.load(std::sync::atomic::Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

/// State for async delete operation with progress
pub struct LocalDeleteOperation {
    /// Completion receiver
    pub completion: mpsc::Receiver<anyhow::Result<()>>,
    /// Progress receiver
    pub progress: mpsc::Receiver<termide_panel_file_manager::DeleteProgress>,
    /// Cancel flag (shared with background thread)
    pub cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl std::fmt::Debug for LocalDeleteOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalDeleteOperation")
            .field(
                "cancel_flag",
                &self.cancel_flag.load(std::sync::atomic::Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

/// State for async VFS upload operation (local→remote file copy)
pub struct VfsUploadState {
    /// VFS operation handle for the upload (with progress)
    pub operation: termide_vfs::VfsUploadOperation,
    /// Local source path being uploaded
    pub source_path: PathBuf,
    /// Remote destination URL
    pub dest_url: String,
    /// Total bytes to upload
    pub total_bytes: u64,
    /// Whether this is a move operation (delete source after upload)
    pub is_move: bool,
    /// When the upload started (for timeout)
    pub started: std::time::Instant,
}

impl std::fmt::Debug for VfsUploadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VfsUploadState")
            .field("source_path", &self.source_path)
            .field("dest_url", &self.dest_url)
            .field("total_bytes", &self.total_bytes)
            .field("is_move", &self.is_move)
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

/// Global application state
#[derive(Debug)]
pub struct AppState {
    /// Should application quit
    pub should_quit: bool,
    /// UI components state
    pub ui: UiState,
    /// Terminal state
    pub terminal: TerminalState,
    /// Current layout mode
    pub layout_mode: LayoutMode,
    /// Current layout information
    pub layout_info: LayoutInfo,
    /// Active modal window
    pub active_modal: Option<ActiveModal>,
    /// Action pending modal result
    pub pending_action: Option<PendingAction>,
    /// Receiver channel for background directory size calculation results
    pub dir_size_receiver: Option<mpsc::Receiver<DirSizeResult>>,
    /// Handle for background git operation (allows cancellation)
    pub git_operation_handle: Option<GitOperationHandle>,
    /// Handle for background script operation (.report. scripts)
    pub script_operation_handle: Option<ScriptOperationHandle>,
    /// Handle for async remote file download operation
    pub download_operation: Option<DownloadOperation>,
    /// Handle for async remote file upload operation
    pub upload_operation: Option<UploadOperation>,
    /// Handle for async batch copy download operation (remote→local)
    pub batch_download_operation: Option<BatchDownloadOperation>,
    /// Handle for async local file copy operation with progress
    pub local_copy_operation: Option<LocalCopyOperation>,
    /// Handle for async local directory copy operation with progress
    pub local_directory_copy_operation: Option<LocalDirectoryCopyOperation>,
    /// Handle for async directory scan operation before copy
    pub local_scan_operation: Option<LocalScanOperation>,
    /// Handle for async delete operation with progress
    pub local_delete_operation: Option<LocalDeleteOperation>,
    /// Handle for async VFS upload operation (local→remote single file)
    pub vfs_upload_state: Option<VfsUploadState>,
    /// Handle for async batch upload operation (local→remote multiple files)
    pub batch_upload_operation: Option<BatchUploadOperation>,
    /// Unified watcher for filesystem and git changes
    pub watcher: Option<UnifiedWatcher>,
    /// Current theme
    pub theme: &'static Theme,
    /// Application configuration
    pub config: Config,
    /// System resource monitor (CPU, RAM)
    pub system_monitor: SystemMonitor,
    /// Last time system resources were updated
    pub last_resource_update: std::time::Instant,
    /// Last time session was saved (for debouncing autosave)
    pub last_session_save: Option<std::time::Instant>,
    /// Flag indicating UI needs to be redrawn (for CPU optimization)
    pub needs_redraw: bool,
    /// Last time spinner was updated (for throttling spinner animation)
    pub last_spinner_update: Option<std::time::Instant>,
    /// LSP manager for language server integration
    pub lsp_manager: Option<LspManager>,
    /// All diagnostics from LSP servers, keyed by file path
    pub all_diagnostics: HashMap<PathBuf, Vec<lsp_types::Diagnostic>>,
    /// User bookmarks
    pub bookmarks: BookmarksConfig,
    /// Unified operation manager for file operations (copy, move, delete, upload, download).
    /// This is the new centralized system that will eventually replace the individual
    /// operation handles (local_copy_operation, batch_download_operation, etc.).
    pub operation_manager: Option<OperationManager>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    /// Create new application state, loading config from file
    pub fn new() -> Self {
        let config = Config::load().unwrap_or_else(|e| {
            eprintln!("Warning: Could not load config: {}. Using defaults.", e);
            Config::default()
        });
        let theme = Theme::get_by_name(&config.general.theme);
        Self::with_config_and_theme(config, theme)
    }

    /// Create new application state with given config and theme
    pub fn with_config_and_theme(config: Config, theme: &'static Theme) -> Self {
        let layout_info = LayoutInfo {
            mode: LayoutMode::Single,
            main_panels_count: 1,
            main_panel_width: DEFAULT_MAIN_PANEL_WIDTH,
        };

        // Create LSP manager if enabled
        let lsp_manager = if config.lsp.enabled {
            let lsp_config = Self::create_lsp_config(&config);
            Some(LspManager::new(lsp_config))
        } else {
            None
        };

        // Load bookmarks from data directory
        let bookmarks = BookmarksConfig::load();

        Self {
            should_quit: false,
            ui: UiState::default(),
            terminal: TerminalState::default(),
            layout_mode: LayoutMode::Single,
            layout_info,
            active_modal: None,
            pending_action: None,
            dir_size_receiver: None,
            git_operation_handle: None,
            script_operation_handle: None,
            download_operation: None,
            upload_operation: None,
            batch_download_operation: None,
            local_copy_operation: None,
            local_directory_copy_operation: None,
            local_scan_operation: None,
            local_delete_operation: None,
            vfs_upload_state: None,
            batch_upload_operation: None,
            watcher: None,
            theme,
            config,
            system_monitor: SystemMonitor::new(),
            last_resource_update: std::time::Instant::now(),
            last_session_save: None,
            needs_redraw: true, // Initial draw needed
            last_spinner_update: None,
            lsp_manager,
            all_diagnostics: HashMap::new(),
            bookmarks,
            operation_manager: None, // Will be initialized when VfsManager is available
        }
    }

    /// Create LSP configuration from app config
    fn create_lsp_config(config: &Config) -> LspConfig {
        let mut servers = std::collections::HashMap::new();

        for (lang, server_config) in &config.lsp.servers {
            servers.insert(
                lang.clone(),
                LspServerConfig {
                    command: server_config.command.clone(),
                    args: server_config.args.clone(),
                    root_markers: server_config.root_markers.clone(),
                },
            );
        }

        LspConfig { servers }
    }

    /// Set new theme and update config
    pub fn set_theme(&mut self, theme_name: &str) {
        self.theme = Theme::get_by_name(theme_name);
        self.config.general.theme = theme_name.to_string();
    }

    /// Request application quit
    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Open menu
    pub fn open_menu(&mut self, menu_index: Option<usize>) {
        self.ui.menu_open = true;
        self.ui.selected_menu_item = menu_index;
        self.ui.selected_dropdown_item = 0;
    }

    /// Close menu
    pub fn close_menu(&mut self) {
        self.ui.menu_open = false;
        self.ui.selected_menu_item = None;
        self.ui.selected_dropdown_item = 0;
        self.ui.close_all_submenus();
    }

    /// Open submenu (e.g., Preferences dropdown)
    pub fn open_submenu(&mut self) {
        self.ui.close_all_submenus();
        self.ui.options_submenu.open();
    }

    /// Close submenu and all nested menus
    pub fn close_submenu(&mut self) {
        self.ui.options_submenu.close();
        self.ui.nested_submenu.close();
    }

    /// Open Sessions submenu
    pub fn open_sessions_submenu(&mut self) {
        self.ui.close_all_submenus();
        self.ui.sessions_submenu.open();
    }

    /// Close Sessions submenu
    pub fn close_sessions_submenu(&mut self) {
        self.ui.sessions_submenu.close();
    }

    /// Open Tools submenu
    pub fn open_tools_submenu(&mut self) {
        self.ui.close_all_submenus();
        self.ui.tools_submenu.open();
    }

    /// Close Tools submenu
    pub fn close_tools_submenu(&mut self) {
        self.ui.tools_submenu.close();
    }

    /// Open Scripts submenu
    pub fn open_scripts_submenu(&mut self) {
        self.ui.close_all_submenus();
        self.ui.scripts_submenu.open();
    }

    /// Close Scripts submenu
    pub fn close_scripts_submenu(&mut self) {
        self.ui.scripts_submenu.close();
        self.ui.scripts_nested.close();
        self.ui.current_scripts_group = None;
    }

    /// Open Scripts nested submenu (for a group)
    pub fn open_scripts_nested_submenu(&mut self, group_name: String) {
        self.ui.scripts_nested.open();
        self.ui.current_scripts_group = Some(group_name);
    }

    /// Close Scripts nested submenu
    pub fn close_scripts_nested_submenu(&mut self) {
        self.ui.scripts_nested.close();
        self.ui.current_scripts_group = None;
    }

    /// Open nested submenu (e.g., Themes list)
    pub fn open_nested_submenu(&mut self, initial_item: usize) {
        self.ui.nested_submenu.open_at(initial_item);
    }

    /// Close nested submenu (return to parent submenu)
    pub fn close_nested_submenu(&mut self) {
        self.ui.nested_submenu.close();
    }

    /// Toggle menu
    pub fn toggle_menu(&mut self) {
        if self.ui.menu_open {
            self.close_menu();
        } else {
            self.open_menu(Some(0));
        }
    }

    /// Move to next menu item
    pub fn next_menu_item(&mut self, menu_count: usize) {
        if let Some(current) = self.ui.selected_menu_item {
            self.ui.selected_menu_item = Some((current + 1) % menu_count);
            self.ui.selected_dropdown_item = 0;
        }
    }

    /// Move to previous menu item
    pub fn prev_menu_item(&mut self, menu_count: usize) {
        if let Some(current) = self.ui.selected_menu_item {
            self.ui.selected_menu_item = Some(if current == 0 {
                menu_count - 1
            } else {
                current - 1
            });
            self.ui.selected_dropdown_item = 0;
        }
    }

    /// Update terminal dimensions
    pub fn update_terminal_size(&mut self, width: u16, height: u16) {
        self.terminal.width = width;
        self.terminal.height = height;
        self.layout_info = LayoutInfo::calculate(width);
        self.layout_mode = self.layout_info.mode;
    }

    /// Get recommended layout based on terminal width
    pub fn get_recommended_layout(&self) -> &'static str {
        self.layout_info.recommended_layout_str()
    }

    /// Close modal window
    pub fn close_modal(&mut self) {
        self.active_modal = None;
    }

    /// Check if modal window is open
    pub fn has_modal(&self) -> bool {
        self.active_modal.is_some()
    }

    /// Get mutable reference to active modal window
    pub fn get_active_modal_mut(&mut self) -> Option<&mut ActiveModal> {
        self.active_modal.as_mut()
    }

    /// Set pending action and open modal window
    pub fn set_pending_action(&mut self, action: PendingAction, modal: ActiveModal) {
        self.pending_action = Some(action);
        self.active_modal = Some(modal);
    }

    /// Take pending action (take ownership)
    pub fn take_pending_action(&mut self) -> Option<PendingAction> {
        self.pending_action.take()
    }

    /// Set error message
    pub fn set_error(&mut self, message: String) {
        self.ui.status_message = Some((message, true));
    }

    /// Set informational message
    pub fn set_info(&mut self, message: String) {
        self.ui.status_message = Some((message, false));
    }

    /// Clear status message
    pub fn clear_status(&mut self) {
        self.ui.status_message = None;
    }

    /// Create EditorConfig with settings from global config
    pub fn editor_config(&self) -> EditorConfig {
        let mut config = EditorConfig::default();
        config.tab_size = self.config.editor.tab_size;
        config.word_wrap = self.config.editor.word_wrap;
        config.vim_mode = self.config.general.vim_mode;
        config.keybindings = self.config.editor.keybindings.clone();
        config
    }

    /// Check if enough time has passed since last session save (debounce check)
    /// Returns true if we should save the session
    pub fn should_save_session(&self) -> bool {
        const DEBOUNCE_DURATION: std::time::Duration = std::time::Duration::from_secs(1);

        match self.last_session_save {
            None => true, // Never saved before
            Some(last_save) => last_save.elapsed() >= DEBOUNCE_DURATION,
        }
    }

    /// Update last session save timestamp
    pub fn update_last_session_save(&mut self) {
        self.last_session_save = Some(std::time::Instant::now());
    }

    /// Save bookmarks to data directory
    pub fn save_bookmarks(&self) {
        if let Err(e) = self.bookmarks.save() {
            termide_logger::error(format!("Failed to save bookmarks: {}", e));
        }
    }

    /// Open bookmarks submenu
    pub fn open_bookmarks_submenu(&mut self) {
        self.ui.close_all_submenus();
        self.ui.bookmarks_submenu.open();
    }

    /// Close bookmarks submenu
    pub fn close_bookmarks_submenu(&mut self) {
        self.ui.bookmarks_submenu.close();
        self.ui.bookmarks_nested.close();
        self.ui.current_bookmarks_group = None;
    }

    /// Open bookmarks nested submenu (for a group)
    pub fn open_bookmarks_nested_submenu(&mut self, group_name: String) {
        self.ui.bookmarks_nested.open();
        self.ui.current_bookmarks_group = Some(group_name);
    }

    /// Close bookmarks nested submenu
    pub fn close_bookmarks_nested_submenu(&mut self) {
        self.ui.bookmarks_nested.close();
        self.ui.current_bookmarks_group = None;
    }

    // ========================================================================
    // Operation Manager Methods
    // ========================================================================

    /// Initialize the operation manager with a VFS manager.
    /// This should be called when the first VFS operation is needed.
    pub fn init_operation_manager(&mut self, vfs_manager: Arc<VfsManager>) {
        if self.operation_manager.is_none() {
            self.operation_manager = Some(OperationManager::new(vfs_manager));
        }
    }

    /// Get reference to operation manager if initialized.
    pub fn operation_manager(&self) -> Option<&OperationManager> {
        self.operation_manager.as_ref()
    }

    /// Get mutable reference to operation manager if initialized.
    pub fn operation_manager_mut(&mut self) -> Option<&mut OperationManager> {
        self.operation_manager.as_mut()
    }

    /// Queue a file operation. Returns the operation ID if successful.
    /// Initializes the operation manager with the provided VFS manager if needed.
    pub fn queue_operation(
        &mut self,
        request: OperationRequest,
        vfs_manager: Arc<VfsManager>,
    ) -> Result<termide_file_ops::OperationId, termide_file_ops::OperationError> {
        self.init_operation_manager(vfs_manager);
        self.operation_manager_mut()
            .expect("operation_manager just initialized")
            .queue_operation(request)
    }

    /// Poll operation manager for events. Returns empty vec if not initialized.
    pub fn poll_operations(&mut self) -> Vec<OperationEvent> {
        self.operation_manager_mut()
            .map(|m| m.poll())
            .unwrap_or_default()
    }

    /// Check if there are any active or queued operations.
    pub fn has_pending_operations(&self) -> bool {
        self.operation_manager()
            .map(|m| m.has_operations())
            .unwrap_or(false)
    }

    /// Cancel all operations.
    pub fn cancel_all_operations(&mut self) {
        if let Some(manager) = self.operation_manager_mut() {
            manager.cancel_all();
        }
    }

    /// Get summary of background operations for status bar display.
    pub fn background_operations_summary(&self) -> Option<BackgroundOperationSummary> {
        self.operation_manager()
            .map(|m| m.background_summary())
            .filter(|s| s.has_operations())
    }
}

// ============================================================================
// Core Trait Implementations
// ============================================================================

impl StateManager for AppState {
    fn ui(&self) -> &UiState {
        &self.ui
    }

    fn ui_mut(&mut self) -> &mut UiState {
        &mut self.ui
    }

    fn set_info(&mut self, msg: String) {
        self.ui.status_message = Some((msg, false));
    }

    fn set_error(&mut self, msg: String) {
        self.ui.status_message = Some((msg, true));
    }

    fn clear_status(&mut self) {
        self.ui.status_message = None;
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }

    fn set_redraw(&mut self, value: bool) {
        self.needs_redraw = value;
    }
}

impl ModalManager for AppState {
    fn active_modal(&self) -> Option<&ActiveModal> {
        self.active_modal.as_ref()
    }

    fn active_modal_mut(&mut self) -> Option<&mut ActiveModal> {
        self.active_modal.as_mut()
    }

    fn open_modal(&mut self, modal: ActiveModal, action: Option<PendingAction>) {
        self.active_modal = Some(modal);
        self.pending_action = action;
    }

    fn close_modal(&mut self) {
        self.active_modal = None;
    }

    fn take_pending_action(&mut self) -> Option<PendingAction> {
        self.pending_action.take()
    }
}
