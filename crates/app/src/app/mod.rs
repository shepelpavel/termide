//! Main application module.
//!
//! Contains the App struct and all event handlers.

// Note: PanelExt is used for panel-specific operations (get current path, save editor)
// that require concrete type access. Common operations use Panel::handle_command().
#![allow(deprecated)]

use anyhow::Result;
use ratatui::{backend::Backend, Terminal};
use std::path::PathBuf;
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
                    // Handle bracketed paste - check modal first, then send to active panel
                    if !self.handle_modal_paste(&text) {
                        self.handle_paste_event(text)?;
                    }
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

                    // Check VFS connection status for FileManager panels and call tick() to process async operations
                    // Collect events first, then process them (to avoid borrow issues)
                    let mut all_fm_events = Vec::new();
                    for panel in self.layout_manager.iter_all_panels_mut() {
                        if let Some(fm) = panel.as_file_manager_mut() {
                            // Call tick() to process VFS operations (connection results, directory listings)
                            let events = fm.tick();
                            if !events.is_empty() {
                                self.state.needs_redraw = true;
                                all_fm_events.extend(events);
                            }
                            // Also check for pending operations for spinner animation
                            if fm.vfs_state().has_pending_operation() {
                                self.state.needs_redraw = true;
                            }
                        }
                    }
                    // Process collected events
                    if !all_fm_events.is_empty() {
                        let _ = self.process_panel_events(all_fm_events);
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
                    if let Some((operation, remote_path, temp_path)) = pending_upload {
                        self.handle_pending_upload(operation, remote_path, temp_path);
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

                    // Check download operation result (remote file download)
                    self.check_download_operation_result();

                    // Check upload operation result (remote file upload from editor)
                    self.check_upload_operation_result();

                    // Check batch upload operation result (local→remote batch copy)
                    self.check_batch_upload_result();

                    // Check batch download operation result (remote→local batch copy)
                    self.check_batch_download_result();

                    // Poll unified operation manager for events (new system)
                    self.poll_operation_manager();

                    // Check local file copy progress (chunked copy with progress)
                    self.check_local_copy_progress();

                    // Check local directory copy progress (background directory copy)
                    self.check_local_directory_copy_progress();

                    // Check local directory scan progress (async scan before copy)
                    self.check_local_scan_progress();

                    // Check pending local batch operation (start after modal rendered)
                    self.check_pending_batch_operation();

                    // Check local delete operation progress
                    self.check_delete_progress();

                    // Sync pause state between BatchOperation and ProgressModal (bidirectional)
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
                    self.update_system_resources();

                    // Poll LSP completion responses for active editor
                    self.poll_lsp_completion();

                    // Update spinner in all modals that support animation
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

    /// Update spinner in all modals that support animation
    /// Throttled to 125ms (8 FPS) to reduce unnecessary redraws
    fn update_modal_spinners(&mut self) {
        use crate::state::ActiveModal;

        const SPINNER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(125);

        // Throttle spinner updates for all modals
        let should_update = self
            .state
            .last_spinner_update
            .is_none_or(|t| t.elapsed() >= SPINNER_INTERVAL);

        if !should_update {
            return;
        }

        match &mut self.state.active_modal {
            Some(ActiveModal::Info(ref mut modal)) => {
                // Update spinner only if calculation is still ongoing
                if self.state.dir_size_receiver.is_some() {
                    modal.advance_spinner();
                    self.state.last_spinner_update = Some(std::time::Instant::now());
                    self.state.needs_redraw = true;
                }
            }
            Some(ActiveModal::InfoAction(ref mut modal)) => {
                // Update spinner only if operation is still ongoing
                if modal.is_operation_in_progress() {
                    modal.advance_spinner();
                    self.state.last_spinner_update = Some(std::time::Instant::now());
                    self.state.needs_redraw = true;
                }
            }
            Some(ActiveModal::Progress(ref mut modal)) => {
                // Always update spinner when progress modal is visible
                modal.advance_spinner();
                self.state.last_spinner_update = Some(std::time::Instant::now());
                self.state.needs_redraw = true;
            }
            _ => {}
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

                // InfoActionModal spinner updated by update_modal_spinners()

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

    /// Check for background script operation result (.report. scripts)
    fn check_script_operation_result(&mut self) {
        use crate::state::ActiveModal;
        use std::sync::mpsc::TryRecvError;
        use termide_modal::InfoModal;

        let handle = match self.state.script_operation_handle.take() {
            Some(h) => h,
            None => return,
        };

        match handle.receiver.try_recv() {
            Ok(result) => {
                // Show result modal
                let title = if result.success {
                    format!("{} ✓", result.script_name)
                } else {
                    format!("{} ✗", result.script_name)
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
                    lines.push((String::new(), "(no output)".to_string()));
                }

                let modal = InfoModal::new(&title, lines);
                self.state.active_modal = Some(ActiveModal::Info(Box::new(modal)));
                self.state.needs_redraw = true;
            }
            Err(TryRecvError::Empty) => {
                // Operation still in progress - put handle back
                self.state.script_operation_handle = Some(handle);
            }
            Err(TryRecvError::Disconnected) => {
                // Thread finished without sending (shouldn't happen)
                // Just ignore
            }
        }
    }

    /// Check download operation result (remote file download)
    fn check_download_operation_result(&mut self) {
        use termide_panel_editor::{Editor, FileState};

        let download = match self.state.download_operation.take() {
            Some(d) => d,
            None => return,
        };

        match download.operation.try_recv() {
            Some(Ok(_)) => {
                // Download complete!
                self.state.close_modal();

                // Get metadata from downloaded temp file
                let (size, mtime) = match std::fs::metadata(&download.temp_path) {
                    Ok(meta) => (meta.len(), meta.modified().ok()),
                    Err(_) => (0, None),
                };

                // Open editor with temp file and mark as remote
                match Editor::open_file_with_config(download.temp_path.clone(), download.config) {
                    Ok(mut editor) => {
                        // Set remote file state
                        editor.set_file_state(FileState::from_remote(
                            download.remote_path.clone(),
                            download.temp_path,
                            mtime,
                            size,
                        ));

                        // Store VfsManager for saves
                        editor.set_vfs_manager(download.vfs_manager);

                        // Initialize LSP
                        if let Some(lsp) = &mut self.state.lsp_manager {
                            editor.init_lsp(lsp);
                        }

                        self.add_panel(Box::new(editor));
                        self.auto_save_session();

                        let filename = download
                            .remote_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("remote file");
                        logger::info(format!("Remote file '{}' opened in editor", filename));
                        self.state.set_info(format!("File {} opened", filename));
                    }
                    Err(e) => {
                        let error_msg = format!("Failed to open downloaded file: {}", e);
                        logger::error(error_msg.clone());
                        self.state.set_error(error_msg);
                        // Clean up temp file
                        let _ = std::fs::remove_file(&download.temp_path);
                    }
                }
            }
            Some(Err(e)) => {
                // Download failed
                self.state.close_modal();
                let error_msg = format!("Download failed: {}", e);
                logger::error(error_msg.clone());
                self.state.set_error(error_msg);
                // Clean up temp file
                let _ = std::fs::remove_file(&download.temp_path);
            }
            None => {
                // Still downloading - check timeout
                if download.started.elapsed().as_secs() > 120 {
                    self.state.close_modal();
                    logger::error("Download timeout (120s)".to_string());
                    self.state.set_error("Download timeout (120s)".to_string());
                    // Clean up temp file
                    let _ = std::fs::remove_file(&download.temp_path);
                } else {
                    // Put back for next tick
                    self.state.download_operation = Some(download);
                }
            }
        }
    }

    /// Handle pending upload operation from Editor (regular Ctrl+S save of remote file)
    fn handle_pending_upload(
        &mut self,
        operation: termide_vfs::VfsOperation<()>,
        remote_path: termide_vfs::VfsPath,
        temp_path: PathBuf,
    ) {
        // Show progress modal
        let filename = remote_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());

        let modal = termide_modal::ProgressModal::indeterminate(
            "Uploading File",
            format!("Uploading {}...", filename),
        );
        self.state.active_modal = Some(crate::state::ActiveModal::Progress(Box::new(modal)));

        // Set uploading flag on active editor
        if let Some(panel) = self.layout_manager.active_panel_mut() {
            if let Some(editor) = panel.as_editor_mut() {
                editor.set_uploading(true);
            }
        }

        // Store operation for polling (reuse existing infrastructure)
        self.state.upload_operation = Some(crate::state::UploadOperation {
            operation,
            remote_path,
            temp_path,
            editor_panel_id: 0, // Active panel
            started: std::time::Instant::now(),
            close_after_upload: false, // Regular save - keep editor open
        });
    }

    /// Check upload operation result (remote file upload)
    fn check_upload_operation_result(&mut self) {
        let upload = match self.state.upload_operation.take() {
            Some(u) => u,
            None => return,
        };

        // Non-blocking poll
        match upload.operation.try_recv() {
            Some(Ok(_)) => {
                // Upload complete!
                self.state.close_modal();

                // Update editor mtime to prevent "changed on disk" warning
                // Note: We use active_panel since the editor should still be active
                // (upload happens during save-before-close which doesn't close until upload completes)
                if let Some(panel) = self.layout_manager.active_panel_mut() {
                    if let Some(editor) = panel.as_editor_mut() {
                        // Update editor mtime from temp file
                        if let Ok(meta) = std::fs::metadata(&upload.temp_path) {
                            if let Ok(mtime) = meta.modified() {
                                editor.update_file_mtime(Some(mtime));
                            }
                        }
                        editor.clear_external_change_detected();
                        editor.set_uploading(false);
                    }
                }

                let filename = upload
                    .remote_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "file".to_string());
                logger::info(format!("Remote file '{}' uploaded successfully", filename));
                self.state.set_info(format!("File {} uploaded", filename));

                // Close editor if this was a "save and close" operation
                if upload.close_after_upload {
                    self.close_panel_at_index(0);
                }
            }
            Some(Err(e)) => {
                // Upload failed
                self.state.close_modal();
                // Clear uploading flag on error
                if let Some(panel) = self.layout_manager.active_panel_mut() {
                    if let Some(editor) = panel.as_editor_mut() {
                        editor.set_uploading(false);
                    }
                }
                let error_msg = format!("Upload failed: {}", e);
                logger::error(error_msg.clone());
                self.state.set_error(error_msg);
            }
            None => {
                // Still uploading - check timeout
                if upload.started.elapsed().as_secs() > 120 {
                    self.state.close_modal();
                    // Clear uploading flag on timeout
                    if let Some(panel) = self.layout_manager.active_panel_mut() {
                        if let Some(editor) = panel.as_editor_mut() {
                            editor.set_uploading(false);
                        }
                    }
                    logger::error("Upload timeout (120s)".to_string());
                    self.state.set_error("Upload timeout (120s)".to_string());
                } else {
                    // Still uploading - spinner updated by update_modal_spinners()
                    // Put back for next tick
                    self.state.upload_operation = Some(upload);
                }
            }
        }
    }

    /// Check batch upload operation result (local→remote batch copy)
    fn check_batch_upload_result(&mut self) {
        let mut upload = match self.state.batch_upload_operation.take() {
            Some(u) => u,
            None => return,
        };

        // Check for progress updates and update modal
        if let Some(progress) = upload.operation.drain_progress() {
            if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                self.state.active_modal
            {
                modal.update_file_progress(progress.bytes_uploaded, progress.total_bytes);
            }
        }

        // Non-blocking poll for completion
        match upload.operation.try_recv() {
            Some(Ok(_)) => {
                // Current file upload complete!
                let filename = std::path::Path::new(&upload.dest_url)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "file".to_string());

                logger::info(format!("File '{}' uploaded successfully", filename));

                // If this was a move operation, delete the local source
                if upload.is_move {
                    if let Err(e) = std::fs::remove_file(&upload.source_path) {
                        logger::warn(format!("Failed to delete source after move: {}", e));
                    }
                }

                // Check if there are more files to upload
                upload.current_index += 1;
                if upload.current_index < upload.all_sources.len() {
                    // Start next file upload
                    let next_source = &upload.all_sources[upload.current_index];
                    let source_name = next_source
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "file".to_string());

                    // Parse remote base path and join with filename
                    if let Ok(remote_base) = termide_vfs::parse_vfs_url(&upload.dest_base_url) {
                        let final_remote = remote_base.join(&source_name);
                        let total_bytes =
                            std::fs::metadata(next_source).map(|m| m.len()).unwrap_or(0);

                        // Update modal progress
                        if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                            self.state.active_modal
                        {
                            modal.update_progress(
                                upload.current_index + 1,
                                Some(next_source.display().to_string()),
                            );
                            modal.update_source_dest(
                                next_source.display().to_string(),
                                final_remote.to_url_string(),
                            );
                            // Reset file progress for new file
                            modal.update_file_progress(0, total_bytes);
                        }

                        // Start upload for next file
                        let upload_op = upload
                            .vfs_manager
                            .upload_with_progress(next_source, &final_remote);

                        // Update upload state
                        upload.operation = upload_op;
                        upload.source_path = next_source.clone();
                        upload.dest_url = final_remote.to_url_string();
                        upload.total_bytes = total_bytes;
                        upload.started = std::time::Instant::now();

                        // Put back for next tick
                        self.state.batch_upload_operation = Some(upload);
                    } else {
                        // Failed to parse URL - abort
                        self.state.close_modal();
                        self.state
                            .set_error("Failed to parse remote URL".to_string());
                    }
                } else {
                    // All files uploaded!
                    self.state.close_modal();
                    let total = upload.all_sources.len();
                    if total == 1 {
                        self.state.set_info(format!("File {} uploaded", filename));
                    } else {
                        self.state.set_info(format!("{} files uploaded", total));
                    }

                    // Refresh file manager panels that show the destination directory
                    if let Ok(dest_path) = termide_vfs::parse_vfs_url(&upload.dest_url) {
                        if let Some(parent) = dest_path.parent() {
                            for group in &mut self.layout_manager.panel_groups {
                                for panel in group.panels_mut() {
                                    if let Some(fm) = panel.as_file_manager_mut() {
                                        if fm.is_remote() {
                                            let fm_path = fm.vfs_state().current_path();
                                            if fm_path.connection_key() == parent.connection_key()
                                                && fm_path.path == parent.path
                                            {
                                                let _ = fm.reload_directory();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Some(Err(e)) => {
                // Upload failed - log error and continue with next file
                logger::error(format!(
                    "Upload failed for {}: {}",
                    upload.source_path.display(),
                    e
                ));

                upload.current_index += 1;
                if upload.current_index < upload.all_sources.len() {
                    // Try next file
                    let next_source = &upload.all_sources[upload.current_index];
                    let source_name = next_source
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "file".to_string());

                    if let Ok(remote_base) = termide_vfs::parse_vfs_url(&upload.dest_base_url) {
                        let final_remote = remote_base.join(&source_name);
                        let total_bytes =
                            std::fs::metadata(next_source).map(|m| m.len()).unwrap_or(0);

                        // Update modal
                        if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                            self.state.active_modal
                        {
                            modal.update_progress(
                                upload.current_index + 1,
                                Some(next_source.display().to_string()),
                            );
                            modal.update_source_dest(
                                next_source.display().to_string(),
                                final_remote.to_url_string(),
                            );
                            modal.update_file_progress(0, total_bytes);
                        }

                        let upload_op = upload
                            .vfs_manager
                            .upload_with_progress(next_source, &final_remote);
                        upload.operation = upload_op;
                        upload.source_path = next_source.clone();
                        upload.dest_url = final_remote.to_url_string();
                        upload.total_bytes = total_bytes;
                        upload.started = std::time::Instant::now();
                        self.state.batch_upload_operation = Some(upload);
                    } else {
                        self.state.close_modal();
                        self.state.set_error(format!("Upload failed: {}", e));
                    }
                } else {
                    // No more files - show error
                    self.state.close_modal();
                    self.state.set_error(format!("Upload failed: {}", e));
                }
            }
            None => {
                // Still uploading - check timeout
                if upload.started.elapsed().as_secs() > 300 {
                    // 5 minute timeout for file upload
                    self.state.close_modal();
                    logger::error("Upload timeout (5 min)".to_string());
                    self.state.set_error("Upload timeout (5 min)".to_string());
                } else {
                    // Still uploading - put back for next tick
                    self.state.batch_upload_operation = Some(upload);
                }
            }
        }
    }

    /// Check batch download operation result (remote→local file copy/move during batch operations)
    fn check_batch_download_result(&mut self) {
        use crate::state::PendingAction;
        use std::sync::atomic::Ordering;

        let mut download = match self.state.batch_download_operation.take() {
            Some(d) => d,
            None => return,
        };

        // Sync pause state: BatchOperation -> download operation
        if let Some(PendingAction::ContinueBatchOperation { ref operation }) =
            self.state.pending_action
        {
            let should_pause = operation.pause_state == termide_state::PauseState::Paused;
            download
                .operation
                .pause_flag
                .store(should_pause, Ordering::Relaxed);
        }

        // Poll progress updates (drain all available progress messages)
        if let Some(progress) = download.operation.drain_progress() {
            // Update last known totals for this item
            download.last_total_files = progress.total_files;
            download.last_total_bytes = progress.total_bytes;

            if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                self.state.active_modal
            {
                // Get cumulative values from batch operation
                let (cumulative_files, cumulative_bytes) =
                    if let Some(PendingAction::ContinueBatchOperation { ref operation }) =
                        self.state.pending_action
                    {
                        (
                            operation.cumulative_files_completed,
                            operation.cumulative_bytes_completed,
                        )
                    } else {
                        (0, 0)
                    };

                // Use update_directory_copy_progress with cumulative + current values
                modal.update_directory_copy_progress(
                    cumulative_files + progress.files_downloaded,
                    cumulative_files + progress.total_files, // Approximate total (will grow as we process more items)
                    cumulative_bytes + progress.bytes_downloaded,
                    cumulative_bytes + progress.total_bytes, // Approximate total
                );
                // Update individual file progress for chunked downloads (progress bar)
                modal.update_individual_file_progress(
                    progress.current_file_bytes,
                    progress.current_file_total,
                );
                // Also update current item being downloaded
                if let Some(ref file) = progress.current_file {
                    modal.update_progress(
                        cumulative_files + progress.files_downloaded,
                        Some(file.clone()),
                    );
                }
                self.state.needs_redraw = true;
            }
        }

        match download.operation.try_recv() {
            Some(Ok(_)) => {
                // Download complete - for Move, delete the source file on remote
                if download.is_move {
                    if let (Some(vfs_source), Some(vfs_manager)) =
                        (&download.vfs_source, &download.vfs_manager)
                    {
                        // Start async delete operation (fire and forget for now)
                        let delete_op = vfs_manager.delete(vfs_source);
                        // Spawn thread to wait for delete result and log error if any
                        std::thread::spawn(move || {
                            if let Err(e) = delete_op.recv() {
                                termide_logger::error(format!(
                                    "Failed to delete remote source after move: {}",
                                    e
                                ));
                            }
                        });
                    }
                }

                // Continue batch operation
                if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                    self.state.pending_action.take()
                {
                    operation.success_count += 1;

                    // Update cumulative counters with completed item's totals
                    operation.cumulative_files_completed += download.last_total_files;
                    operation.cumulative_bytes_completed += download.last_total_bytes;

                    // Update progress modal
                    if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                        self.state.active_modal
                    {
                        modal.update_progress(
                            operation.current_index + 1,
                            Some(download.dest_path.display().to_string()),
                        );
                    }

                    operation.current_index += 1;
                    self.process_batch_operation(operation);
                }
            }
            Some(Err(e)) => {
                // Download failed - record error and continue
                if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                    self.state.pending_action.take()
                {
                    operation.error_count += 1;
                    logger::error(format!(
                        "Batch download failed for {}: {}",
                        download.dest_path.display(),
                        e
                    ));

                    // Still update cumulative counters for the failed item
                    operation.cumulative_files_completed += download.last_total_files;
                    operation.cumulative_bytes_completed += download.last_total_bytes;

                    operation.current_index += 1;
                    self.process_batch_operation(operation);
                }
            }
            None => {
                // Still downloading - check timeout (5 minutes for potentially large directories)
                if download.started.elapsed().as_secs() > 300 {
                    // Timeout - record error and continue
                    if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                        self.state.pending_action.take()
                    {
                        operation.error_count += 1;
                        logger::error(format!(
                            "Batch download timeout for {}",
                            download.dest_path.display()
                        ));

                        // Update cumulative counters even for timeout
                        operation.cumulative_files_completed += download.last_total_files;
                        operation.cumulative_bytes_completed += download.last_total_bytes;

                        operation.current_index += 1;
                        self.process_batch_operation(operation);
                    }
                } else {
                    // Put back for next tick
                    self.state.batch_download_operation = Some(download);
                }
            }
        }
    }

    /// Check and update progress for ongoing local file copy operation
    fn check_local_copy_progress(&mut self) {
        use crate::state::PendingAction;
        use std::sync::atomic::Ordering;

        let copy_op = match self.state.local_copy_operation.take() {
            Some(op) => op,
            None => return,
        };

        // Sync pause state: BatchOperation -> CopyOperation
        if let Some(PendingAction::ContinueBatchOperation { ref operation }) =
            self.state.pending_action
        {
            let should_pause = operation.pause_state == termide_state::PauseState::Paused;
            copy_op.pause_flag.store(should_pause, Ordering::Relaxed);
        }

        // Poll progress updates (drain all available progress messages)
        while let Ok(progress) = copy_op.progress.try_recv() {
            if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                self.state.active_modal
            {
                modal.update_file_progress(progress.bytes_copied, progress.total_bytes);
                self.state.needs_redraw = true;
            }
        }

        // Poll completion status
        match copy_op.completion.try_recv() {
            Ok(Ok(_)) => {
                // Copy complete - for Move, delete the source file
                if copy_op.is_move {
                    if let Err(e) = std::fs::remove_file(&copy_op.source_path) {
                        logger::error(format!(
                            "Failed to delete source after move: {}: {}",
                            copy_op.source_path.display(),
                            e
                        ));
                    }
                }

                // Continue batch operation
                if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                    self.state.pending_action.take()
                {
                    // Track completed destination for cleanup if operation is cancelled later
                    operation.add_completed_destination(copy_op.dest_path.clone());
                    operation.increment_success();
                    operation.advance();
                    self.process_batch_operation(operation);
                }
            }
            Ok(Err(e)) => {
                // Check if this is a cancellation error
                let error_msg = e.to_string();
                let is_cancellation = error_msg.contains("cancelled by user");

                if is_cancellation {
                    // User cancelled - show modal with cleanup options
                    use termide_modal::{ActiveModal, ChoiceModal};

                    // Extract batch operation info before setting new pending action
                    let (all_dest_paths, batch_operation) = self
                        .state
                        .pending_action
                        .take()
                        .and_then(|action| {
                            if let PendingAction::ContinueBatchOperation { operation } = action {
                                Some((
                                    operation.completed_destinations.clone(),
                                    Some(Box::new(operation)),
                                ))
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();

                    // Show different options based on whether there are completed files
                    let buttons = if all_dest_paths.is_empty() {
                        // Single file - only two options
                        vec!["Delete".to_string(), "Keep".to_string()]
                    } else {
                        // Multiple files - three options
                        vec![
                            "Delete partial".to_string(),
                            "Delete all".to_string(),
                            "Keep all".to_string(),
                        ]
                    };
                    let modal = ChoiceModal::buttons_only("Operation Cancelled", buttons);
                    self.state.active_modal = Some(ActiveModal::Choice(Box::new(modal)));
                    self.state.pending_action = Some(PendingAction::CancelCopyCleanup {
                        partial_path: copy_op.dest_path.clone(),
                        all_dest_paths,
                        is_directory: false,
                        batch_operation,
                    });
                } else {
                    // Other error - record and continue
                    if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                        self.state.pending_action.take()
                    {
                        operation.increment_error();
                        logger::error(format!(
                            "File copy failed for {}: {}",
                            copy_op.dest_path.display(),
                            e
                        ));
                        operation.advance();
                        self.process_batch_operation(operation);
                    }
                }
            }
            Err(_) => {
                // Still copying - put back for next tick
                self.state.local_copy_operation = Some(copy_op);
            }
        }
    }

    /// Check and update progress for ongoing local directory copy operation
    fn check_local_directory_copy_progress(&mut self) {
        use crate::state::PendingAction;
        use std::sync::atomic::Ordering;

        let mut copy_op = match self.state.local_directory_copy_operation.take() {
            Some(op) => op,
            None => return,
        };

        // Sync pause state: BatchOperation -> DirectoryCopyOperation
        if let Some(PendingAction::ContinueBatchOperation { ref operation }) =
            self.state.pending_action
        {
            let should_pause = operation.pause_state == termide_state::PauseState::Paused;
            copy_op.pause_flag.store(should_pause, Ordering::Relaxed);
        }

        // Poll progress updates (drain all available progress messages)
        while let Ok(progress) = copy_op.progress.try_recv() {
            // Track current file being copied (for cleanup on cancel)
            copy_op.current_file = Some(progress.current_file.clone());

            if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                self.state.active_modal
            {
                modal.update_directory_copy_progress(
                    progress.files_completed,
                    progress.total_files,
                    progress.bytes_copied,
                    progress.total_bytes,
                );
                self.state.needs_redraw = true;
            }
        }

        // Poll completion status
        match copy_op.completion.try_recv() {
            Ok(Ok(_)) => {
                // Copy complete - for Move, delete the source directory
                if copy_op.is_move {
                    if let Err(e) = std::fs::remove_dir_all(&copy_op.source_path) {
                        logger::error(format!(
                            "Failed to delete source directory after move: {}: {}",
                            copy_op.source_path.display(),
                            e
                        ));
                    }
                }

                // Continue batch operation
                if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                    self.state.pending_action.take()
                {
                    // Track completed destination for cleanup if operation is cancelled later
                    operation.add_completed_destination(copy_op.dest_path.clone());
                    operation.increment_success();
                    operation.advance();
                    self.process_batch_operation(operation);
                }
            }
            Ok(Err(e)) => {
                // Check if this is a cancellation error
                let error_msg = e.to_string();
                let is_cancellation = error_msg.contains("cancelled by user");

                if is_cancellation {
                    // User cancelled directory copy - show 3 cleanup options
                    use termide_modal::{ActiveModal, ChoiceModal};

                    // Extract batch operation info
                    let batch_operation = self.state.pending_action.take().and_then(|action| {
                        if let PendingAction::ContinueBatchOperation { operation } = action {
                            Some(Box::new(operation))
                        } else {
                            None
                        }
                    });

                    // For directory copy: always show 3 options
                    // 0 = Keep all (keep everything as is)
                    // 1 = Delete partial (only the interrupted file)
                    // 2 = Delete all (entire destination directory)
                    let buttons = vec![
                        "Keep all".to_string(),
                        "Delete partial".to_string(),
                        "Delete all".to_string(),
                    ];
                    let modal = ChoiceModal::buttons_only("Operation Cancelled", buttons);
                    self.state.active_modal = Some(ActiveModal::Choice(Box::new(modal)));
                    self.state.pending_action = Some(PendingAction::CancelCopyCleanup {
                        partial_path: copy_op.current_file.unwrap_or_default(), // The file being copied
                        all_dest_paths: vec![copy_op.dest_path.clone()], // The destination directory
                        is_directory: true,
                        batch_operation,
                    });
                } else {
                    // Other error - record and continue
                    if let Some(PendingAction::ContinueBatchOperation { mut operation }) =
                        self.state.pending_action.take()
                    {
                        operation.increment_error();
                        logger::error(format!(
                            "Directory copy failed for {}: {}",
                            copy_op.dest_path.display(),
                            e
                        ));
                        operation.advance();
                        self.process_batch_operation(operation);
                    }
                }
            }
            Err(_) => {
                // Still copying - put back for next tick
                self.state.local_directory_copy_operation = Some(copy_op);
            }
        }
    }

    /// Check and update progress for ongoing directory scan operation
    fn check_local_scan_progress(&mut self) {
        let scan_op = match self.state.local_scan_operation.take() {
            Some(op) => op,
            None => return,
        };

        // Poll progress updates (drain all available progress messages)
        while let Ok(progress) = scan_op.progress.try_recv() {
            if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                self.state.active_modal
            {
                let current_dir = if !progress.current_dir.as_os_str().is_empty() {
                    Some(progress.current_dir.display().to_string())
                } else {
                    None
                };
                modal.update_scan_progress(progress.files_count, progress.total_bytes, current_dir);
                self.state.needs_redraw = true;
            }
        }

        // Poll completion status
        match scan_op.completion.try_recv() {
            Ok(Ok(scan_result)) => {
                // Scan complete - start the actual directory copy
                logger::info(format!(
                    "Directory scan complete: {} files, {} bytes",
                    scan_result.files.len(),
                    scan_result.total_bytes
                ));

                // Check if this is a move operation
                let is_move = scan_op
                    .batch_operation
                    .as_ref()
                    .map(|op| op.operation_type == termide_state::BatchOperationType::Move)
                    .unwrap_or(false);

                // Transition modal from scanning to copying mode
                let title = if is_move { "Move" } else { "Copy" };
                if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                    self.state.active_modal
                {
                    modal.finish_scanning(
                        scan_result.files.len(),
                        scan_result.total_bytes,
                        scan_op.dest_path.display().to_string(),
                        title,
                    );
                }

                // Start the actual directory copy with scan results
                match termide_panel_file_manager::copy_directory_with_progress(
                    &scan_op.source_path,
                    &scan_op.dest_path,
                ) {
                    Ok(copy_op) => {
                        // Store copy operation for async handling
                        self.state.local_directory_copy_operation =
                            Some(crate::state::LocalDirectoryCopyOperation {
                                completion: copy_op.completion,
                                progress: copy_op.progress,
                                source_path: scan_op.source_path.clone(),
                                dest_path: scan_op.dest_path.clone(),
                                is_move,
                                pause_flag: copy_op.pause_flag,
                                cancel_flag: copy_op.cancel_flag,
                                current_file: None,
                            });

                        // Restore batch operation as pending action
                        if let Some(operation) = scan_op.batch_operation {
                            self.state.pending_action =
                                Some(crate::state::PendingAction::ContinueBatchOperation {
                                    operation: *operation,
                                });
                        }
                    }
                    Err(e) => {
                        // Copy failed to start - show error and continue batch
                        logger::error(format!("Failed to start directory copy: {}", e));
                        if let Some(mut operation) = scan_op.batch_operation {
                            operation.increment_error();
                            operation.advance();
                            self.state.close_modal();
                            self.process_batch_operation(*operation);
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                // Scan failed or was cancelled
                let error_msg = e.to_string();
                let is_cancellation = error_msg.contains("cancelled");

                if is_cancellation {
                    // User cancelled - close modal and show status
                    self.state.close_modal();
                    self.state.set_info("Directory scan cancelled".to_string());

                    // Continue batch operation without this directory
                    if let Some(mut operation) = scan_op.batch_operation {
                        operation.increment_skipped();
                        operation.advance();
                        self.process_batch_operation(*operation);
                    }
                } else {
                    // Other error - record and continue
                    logger::error(format!("Directory scan failed: {}", e));
                    if let Some(mut operation) = scan_op.batch_operation {
                        operation.increment_error();
                        operation.advance();
                        self.state.close_modal();
                        self.process_batch_operation(*operation);
                    }
                }
            }
            Err(_) => {
                // Still scanning - put back for next tick
                self.state.local_scan_operation = Some(scan_op);
            }
        }
    }

    /// Check if there's a pending local batch operation that needs to start
    /// (after progress modal has been rendered)
    fn check_pending_batch_operation(&mut self) {
        use crate::state::{ActiveModal, PendingAction};

        // Don't start new operation if background copy/download/scan is already in progress
        if self.state.local_copy_operation.is_some()
            || self.state.local_directory_copy_operation.is_some()
            || self.state.local_scan_operation.is_some()
            || self.state.batch_download_operation.is_some()
        {
            return;
        }

        // Check if we have a pending batch operation with progress modal open
        if let Some(ActiveModal::Progress(_)) = &self.state.active_modal {
            if let Some(PendingAction::ContinueBatchOperation { operation }) =
                self.state.pending_action.take()
            {
                // Modal has been rendered, now start the actual batch operation
                self.process_batch_operation(operation);
            }
        }
    }

    /// Check and update progress for ongoing local delete operation
    fn check_delete_progress(&mut self) {
        let delete_op = match self.state.local_delete_operation.take() {
            Some(op) => op,
            None => return,
        };

        // Poll progress updates (drain all available progress messages)
        while let Ok(progress) = delete_op.progress.try_recv() {
            if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                self.state.active_modal
            {
                modal.update_delete_progress(progress.files_deleted, progress.total_files);
                self.state.needs_redraw = true;
            }
        }

        // Poll completion status
        match delete_op.completion.try_recv() {
            Ok(Ok(_)) => {
                // Delete complete - close modal and refresh FileManager
                self.state.close_modal();

                // Refresh FileManager and clear selection
                if let Some(panel) = self.layout_manager.active_panel_mut() {
                    if let Some(fm) = panel.as_file_manager_mut() {
                        fm.clear_selection();
                        let _ = fm.load_directory();
                    }
                }

                let t = termide_i18n::t();
                self.state.set_info(t.status_item_deleted().to_string());
                logger::info("Delete operation completed successfully".to_string());
            }
            Ok(Err(e)) => {
                // Delete failed or cancelled
                self.state.close_modal();

                // Refresh FileManager anyway (partial deletion may have occurred)
                if let Some(panel) = self.layout_manager.active_panel_mut() {
                    if let Some(fm) = panel.as_file_manager_mut() {
                        fm.clear_selection();
                        let _ = fm.load_directory();
                    }
                }

                let error_msg = e.to_string();
                if error_msg.contains("cancelled") {
                    self.state.set_info("Delete cancelled".to_string());
                    logger::info("Delete operation cancelled by user".to_string());
                } else {
                    self.state.set_error(format!("Delete failed: {}", e));
                    logger::error(format!("Delete operation failed: {}", e));
                }
            }
            Err(_) => {
                // Still deleting - put back for next tick
                self.state.local_delete_operation = Some(delete_op);
            }
        }
    }

    /// Poll the unified operation manager for events (new file-ops system).
    /// This handles events from the centralized operation manager which will
    /// eventually replace the individual operation handles.
    fn poll_operation_manager(&mut self) {
        use termide_file_ops::{OperationEvent, OperationResult};

        let events = self.state.poll_operations();

        for event in events {
            match event {
                OperationEvent::Started(id) => {
                    logger::info(format!("Operation {} started", id));
                }
                OperationEvent::Progress(_id, progress) => {
                    // Update progress modal if active
                    if let Some(crate::state::ActiveModal::Progress(ref mut modal)) =
                        self.state.active_modal
                    {
                        modal
                            .update_file_progress(progress.bytes_transferred, progress.total_bytes);
                        // ETA is calculated internally by ProgressModal
                        self.state.needs_redraw = true;
                    }
                }
                OperationEvent::Completed(id, result) => {
                    match result {
                        OperationResult::Success | OperationResult::SuccessWithPath(_) => {
                            logger::info(format!("Operation {} completed successfully", id));
                            // Refresh file managers
                            for panel in self.layout_manager.iter_all_panels_mut() {
                                if let Some(fm) = panel.as_file_manager_mut() {
                                    let _ = fm.load_directory();
                                }
                            }
                        }
                        OperationResult::Failed(err) => {
                            logger::error(format!("Operation {} failed: {}", id, err));
                            self.state.set_error(format!("Operation failed: {}", err));
                        }
                        OperationResult::Cancelled => {
                            logger::info(format!("Operation {} cancelled", id));
                            self.state.set_info("Operation cancelled".to_string());
                        }
                    }
                }
                OperationEvent::Paused(id) => {
                    logger::info(format!("Operation {} paused", id));
                }
                OperationEvent::Resumed(id) => {
                    logger::info(format!("Operation {} resumed", id));
                }
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
