//! Panel event processing for the application.
//!
//! Processes `PanelEvent`s emitted by panels and translates them
//! into application state changes.

// Note: PanelExt is used for panel-specific operations (mouse clicks, resize)
// that require concrete type access. Common operations use Panel::handle_command().
#![allow(deprecated)]

use anyhow::Result;
use std::path::PathBuf;

use super::App;
use crate::state::PendingAction;
use crate::PanelExt;
use termide_core::{GitOperationType, PanelCommand, PanelEvent};
use termide_i18n as i18n;
use termide_panel_editor::Editor;

impl App {
    /// Process events emitted by a panel.
    ///
    /// This method handles all `PanelEvent` variants and translates them
    /// into appropriate application state changes.
    pub(super) fn process_panel_events(&mut self, events: Vec<PanelEvent>) -> Result<()> {
        for event in events {
            self.process_single_event(event)?;
        }
        Ok(())
    }

    /// Process a single panel event.
    pub(super) fn process_single_event(&mut self, event: PanelEvent) -> Result<()> {
        match event {
            // === File operations ===
            PanelEvent::OpenFile(path) => {
                self.event_open_file(path)?;
            }

            PanelEvent::ViewFile(path) => {
                self.event_view_file(path)?;
            }

            PanelEvent::OpenFileAt { path, line, column } => {
                self.event_open_file_at(path, line, column)?;
            }

            PanelEvent::ExecuteFile(path) => {
                self.event_execute_file(path)?;
            }

            PanelEvent::RunCommand { command, cwd } => {
                self.event_run_command(command, cwd)?;
            }

            PanelEvent::PreviewMedia(path) => {
                self.event_preview_media(path)?;
            }

            PanelEvent::OpenExternal(path) => {
                self.event_open_external(path)?;
            }

            PanelEvent::OpenRemoteFile(url) => {
                self.event_open_remote_file(url)?;
            }

            PanelEvent::ClosePanel => {
                // Request close of current panel (with confirmation if needed)
                self.handle_close_panel_request()?;
            }

            // === Status messages ===
            PanelEvent::ShowMessage(message) => {
                self.state.set_info(message);
            }

            PanelEvent::ShowError(message) => {
                self.state.set_error(message);
            }

            PanelEvent::SetStatusMessage { message, is_error } => {
                if is_error {
                    self.state.set_error(message);
                } else {
                    self.state.set_info(message);
                }
            }

            PanelEvent::ClearStatus => {
                self.state.clear_status();
            }

            // === Panel navigation ===
            PanelEvent::NextPanel => {
                self.layout_manager.next_group();
                self.notify_outline_file_opened();
            }

            PanelEvent::PrevPanel => {
                self.layout_manager.prev_group();
                self.notify_outline_file_opened();
            }

            PanelEvent::VimPanelNavigation { direction } => {
                use termide_core::VimPanelDirection;
                match direction {
                    VimPanelDirection::Left => self.layout_manager.prev_group(),
                    VimPanelDirection::Right => self.layout_manager.next_group(),
                    VimPanelDirection::Up => {
                        self.layout_manager.prev_panel_in_group();
                    }
                    VimPanelDirection::Down => {
                        self.layout_manager.next_panel_in_group();
                    }
                }
                self.notify_outline_file_opened();
            }

            // === Open panels ===
            PanelEvent::OpenDiagnosticsPanel => {
                self.handle_open_diagnostics()?;
            }

            // === Clipboard ===
            PanelEvent::CopyToClipboard(text) => {
                if let Err(e) = termide_clipboard::copy(&text) {
                    log::error!("Failed to copy to clipboard: {}", e);
                }
            }

            // === Events not yet implemented ===
            PanelEvent::NeedsRedraw => {
                // UI will redraw on next frame anyway
            }

            PanelEvent::Quit => {
                log::debug!("Quit event received");
                self.handle_quit_request()?;
            }

            PanelEvent::SaveFile(path) => {
                self.event_save_file(path)?;
            }

            PanelEvent::CloseFile => {
                // Same as ClosePanel for now
                self.handle_close_panel_request()?;
            }

            PanelEvent::NavigateTo(path) => {
                self.event_navigate_to(path)?;
            }

            PanelEvent::OpenPath { path, select_file } => {
                self.event_open_path(path, select_file)?;
            }

            PanelEvent::GotoLine(line) => {
                self.event_goto_line(line);
            }

            PanelEvent::ShowConfirm {
                message,
                on_confirm,
            } => {
                self.event_show_confirm(message, on_confirm);
            }

            PanelEvent::ShowInput {
                prompt,
                initial_value,
                on_submit,
            } => {
                self.event_show_input(prompt, initial_value, on_submit);
            }

            PanelEvent::ShowSelect {
                title,
                options,
                on_select,
            } => {
                self.event_show_select(title, options, on_select);
            }

            PanelEvent::ShowSearch {
                mode,
                initial_query,
            } => {
                self.event_show_search(mode, initial_query);
            }

            PanelEvent::ShowReplace { find, replace } => {
                self.event_show_replace(find, replace);
            }

            PanelEvent::ShowConflict {
                source,
                destination,
                remaining,
            } => {
                self.event_show_conflict(source, destination, remaining);
            }

            PanelEvent::WatchPath(path) => {
                self.event_watch_path(path);
            }

            PanelEvent::UnwatchPath(path) => {
                self.event_unwatch_path(path);
            }

            PanelEvent::RefreshGitStatus(path) => {
                self.event_refresh_git_status(path);
            }

            PanelEvent::RequestPaste => {
                self.event_paste_to_active_panel()?;
            }

            PanelEvent::FocusPanel(name) => {
                self.event_focus_panel(&name);
            }

            PanelEvent::SplitPanel { direction, .. } => {
                self.event_split_panel(direction);
            }

            PanelEvent::GitOperation {
                operation,
                repo_path,
            } => {
                self.event_git_operation(operation, repo_path)?;
            }

            PanelEvent::CancelGitOperation => {
                self.event_cancel_git_operation();
            }

            PanelEvent::OpenGitDiff {
                repo_path,
                commit_hash,
                file_path,
            } => {
                self.event_open_git_diff(repo_path, commit_hash, file_path)?;
            }

            PanelEvent::OpenGitStash { repo_path } => {
                self.event_open_git_stash(repo_path)?;
            }

            // === Operations panel ===
            PanelEvent::ToggleOperationPause(op_id) => {
                self.event_toggle_operation_pause(op_id);
            }

            PanelEvent::CancelOperation(op_id) => {
                self.event_cancel_operation(op_id);
            }

            PanelEvent::OpenOperationsPanel => {
                self.open_operations_panel()?;
            }

            PanelEvent::OpenOutlinePanel => {
                self.handle_open_outline()?;
            }

            PanelEvent::OpenReferencesPanel {
                locations,
                symbol_name,
            } => {
                self.handle_open_references_panel(locations, symbol_name)?;
            }

            PanelEvent::OpenDirectorySwitcher => {
                self.handle_open_directory_switcher()?;
            }
        }
        Ok(())
    }

