//! Main application module.
//!
//! Contains the App struct and all event handlers.

// Note: PanelExt is used for panel-specific operations (get current path, save editor)
// that require concrete type access. Common operations use Panel::handle_command().
#![allow(deprecated)]

use anyhow::Result;
use ratatui::{backend::Backend, Terminal};
use std::str::FromStr;
use std::time::Duration;

use termide_app_core::{LayoutController, PanelProvider};
use termide_app_event::DefaultHotkeyProcessor;
use termide_core::event::{Event, EventHandler};
use termide_layout::LayoutManager;

use crate::state::AppState;
use crate::PanelExt;

// Panel trait re-export
pub use termide_core::Panel;

mod background_ops;
mod event_handler;
mod file_transfer_ops;
mod global_hotkeys;
mod key_handler;
mod local_ops;
mod menu_actions;
mod modal;
mod modal_handler;
mod mouse_handler;
mod operation_manager_handler;
mod panel_manager;
mod panel_operations;
mod session;
mod watcher;

/// Main application
pub struct App {
    state: AppState,
    layout_manager: LayoutManager,
    event_handler: EventHandler,
    /// Project root directory (used for per-project session storage)
    project_root: std::path::PathBuf,
    /// Global hotkey processor
    hotkey_processor: DefaultHotkeyProcessor,
}

impl App {
    /// Create a new application
    pub fn new() -> Self {
        let mut state = AppState::new();

        // Get project root from current working directory
        let project_root = std::env::current_dir().unwrap_or_else(|_| {
            // Fallback to home directory if current_dir fails
            dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"))
        });

        // Initialize logger in session directory (before other initializations that log)
        // Use config override if specified, otherwise use session directory with unique filename
        let log_file_path = if let Some(ref path) = state.config.logging.file_path {
            std::path::PathBuf::from(path)
        } else {
            termide_session::Session::get_session_dir(&project_root)
                .map(|dir| {
                    // Cleanup old log files (older than 24 hours)
                    let _ = termide_session::cleanup_old_logs(&dir);
                    dir.join(termide_session::generate_log_filename())
                })
                .unwrap_or_else(|_| {
                    std::env::temp_dir().join(termide_session::generate_log_filename())
                })
        };
        let min_log_level = termide_logger::LogLevel::from_str(&state.config.logging.min_level)
            .ok()
            .unwrap_or(termide_logger::LogLevel::Info);
        termide_logger::init(
            log_file_path,
            termide_config::constants::MAX_LOG_ENTRIES,
            min_log_level,
        );
        log::info!("Application started");

        // Initialize unified watcher for filesystem and git events
        match termide_watcher::create_watcher() {
            Ok(watcher) => {
                state.watcher = Some(watcher);
                log::info!("Unified watcher initialized");
            }
            Err(e) => {
                log::error!("Failed to initialize watcher: {}", e);
            }
        }

        // Clean up old sessions (configurable retention period)
        let retention_days = state.config.general.session_retention_days;
        if let Err(e) = termide_session::cleanup_old_sessions(&project_root, retention_days) {
            log::warn!("Failed to cleanup old sessions: {}", e);
        }

        // Create hotkey processor from config before moving state
        let hotkey_processor =
            DefaultHotkeyProcessor::from_config(&state.config.general.keybindings);

        Self {
            state,
            layout_manager: LayoutManager::new(),
            event_handler: EventHandler::new(Duration::from_millis(
                termide_config::constants::EVENT_HANDLER_INTERVAL_MS,
            )),
            project_root,
            hotkey_processor,
        }
    }

    /// Create a new application with specified terminal size
    /// This is useful during initialization to set proper terminal dimensions
    /// before creating panels
    pub fn new_with_size(width: u16, height: u16) -> Self {
        let mut app = Self::new();
        app.state.update_terminal_size(width, height);
        app
    }

    /// Log git availability status to journal
    pub fn log_git_status(&self, git_available: bool) {
        if git_available {
            log::info!("Git detected and available");
        } else {
            log::warn!("Git not found - git integration disabled");
        }
    }

