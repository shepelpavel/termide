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
use termide_i18n::t;
use termide_layout::LayoutManager;
use termide_logger as logger;

use crate::LayoutManagerSession;

use crate::state::AppState;
use crate::PanelExt;

// Panel trait re-export
pub use termide_core::Panel;

mod event_handler;
mod global_hotkeys;
mod key_handler;
mod menu_actions;
mod modal;
mod modal_handler;
mod mouse_handler;
mod panel_manager;
mod panel_operations;

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
        termide_logger::info("Application started");

        // Initialize unified watcher for filesystem and git events
        match termide_watcher::create_watcher() {
            Ok(watcher) => {
                state.watcher = Some(watcher);
                termide_logger::info("Unified watcher initialized");
            }
            Err(e) => {
                termide_logger::error(format!("Failed to initialize watcher: {}", e));
            }
        }

        // Clean up old sessions (configurable retention period)
        let retention_days = state.config.general.session_retention_days;
        if let Err(e) = termide_session::cleanup_old_sessions(&project_root, retention_days) {
            termide_logger::warn(format!("Failed to cleanup old sessions: {}", e));
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
        let tr = t();
        if git_available {
            logger::info(tr.git_detected());
        } else {
            logger::warn(tr.git_not_found());
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
                    // Handle bracketed paste - send to active panel
                    self.handle_paste_event(text)?;
                    self.state.needs_redraw = true;
                }
                Event::Tick => {
                    // Check terminal panels for pending output (efficient redraw trigger)
                    for panel in self.layout_manager.iter_all_panels_mut() {
                        if let Some(terminal) = panel.as_terminal_mut() {
                            if terminal.has_pending_output() {
                                self.state.needs_redraw = true;
                                break; // One terminal with output is enough to trigger redraw
                            }
                        }
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

                    // Update system resource monitoring (CPU, RAM)
                    self.update_system_resources();

                    // Poll LSP completion responses for active editor
                    self.poll_lsp_completion();

                    // Update spinner in Info modal if it's open
                    self.update_info_modal_spinner();
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

    /// Check channel for directory size calculation results
    fn check_dir_size_update(&mut self) {
        use crate::state::ActiveModal;
        use termide_panel_file_manager::FileManager;

        if let Some(rx) = &self.state.dir_size_receiver {
            // Try to receive result without blocking
            if let Ok(result) = rx.try_recv() {
                let t = termide_i18n::t();
                let formatted_size = FileManager::format_size_static(result.size);

                // Update Info or InfoAction modal if open
                match &mut self.state.active_modal {
                    Some(ActiveModal::Info(ref mut modal)) => {
                        modal.update_value(t.file_info_size(), formatted_size);
                        self.state.needs_redraw = true;
                    }
                    Some(ActiveModal::InfoAction(ref mut modal)) => {
                        modal.update_value(t.file_info_size(), formatted_size);
                        self.state.needs_redraw = true;
                    }
                    _ => {}
                }

                // Clear channel
                self.state.dir_size_receiver = None;
            }
        }
    }

    /// Check unified watcher for git and filesystem events
    fn check_watcher_events(&mut self) {
        use std::collections::HashSet;
        use termide_core::{CommandResult, PanelCommand};
        use termide_git::find_repo_root;
        use termide_watcher::WatchEvent;

        let Some(watcher) = &mut self.state.watcher else {
            return;
        };

        // Lazy registration: register panel directories with watcher
        for panel in self.layout_manager.iter_all_panels_mut() {
            // Use GetFsWatchInfo to check watch state
            if let CommandResult::FsWatchInfo {
                watched_root,
                current_path,
                is_git_repo: _,
            } = panel.handle_command(PanelCommand::GetFsWatchInfo)
            {
                if watched_root.is_none() {
                    // Determine the new watched root
                    let repo_root = find_repo_root(&current_path);
                    let is_git_repo = repo_root.is_some();
                    let new_root = repo_root.unwrap_or_else(|| current_path.clone());

                    // Watch new root (now fast - respects .gitignore)
                    if is_git_repo {
                        if !watcher.is_watching_repo(&new_root) {
                            let _ = watcher.watch_repository(new_root.clone());
                        }
                    } else if !watcher.is_watching_dir(&new_root) {
                        let _ = watcher.watch_directory(new_root.clone());
                    }

                    // Update panel's watched root
                    panel.handle_command(PanelCommand::SetFsWatchRoot {
                        root: Some(new_root),
                        is_git_repo,
                    });
                }
            }

            // Also handle Editor panels via GetRepoRoot
            if let CommandResult::RepoRoot(Some(repo_root)) =
                panel.handle_command(PanelCommand::GetRepoRoot)
            {
                if !watcher.is_watching_repo(&repo_root) {
                    let _ = watcher.watch_repository(repo_root);
                }
            }
        }

        // Poll events from unified watcher
        let events = watcher.poll_events();
        if events.is_empty() {
            return;
        }

        // Separate git and fs events
        let mut git_repos: HashSet<std::path::PathBuf> = HashSet::new();
        let mut fs_paths: HashSet<std::path::PathBuf> = HashSet::new();

        let mut gitignore_changed_repos: Vec<std::path::PathBuf> = Vec::new();

        for event in events {
            match event {
                WatchEvent::GitCommit(repo_root) => {
                    git_repos.insert(repo_root);
                }
                WatchEvent::DirectoryChanged { changed, .. } => {
                    fs_paths.insert(changed);
                }
                WatchEvent::FileChanged(path) => {
                    fs_paths.insert(path);
                }
                WatchEvent::GitignoreChanged(repo_root) => {
                    gitignore_changed_repos.push(repo_root);
                }
            }
        }

        // Handle .gitignore changes - reinitialize watcher
        for repo_root in gitignore_changed_repos {
            watcher.unwatch_repository(&repo_root);
            let _ = watcher.watch_repository(repo_root);
        }

        // Process git events
        if !git_repos.is_empty() {
            let repo_paths: Vec<&std::path::Path> = git_repos.iter().map(|p| p.as_path()).collect();

            for panel in self.layout_manager.iter_all_panels_mut() {
                if panel
                    .handle_command(PanelCommand::OnGitUpdate {
                        repo_paths: &repo_paths,
                    })
                    .needs_redraw()
                {
                    self.state.needs_redraw = true;
                }
            }
        }

        // Process filesystem events
        for panel in self.layout_manager.iter_all_panels_mut() {
            for path in &fs_paths {
                if panel
                    .handle_command(PanelCommand::OnFsUpdate { changed_path: path })
                    .needs_redraw()
                {
                    self.state.needs_redraw = true;
                    break;
                }
            }
        }
    }

    /// Check async git status results for FileManager panels
    fn check_fm_git_status_async(&mut self) {
        for group in &mut self.layout_manager.panel_groups {
            for panel in group.panels_mut() {
                if let Some(fm) = panel.as_file_manager_mut() {
                    // Check for async git status results
                    if fm.check_git_status_async() {
                        self.state.needs_redraw = true;
                    }
                }
            }
        }
    }

    /// Check and apply pending git diff updates (debounced) and async git diff results
    fn check_pending_git_diff_updates(&mut self) {
        use termide_core::PanelCommand;

        // Check all panels for pending git diff updates and async results using handle_command
        for panel in self.layout_manager.iter_all_panels_mut() {
            // Check debounced buffer updates
            panel.handle_command(PanelCommand::CheckPendingGitDiff);
            // Check async git diff results (from background thread)
            if panel
                .handle_command(PanelCommand::CheckGitDiffReceiver)
                .needs_redraw()
            {
                self.state.needs_redraw = true;
            }
        }
    }

    /// Poll LSP status for all editors and completion for active editor
    fn poll_lsp_completion(&mut self) {
        // First, update LSP loading status for ALL editors (not just active)
        // This ensures spinners disappear and animate correctly for all panels
        let mut any_loading = false;
        for panel in self.layout_manager.iter_all_panels_mut() {
            if let Some(editor) = panel.as_editor_mut() {
                // Check if server loading status changed
                if let Some(ref lsp_manager) = self.state.lsp_manager {
                    if editor.update_lsp_loading_status(lsp_manager) {
                        // Server is now ready, trigger redraw to remove spinner
                        self.state.needs_redraw = true;
                    }
                }

                // Track if any editor is still loading (for spinner animation)
                if editor.is_lsp_loading() {
                    any_loading = true;
                }
            }
        }

        // Request periodic redraw for spinner animation while any editor is loading
        if any_loading {
            self.state.needs_redraw = true;
        }

        // Poll for diagnostics from LSP and dispatch to editors and diagnostics panel
        if let Some(ref lsp_manager) = self.state.lsp_manager {
            while let Some(params) = lsp_manager.poll_diagnostics() {
                // Convert URI to path - parse as URL then extract file path
                let uri_str = params.uri.as_str();
                if let Some(path_str) = uri_str.strip_prefix("file://") {
                    // On Unix paths start with /, on Windows with drive letter
                    #[cfg(unix)]
                    let path = std::path::PathBuf::from(path_str);
                    #[cfg(windows)]
                    let path = std::path::PathBuf::from(path_str.trim_start_matches('/'));

                    // Store in app state for later use (e.g., when opening diagnostics panel)
                    self.state
                        .all_diagnostics
                        .insert(path.clone(), params.diagnostics.clone());

                    // Find editor with this file and update diagnostics
                    for panel in self.layout_manager.iter_all_panels_mut() {
                        if let Some(editor) = panel.as_editor_mut() {
                            if editor.file_path() == Some(&path) {
                                editor.update_diagnostics(params.diagnostics.clone());
                                self.state.needs_redraw = true;
                                break;
                            }
                        }
                    }

                    // Update diagnostics panel if open
                    for panel in self.layout_manager.iter_all_panels_mut() {
                        if let Some(diag_panel) = panel.as_diagnostics_panel_mut() {
                            diag_panel.update_diagnostics(path.clone(), &params.diagnostics);
                            self.state.needs_redraw = true;
                        }
                    }
                }
            }
        }

        // Now handle completion and hover for the active editor only
        let mut pending_definition_event = None;
        if let Some(panel) = self.layout_manager.active_panel_mut() {
            if let Some(editor) = panel.as_editor_mut() {
                // Check if there's a pending completion response
                let had_popup_before = editor.has_completion_popup();
                editor.poll_completion();
                let has_popup_now = editor.has_completion_popup();

                // Check auto-completion timer if enabled
                if self.state.config.lsp.auto_completion {
                    if let Some(ref lsp_manager) = self.state.lsp_manager {
                        let delay_ms = self.state.config.lsp.completion_delay_ms;
                        if editor.check_auto_completion(lsp_manager, delay_ms) {
                            // Completion request triggered, needs redraw
                            self.state.needs_redraw = true;
                        }
                    }
                }

                // Trigger redraw if popup state changed
                if had_popup_before != has_popup_now {
                    self.state.needs_redraw = true;
                }

                // Check hover timer and request hover if expired
                if let Some(ref lsp_manager) = self.state.lsp_manager {
                    let delay_ms = self.state.config.lsp.hover_delay_ms;
                    if editor.check_hover_timer(lsp_manager, delay_ms) {
                        self.state.needs_redraw = true;
                    }
                }

                // Poll for hover response
                let had_hover_popup = editor.has_hover_popup();
                editor.poll_hover();
                if had_hover_popup != editor.has_hover_popup() {
                    self.state.needs_redraw = true;
                }

                // Poll for definition response (Ctrl+click go-to-definition)
                if let Some(event) = editor.poll_definition() {
                    // Store event to be processed after we release the borrow
                    pending_definition_event = Some(event);
                    self.state.needs_redraw = true;
                }
            }
        }

        // Process pending definition event (outside of panel borrow)
        if let Some(event) = pending_definition_event {
            let _ = self.process_panel_events(vec![event]);
        }
    }

    /// Update system resource monitoring (CPU, RAM)
    /// Respects the configured update interval
    fn update_system_resources(&mut self) {
        let interval =
            std::time::Duration::from_millis(self.state.config.logging.resource_monitor_interval);
        let elapsed = self.state.last_resource_update.elapsed();

        if elapsed >= interval {
            self.state.system_monitor.update();
            self.state.last_resource_update = std::time::Instant::now();
            self.state.needs_redraw = true;
        }
    }

    /// Update spinner in Info modal if it's open
    /// Throttled to 125ms (8 FPS) to reduce unnecessary redraws
    fn update_info_modal_spinner(&mut self) {
        use crate::state::ActiveModal;

        const SPINNER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(125);

        if let Some(ActiveModal::Info(ref mut modal)) = self.state.active_modal {
            // Update spinner only if calculation is still ongoing
            if self.state.dir_size_receiver.is_some() {
                // Throttle spinner updates
                let should_update = self
                    .state
                    .last_spinner_update
                    .is_none_or(|t| t.elapsed() >= SPINNER_INTERVAL);

                if should_update {
                    modal.advance_spinner();
                    self.state.last_spinner_update = Some(std::time::Instant::now());
                    self.state.needs_redraw = true;
                }
            }
        }
    }

    /// Check for background git operation result (push/pull)
    fn check_git_operation_result(&mut self) {
        use crate::state::ActiveModal;
        use std::sync::mpsc::TryRecvError;
        use termide_core::PanelCommand;
        use termide_modal::InfoModal;

        let handle = match self.state.git_operation_handle.take() {
            Some(h) => h,
            None => return,
        };

        match handle.receiver.try_recv() {
            Ok(result) => {
                self.state.ui.git_operation_in_progress = false;
                self.state.clear_status();
                // Notify all panels about git operation completed (shows Push/Pull buttons)
                self.notify_git_operation_state(false, None, 0);

                // Show result modal
                let t = termide_i18n::t();
                let title = if result.success {
                    if result.operation == "push" {
                        t.git_push_success()
                    } else {
                        t.git_pull_success()
                    }
                } else if result.operation == "push" {
                    t.git_push_failed()
                } else {
                    t.git_pull_failed()
                };

                // Collect output lines (no labels, just plain text)
                let mut lines = vec![];

                // Add stdout lines
                for line in result.stdout.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        lines.push((String::new(), trimmed.to_string()));
                    }
                }

                // Add stderr lines
                for line in result.stderr.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        lines.push((String::new(), trimmed.to_string()));
                    }
                }

                // Fallback if no output
                if lines.is_empty() {
                    lines.push((String::new(), t.git_completed().to_string()));
                }

                let modal = InfoModal::new(title, lines);
                self.state.active_modal = Some(ActiveModal::Info(Box::new(modal)));
                self.state.needs_redraw = true;

                // Refresh all git panels
                for panel in self.layout_manager.iter_all_panels_mut() {
                    panel.handle_command(PanelCommand::Reload);
                }
            }
            Err(TryRecvError::Empty) => {
                // Operation still in progress
                // Advance spinner frame for animation
                self.state.ui.spinner_frame = self.state.ui.spinner_frame.wrapping_add(1);

                // Notify all panels with updated spinner frame
                let operation = Some(handle.operation.clone());
                let spinner_frame = self.state.ui.spinner_frame;
                for panel in self.layout_manager.iter_all_panels_mut() {
                    panel.handle_command(PanelCommand::SetGitOperationInProgress {
                        in_progress: true,
                        operation: operation.clone(),
                        spinner_frame,
                    });
                }

                // Also advance spinner on InfoActionModal if open
                if let Some(ActiveModal::InfoAction(ref mut modal)) = self.state.active_modal {
                    if modal.is_operation_in_progress() {
                        modal.advance_spinner();
                    }
                }

                // Put handle back
                self.state.git_operation_handle = Some(handle);
                self.state.needs_redraw = true;
            }
            Err(TryRecvError::Disconnected) => {
                // Thread finished without sending (shouldn't happen)
                self.state.ui.git_operation_in_progress = false;
                self.state.clear_status();
                // Notify all panels about git operation completed (shows Push/Pull buttons)
                self.notify_git_operation_state(false, None, 0);
            }
        }
    }

    /// Save current session to file
    fn save_session(&mut self) -> Result<()> {
        // Get session directory for this project
        let session_dir = termide_session::Session::get_session_dir(&self.project_root)?;

        // Serialize layout to session (may save temporary buffers)
        let session = self.layout_manager.to_session(&session_dir);

        // Save session to file
        session.save(&self.project_root)?;
        termide_logger::info("Session saved");
        Ok(())
    }

    /// Load session from file and restore layout
    pub fn load_session(&mut self) -> Result<()> {
        // Load session for this project
        let session = termide_session::Session::load(&self.project_root)?;

        // Get session directory for restoring temporary buffers
        let session_dir = termide_session::Session::get_session_dir(&self.project_root)?;

        // Get terminal dimensions for creating Terminal panels
        let term_height = self.state.terminal.height.saturating_sub(3);
        let term_width = self.state.terminal.width.saturating_sub(2);

        // Restore layout from session
        self.layout_manager = LayoutManager::from_session(
            session,
            &session_dir,
            term_height,
            term_width,
            self.state.editor_config(),
        )?;
        termide_logger::info("Session loaded");

        // Initialize LSP for all restored editors
        if let Some(ref mut lsp_manager) = self.state.lsp_manager {
            for group in &mut self.layout_manager.panel_groups {
                for panel in group.panels_mut() {
                    if let Some(editor) = panel.as_editor_mut() {
                        editor.init_lsp(lsp_manager);
                    }
                }
            }
        }

        // Clean up orphaned buffer files (not referenced in session anymore)
        if let Err(e) = termide_session::cleanup_orphaned_buffers(&session_dir) {
            termide_logger::warn(format!("Failed to cleanup orphaned buffers: {}", e));
        }

        Ok(())
    }

    /// Auto-save session (ignores errors to not disrupt user experience)
    pub fn auto_save_session(&mut self) {
        if let Err(e) = self.save_session() {
            // Log error but don't interrupt user workflow
            termide_logger::error(format!("Failed to auto-save session: {}", e));
        }
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