    /// Handle RequestPaste event - paste clipboard to active panel
    fn event_paste_to_active_panel(&mut self) -> Result<()> {
        if let Some(panel) = self.layout_manager.active_panel_mut() {
            panel.handle_command(PanelCommand::Paste);
        }
        Ok(())
    }

    /// Handle bracketed paste event - paste text directly to active panel
    pub fn handle_paste_event(&mut self, text: String) -> Result<()> {
        if let Some(panel) = self.layout_manager.active_panel_mut() {
            panel.handle_command(PanelCommand::PasteText { text });
        }
        Ok(())
    }

    /// Handle OpenFile event - open file in editor (reuse existing tab if already open)
    fn event_open_file(&mut self, file_path: PathBuf) -> Result<()> {
        self.close_help_panels();
        log::debug!(
            "Opening file via event: {}",
            file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
        );

        // Check if the file is already open — focus it instead of creating a duplicate
        if self.focus_editor_by_path(&file_path) {
            return Ok(());
        }

        let _ = self.open_editor_for_file(file_path);
        Ok(())
    }

    /// Handle ViewFile event - open file in read-only editor mode
    fn event_view_file(&mut self, file_path: PathBuf) -> Result<()> {
        self.close_help_panels();
        log::debug!(
            "Viewing file via event: {}",
            file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
        );
        let _ = self.open_editor_for_file_readonly(file_path);
        Ok(())
    }

    /// Handle OpenFileAt event - open file in editor at specific location (for go-to-definition)
    fn event_open_file_at(&mut self, file_path: PathBuf, line: usize, column: usize) -> Result<()> {
        self.close_help_panels();
        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        let t = i18n::t();
        log::debug!(
            "Opening file at {}:{} via event: {}",
            line + 1,
            column,
            filename
        );

        // First check if the file is already open in an editor
        let mut found_existing = false;
        for panel in self.layout_manager.iter_all_panels_mut() {
            if let Some(editor) = panel.as_editor_mut() {
                if editor.file_path() == Some(&file_path) {
                    // File is already open - just move cursor to position
                    editor.goto_position(line, column);
                    log::info!(
                        "Jumped to {}:{} in already-open file '{}'",
                        line + 1,
                        column,
                        filename
                    );
                    found_existing = true;
                    break;
                }
            }
        }
        if found_existing {
            self.state
                .set_info(format!("{}:{}:{}", filename, line + 1, column));
            self.notify_outline_file_opened();
            return Ok(());
        }

        // File not open - open it and move to position
        match Editor::open_file_with_config(file_path.clone(), self.state.editor_config()) {
            Ok(mut editor_panel) => {
                // Move cursor to the requested position
                editor_panel.goto_position(line, column);

                // Initialize LSP for the editor
                if let Some(ref mut lsp_manager) = self.state.lsp_manager {
                    editor_panel.init_lsp(lsp_manager);
                }

                self.add_panel(Box::new(editor_panel));
                self.notify_outline_file_opened();
                self.auto_save_session();
                log::info!("File '{}' opened at {}:{}", filename, line + 1, column);
                self.state.set_info(t.editor_file_opened(filename));
            }
            Err(e) => {
                let error_msg = t.status_error_open_file(filename, &e.to_string());
                log::error!("Error opening '{}': {}", filename, e);
                self.state.set_error(error_msg);
            }
        }
        Ok(())
    }

