//! LSP integration methods for the Editor.
//!
//! This module contains all LSP-related functionality including:
//! - Initialization and cleanup
//! - Completion (request, accept, filter, navigate)
//! - Hover (request, display, close)
//! - Go-to-definition
//! - Diagnostics
//! - Auto-completion scheduling

use termide_core::PanelEvent;

use crate::{completion_popup, hover_popup};

use super::{CompletionTriggerKind, Editor, LspManager, ServerStatus};

impl Editor {
    // =========================================================================
    // LSP Initialization & Lifecycle
    // =========================================================================

    /// Initialize LSP for this editor's file.
    ///
    /// Should be called after opening a file to enable LSP features.
    /// This detects the language, starts the appropriate server if configured,
    /// and sends the `didOpen` notification.
    pub fn init_lsp(&mut self, lsp_manager: &mut LspManager) {
        if let Some(path) = self.buffer.file_path() {
            self.lsp.init_for_file(path, lsp_manager);

            if self.lsp.enabled {
                let content = self.buffer.to_string();
                self.lsp.did_open(path, &content, lsp_manager);
            }
        }
    }

    /// Notify LSP about buffer content change.
    ///
    /// Should be called after any text modification (insert, delete, etc.)
    /// to keep the language server in sync with the editor content.
    pub fn notify_lsp_change(&mut self, lsp_manager: &LspManager) {
        if let Some(path) = self.buffer.file_path() {
            let content = self.buffer.to_string();
            self.lsp.did_change(path, &content, lsp_manager);
        }
    }

    /// Cleanup LSP when editor is closed.
    ///
    /// Sends the `didClose` notification to the language server.
    pub fn cleanup_lsp(&self, lsp_manager: &LspManager) {
        if let Some(path) = self.buffer.file_path() {
            self.lsp.did_close(path, lsp_manager);
        }
    }

    /// Check if LSP is enabled for this editor.
    pub fn lsp_enabled(&self) -> bool {
        self.lsp.enabled
    }

    /// Get the language ID for this editor's file.
    pub fn lsp_language(&self) -> Option<&str> {
        self.lsp.language_id.as_deref()
    }

    /// Mark that the buffer has changed (for LSP notification).
    pub fn mark_lsp_changed(&mut self) {
        self.lsp.mark_changed();
    }

    /// Check if there are pending LSP changes that need to be sent.
    pub fn has_pending_lsp_change(&self) -> bool {
        self.lsp.has_pending_change()
    }

    /// Send pending LSP change notification if needed.
    ///
    /// Returns true if a notification was sent.
    pub fn flush_lsp_changes(&mut self, lsp_manager: &LspManager) -> bool {
        if self.lsp.has_pending_change() {
            self.notify_lsp_change(lsp_manager);
            self.lsp.clear_pending_change();
            true
        } else {
            false
        }
    }

    // =========================================================================
    // LSP Completion
    // =========================================================================

    /// Trigger completion popup request.
    ///
    /// This marks that completion is requested. The actual LSP request
    /// should be made by the app layer which has access to LspManager.
    pub fn trigger_completion(&mut self) {
        // Mark that completion is requested - will be picked up by app layer
        // The actual request requires LspManager which we don't have here
        self.lsp.completion_requested = true;

        // Save the start of the word as trigger column for prefix calculation
        self.lsp.completion_trigger_column = self.find_word_start_column();
    }

    /// Find the column position of the start of the current word.
    ///
    /// Used to determine the prefix when triggering completion.
    fn find_word_start_column(&self) -> usize {
        let line_text = self
            .buffer
            .line(self.cursor.line)
            .map(|cow| cow.to_string())
            .unwrap_or_default();

        let cursor_col = self.cursor.column;
        if cursor_col == 0 || line_text.is_empty() {
            return 0;
        }

        // Walk backwards from cursor to find word start
        let chars: Vec<char> = line_text.chars().collect();
        let mut start = cursor_col.min(chars.len());

        while start > 0 {
            let ch = chars[start - 1];
            // Stop at non-identifier characters
            if !ch.is_alphanumeric() && ch != '_' {
                break;
            }
            start -= 1;
        }

        start
    }

