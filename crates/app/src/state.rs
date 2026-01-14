//! Application state and types.
//!
//! Re-exports pure types from termide-state crate and defines
//! complex types that depend on other application modules.
//!
//! Implements core traits from termide-app-core for standardized
//! state management and modal handling.

use std::sync::mpsc;

use termide_config::constants::DEFAULT_MAIN_PANEL_WIDTH;
use termide_config::Config;
use termide_panel_editor::EditorConfig;
use termide_system_monitor::SystemMonitor;
use termide_theme::Theme;
use termide_watcher::UnifiedWatcher;

// Import core traits
use termide_app_core::{ModalManager, StateManager};

// Re-export pure types from state crate
pub use termide_state::{
    BatchOperation, BatchOperationType, ConflictMode, DirSizeResult, LayoutInfo, LayoutMode,
    PendingAction, RenamePattern, TerminalState, UiState,
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
            watcher: None,
            theme,
            config,
            system_monitor: SystemMonitor::new(),
            last_resource_update: std::time::Instant::now(),
            last_session_save: None,
            needs_redraw: true, // Initial draw needed
            last_spinner_update: None,
        }
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
        self.close_submenu();
        self.close_sessions_submenu();
        self.close_tools_submenu();
    }

    /// Open submenu (e.g., Preferences dropdown)
    pub fn open_submenu(&mut self) {
        self.ui.submenu_open = true;
        self.ui.selected_submenu_item = 0;
        self.ui.nested_submenu_open = false;
    }

    /// Close submenu and all nested menus
    pub fn close_submenu(&mut self) {
        self.ui.submenu_open = false;
        self.ui.selected_submenu_item = 0;
        self.ui.nested_submenu_open = false;
        self.ui.selected_nested_item = 0;
    }

    /// Open Sessions submenu
    pub fn open_sessions_submenu(&mut self) {
        self.ui.sessions_submenu_open = true;
        self.ui.selected_sessions_item = 0;
    }

    /// Close Sessions submenu
    pub fn close_sessions_submenu(&mut self) {
        self.ui.sessions_submenu_open = false;
        self.ui.selected_sessions_item = 0;
    }

    /// Open Tools submenu
    pub fn open_tools_submenu(&mut self) {
        self.ui.tools_submenu_open = true;
        self.ui.selected_tools_item = 0;
    }

    /// Close Tools submenu
    pub fn close_tools_submenu(&mut self) {
        self.ui.tools_submenu_open = false;
        self.ui.selected_tools_item = 0;
    }

    /// Open nested submenu (e.g., Themes list)
    pub fn open_nested_submenu(&mut self, initial_item: usize) {
        self.ui.nested_submenu_open = true;
        self.ui.selected_nested_item = initial_item;
    }

    /// Close nested submenu (return to parent submenu)
    pub fn close_nested_submenu(&mut self) {
        self.ui.nested_submenu_open = false;
        self.ui.selected_nested_item = 0;
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