    /// Handle ExecuteFile event - run executable in a new terminal
    fn event_execute_file(&mut self, file_path: PathBuf) -> Result<()> {
        self.close_help_panels();

        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");

        // Working directory = directory containing the file
        let working_dir = file_path.parent().map(|p| p.to_path_buf());

        // Command to execute
        let command = file_path.to_string_lossy().into_owned();

        match self.create_terminal_panel(working_dir) {
            Ok(mut terminal) => {
                // Send command to execute the file
                let _ = terminal.send_command(&command);
                self.add_panel(Box::new(terminal));
                self.auto_save_session();
                log::info!("Executing '{}' in terminal", filename);
            }
            Err(e) => {
                log::error!("Failed to create terminal for '{}': {}", filename, e);
            }
        }
        Ok(())
    }

    /// Handle RunCommand event - run command in a new terminal
    fn event_run_command(&mut self, command: String, cwd: Option<PathBuf>) -> Result<()> {
        self.close_help_panels();

        match self.create_terminal_panel(cwd) {
            Ok(mut terminal) => {
                let _ = terminal.send_command(&command);
                self.add_panel(Box::new(terminal));
                self.auto_save_session();
                log::info!("Running '{}' in terminal", command);
            }
            Err(e) => {
                log::error!("Failed to create terminal for command '{}': {}", command, e);
            }
        }
        Ok(())
    }