    /// Check if completion was requested and clear the flag.
    pub fn take_completion_request(&mut self) -> Option<(usize, usize)> {
        if self.lsp.completion_requested {
            self.lsp.completion_requested = false;
            Some((self.cursor.line, self.cursor.column))
        } else {
            None
        }
    }

    /// Request completion from LSP at current cursor position.
    pub fn request_completion(&mut self, lsp_manager: &LspManager) {
        if let Some(path) = self.buffer.file_path() {
            self.lsp.request_completion(
                path,
                self.cursor.line,
                self.cursor.column,
                CompletionTriggerKind::INVOKED,
                None,
                lsp_manager,
            );
        }
    }

    /// Poll for completion response and show popup if available.
    pub fn poll_completion(&mut self) {
        if let Some(response) = self.lsp.poll_completion() {
            let mut popup = completion_popup::CompletionPopup::from_response(response);

            // Get the prefix (text between trigger column and cursor)
            let prefix = self.get_completion_prefix();
            if !prefix.is_empty() {
                popup.set_filter(&prefix);
            }

            if !popup.is_empty() {
                self.lsp.completion_popup = Some(popup);
            }
        }
    }

    /// Get the prefix text for completion (from trigger column to cursor).
    fn get_completion_prefix(&self) -> String {
        let line_text = self
            .buffer
            .line(self.cursor.line)
            .map(|cow| cow.to_string())
            .unwrap_or_default();

        let trigger_col = self.lsp.completion_trigger_column;
        let cursor_col = self.cursor.column;

        if cursor_col > trigger_col && cursor_col <= line_text.chars().count() {
            line_text
                .chars()
                .skip(trigger_col)
                .take(cursor_col - trigger_col)
                .collect()
        } else {
            String::new()
        }
    }

    /// Accept selected completion item.
    ///
    /// Inserts the completion text and closes the popup.
    /// Deletes from trigger column to cursor, then inserts completion text.
    pub fn accept_completion(&mut self) {
        if let Some(popup) = &self.lsp.completion_popup {
            if let Some(insert_text) = popup.selected_insert_text() {
                // Calculate total chars to delete:
                // From trigger_column to current cursor position
                let trigger_col = self.lsp.completion_trigger_column;
                let cursor_col = self.cursor.column;
                let chars_to_delete = cursor_col.saturating_sub(trigger_col);

                // Delete the prefix + any additional typed characters
                for _ in 0..chars_to_delete {
                    let _ = self.backspace();
                }

                // Insert the completion text
                for ch in insert_text.chars() {
                    let _ = self.insert_char(ch);
                }
            }
        }
        self.lsp.completion_popup = None;
    }

    /// Cancel completion popup.
    pub fn cancel_completion(&mut self) {
        self.lsp.completion_popup = None;
    }

    /// Select next completion item.
    pub fn next_completion(&mut self) {
        if let Some(popup) = &mut self.lsp.completion_popup {
            popup.select_next();
        }
    }

    /// Select previous completion item.
    pub fn prev_completion(&mut self) {
        if let Some(popup) = &mut self.lsp.completion_popup {
            popup.select_prev();
        }
    }

    /// Filter completion with typed character.
    ///
    /// Also inserts the character into the buffer.
    pub fn filter_completion(&mut self, ch: char) {
        // Insert char into buffer
        let _ = self.insert_char(ch);

        // Update popup filter
        if let Some(popup) = &mut self.lsp.completion_popup {
            popup.append_filter(ch);
            // Close popup if no matches
            if popup.is_empty() {
                self.lsp.completion_popup = None;
            }
        }
    }

    /// Remove last filter character from completion.
    ///
    /// Also performs backspace in the buffer.
    pub fn backspace_completion(&mut self) {
        // Perform backspace in buffer
        let _ = self.backspace();

        // Update popup filter
        if let Some(popup) = &mut self.lsp.completion_popup {
            popup.backspace_filter();
        }
    }

    /// Check if completion popup is open.
    pub fn has_completion_popup(&self) -> bool {
        self.lsp.completion_popup.is_some()
    }

    // =========================================================================
    // LSP Hover
    // =========================================================================

