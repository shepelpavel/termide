//! Modal window handling for the application.

use anyhow::Result;

use super::App;
use crate::state::ActiveModal;
use termide_modal::{
    Modal, ModalResult, ReplaceAction, ReplaceModalResult, SearchAction, SearchModalResult,
};
use termide_ui::path_utils;

/// Helper to convert typed ModalResult to Box<dyn Any>
fn box_modal_result<T: 'static>(result: ModalResult<T>) -> ModalResult<Box<dyn std::any::Any>> {
    match result {
        ModalResult::Confirmed(value) => {
            ModalResult::Confirmed(Box::new(value) as Box<dyn std::any::Any>)
        }
        ModalResult::Cancelled => ModalResult::Cancelled,
    }
}

/// Result of processing search/replace modal
enum SearchReplaceResult {
    /// Keep modal open (navigation action)
    KeepOpen,
    /// Close modal
    Close,
    /// Modal cancelled - close and clear search
    Cancelled,
    /// Not a search/replace modal
    NotApplicable,
}

impl App {
    /// Handle keyboard event in modal window
    pub(super) fn handle_modal_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        // Get mutable reference to active modal window
        if let Some(modal) = self.state.get_active_modal_mut() {
            // Handle event in corresponding modal window
            let modal_result = match modal {
                ActiveModal::Commit(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::Confirm(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::Input(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::Select(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::Overwrite(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::Conflict(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::Info(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::InfoAction(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::RenamePattern(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::EditableSelect(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::Search(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::Replace(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::Sessions(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::FileSearch(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::ContentSearch(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::DirectoryPicker(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::SaveAs(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::DirectorySwitcher(m) => m.handle_key(key)?.map(box_modal_result),
                ActiveModal::BookmarkAdd(m) => m.handle_key(key)?.map(box_modal_result),
            };

            // If modal window returned result, handle it
            if let Some(result) = modal_result {
                // Check modal type before taking state references
                let is_rename_pattern = matches!(modal, ActiveModal::RenamePattern(_));
                let is_search = matches!(modal, ActiveModal::Search(_));
                let is_replace = matches!(modal, ActiveModal::Replace(_));

                // Handle cancellation from RenamePattern - return to ConflictModal
                if is_rename_pattern && matches!(result, ModalResult::Cancelled) {
                    // Take operation from pending action and return to ConflictModal
                    #[allow(clippy::collapsible_match)]
                    if let Some(action) = self.state.take_pending_action() {
                        if let termide_state::PendingAction::RenameWithPattern {
                            operation, ..
                        } = action
                        {
                            use termide_modal::ConflictModal;

                            if let Some(source) = operation.current_source() {
                                let final_dest = path_utils::resolve_batch_destination_path(
                                    source,
                                    &operation.destination,
                                    operation.sources.len() == 1,
                                );

                                let remaining_items = operation
                                    .sources
                                    .len()
                                    .saturating_sub(operation.current_index + 1);
                                let modal =
                                    ConflictModal::new(source, &final_dest, remaining_items);
                                self.state.pending_action =
                                    Some(termide_state::PendingAction::ContinueBatchOperation {
                                        operation,
                                    });
                                self.state.active_modal =
                                    Some(ActiveModal::Conflict(Box::new(modal)));
                                return Ok(());
                            }
                        }
                    }
                }

                // Handle search/replace modals with shared helper
                if self
                    .handle_search_replace_modal(is_search, is_replace, &result)
                    .is_some()
                {
                    return Ok(());
                }

                self.state.close_modal();
                if let ModalResult::Confirmed(value) = result {
                    self.handle_modal_result(value)?;
                }
            }
        }
        Ok(())
    }

    /// Handle mouse event in modal window
    pub(super) fn handle_modal_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        modal_area: ratatui::layout::Rect,
    ) -> Result<()> {
        // Get mutable reference to active modal window
        if let Some(modal) = self.state.get_active_modal_mut() {
            // Handle event in corresponding modal window
            let modal_result = match modal {
                ActiveModal::Commit(m) => m.handle_mouse(mouse, modal_area)?.map(box_modal_result),
                ActiveModal::Confirm(m) => m.handle_mouse(mouse, modal_area)?.map(box_modal_result),
                ActiveModal::Input(m) => m.handle_mouse(mouse, modal_area)?.map(box_modal_result),
                ActiveModal::Select(m) => m.handle_mouse(mouse, modal_area)?.map(box_modal_result),
                ActiveModal::Overwrite(m) => {
                    m.handle_mouse(mouse, modal_area)?.map(box_modal_result)
                }
                ActiveModal::Conflict(m) => {
                    m.handle_mouse(mouse, modal_area)?.map(box_modal_result)
                }
                ActiveModal::Info(m) => m.handle_mouse(mouse, modal_area)?.map(box_modal_result),
                ActiveModal::InfoAction(m) => {
                    m.handle_mouse(mouse, modal_area)?.map(box_modal_result)
                }
                ActiveModal::RenamePattern(m) => {
                    m.handle_mouse(mouse, modal_area)?.map(box_modal_result)
                }
                ActiveModal::EditableSelect(m) => {
                    m.handle_mouse(mouse, modal_area)?.map(box_modal_result)
                }
                ActiveModal::Search(m) => m.handle_mouse(mouse, modal_area)?.map(box_modal_result),
                ActiveModal::Replace(m) => m.handle_mouse(mouse, modal_area)?.map(box_modal_result),
                ActiveModal::Sessions(m) => {
                    m.handle_mouse(mouse, modal_area)?.map(box_modal_result)
                }
                ActiveModal::FileSearch(m) => {
                    m.handle_mouse(mouse, modal_area)?.map(box_modal_result)
                }
                ActiveModal::ContentSearch(m) => {
                    m.handle_mouse(mouse, modal_area)?.map(box_modal_result)
                }
                ActiveModal::DirectoryPicker(m) => {
                    m.handle_mouse(mouse, modal_area)?.map(box_modal_result)
                }
                ActiveModal::SaveAs(m) => m.handle_mouse(mouse, modal_area)?.map(box_modal_result),
                ActiveModal::DirectorySwitcher(m) => {
                    m.handle_mouse(mouse, modal_area)?.map(box_modal_result)
                }
                ActiveModal::BookmarkAdd(m) => {
                    m.handle_mouse(mouse, modal_area)?.map(box_modal_result)
                }
            };

            // If modal window returned result, handle it
            if let Some(result) = modal_result {
                // Check modal type before taking state references
                let is_search = matches!(modal, ActiveModal::Search(_));
                let is_replace = matches!(modal, ActiveModal::Replace(_));

                // Handle search/replace modals with shared helper
                if self
                    .handle_search_replace_modal(is_search, is_replace, &result)
                    .is_some()
                {
                    return Ok(());
                }

                // Check if this is a git push/pull action that should keep modal open
                if self.handle_git_push_pull_from_modal(&result)? {
                    return Ok(());
                }

                self.state.close_modal();
                if let ModalResult::Confirmed(value) = result {
                    self.handle_modal_result(value)?;
                }
            }
        }
        Ok(())
    }

    /// Handle modal window result
    pub(super) fn handle_modal_result(&mut self, value: Box<dyn std::any::Any>) -> Result<()> {
        use termide_state::PendingAction;

        if let Some(action) = self.state.take_pending_action() {
            match action {
                PendingAction::CreateFile {
                    panel_index,
                    directory,
                } => {
                    self.handle_create_file(panel_index, directory, value)?;
                }
                PendingAction::CreateDirectory {
                    panel_index,
                    directory,
                } => {
                    self.handle_create_directory(panel_index, directory, value)?;
                }
                PendingAction::DeletePath { panel_index, paths } => {
                    self.handle_delete_path(panel_index, paths, value)?;
                }
                PendingAction::SaveFileAs {
                    panel_index,
                    directory,
                } => {
                    self.handle_save_file_as(panel_index, directory, value)?;
                }
                PendingAction::ClosePanel { panel_index } => {
                    self.handle_close_panel(panel_index, value)?;
                }
                PendingAction::CloseEditorWithSave { panel_index } => {
                    self.handle_close_editor_with_save(panel_index, value)?;
                }
                PendingAction::CloseEditorExternal { panel_index } => {
                    self.handle_close_editor_external(panel_index, value)?;
                }
                PendingAction::CloseEditorConflict { panel_index } => {
                    self.handle_close_editor_conflict(panel_index, value)?;
                }
                PendingAction::OverwriteDecision {
                    panel_index,
                    source,
                    destination,
                    is_move,
                } => {
                    self.handle_overwrite_decision(
                        panel_index,
                        source,
                        destination,
                        is_move,
                        value,
                    )?;
                }
                PendingAction::CopyPath {
                    panel_index,
                    sources,
                    target_directory,
                } => {
                    self.handle_copy_path(panel_index, sources, target_directory, value)?;
                }
                PendingAction::MovePath {
                    panel_index,
                    sources,
                    target_directory,
                } => {
                    self.handle_move_path(panel_index, sources, target_directory, value)?;
                }
                PendingAction::BatchFileOperation { operation } => {
                    self.process_batch_operation(operation);
                }
                PendingAction::ContinueBatchOperation { operation } => {
                    self.handle_continue_batch_operation(operation, value)?;
                }
                PendingAction::RenameWithPattern {
                    operation,
                    original_name,
                } => {
                    self.handle_rename_with_pattern(operation, original_name, value)?;
                }
                PendingAction::Search => {
                    self.handle_search(value)?;
                }
                PendingAction::Replace => {
                    // ReplaceModal is handled entirely through handle_replace_action
                    // called from handle_modal_key/handle_modal_mouse (lines 183-233, 383-434).
                    // No additional processing needed here, similar to how SearchModal works.
                }
                PendingAction::QuitApplication => {
                    // User confirmed quit - exit application
                    self.state.quit();
                }
                PendingAction::SwitchSession => {
                    self.handle_switch_session(value)?;
                }
                PendingAction::NewSession => {
                    self.handle_new_session_result(value)?;
                }
                PendingAction::ChangeRootPath => {
                    self.handle_change_root_path_result(value)?;
                }
                PendingAction::FileSearch { panel_index } => {
                    self.handle_file_search(panel_index, value)?;
                }
                PendingAction::ContentSearch { panel_index } => {
                    self.handle_content_search(panel_index, value)?;
                }
                // Navigation actions are handled in key_handler, should not get here
                PendingAction::NextPanel | PendingAction::PrevPanel => {}
                // Git actions will open panels directly, should not get here
                PendingAction::OpenGitStatus | PendingAction::OpenGitLog => {}
                // Git file action from File Info modal
                PendingAction::GitFileAction {
                    file_path,
                    repo_path,
                    is_staged,
                } => {
                    self.handle_git_file_action(value, &file_path, &repo_path, is_staged)?;
                }
                // Git commit action
                PendingAction::GitCommit { repo_path } => {
                    self.handle_git_commit(value, &repo_path)?;
                }
                // Git revert file action (with confirmation)
                PendingAction::GitRevertFile {
                    file_path,
                    repo_path,
                    is_staged,
                } => {
                    self.handle_git_revert_file(value, &file_path, &repo_path, is_staged)?;
                }
                // Switch active panel's working directory
                PendingAction::SwitchDirectory => {
                    self.handle_switch_directory(value)?;
                }
                // Add bookmark
                PendingAction::AddBookmark => {
                    self.handle_add_bookmark_result(value)?;
                }
            }
        }
        Ok(())
    }

    /// Handle bookmark add result
    fn handle_add_bookmark_result(&mut self, value: Box<dyn std::any::Any>) -> Result<()> {
        use std::path::Path;
        use termide_config::Bookmark;
        use termide_modal::BookmarkAddResult;

        if let Some(result) = value.downcast_ref::<BookmarkAddResult>() {
            let mut bookmark = Bookmark::new(result.path.clone());

            // Use provided description or generate from path (last component)
            let description = match &result.description {
                Some(desc) => desc.clone(),
                None => Path::new(&result.path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| result.path.clone()),
            };
            bookmark = bookmark.with_description(description);

            if let Some(group) = &result.group {
                bookmark = bookmark.with_group(group.clone());
            }

            self.state.bookmarks.add(bookmark);
            self.state.save_bookmarks();
        }
        Ok(())
    }

    /// Handle search result
    fn handle_search(&mut self, value: Box<dyn std::any::Any>) -> Result<()> {
        if let Some(query) = value.downcast_ref::<String>() {
            // Start search in active editor (case insensitive by default)
            if let Some(editor) = self.active_editor_mut() {
                editor.start_search(query.clone(), false);
            }
        }
        Ok(())
    }

    /// Handle replace action from ReplaceModal
    fn handle_replace_action(&mut self, replace_result: &ReplaceModalResult) -> Result<()> {
        // Get active editor
        if let Some(editor) = self.active_editor_mut() {
            match replace_result.action {
                ReplaceAction::Search => {
                    // Perform new search/replace (or update existing)
                    editor.start_replace(
                        replace_result.find_query.clone(),
                        replace_result.replace_with.clone(),
                        false,
                    );
                }
                ReplaceAction::Next => {
                    // Update only replace_with value without rebuilding search
                    editor.update_replace_with(replace_result.replace_with.clone());
                    // Navigate to next match
                    editor.search_next();
                }
                ReplaceAction::Previous => {
                    // Update only replace_with value without rebuilding search
                    editor.update_replace_with(replace_result.replace_with.clone());
                    // Navigate to previous match
                    editor.search_prev();
                }
                ReplaceAction::Replace => {
                    // Update only replace_with value without rebuilding search
                    // This preserves the current_match index for sequential replacement
                    editor.update_replace_with(replace_result.replace_with.clone());
                    // Replace current match and position cursor on next match
                    editor.replace_current()?;
                    // Don't call search_next() - replace_current() already positions cursor correctly
                }
                ReplaceAction::ReplaceAll => {
                    // Update search state with latest values from modal before replacing all
                    editor.start_replace(
                        replace_result.find_query.clone(),
                        replace_result.replace_with.clone(),
                        false,
                    );
                    // Replace all matches (now uses updated replace_with)
                    editor.replace_all()?;
                }
            }
        }
        Ok(())
    }

    /// Handle search action from SearchModal
    fn handle_search_action(&mut self, search_result: &SearchModalResult) -> Result<()> {
        // Get active editor
        if let Some(editor) = self.active_editor_mut() {
            match search_result.action {
                SearchAction::Search => {
                    // Perform new search (or update existing)
                    editor.start_search(search_result.query.clone(), false);
                }
                SearchAction::Next => {
                    // Navigate to next match
                    editor.search_next();
                }
                SearchAction::Previous => {
                    // Navigate to previous match
                    editor.search_prev();
                }
                SearchAction::CloseWithSelection => {
                    // Just ensure search is active (will be handled by modal close logic)
                    // Selection is already set by editor methods
                }
            }
        }
        Ok(())
    }

    /// Process search modal result and determine what to do
    fn process_search_modal_result(
        &mut self,
        result: &ModalResult<Box<dyn std::any::Any>>,
    ) -> SearchReplaceResult {
        if let ModalResult::Confirmed(value) = result {
            if let Some(search_result) = value.downcast_ref::<SearchModalResult>() {
                // Handle search action in editor
                if self.handle_search_action(search_result).is_err() {
                    return SearchReplaceResult::Close;
                }

                // Get match info from active editor
                let match_info = self
                    .active_editor_mut()
                    .and_then(|editor| editor.get_search_match_info());

                // Check if we should close modal
                if matches!(search_result.action, SearchAction::CloseWithSelection) {
                    return SearchReplaceResult::Close;
                }

                // Update match info in modal for other actions
                if let Some((current, total)) = match_info {
                    if let Some(ActiveModal::Search(search_modal)) = &mut self.state.active_modal {
                        search_modal.set_match_info(current, total);
                    }
                }

                return SearchReplaceResult::KeepOpen;
            }
        } else if matches!(result, ModalResult::Cancelled) {
            return SearchReplaceResult::Cancelled;
        }
        SearchReplaceResult::NotApplicable
    }

    /// Process replace modal result and determine what to do
    fn process_replace_modal_result(
        &mut self,
        result: &ModalResult<Box<dyn std::any::Any>>,
    ) -> SearchReplaceResult {
        if let ModalResult::Confirmed(value) = result {
            if let Some(replace_result) = value.downcast_ref::<ReplaceModalResult>() {
                // Handle replace action in editor
                if self.handle_replace_action(replace_result).is_err() {
                    return SearchReplaceResult::Close;
                }

                // Get match info from active editor
                let match_info = self
                    .active_editor_mut()
                    .and_then(|editor| editor.get_search_match_info());

                // Check if we should close modal
                if matches!(replace_result.action, ReplaceAction::ReplaceAll) {
                    return SearchReplaceResult::Close;
                }

                // Update match info in modal for other actions
                if let Some((current, total)) = match_info {
                    if let Some(ActiveModal::Replace(replace_modal)) = &mut self.state.active_modal
                    {
                        replace_modal.set_match_info(current, total);
                    }
                }

                return SearchReplaceResult::KeepOpen;
            }
        } else if matches!(result, ModalResult::Cancelled) {
            return SearchReplaceResult::Cancelled;
        }
        SearchReplaceResult::NotApplicable
    }

    /// Handle search/replace modal result and return whether to continue processing
    fn handle_search_replace_modal(
        &mut self,
        is_search: bool,
        is_replace: bool,
        result: &ModalResult<Box<dyn std::any::Any>>,
    ) -> Option<()> {
        if is_search {
            match self.process_search_modal_result(result) {
                SearchReplaceResult::KeepOpen => return Some(()),
                SearchReplaceResult::Close => {
                    self.state.close_modal();
                    return Some(());
                }
                SearchReplaceResult::Cancelled => {
                    self.state.close_modal();
                    if let Some(editor) = self.active_editor_mut() {
                        editor.close_search();
                    }
                    return Some(());
                }
                SearchReplaceResult::NotApplicable => {}
            }
        }

        if is_replace {
            match self.process_replace_modal_result(result) {
                SearchReplaceResult::KeepOpen => return Some(()),
                SearchReplaceResult::Close => {
                    self.state.close_modal();
                    return Some(());
                }
                SearchReplaceResult::Cancelled => {
                    self.state.close_modal();
                    if let Some(editor) = self.active_editor_mut() {
                        editor.close_search();
                    }
                    return Some(());
                }
                SearchReplaceResult::NotApplicable => {}
            }
        }

        None // Continue with normal modal handling
    }

    /// Handle git file action from InfoActionModal
    fn handle_git_file_action(
        &mut self,
        value: Box<dyn std::any::Any>,
        file_path: &std::path::Path,
        repo_path: &std::path::Path,
        is_staged: bool,
    ) -> Result<()> {
        use termide_modal::InfoActionResult;

        if let Some(result) = value.downcast_ref::<InfoActionResult>() {
            match result {
                InfoActionResult::Action(action) => match action.as_str() {
                    "git_status" => {
                        // Open Git Status panel for the repository
                        if !self.find_and_focus_panel_by_name("git_status") {
                            // Not found, create new one
                            let git_status_panel =
                                termide_panel_git_status::GitStatusPanel::new_for_repo(
                                    repo_path.to_path_buf(),
                                );
                            self.add_panel(Box::new(git_status_panel));
                        }
                        self.auto_save_session();
                    }
                    "stage" => {
                        if let Err(e) = termide_git::stage_file(repo_path, file_path) {
                            self.state.set_error(format!("Stage error: {}", e));
                        } else {
                            self.state.set_info("File staged".to_string());
                        }
                    }
                    "unstage" => {
                        if let Err(e) = termide_git::unstage_file(repo_path, file_path) {
                            self.state.set_error(format!("Unstage error: {}", e));
                        } else {
                            self.state.set_info("File unstaged".to_string());
                        }
                    }
                    "edit" | "diff" => {
                        // Open file in editor (editor shows git diff markers automatically)
                        let full_path = repo_path.join(file_path);
                        let _ = self.open_editor_for_file(full_path);
                    }
                    "revert" => {
                        // Open confirmation modal before reverting
                        let t = termide_i18n::t();
                        let confirm_msg =
                            format!("{}\n\n{}", file_path.display(), t.git_revert_confirm());
                        let modal =
                            termide_modal::ConfirmModal::new(t.git_action_revert(), &confirm_msg);
                        self.state.set_pending_action(
                            termide_state::PendingAction::GitRevertFile {
                                file_path: file_path.to_path_buf(),
                                repo_path: repo_path.to_path_buf(),
                                is_staged,
                            },
                            ActiveModal::Confirm(Box::new(modal)),
                        );
                    }
                    _ => {
                        // Close the modal for "close" or any unknown action
                    }
                },
                InfoActionResult::Closed => {
                    // Just close the modal
                }
                InfoActionResult::CancelOperation => {
                    // This is handled in handle_git_push_pull_from_modal, should not reach here
                }
            }
        }
        Ok(())
    }

    /// Handle git commit action from CommitModal
    fn handle_git_commit(
        &mut self,
        value: Box<dyn std::any::Any>,
        repo_path: &std::path::Path,
    ) -> Result<()> {
        // value is the commit message (String)
        if let Some(message) = value.downcast_ref::<String>() {
            match termide_git::commit(repo_path, message) {
                Ok(commit_id) => {
                    self.state.set_info(format!(
                        "Committed: {}",
                        &commit_id[..8.min(commit_id.len())]
                    ));
                    // Trigger git update event to refresh panels
                    self.send_git_update(repo_path);
                }
                Err(e) => {
                    self.state.set_error(format!("Commit failed: {}", e));
                }
            }
        }
        Ok(())
    }

    /// Handle git revert file action (after confirmation)
    fn handle_git_revert_file(
        &mut self,
        value: Box<dyn std::any::Any>,
        file_path: &std::path::Path,
        repo_path: &std::path::Path,
        is_staged: bool,
    ) -> Result<()> {
        // value is bool from ConfirmModal
        if let Some(&confirmed) = value.downcast_ref::<bool>() {
            if confirmed {
                // If file is staged, unstage it first
                if is_staged {
                    if let Err(e) = termide_git::unstage_file(repo_path, file_path) {
                        self.state.set_error(format!("Unstage error: {}", e));
                        return Ok(());
                    }
                }
                // Now revert the file
                if let Err(e) = termide_git::revert_file(repo_path, file_path) {
                    self.state.set_error(format!("Revert error: {}", e));
                } else {
                    self.state.set_info("File reverted".to_string());
                    // Trigger git update event to refresh panels
                    self.send_git_update(repo_path);
                }
            }
        }
        Ok(())
    }

    /// Send git update event to refresh git panels
    fn send_git_update(&mut self, repo_path: &std::path::Path) {
        use termide_core::PanelCommand;
        // Send OnGitUpdate command to all panels
        let repo_paths: Vec<&std::path::Path> = vec![repo_path];
        for panel in self.layout_manager.iter_all_panels_mut() {
            let _ = panel.handle_command(PanelCommand::OnGitUpdate {
                repo_paths: &repo_paths,
            });
        }
    }

    /// Handle git push/pull actions from InfoActionModal
    /// Returns true if the action was handled (and modal should stay open or closed by us)
    fn handle_git_push_pull_from_modal(
        &mut self,
        result: &ModalResult<Box<dyn std::any::Any>>,
    ) -> Result<bool> {
        use termide_core::GitOperationType;
        use termide_modal::InfoActionResult;
        use termide_state::PendingAction;

        // Check if the pending action is a git file action
        let is_git_file_action = matches!(
            &self.state.pending_action,
            Some(PendingAction::GitFileAction { .. })
        );

        if !is_git_file_action {
            return Ok(false);
        }

        // Check if result is push, pull, or cancel operation
        if let ModalResult::Confirmed(value) = result {
            if let Some(action_result) = value.downcast_ref::<InfoActionResult>() {
                match action_result {
                    InfoActionResult::Action(action) => {
                        let operation = match action.as_str() {
                            "push" => GitOperationType::Push,
                            "pull" => GitOperationType::Pull,
                            _ => return Ok(false),
                        };

                        // Get repo_path from pending action
                        let repo_path = match &self.state.pending_action {
                            Some(PendingAction::GitFileAction { repo_path, .. }) => {
                                repo_path.clone()
                            }
                            _ => return Ok(false),
                        };

                        // Set operation in progress on the modal
                        if let Some(ActiveModal::InfoAction(modal)) = &mut self.state.active_modal {
                            modal.set_operation_in_progress(Some(action.clone()));
                        }

                        // Start background git operation
                        self.event_git_operation(operation, repo_path)?;

                        return Ok(true);
                    }
                    InfoActionResult::CancelOperation => {
                        // Cancel the running git operation
                        self.event_cancel_git_operation();

                        // Clear operation state on modal but keep it open
                        if let Some(ActiveModal::InfoAction(modal)) = &mut self.state.active_modal {
                            modal.set_operation_in_progress(None);
                        }

                        return Ok(true);
                    }
                    _ => return Ok(false),
                }
            }
        }

        Ok(false)
    }
}