    /// Handle PreviewMedia event - preview image/video using native graphics or system viewer
    fn event_preview_media(&mut self, file_path: PathBuf) -> Result<()> {
        use termide_panel_image::ImagePanel;

        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();

        // Check if file is an image by extension
        let is_image = file_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                matches!(
                    ext.to_lowercase().as_str(),
                    "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tiff" | "tif"
                )
            })
            .unwrap_or(false);

        // Try native graphics rendering for images if protocol is available
        if is_image && ImagePanel::graphics_available() {
            // Try to reuse existing ImagePanel
            if let Some(panel) = self.layout_manager.find_and_expand_panel_by_name("image") {
                if let Some(image_panel) = panel.as_any_mut().downcast_mut::<ImagePanel>() {
                    image_panel.set_image(file_path);
                    self.state.needs_redraw = true;
                    log::info!("Updating preview to '{}'", filename);
                    return Ok(());
                }
            }

            // No existing panel - create new one without changing focus
            self.close_help_panels();
            match ImagePanel::new(file_path.clone()) {
                Ok(panel) => {
                    self.add_panel_without_focus(Box::new(panel));
                    self.auto_save_session();
                    log::info!("Previewing '{}' with native graphics", filename);
                    return Ok(());
                }
                Err(e) => {
                    log::debug!(
                        "Native graphics failed for '{}': {}, falling back to xdg-open",
                        filename,
                        e
                    );
                }
            }
        }

        // Fallback to system default viewer (xdg-open)
        log::info!("Opening '{}' with system viewer", filename);
        if let Err(e) = open::that(&file_path) {
            log::error!("Failed to open '{}': {}", filename, e);
            self.state
                .set_error(format!("Failed to open {}: {}", filename, e));
        }
        Ok(())
    }

    /// Handle OpenExternal event - open file with system default application
    fn event_open_external(&mut self, file_path: PathBuf) -> Result<()> {
        let t = termide_i18n::t();
        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();

        // Show status message
        self.state.set_info(t.status_opening_external(&filename));
        log::info!("Opening '{}' with system viewer", filename);

        if let Err(e) = open::that(&file_path) {
            log::error!("Failed to open '{}': {}", filename, e);
            self.state
                .set_error(format!("Failed to open {}: {}", filename, e));
        }
        Ok(())
    }

    /// Handle OpenRemoteFile event - open remote file via VFS
    fn event_open_remote_file(&mut self, url: String) -> Result<()> {
        self.close_help_panels();

        // Parse URL to VfsPath
        let vfs_path = match termide_vfs::parse_vfs_url(&url) {
            Ok(path) => path,
            Err(e) => {
                let error_msg = format!("Invalid remote URL: {}", e);
                log::error!("{}", error_msg);
                self.state.set_error(error_msg);
                return Ok(());
            }
        };

        let filename = vfs_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("remote")
            .to_string();

        // Get VfsManager from active FileManager panel
        let vfs_manager = if let Some(panel) = self.layout_manager.active_panel() {
            if let Some(fm) = panel
                .as_any()
                .downcast_ref::<termide_panel_file_manager::FileManager>()
            {
                fm.vfs_state().manager_arc()
            } else {
                let error_msg = "No file manager panel available for remote file access";
                log::error!("{}", error_msg);
                self.state.set_error(error_msg.to_string());
                return Ok(());
            }
        } else {
            let error_msg = "No active panel";
            log::error!("{}", error_msg);
            self.state.set_error(error_msg.to_string());
            return Ok(());
        };

        log::debug!("Opening remote file: {}", url);

        // Create temp directory for remote files
        let temp_dir = std::env::temp_dir().join("termide-remote-edit");
        if let Err(e) = std::fs::create_dir_all(&temp_dir) {
            let error_msg = format!("Failed to create temp directory: {}", e);
            log::error!("{}", error_msg);
            self.state.set_error(error_msg);
            return Ok(());
        }

        // Generate unique temp file name
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let temp_path = temp_dir.join(format!("{}_{}", timestamp, filename));

        // Create download request via OperationManager
        let request =
            termide_file_ops::OperationRequest::download(vfs_path.clone(), temp_path.clone());

        // Start download via OperationManager (no modal)
        match self.state.start_operation_now(request, vfs_manager.clone()) {
            Ok(operation_id) => {
                // Track the operation in the operations panel
                self.state.track_operation(
                    operation_id,
                    crate::state::OperationType::CopyDownload,
                    vfs_path.to_url_string(),
                    temp_path.display().to_string(),
                    1,
                    0,
                );

                // Store pending editor download metadata for post-processing
                self.state.pending_editor_download = Some(crate::state::PendingEditorDownload {
                    operation_id,
                    remote_path: vfs_path,
                    temp_path,
                    config: self.state.editor_config(),
                    vfs_manager,
                });

                // Open operations panel to show progress
                self.open_operations_panel()?;

                log::info!(
                    "Started downloading remote file '{}' (op {})",
                    filename,
                    operation_id
                );
            }
            Err(e) => {
                let error_msg = format!("Failed to start download: {}", e);
                log::error!("{}", error_msg);
                self.state.set_error(error_msg);
            }
        }

        Ok(())
    }

    /// Handle GotoLine event - move cursor to specific line in editor
    fn event_goto_line(&mut self, line: usize) {
        if let Some(panel) = self.layout_manager.active_panel_mut() {
            if let Some(editor) = panel.as_editor_mut() {
                // Convert from 1-based (user-facing) to 0-based (internal)
                let line_0based = line.saturating_sub(1);
                editor.set_cursor_line(line_0based);
                log::debug!("Moved to line {}", line);
            }
        }
    }

    /// Handle NavigateTo event - navigate file manager to path
    fn event_navigate_to(&mut self, path: PathBuf) -> Result<()> {
        if let Some(panel) = self.layout_manager.active_panel_mut() {
            if let Some(fm) = panel.as_file_manager_mut() {
                if let Err(e) = fm.navigate_to(path.clone()) {
                    log::error!("Navigation failed: {}", e);
                    self.state
                        .set_error(format!("Cannot navigate to: {}", path.display()));
                } else {
                    // Navigation resets watched_root; trigger watcher re-registration
                    self.state.needs_watcher_registration = true;
                }
            }
        }
        Ok(())
    }

    /// Handle OpenPath event - open path in new file manager panel
    fn event_open_path(
        &mut self,
        path: PathBuf,
        select_file: Option<std::ffi::OsString>,
    ) -> Result<()> {
        use termide_panel_file_manager::FileManager;

        // Create new file manager panel at the given path
        let mut fm = FileManager::new_with_path(path.clone());

        // If a file should be selected, find and select it
        if let Some(file_name) = select_file {
            fm.select_by_name(&file_name);
        }

        // Add panel to layout
        self.add_panel(Box::new(fm));
        self.auto_save_session();

        Ok(())
    }

    /// Handle SaveFile event - save file at given path
    fn event_save_file(&mut self, path: PathBuf) -> Result<()> {
        // Store info needed for LSP notification (before mutable borrow)
        let mut lsp_info: Option<(String, std::path::PathBuf)> = None;

        if let Some(panel) = self.layout_manager.active_panel_mut() {
            if let Some(editor) = panel.as_editor_mut() {
                match editor.save_file_as(path.clone()) {
                    Ok(()) => {
                        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                        self.state.set_info(format!("Saved: {}", filename));

                        // Collect LSP info for didSave notification
                        if let Some(lang) = editor.lsp_language() {
                            lsp_info = Some((lang.to_string(), path.clone()));
                        }
                    }
                    Err(e) => {
                        log::error!("Save failed: {}", e);
                        self.state.set_error(format!("Save failed: {}", e));
                    }
                }
            }
        }

        // Send LSP didSave notification (triggers full analysis for semantic errors)
        if let Some((lang, file_path)) = lsp_info {
            if let Some(ref lsp_manager) = self.state.lsp_manager {
                lsp_manager.did_save(&lang, &file_path, None);
            }
        }

        Ok(())
    }

    /// Handle WatchPath event - register path with file watcher
    fn event_watch_path(&mut self, path: PathBuf) {
        if let Some(watcher) = &mut self.state.watcher {
            if path.is_dir() {
                // Check if it's a git repo
                if termide_git::find_repo_root(&path).is_some() {
                    if let Err(e) = watcher.watch_repository(path.clone()) {
                        log::error!("Failed to watch repository {}: {}", path.display(), e);
                    }
                } else if let Err(e) = watcher.watch_directory(path.clone()) {
                    log::error!("Failed to watch directory {}: {}", path.display(), e);
                }
            }
        }
    }

    /// Handle RefreshGitStatus event - refresh git status for panels in path
    fn event_refresh_git_status(&mut self, path: PathBuf) {
        // Reload FileManagers whose current path starts with the given path
        for panel in self.layout_manager.iter_all_panels_mut() {
            if let Some(fm) = panel.as_file_manager_mut() {
                if fm.current_path().starts_with(&path) || path.starts_with(fm.current_path()) {
                    let _ = fm.reload_directory();
                }
            }
        }
    }

    /// Handle UnwatchPath event - unregister path from file watcher
    fn event_unwatch_path(&mut self, path: PathBuf) {
        if let Some(watcher) = &mut self.state.watcher {
            if termide_git::find_repo_root(&path).is_some() {
                watcher.unwatch_repository(&path);
            } else {
                watcher.unwatch_directory(&path);
            }
        }
    }

    /// Handle ShowConflict event - show file conflict resolution modal
    fn event_show_conflict(&mut self, source: PathBuf, destination: PathBuf, remaining: usize) {
        use crate::state::{ActiveModal, BatchOperation, BatchOperationType, PendingAction};
        use termide_modal::ConflictModal;

        // Create a minimal batch operation for conflict resolution
        let operation = BatchOperation::new(
            BatchOperationType::Copy, // Default to copy, actual type determined by context
            vec![source.clone()],
            destination.parent().unwrap_or(&destination).to_path_buf(),
        );

        let modal = ConflictModal::new(&source, &destination, remaining);
        self.state.set_pending_action(
            PendingAction::ContinueBatchOperation { operation },
            ActiveModal::Conflict(Box::new(modal)),
        );
    }

    /// Handle ShowSelect event - show selection modal
    fn event_show_select(
        &mut self,
        title: String,
        options: Vec<String>,
        on_select: termide_core::SelectAction,
    ) {
        use crate::state::{ActiveModal, PendingAction};
        use termide_modal::SelectModal;

        // Map SelectAction to PendingAction
        let pending_action = match on_select {
            termide_core::SelectAction::SelectTheme => {
                // Theme selection is handled differently
                return;
            }
            termide_core::SelectAction::SelectLanguage => {
                // Language selection is handled differently
                return;
            }
            termide_core::SelectAction::SelectEncoding => {
                // Encoding selection is handled differently
                return;
            }
            termide_core::SelectAction::CloseEditorChoice => PendingAction::CloseEditorWithSave,
            termide_core::SelectAction::Custom(_) => {
                // Custom actions not yet supported
                return;
            }
        };

        let modal = SelectModal::single(title, "", options);
        self.state
            .set_pending_action(pending_action, ActiveModal::Select(Box::new(modal)));
    }

    /// Handle ShowSearch event - show search modal
    fn event_show_search(
        &mut self,
        mode: termide_core::SearchMode,
        _initial_query: Option<String>,
    ) {
        use crate::state::{ActiveModal, PendingAction};
        use termide_modal::SearchModal;

        let modal = SearchModal::new(mode);

        self.state
            .set_pending_action(PendingAction::Search, ActiveModal::Search(Box::new(modal)));
    }

    /// Handle ShowReplace event - show replace modal
    fn event_show_replace(&mut self, _find: Option<String>, _replace: Option<String>) {
        use crate::state::{ActiveModal, PendingAction};
        use termide_modal::ReplaceModal;

        // Note: ReplaceModal doesn't support initial values yet
        let modal = ReplaceModal::new();

        self.state.set_pending_action(
            PendingAction::Replace,
            ActiveModal::Replace(Box::new(modal)),
        );
    }

    /// Handle ShowInput event - show input modal
    fn event_show_input(
        &mut self,
        prompt: String,
        initial_value: String,
        on_submit: termide_core::InputAction,
    ) {
        use crate::state::{ActiveModal, PendingAction};
        use termide_modal::InputModal;

        // Map InputAction to PendingAction
        let pending_action = match &on_submit {
            termide_core::InputAction::RenameFile { from } => PendingAction::MovePath {
                sources: vec![from.clone()],
                target_directory: from.parent().map(|p| p.to_path_buf()),
            },
            termide_core::InputAction::CreateFile { in_dir } => PendingAction::CreateFile {
                directory: in_dir.clone(),
            },
            termide_core::InputAction::CreateDirectory { in_dir } => {
                PendingAction::CreateDirectory {
                    directory: in_dir.clone(),
                }
            }
            termide_core::InputAction::SearchInFile => PendingAction::Search,
            termide_core::InputAction::SearchReplace => PendingAction::Replace,
            termide_core::InputAction::GotoLine => {
                // GotoLine is handled directly, not through modal
                return;
            }
            termide_core::InputAction::SaveFileAs { directory } => PendingAction::SaveFileAs {
                directory: directory.clone(),
            },
            termide_core::InputAction::CopyTo { sources } => PendingAction::CopyPath {
                sources: sources.clone(),
                target_directory: None,
                create_symlink: false,
            },
            termide_core::InputAction::MoveTo { sources } => PendingAction::MovePath {
                sources: sources.clone(),
                target_directory: None,
            },
            termide_core::InputAction::RenameSymbol {
                file_path,
                line,
                column,
            } => PendingAction::LspRenameSymbol {
                file_path: file_path.clone(),
                line: *line,
                column: *column,
            },
        };

        // Create input modal
        let modal = InputModal::with_default("Input", prompt, &initial_value);
        self.state
            .set_pending_action(pending_action, ActiveModal::Input(Box::new(modal)));
    }

    /// Handle ShowConfirm event - show confirmation modal
    fn event_show_confirm(&mut self, message: String, on_confirm: termide_core::ConfirmAction) {
        use crate::state::{ActiveModal, PendingAction};
        use termide_modal::ConfirmModal;

        // Determine title based on action type
        let t = i18n::t();
        let is_quit = matches!(on_confirm, termide_core::ConfirmAction::QuitApplication);
        let title = if is_quit {
            t.app_quit_title()
        } else {
            t.modal_yes()
        };

        // Map ConfirmAction to PendingAction
        let pending_action = match on_confirm {
            termide_core::ConfirmAction::DeleteFile(path) => {
                PendingAction::DeletePath { paths: vec![path] }
            }
            termide_core::ConfirmAction::DeletePaths(paths) => PendingAction::DeletePath { paths },
            termide_core::ConfirmAction::DeleteDirectory(path) => {
                PendingAction::DeletePath { paths: vec![path] }
            }
            termide_core::ConfirmAction::DiscardChanges(_path) => PendingAction::ClosePanel,
            termide_core::ConfirmAction::CloseWithoutSaving => PendingAction::CloseEditorWithSave,
            termide_core::ConfirmAction::QuitApplication => PendingAction::QuitApplication,
        };

        // Create confirmation modal
        let modal = ConfirmModal::new(title, message);
        self.state
            .set_pending_action(pending_action, ActiveModal::Confirm(Box::new(modal)));
    }

    /// Handle SplitPanel event - toggle panel stacking/splitting
    fn event_split_panel(&mut self, direction: termide_core::SplitDirection) {
        let terminal_width = self.state.terminal.width;

        match direction {
            termide_core::SplitDirection::Horizontal => {
                // Horizontal split: create new column (unstack if multiple panels in group)
                if let Err(e) = self.layout_manager.toggle_panel_stacking(terminal_width) {
                    log::debug!("Split failed: {}", e);
                }
            }
            termide_core::SplitDirection::Vertical => {
                // Vertical split: stack in same column (merge if single panel)
                if let Err(e) = self.layout_manager.toggle_panel_stacking(terminal_width) {
                    log::debug!("Stack failed: {}", e);
                }
            }
        }
    }

    /// Handle FocusPanel event - focus panel by name/title
    fn event_focus_panel(&mut self, name: &str) {
        // First, find the matching panel indices
        let mut found: Option<(usize, usize, String)> = None;
        for (group_idx, group) in self.layout_manager.panel_groups.iter().enumerate() {
            for (panel_idx, panel) in group.panels().iter().enumerate() {
                if panel.title().contains(name) {
                    found = Some((group_idx, panel_idx, panel.title().to_string()));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }

        // Then, apply the focus change
        if let Some((group_idx, panel_idx, title)) = found {
            if let Some(group) = self.layout_manager.panel_groups.get_mut(group_idx) {
                group.set_expanded(panel_idx);
            }
            self.layout_manager.focus = group_idx;
            self.notify_outline_file_opened();
            log::debug!("Focused panel: {}", title);
        } else {
            log::debug!("Panel not found: {}", name);
        }
    }

    /// Handle GitOperation event - run git push/pull in background thread
    pub(super) fn event_git_operation(
        &mut self,
        operation: GitOperationType,
        repo_path: PathBuf,
    ) -> Result<()> {
        use crate::state::{GitOperationHandle, GitOperationResult};
        use std::process::{Command, Stdio};
        use std::sync::mpsc;
        use std::thread;

        // Prevent multiple concurrent operations
        if self.state.ui.git_operation_in_progress {
            log::debug!("Git operation already in progress, ignoring");
            return Ok(());
        }

        let cmd = match operation {
            GitOperationType::Push => "push",
            GitOperationType::Pull => "pull",
            GitOperationType::Fetch => "fetch",
        };
        let cmd_str = cmd.to_string();

        // Spawn the git process with piped stdout/stderr to capture output
        // and prevent it from corrupting the TUI
        let child = match Command::new("git")
            .arg(&cmd_str)
            .current_dir(&repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                self.state.set_error(format!("Failed to spawn git: {}", e));
                return Ok(());
            }
        };

        // Get PID before moving child to thread
        let pid = child.id();
        log::info!("Running git {} in {:?} (PID: {})", cmd, repo_path, pid);

        // Set operation state
        self.state.ui.git_operation_in_progress = true;
        self.notify_git_operation_state(true, Some(cmd_str.clone()), 0);

        // Show status message
        let t = i18n::t();
        let msg = match operation {
            GitOperationType::Push => t.git_push_in_progress(),
            GitOperationType::Pull => t.git_pull_in_progress(),
            GitOperationType::Fetch => t.git_fetch_in_progress(),
        };
        self.state.set_info(msg);

        // Spawn background thread to wait for result
        let (tx, rx) = mpsc::channel();
        let cmd_for_thread = cmd_str.clone();

        thread::spawn(move || {
            let output = child.wait_with_output();

            let result = match output {
                Ok(out) => GitOperationResult {
                    operation: cmd_for_thread,
                    success: out.status.success(),
                    stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                    stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                },
                Err(e) => GitOperationResult {
                    operation: cmd_for_thread,
                    success: false,
                    stdout: String::new(),
                    stderr: e.to_string(),
                },
            };
            let _ = tx.send(result);
        });

        // Store handle for polling and cancellation
        self.state.git_operation_handle = Some(GitOperationHandle {
            receiver: rx,
            pid,
            operation: cmd_str,
            started_at: std::time::Instant::now(),
        });

        Ok(())
    }

    /// Handle CancelGitOperation event - kill running git process
    pub(super) fn event_cancel_git_operation(&mut self) {
        if let Some(handle) = self.state.git_operation_handle.take() {
            log::info!("Cancelling git {} (PID: {})", handle.operation, handle.pid);

            // Kill process by PID
            #[cfg(unix)]
            {
                let _ = std::process::Command::new("kill")
                    .arg("-TERM")
                    .arg(handle.pid.to_string())
                    .status();
            }
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &handle.pid.to_string(), "/F"])
                    .status();
            }
        }

        self.state.ui.git_operation_in_progress = false;
        self.notify_git_operation_state(false, None, 0);

        // Show cancellation message
        let t = i18n::t();
        self.state.set_info(t.git_operation_cancelled().to_string());
    }

    /// Notify all panels about git operation in progress state
    pub(super) fn notify_git_operation_state(
        &mut self,
        in_progress: bool,
        operation: Option<String>,
        spinner_frame: usize,
    ) {
        for panel in self.layout_manager.iter_all_panels_mut() {
            panel.handle_command(PanelCommand::SetGitOperationInProgress {
                in_progress,
                operation: operation.clone(),
                spinner_frame,
            });
        }
    }

    /// Handle OpenGitDiff event - open git diff panel for repository.
    /// Reuses existing panel if one with matching arguments is already open.
    fn event_open_git_diff(
        &mut self,
        repo_path: PathBuf,
        commit_hash: Option<String>,
        file_path: Option<PathBuf>,
    ) -> Result<()> {
        use termide_panel_git_diff::GitDiffPanel;

        log::debug!(
            "Opening Git Diff panel for {:?} (commit: {:?}, file: {:?})",
            repo_path,
            commit_hash,
            file_path
        );
        self.close_help_panels();

        // Check if a matching GitDiffPanel is already open
        let file_filter_str = file_path.as_ref().map(|p| p.to_string_lossy().to_string());
        for (group_idx, group) in self.layout_manager.panel_groups.iter_mut().enumerate() {
            for (panel_idx, panel) in group.panels().iter().enumerate() {
                if let Some(diff) = panel.as_any().downcast_ref::<GitDiffPanel>() {
                    if diff.repo_path() == repo_path
                        && diff.commit_hash() == commit_hash.as_deref()
                        && diff.file_filter() == file_filter_str.as_deref()
                    {
                        self.layout_manager.focus = group_idx;
                        group.set_expanded(panel_idx);
                        return Ok(());
                    }
                }
            }
        }

        let panel = match (&commit_hash, &file_path) {
            (_, Some(file)) => GitDiffPanel::new_with_file_filter(repo_path, file.clone()),
            (Some(hash), None) => GitDiffPanel::new_for_commit(repo_path, hash.clone()),
            (None, None) => GitDiffPanel::new(repo_path),
        };
        self.add_panel(Box::new(panel));
        self.auto_save_session();

        Ok(())
    }

    /// Handle OpenGitStash event - open git stash panel for repository (singleton)
    fn event_open_git_stash(&mut self, repo_path: PathBuf) -> Result<()> {
        log::debug!("Opening Git Stash panel for {:?}", repo_path);
        self.close_help_panels();

        if !self.find_and_focus_panel_by_name("git_stash") {
            let panel = termide_panel_git_stash::GitStashPanel::new(repo_path);
            self.add_panel(Box::new(panel));
        }

        Ok(())
    }

    // ========================================================================
    // Operations Panel Methods
    // ========================================================================

    /// Start a tracked operation with auto-opening of Operations panel.
    /// This wraps start_operation_now and adds tracking + panel opening.
    #[allow(clippy::too_many_arguments)]
    pub fn start_tracked_operation(
        &mut self,
        request: termide_file_ops::OperationRequest,
        vfs_manager: std::sync::Arc<termide_vfs::VfsManager>,
        op_type: crate::state::OperationType,
        source: String,
        dest: String,
        total_files: usize,
        total_bytes: u64,
    ) -> anyhow::Result<termide_file_ops::OperationId> {
        // Start the operation
        let operation_id = self.state.start_operation_now(request, vfs_manager)?;

        // Track the operation
        self.state.track_operation(
            operation_id,
            op_type,
            source,
            dest,
            total_files,
            total_bytes,
        );

        // Open the operations panel with focus on the new operation
        let _ = self.open_operations_panel_with_focus(operation_id);

        Ok(operation_id)
    }

    /// Update operations panel data before rendering.
    /// Called from render loop to sync panel with active_operations.
    pub fn update_operations_panel(&mut self) {
        let has_ops = self.state.has_active_operations();
        // Skip if no operations and panel was already synced empty
        if !has_ops && !self.state.operations_panel_dirty {
            return;
        }
        if !has_ops {
            // All operations finished — close the panel entirely
            self.state.operations_panel_dirty = false;
            self.state.last_operations_elapsed_redraw = None;
            self.close_operations_panel();
            return;
        }
        self.state.operations_panel_dirty = true;
        // Force redraw every 1s to update elapsed time display in operation cards
        let should_redraw = self
            .state
            .last_operations_elapsed_redraw
            .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1));
        if should_redraw {
            self.state.last_operations_elapsed_redraw = Some(std::time::Instant::now());
            self.state.needs_redraw = true;
        }
        // Find operations panel and update its data
        for group in &mut self.layout_manager.panel_groups {
            for panel in group.panels_mut() {
                if let Some(ops_panel) = panel
                    .as_any_mut()
                    .downcast_mut::<termide_panel_operations::OperationsPanel>()
                {
                    let ops_list = self.state.operations_list();
                    ops_panel.update_operations(&ops_list);
                    return;
                }
            }
        }
    }

    // ========================================================================
    // Operations Panel Event Handlers
    // ========================================================================

    /// Handle ToggleOperationPause event - pause or resume an operation
    fn event_toggle_operation_pause(&mut self, op_id: termide_file_ops::OperationId) {
        // Check if operation is paused
        let is_paused = self
            .state
            .active_operations
            .get(&op_id)
            .map(|op| op.is_paused)
            .unwrap_or(false);

        // Resolve batch tracking ID to actual OperationManager sub-operation ID
        let real_id = if self.state.batch_tracking_id == Some(op_id) {
            self.state.batch_sub_operation_id.unwrap_or(op_id)
        } else {
            op_id
        };

        if let Some(manager) = self.state.operation_manager_mut() {
            if is_paused {
                manager.resume(real_id);
                log::debug!("Resumed operation {}", real_id);
            } else {
                manager.pause(real_id);
                log::debug!("Paused operation {}", real_id);
            }
        }

        // Update batch tracking paused state (UI card)
        self.state.set_batch_paused(!is_paused);

        // Also sync pause state into the pending BatchOperation so that
        // process_batch_operation() won't start the next sub-op while paused.
        if self.state.batch_tracking_id == Some(op_id) {
            if let Some(PendingAction::ContinueBatchOperation { ref mut operation }) =
                self.state.pending_action
            {
                operation.pause_state = if !is_paused {
                    termide_state::PauseState::Paused
                } else {
                    termide_state::PauseState::Running
                };
            }
        }

        self.state.needs_redraw = true;
    }

    /// Handle CancelOperation event - cancel an operation
    fn event_cancel_operation(&mut self, op_id: termide_file_ops::OperationId) {
        // Check if this is a script operation (not managed by OperationManager)
        if let Some(op) = self.state.active_operations.get(&op_id) {
            if op.op_type.is_script() {
                // Kill the process and remove from tracking
                use crate::state::kill_process_tree;

                // Kill bg_script process if present
                if let Some(pos) = self
                    .state
                    .bg_script_handles
                    .iter()
                    .position(|(id, _, _)| *id == op_id)
                {
                    let (_, _, pid) = self.state.bg_script_handles.remove(pos);
                    kill_process_tree(pid);
                }

                // Kill report script process if it matches
                if let Some(pos) = self
                    .state
                    .script_operation_handles
                    .iter()
                    .position(|h| h.operation_id == Some(op_id))
                {
                    let handle = self.state.script_operation_handles.remove(pos);
                    if let Some(pid) = handle.pid {
                        kill_process_tree(pid);
                    }
                }

                self.state.untrack_operation(op_id);
                self.state.needs_redraw = true;
                return;
            }
        }

        // Resolve batch tracking ID to actual OperationManager sub-operation ID
        let real_id = if self.state.batch_tracking_id == Some(op_id) {
            self.state.batch_sub_operation_id.unwrap_or(op_id)
        } else {
            op_id
        };

        if let Some(manager) = self.state.operation_manager_mut() {
            manager.cancel(real_id);
            log::debug!("Cancelled operation {}", real_id);
        }
        self.state.needs_redraw = true;
    }

    /// Open or expand the Operations panel without stealing focus.
    /// The panel is inserted right after the currently expanded panel in the accordion,
    /// so when it closes, the previous panel will naturally be shown again.
    pub(super) fn open_operations_panel(&mut self) -> Result<()> {
        use termide_panel_operations::OperationsPanel;

        // Check if operations panel already exists — expand it without changing focus
        for (group_idx, group) in self.layout_manager.panel_groups.iter().enumerate() {
            for (panel_idx, panel) in group.panels().iter().enumerate() {
                if panel.name() == "operations" {
                    if let Some(group) = self.layout_manager.get_group_mut(group_idx) {
                        group.set_expanded(panel_idx);
                    }
                    return Ok(());
                }
            }
        }

        // Uses WidthPreference::PreferNarrow from OperationsPanel
        let panel = Box::new(OperationsPanel::new());
        self.add_panel_without_focus(panel);
        self.auto_save_session();
        Ok(())
    }

    /// Open operations panel and select specific operation without stealing focus.
    pub(super) fn open_operations_panel_with_focus(
        &mut self,
        op_id: termide_file_ops::OperationId,
    ) -> Result<()> {
        self.open_operations_panel()?;

        // Find the operations panel, update its data and select the operation
        for group in self.layout_manager.panel_groups.iter_mut() {
            for panel in group.panels_mut().iter_mut() {
                if let Some(ops_panel) = panel
                    .as_any_mut()
                    .downcast_mut::<termide_panel_operations::OperationsPanel>()
                {
                    // Update operations snapshot
                    let ops_list = self.state.operations_list();
                    ops_panel.update_operations(&ops_list);

                    // Select the specific operation
                    if let Some(index) = self.state.operation_index(op_id) {
                        ops_panel.set_selected(index);
                    }

                    return Ok(());
                }
            }
        }
        Ok(())
    }
}
