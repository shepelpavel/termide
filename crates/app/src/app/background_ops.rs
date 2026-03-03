//! Background operation handlers.
//!
//! Contains tick handlers for various background operations:
//! - Directory size calculation
//! - Git operations (push/pull, status, diff)
//! - Script execution
//! - System resource monitoring
//! - Modal spinners
//! - LSP completion polling

use std::sync::mpsc::TryRecvError;

use termide_core::PanelCommand;
use termide_modal::InfoModal;

use crate::state::ActiveModal;
use crate::PanelExt;

use super::App;

impl App {
    /// Check channel for directory size calculation results
    pub(super) fn check_dir_size_update(&mut self) {
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

    /// Check async git status results for all FileManager panels.
    /// Uses try_recv (O(1)) so it's safe to run for collapsed panels too —
    /// otherwise the receiver never drains and the spinner hangs forever.
    pub(super) fn check_fm_git_status_async(&mut self) {
        for panel in self.layout_manager.iter_all_panels_mut() {
            if let Some(fm) = panel.as_file_manager_mut() {
                if fm.check_git_status_async() {
                    self.state.needs_redraw = true;
                }
            }
        }
    }

    /// Check and apply pending git diff updates (debounced) and async git diff results.
    /// Runs for all panels because CheckGitDiffReceiver is a cheap try_recv
    /// that must be drained to avoid stale receivers in collapsed panels.
    pub(super) fn check_pending_git_diff_updates(&mut self) {
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

    /// Check for background git operation result (push/pull/fetch)
    pub(super) fn check_git_operation_result(&mut self) {
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

                // Fetch is silent - no modal, just refresh
                if result.operation == "fetch" {
                    // Refresh all git panels silently
                    for panel in self.layout_manager.iter_all_panels_mut() {
                        panel.handle_command(PanelCommand::Reload);
                    }
                    self.state.needs_redraw = true;
                    return;
                }

                // Show result modal for push/pull
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
                // Throttle spinner animation to 125ms (8 FPS) to reduce CPU usage
                const GIT_SPINNER_INTERVAL: std::time::Duration =
                    std::time::Duration::from_millis(125);
                let should_advance = self
                    .state
                    .last_git_spinner_update
                    .is_none_or(|t| t.elapsed() >= GIT_SPINNER_INTERVAL);

                if should_advance {
                    // Advance spinner frame for animation
                    self.state.ui.spinner_frame = self.state.ui.spinner_frame.wrapping_add(1);
                    self.state.last_git_spinner_update = Some(std::time::Instant::now());

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
                    self.state.needs_redraw = true;
                }

                // Put handle back
                self.state.git_operation_handle = Some(handle);
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
    pub(super) fn check_script_operation_result(&mut self) {
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

    /// Poll LSP status for expanded editors and completion for active editor
    pub(super) fn poll_lsp_completion(&mut self) {
        // Update LSP loading status for expanded editors only
        // Collapsed editors will catch up when they are expanded again
        let mut any_loading = false;
        for panel in self.layout_manager.iter_expanded_panels_mut() {
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
        // Throttle to 125ms (8 FPS) to reduce CPU usage
        if any_loading {
            const LSP_SPINNER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(125);
            let should_redraw = self
                .state
                .last_lsp_loading_redraw
                .is_none_or(|t| t.elapsed() >= LSP_SPINNER_INTERVAL);
            if should_redraw {
                self.state.last_lsp_loading_redraw = Some(std::time::Instant::now());
                self.state.needs_redraw = true;
            }
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
            if let Err(e) = self.process_panel_events(vec![event]) {
                log::error!("Error processing definition event: {}", e);
            }
        }
    }

    /// Update system resource monitoring (CPU, RAM)
    /// Respects the configured update interval.
    /// Only triggers redraw if display values actually changed.
    pub(super) fn update_system_resources(&mut self) {
        let interval =
            std::time::Duration::from_millis(self.state.config.logging.resource_monitor_interval);
        let elapsed = self.state.last_resource_update.elapsed();

        if elapsed >= interval {
            let old_stats = self.state.system_monitor.stats();
            self.state.system_monitor.update();
            self.state.last_resource_update = std::time::Instant::now();
            let new_stats = self.state.system_monitor.stats();
            // Only redraw if display values actually changed (rounded CPU% or MB of memory)
            if old_stats.cpu_usage.round() as u8 != new_stats.cpu_usage.round() as u8
                || old_stats.memory_used / (1024 * 1024) != new_stats.memory_used / (1024 * 1024)
            {
                self.state.needs_redraw = true;
            }
        }
    }

    /// Update spinner in all modals that support animation
    /// Throttled to 125ms (8 FPS) to reduce unnecessary redraws
    pub(super) fn update_modal_spinners(&mut self) {
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
}