    /// Check if hover was requested (via mouse hover) and clear the flag.
    pub fn take_hover_request(&mut self) -> Option<(usize, usize)> {
        self.lsp.pending_hover_request.take()
    }

    /// Request hover info from LSP at specified position.
    pub fn request_hover(&mut self, line: usize, column: usize, lsp_manager: &LspManager) {
        if let Some(path) = self.buffer.file_path() {
            self.lsp.request_hover(path, line, column, lsp_manager);
        }
    }

    /// Request hover info at the current cursor position.
    ///
    /// This schedules a hover request that will be processed in tick() where LspManager is available.
    pub fn request_hover_at_cursor(&mut self) {
        // Close any existing hover popup first
        self.close_hover_popup();
        // Store cursor position for hover request
        self.lsp.pending_hover_request = Some((self.cursor.line, self.cursor.column));
    }

    /// Poll for hover response and show popup if available.
    pub fn poll_hover(&mut self) {
        if let Some(response) = self.lsp.poll_hover() {
            if let Some(popup) = hover_popup::HoverPopup::from_hover(response) {
                self.lsp.hover_popup = Some(popup);
            }
        }
    }

    /// Close hover popup.
    pub fn close_hover_popup(&mut self) {
        self.lsp.hover_popup = None;
        self.lsp.hover_popup_rect = None;
        self.lsp.pending_ctrl_click = None;
    }

    /// Cancel hover timer and close popup.
    ///
    /// Call this on any key press to cancel pending hover requests.
    pub fn cancel_hover_and_close_popup(&mut self) {
        self.lsp.cancel_hover_timer();
        self.close_hover_popup();
    }

    /// Check if hover popup is open.
    pub fn has_hover_popup(&self) -> bool {
        self.lsp.hover_popup.is_some()
    }

    /// Check and trigger delayed hover request if timer expired.
    ///
    /// Call this periodically (e.g., in tick/poll).
    /// Returns true if hover was requested.
    pub fn check_hover_timer(&mut self, lsp_manager: &LspManager, delay_ms: u64) -> bool {
        // Don't trigger if hover popup is already open
        if self.lsp.hover_popup.is_some() {
            self.lsp.cancel_hover_timer();
            return false;
        }

        if let Some((line, col)) = self.lsp.check_hover_timer(delay_ms) {
            self.request_hover(line, col, lsp_manager);
            true
        } else {
            false
        }
    }

    // =========================================================================
    // LSP Go-to-Definition
    // =========================================================================

    /// Check if go-to-definition was requested (via Ctrl+click) and clear the flag.
    pub fn take_definition_request(&mut self) -> Option<(usize, usize)> {
        self.lsp.pending_definition_request.take()
    }

    /// Request go-to-definition at cursor position.
    ///
    /// This schedules a definition request that will be processed in tick() where LspManager is available.
    pub fn request_definition_at_cursor(&mut self) {
        // Store cursor position for definition request
        self.lsp.pending_definition_request = Some((self.cursor.line, self.cursor.column));
    }

    /// Request go-to-definition from LSP at specified position.
    pub fn request_definition(&mut self, line: usize, column: usize, lsp_manager: &LspManager) {
        if let Some(path) = self.buffer.file_path() {
            self.lsp.request_definition(path, line, column, lsp_manager);
        }
    }

    /// Poll for definition response and convert to PanelEvent.
    ///
    /// Returns `Some(PanelEvent::OpenFileAt)` if a definition location was received.
    pub fn poll_definition(&mut self) -> Option<PanelEvent> {
        use lsp_types::GotoDefinitionResponse;
        use std::path::PathBuf;

        let response = self.lsp.poll_definition()?;

        // Extract location from response (take first if multiple)
        let (uri, position) = match response {
            GotoDefinitionResponse::Scalar(location) => (location.uri, location.range.start),
            GotoDefinitionResponse::Array(locations) => {
                let loc = locations.into_iter().next()?;
                (loc.uri, loc.range.start)
            }
            GotoDefinitionResponse::Link(links) => {
                let link = links.into_iter().next()?;
                (link.target_uri, link.target_selection_range.start)
            }
        };

        // Convert file:// URI to PathBuf
        let uri_str = uri.as_str();
        if !uri_str.starts_with("file://") {
            return None;
        }
        let path_str = &uri_str[7..]; // Skip "file://"
        #[cfg(unix)]
        let path = PathBuf::from(path_str);
        #[cfg(windows)]
        let path = PathBuf::from(path_str.trim_start_matches('/'));

        // LSP uses 0-based line/column
        let line = position.line as usize;
        let column = position.character as usize;

        Some(PanelEvent::OpenFileAt { path, line, column })
    }