    /// Add a panel (automatically stacks if width threshold is reached)
    pub fn add_panel(&mut self, mut panel: Box<dyn Panel>) {
        use termide_core::PanelCommand;

        // Notify new panel about current git operation state
        if self.state.ui.git_operation_in_progress {
            let operation = self
                .state
                .git_operation_handle
                .as_ref()
                .map(|h| h.operation.clone());
            panel.handle_command(PanelCommand::SetGitOperationInProgress {
                in_progress: true,
                operation,
                spinner_frame: self.state.ui.spinner_frame,
            });
        }

        let terminal_width = self.state.terminal.width;
        let config = &self.state.config;
        self.layout_manager.add_panel(panel, config, terminal_width);
    }

    /// Add panel without changing focus.
    /// Used for preview panels where focus should stay on the source panel.
    pub fn add_panel_without_focus(&mut self, mut panel: Box<dyn Panel>) {
        use termide_core::PanelCommand;

        // Notify new panel about current git operation state
        if self.state.ui.git_operation_in_progress {
            let operation = self
                .state
                .git_operation_handle
                .as_ref()
                .map(|h| h.operation.clone());
            panel.handle_command(PanelCommand::SetGitOperationInProgress {
                in_progress: true,
                operation,
                spinner_frame: self.state.ui.spinner_frame,
            });
        }

        let terminal_width = self.state.terminal.width;
        let config = &self.state.config;
        self.layout_manager
            .add_panel_without_focus(panel, config, terminal_width);
    }