    // =========================================================================
    // LSP Find References
    // =========================================================================

    /// Schedule a find-references request at cursor position (called from handle_key).
    pub fn request_references_at_cursor(&mut self) {
        self.lsp.pending_references_request = Some((self.cursor.line, self.cursor.column));
    }

    /// Check if find-references was requested and clear the flag.
    pub fn take_references_request(&mut self) -> Option<(usize, usize)> {
        self.lsp.pending_references_request.take()
    }

    /// Send find-references request to LSP at specified position.
    pub fn request_references(&mut self, line: usize, column: usize, lsp_manager: &LspManager) {
        if let Some(path) = self.buffer.file_path() {
            self.lsp.request_references(path, line, column, lsp_manager);
        }
    }

    /// Poll for references response (non-blocking).
    ///
    /// Returns `Some(locations)` if a response was received (may be empty if no references found).
    pub fn poll_references(&mut self) -> Option<Vec<lsp_types::Location>> {
        self.lsp.poll_references()
    }

    // =========================================================================
    // LSP Rename Symbol
    // =========================================================================

    /// Schedule a rename-symbol request at cursor position (called from handle_key).
    pub fn request_rename_at_cursor(&mut self) {
        self.lsp.pending_rename_request = Some((self.cursor.line, self.cursor.column));
    }

    /// Check if rename was requested and clear the flag.
    pub fn take_rename_request(&mut self) -> Option<(usize, usize)> {
        self.lsp.pending_rename_request.take()
    }

    /// Get the word at cursor position (for rename modal prefill).
    pub fn get_word_at_cursor(&self) -> String {
        use crate::selection::select_word;
        let Some((sel, _)) = select_word(&self.buffer, &self.cursor) else {
            return String::new();
        };
        let line_text = self
            .buffer
            .line(self.cursor.line)
            .map(|cow| cow.to_string())
            .unwrap_or_default();
        let start = sel.start().column;
        let end = sel.end().column;
        line_text.chars().skip(start).take(end - start).collect()
    }

    /// Send rename request to LSP at specified position.
    pub fn request_rename(
        &mut self,
        line: usize,
        column: usize,
        new_name: String,
        lsp_manager: &LspManager,
    ) {
        if let Some(path) = self.buffer.file_path() {
            self.lsp
                .request_rename(path, line, column, new_name, lsp_manager);
        }
    }

    /// Poll for rename response (non-blocking).
    pub fn poll_rename(&mut self) -> Option<lsp_types::WorkspaceEdit> {
        self.lsp.poll_rename()
    }

    // =========================================================================
    // LSP Diagnostics
    // =========================================================================

    /// Update diagnostics from LSP.
    pub fn update_diagnostics(&mut self, diagnostics: Vec<lsp_types::Diagnostic>) {
        self.lsp.update_diagnostics(diagnostics);
        self.render_cache.invalidate_diagnostic_cache();
    }

    // =========================================================================
    // LSP Server Status
    // =========================================================================

    /// Check if LSP server is loading (for spinner display).
    pub fn is_lsp_loading(&self) -> bool {
        self.lsp.server_loading
    }