    /// Run the main application loop
    pub fn run<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        render_fn: impl Fn(&mut ratatui::Frame<'_>, &mut AppState, &mut LayoutManager),
    ) -> Result<()>
    where
        B::Error: Send + Sync + 'static,
    {
        // Initialize terminal dimensions
        let size = terminal.size()?;
        self.state.update_terminal_size(size.width, size.height);

        while !self.state.should_quit {
            // Process events
            match self.event_handler.next()? {
                Event::Key(key) => {
                    self.handle_key_event(key)?;
                    self.state.needs_redraw = true;
                }
                Event::Mouse(mouse) => {
                    self.handle_mouse_event(mouse)?;
                    self.state.needs_redraw = true;
                }
                Event::Resize(width, height) => {
                    // Update terminal dimensions in state
                    self.state.update_terminal_size(width, height);

                    // Пропорционально перераспределить ширины групп при изменении размера терминала
                    self.layout_manager
                        .redistribute_widths_proportionally(width);
                    self.state.needs_redraw = true;
                }
                Event::FocusLost => {
                    // Save session on focus loss (with debounce)
                    if self.state.should_save_session() {
                        self.auto_save_session();
                        self.state.update_last_session_save();
                    }
                }
                Event::FocusGained => {
                    // Redraw on focus gain to refresh display
                    self.state.needs_redraw = true;
                }
                Event::Paste(text) => {
                    // Handle bracketed paste - check modal first, then send to active panel
                    if !self.handle_modal_paste(&text) {
                        self.handle_paste_event(text)?;
                    }
                    self.state.needs_redraw = true;
                }
                Event::MouseScrollCoalesced { event, delta } => {
                    // Handle coalesced scroll events (batched for performance)
                    self.handle_coalesced_scroll(event, delta)?;
                    self.state.needs_redraw = true;
                }
                Event::Tick => {
                    // Debounce scroll renders: consume pending flag and trigger redraw
                    if self.state.pending_scroll_render {
                        self.state.pending_scroll_render = false;
                        self.state.needs_redraw = true;
                    }

                    // Detect active scrolling (within 100ms of last scroll event)
                    // Skip heavy operations during scrolling to prevent UI lag
                    let is_scrolling = self
                        .state
                        .last_mouse_scroll
                        .map(|t| t.elapsed() < Duration::from_millis(100))
                        .unwrap_or(false);

                    // Check terminal panels for pending output (efficient redraw trigger)
                    // This is fast, always run it
                    for panel in self.layout_manager.iter_all_panels_mut() {
                        if let Some(terminal) = panel.as_terminal_mut() {
                            if terminal.has_pending_output() {
                                self.state.needs_redraw = true;
                                break; // One terminal with output is enough to trigger redraw
                            }
                        }
                    }

                    // Skip heavy operations during active scrolling
                    if !is_scrolling {
                        // Call tick() on all panels to process periodic operations
                        // (FileManager: VFS operations, Editor/Terminal: auto-scroll during selection drag)
                        // Collect events first, then process them (to avoid borrow issues)
                        let mut all_panel_events = Vec::new();
                        for panel in self.layout_manager.iter_all_panels_mut() {
                            // Call tick() on all panels
                            let events = panel.tick();
                            if !events.is_empty() {
                                self.state.needs_redraw = true;
                                all_panel_events.extend(events);
                            }

                            // FileManager-specific: check for pending operations for spinner animation
                            if let Some(fm) = panel.as_file_manager_mut() {
                                if fm.vfs_state().has_pending_operation() {
                                    self.state.needs_redraw = true;
                                }
                            }
                        }
                        // Process collected events
                        if !all_panel_events.is_empty() {
                            let _ = self.process_panel_events(all_panel_events);
                        }

                        // Check for modal requests from FileManager panels (e.g., VFS error modals)
                        // This must happen after tick() processing to show connection error modals
                        let modal_request = self
                            .layout_manager
                            .iter_all_panels_mut()
                            .find_map(|panel| panel.take_modal_request());
                        if let Some((action, modal)) = modal_request {
                            let _ = self.handle_modal_request(action, modal);
                            self.state.needs_redraw = true;
                        }

                        // Check for pending upload operations from Editor panels (Ctrl+S remote saves)
                        let pending_upload = self
                            .layout_manager
                            .iter_all_panels_mut()
                            .find_map(|panel| panel.take_pending_upload());
                        if let Some((temp_path, remote_path, vfs_manager)) = pending_upload {
                            self.handle_pending_upload(temp_path, remote_path, vfs_manager);
                            self.state.needs_redraw = true;
                        }

                        // Check channel for directory size calculation results
                        self.check_dir_size_update();

                        // Check unified watcher for git and filesystem events
                        self.check_watcher_events();

                        // Check async git status results for FileManager panels
                        self.check_fm_git_status_async();

                        // Check pending git diff updates (debounced)
                        self.check_pending_git_diff_updates();

                        // Check background git operation result (push/pull)
                        self.check_git_operation_result();

                        // Check background script operation result (.report. scripts)
                        self.check_script_operation_result();

                        // Poll unified operation manager for events (new system)
                        self.poll_operation_manager();

                        // Sync active operations data to the operations panel
                        self.update_operations_panel();

                        // Check pending local batch operation (start after modal rendered)
                        self.check_pending_batch_operation();
                    }

                    // Sync pause state between BatchOperation and ProgressModal (bidirectional)
                    // This is fast, always run it
                    if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                        self.state.active_modal
                    {
                        if let Some(termide_state::PendingAction::ContinueBatchOperation {
                            ref mut operation,
                        }) = self.state.pending_action
                        {
                            let modal_paused = modal.is_paused();
                            let operation_paused =
                                operation.pause_state == termide_state::PauseState::Paused;

                            // If states differ, sync from modal to operation (user interaction takes priority)
                            if modal_paused != operation_paused {
                                operation.pause_state = if modal_paused {
                                    termide_state::PauseState::Paused
                                } else {
                                    termide_state::PauseState::Running
                                };
                            }
                        }
                    }

                    // Update system resource monitoring (CPU, RAM)
                    // This is fast, always run it
                    self.update_system_resources();

                    // Poll LSP completion responses for active editor
                    // This is fast, always run it
                    self.poll_lsp_completion();

                    // Update spinner in all modals that support animation
                    // This is fast, always run it
                    self.update_modal_spinners();
                }
            }

            // Check and close panels that should auto-close
            self.check_auto_close_panels()?;

            // Render UI only when needed (reduces idle CPU from 24fps to near-zero)
            if self.state.needs_redraw {
                terminal.draw(|frame| {
                    render_fn(frame, &mut self.state, &mut self.layout_manager);
                })?;
                self.state.needs_redraw = false;
            }
        }

        Ok(())
    }

    /// Check and close panels that should auto-close
    fn check_auto_close_panels(&mut self) -> Result<()> {
        // Check if active panel should auto-close
        let should_close = {
            if let Some(panel) = self.layout_manager.active_panel_mut() {
                panel.should_auto_close()
            } else {
                false
            }
        };

        if should_close && self.layout_manager.can_close_active() {
            // Calculate available width for panel groups
            let terminal_width = self.state.terminal.width;

            let _ = self.layout_manager.close_active_panel(terminal_width);
            self.auto_save_session();
        }

        Ok(())
    }

    // ===== Panel downcast helpers =====
    // These helper methods reduce boilerplate for accessing specific panel types

    /// Get mutable reference to active editor panel
    /// Helper to avoid nested if-let chains: `if let Some(panel) = ... { if let Some(editor) = ... }`
    fn active_editor_mut(&mut self) -> Option<&mut termide_panel_editor::Editor> {
        self.layout_manager
            .active_panel_mut()
            .and_then(|panel| panel.as_editor_mut())
    }

    /// Get reference to AppState
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Get mutable reference to AppState
    pub fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
    }

    /// Get reference to LayoutManager
    pub fn layout_manager(&self) -> &LayoutManager {
        &self.layout_manager
    }

    /// Get mutable reference to LayoutManager
    pub fn layout_manager_mut(&mut self) -> &mut LayoutManager {
        &mut self.layout_manager
    }

    /// Find existing panel by name and focus on it
    /// Returns true if found and focused, false if not found
    fn find_and_focus_panel_by_name(&mut self, name: &str) -> bool {
        for (group_idx, group) in self.layout_manager.panel_groups.iter_mut().enumerate() {
            for (panel_idx, panel) in group.panels().iter().enumerate() {
                if panel.name() == name {
                    self.layout_manager.focus = group_idx;
                    group.set_expanded(panel_idx);
                    return true;
                }
            }
        }
        false
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Core Trait Implementations
// ============================================================================

impl PanelProvider for App {
    fn active_panel(&self) -> Option<&dyn Panel> {
        self.layout_manager
            .active_panel()
            .map(|p| p.as_ref() as &dyn Panel)
    }

    fn active_panel_mut(&mut self) -> Option<&mut Box<dyn Panel>> {
        self.layout_manager.active_panel_mut()
    }

    fn active_panel_index(&self) -> Option<usize> {
        self.layout_manager.active_group_index()
    }

    fn panel_count(&self) -> usize {
        self.layout_manager.panel_count()
    }

    fn iter_panels_mut(&mut self) -> Box<dyn Iterator<Item = &mut Box<dyn Panel>> + '_> {
        Box::new(self.layout_manager.iter_all_panels_mut())
    }
}

impl LayoutController for App {
    fn add_panel(&mut self, panel: Box<dyn Panel>) {
        // Use the main add_panel method which handles git operation state notification
        App::add_panel(self, panel);
    }

    fn close_active(&mut self) -> Result<()> {
        let terminal_width = self.state.terminal.width;
        self.layout_manager.close_active_panel(terminal_width)
    }

    fn next_group(&mut self) {
        self.layout_manager.next_group();
    }

    fn prev_group(&mut self) {
        self.layout_manager.prev_group();
    }

    fn next_in_group(&mut self) {
        self.layout_manager.next_panel_in_group();
    }

    fn prev_in_group(&mut self) {
        self.layout_manager.prev_panel_in_group();
    }

    fn set_focus(&mut self, index: usize) {
        self.layout_manager.set_focus(index);
    }
}