    /// Update server loading status from actual LSP server status.
    ///
    /// Returns true if status changed (needs redraw).
    pub fn update_lsp_loading_status(&mut self, lsp_manager: &LspManager) -> bool {
        // Clone paths to avoid borrow issues
        let file_path = match self.file_path() {
            Some(p) => p.to_path_buf(),
            None => return false,
        };
        let lang = match &self.lsp.language_id {
            Some(l) => l.clone(),
            None => return false,
        };

        // Get current server status and update status text
        let status = lsp_manager.server_status(&lang, &file_path);
        let new_status_text = match status {
            Some(ServerStatus::Starting) => Some("starting".to_string()),
            Some(ServerStatus::Indexing) => Some("indexing".to_string()),
            _ => None,
        };
        let status_text_changed = self.lsp.server_status_text != new_status_text;
        self.lsp.server_status_text = new_status_text;

        // Check if server went back to indexing (e.g., after file changes)
        if !self.lsp.server_loading && lsp_manager.server_is_indexing(&lang, &file_path) {
            self.lsp.server_loading = true;
            return true;
        }

        // Check if server became ready
        if self.lsp.server_loading && lsp_manager.server_is_ready(&lang, &file_path) {
            self.lsp.server_loading = false;
            return true;
        }

        // Return true if status text changed (for redraw)
        status_text_changed
    }

    // =========================================================================
    // Auto-Completion Scheduling
    // =========================================================================

    /// Schedule auto-completion for the inserted character.
    ///
    /// Call this after inserting a character when auto_completion is enabled.
    /// For trigger characters (`.`, `:`, etc.), triggers immediately.
    /// For word characters, schedules delayed completion.
    pub fn schedule_auto_completion(&mut self, ch: char, lsp_manager: &LspManager) {
        // Don't schedule if popup is already open or server is loading
        if self.lsp.completion_popup.is_some() || self.lsp.server_loading {
            return;
        }

        // Trigger characters - trigger immediately with TriggerCharacter kind
        const TRIGGER_CHARS: &[char] = &['.', ':', '<', '('];
        if TRIGGER_CHARS.contains(&ch) {
            self.trigger_completion_with_kind(
                lsp_manager,
                CompletionTriggerKind::TRIGGER_CHARACTER,
                Some(ch.to_string()),
            );
            return;
        }

        // Word characters - schedule delayed completion
        if ch.is_alphanumeric() || ch == '_' {
            self.lsp.schedule_completion();
        }
    }

    /// Trigger completion with specific trigger kind.
    fn trigger_completion_with_kind(
        &mut self,
        lsp_manager: &LspManager,
        trigger_kind: CompletionTriggerKind,
        trigger_character: Option<String>,
    ) {
        let file_path = self.file_path().map(|p| p.to_path_buf());
        if let Some(file_path) = file_path {
            // Store trigger column for prefix calculation
            self.lsp.completion_trigger_column = self.get_word_start_column();
            self.lsp.completion_requested = true;

            self.lsp.request_completion(
                &file_path,
                self.cursor.line,
                self.cursor.column,
                trigger_kind,
                trigger_character,
                lsp_manager,
            );
        }
    }

    /// Check and trigger delayed auto-completion if timer expired.
    ///
    /// Call this periodically (e.g., in tick/poll).
    /// Returns true if completion was triggered.
    pub fn check_auto_completion(&mut self, lsp_manager: &LspManager, delay_ms: u64) -> bool {
        // Don't trigger if popup is open
        if self.lsp.completion_popup.is_some() {
            self.lsp.cancel_completion_timer();
            return false;
        }

        if self.lsp.check_completion_timer(delay_ms) {
            self.trigger_completion_with_kind(lsp_manager, CompletionTriggerKind::INVOKED, None);
            true
        } else {
            false
        }
    }

    /// Get the column where current word starts (for completion prefix).
    fn get_word_start_column(&self) -> usize {
        if let Some(line_text) = self.buffer.line(self.cursor.line) {
            let prefix: String = line_text.chars().take(self.cursor.column).collect();
            // Find start of current word (alphanumeric + underscore)
            let word_start = prefix
                .char_indices()
                .rev()
                .find(|(_, c)| !c.is_alphanumeric() && *c != '_')
                .map(|(i, _)| i + 1)
                .unwrap_or(0);
            word_start
        } else {
            self.cursor.column
        }
    }

    /// Take the last inserted character (if any) and clear it.
    ///
    /// Used by key_handler to schedule auto-completion after character insertion.
    pub fn take_last_inserted_char(&mut self) -> Option<char> {
        self.lsp.last_inserted_char.take()
    }
}
