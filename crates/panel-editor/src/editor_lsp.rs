//! LSP integration methods for the Editor.
//!
//! This module contains all LSP-related functionality including:
//! - Initialization and cleanup
//! - Completion (request, accept, filter, navigate)
//! - Hover (request, display, close)
//! - Go-to-definition
//! - Diagnostics
//! - Auto-completion scheduling

use termide_buffer::Cursor;
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
    /// Prefers the server-provided `textEdit` (replace its exact range with the
    /// new text) plus any `additionalTextEdits` (e.g. an added `use` import).
    /// Only when the item carries no `textEdit` do we fall back to the
    /// prefix-deletion heuristic — that heuristic mis-handled cases like `$va`
    /// (treats `$` as a word boundary → `$$var`) and produced duplicated text.
    pub fn accept_completion(&mut self) {
        // Pull owned data out first so the popup borrow ends before we mutate
        // the buffer.
        let resolution = self
            .lsp
            .completion_popup
            .as_ref()
            .and_then(|popup| popup.selected_resolution());

        if let Some(resolution) = resolution {
            if let Some(primary) = resolution.primary {
                // Authoritative path: apply the server's edits verbatim.
                let mut edits = Vec::with_capacity(1 + resolution.additional.len());
                edits.push(primary);
                edits.extend(resolution.additional);
                self.apply_completion_text_edits(&edits);
            } else {
                // Fallback: replace the typed prefix with the item text.
                let trigger_col = self.lsp.completion_trigger_column;
                let cursor_col = self.cursor.column;
                let chars_to_delete = cursor_col.saturating_sub(trigger_col);
                for _ in 0..chars_to_delete {
                    let _ = self.backspace();
                }
                for ch in resolution.fallback_text.chars() {
                    let _ = self.insert_char(ch);
                }
                // Side-effect edits (imports) can still accompany a textEdit-less item.
                if !resolution.additional.is_empty() {
                    let additional = resolution.additional.clone();
                    self.apply_completion_text_edits(&additional);
                }
            }
        }
        self.lsp.completion_popup = None;
    }

    /// Apply a set of LSP `TextEdit`s to the buffer.
    ///
    /// Edits are applied bottom-up (descending start position) so that earlier
    /// edits don't invalidate the ranges of later ones. The cursor is placed at
    /// the end of the lowest edit (the one nearest the caret — typically the
    /// completion itself), adjusted for line shifts introduced by edits above
    /// it (e.g. an inserted `use` line).
    fn apply_completion_text_edits(&mut self, edits: &[lsp_types::TextEdit]) {
        let Some(min_line) = edits.iter().map(|e| e.range.start.line as usize).min() else {
            return;
        };

        if let Some(cursor) = apply_text_edits(&mut self.buffer, edits) {
            self.cursor = cursor;
        }
        self.clamp_cursor();
        self.invalidate_cache_after_edit(min_line, true);
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

    // =========================================================================
    // LSP Code Actions
    // =========================================================================

    /// Mark that code actions were requested (Ctrl+.). The actual request is
    /// issued by the app layer, which holds the `LspManager`.
    pub fn trigger_code_action(&mut self) {
        self.lsp.code_action_requested = true;
    }

    /// Consume the code-action request flag.
    pub fn take_code_action_request(&mut self) -> bool {
        std::mem::take(&mut self.lsp.code_action_requested)
    }

    /// Request code actions for the current line (so a quick-fix like "Import
    /// class" sees the line's diagnostic as context).
    pub fn request_code_action(&mut self, lsp_manager: &LspManager) {
        let Some(path) = self.buffer.file_path().map(|p| p.to_path_buf()) else {
            return;
        };
        let line = self.cursor.line as u32;
        let line_len = self
            .buffer
            .line(self.cursor.line)
            .map(|l| l.trim_end_matches('\n').chars().count())
            .unwrap_or(0) as u32;
        let range = lsp_types::Range::new(
            lsp_types::Position::new(line, 0),
            lsp_types::Position::new(line, line_len),
        );
        self.lsp.request_code_action(&path, range, lsp_manager);
    }

    /// Poll for a code-action response and open the popup if any actions apply.
    pub fn poll_code_action(&mut self) {
        if let Some(response) = self.lsp.poll_code_action() {
            self.lsp.code_action_popup =
                crate::code_action_popup::CodeActionPopup::from_response(response);
        }
    }

    /// Whether the code-action popup is open.
    pub fn has_code_action_popup(&self) -> bool {
        self.lsp.code_action_popup.is_some()
    }

    /// Accept the selected code action: stash its `WorkspaceEdit` for the app
    /// layer to apply (edits may span files / reload other editors) and close
    /// the popup.
    pub fn accept_code_action(&mut self) {
        self.lsp.pending_code_action_edit = self
            .lsp
            .code_action_popup
            .as_ref()
            .and_then(|popup| popup.selected_edit());
        self.lsp.code_action_popup = None;
    }

    /// Take the pending code-action `WorkspaceEdit`, if the user accepted one.
    pub fn take_code_action_edit(&mut self) -> Option<lsp_types::WorkspaceEdit> {
        self.lsp.pending_code_action_edit.take()
    }

    /// Cancel the code-action popup.
    pub fn cancel_code_action(&mut self) {
        self.lsp.code_action_popup = None;
    }

    /// Select the next code action.
    pub fn next_code_action(&mut self) {
        if let Some(popup) = &mut self.lsp.code_action_popup {
            popup.select_next();
        }
    }

    /// Select the previous code action.
    pub fn prev_code_action(&mut self) {
        if let Some(popup) = &mut self.lsp.code_action_popup {
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

        // Prefer the trigger characters the server advertised (e.g. phpactor
        // reports `>` so `->` opens the popup); fall back to a built-in set
        // when the server hasn't reported capabilities yet or advertises none.
        // Read against the same `language_id` used to issue the request so the
        // server lookup matches.
        let server_triggers = match (self.buffer.file_path(), self.lsp.language_id.as_deref()) {
            (Some(path), Some(lang)) => lsp_manager.completion_trigger_characters(lang, path),
            _ => Vec::new(),
        };

        if char_triggers_completion(ch, &server_triggers) {
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

/// Apply LSP `TextEdit`s to `buffer`, returning where the caret should land.
///
/// Edits are applied bottom-up (descending start position) so an earlier edit
/// never shifts the range of a later one. The returned cursor follows the
/// lowest edit (nearest the caret — usually the completion text itself),
/// shifted down by the net lines that edits *above* it inserted (e.g. a `use`
/// import added at the top of the file).
fn apply_text_edits(
    buffer: &mut termide_buffer::TextBuffer,
    edits: &[lsp_types::TextEdit],
) -> Option<Cursor> {
    if edits.is_empty() {
        return None;
    }

    let primary = edits
        .iter()
        .max_by_key(|e| (e.range.start.line, e.range.start.character))
        .cloned();

    let mut ordered: Vec<&lsp_types::TextEdit> = edits.iter().collect();
    ordered.sort_by(|a, b| {
        (b.range.start.line, b.range.start.character)
            .cmp(&(a.range.start.line, a.range.start.character))
    });

    for edit in ordered {
        let start = Cursor::at(
            edit.range.start.line as usize,
            edit.range.start.character as usize,
        );
        let end = Cursor::at(
            edit.range.end.line as usize,
            edit.range.end.character as usize,
        );
        let _ = buffer.delete_range(&start, &end);
        if !edit.new_text.is_empty() {
            let _ = buffer.insert(&start, &edit.new_text);
        }
    }

    primary.map(|primary| {
        let lines_added_above: i64 = edits
            .iter()
            .filter(|e| e.range.start.line < primary.range.start.line)
            .map(|e| {
                let inserted = e.new_text.matches('\n').count() as i64;
                let removed = (e.range.end.line - e.range.start.line) as i64;
                inserted - removed
            })
            .sum();
        let new_lines = primary.new_text.matches('\n').count();
        let line = (primary.range.start.line as i64 + lines_added_above + new_lines as i64).max(0)
            as usize;
        let column = if new_lines == 0 {
            primary.range.start.character as usize + primary.new_text.chars().count()
        } else {
            primary
                .new_text
                .rsplit('\n')
                .next()
                .map(|s| s.chars().count())
                .unwrap_or(0)
        };
        Cursor::at(line, column)
    })
}

/// Built-in completion trigger characters, used only when the language server
/// advertises none (or hasn't reported its capabilities yet).
const FALLBACK_TRIGGERS: &[char] = &['.', ':', '<', '('];

/// Whether typing `ch` should immediately open completion.
///
/// The server's advertised `triggerCharacters` take precedence (so e.g.
/// phpactor's `>` opens the popup right after `->`); the built-in
/// [`FALLBACK_TRIGGERS`] are used only when the server advertises none.
fn char_triggers_completion(ch: char, server_triggers: &[String]) -> bool {
    if server_triggers.is_empty() {
        FALLBACK_TRIGGERS.contains(&ch)
    } else {
        server_triggers
            .iter()
            .any(|trigger| trigger.starts_with(ch))
    }
}

#[cfg(test)]
mod text_edit_tests {
    use super::apply_text_edits;
    use lsp_types::{Position, Range, TextEdit};
    use termide_buffer::TextBuffer;

    fn edit(sl: u32, sc: u32, el: u32, ec: u32, text: &str) -> TextEdit {
        TextEdit {
            range: Range::new(Position::new(sl, sc), Position::new(el, ec)),
            new_text: text.to_string(),
        }
    }

    #[test]
    fn text_edit_replaces_prefix_without_duplication() {
        // Regression for #22: typing `$va`, selecting `$var` must yield `$var`,
        // not `$$var`. The server's textEdit replaces the whole `$va`.
        let mut buffer = TextBuffer::from_text("$va");
        let cursor = apply_text_edits(&mut buffer, &[edit(0, 0, 0, 3, "$var")]);
        assert_eq!(buffer.to_string(), "$var");
        let cursor = cursor.expect("primary edit yields a cursor");
        assert_eq!((cursor.line, cursor.column), (0, 4));
    }

    #[test]
    fn class_prefix_is_replaced_not_appended() {
        // `Ord` -> `Order` must not become `OrdOrder`.
        let mut buffer = TextBuffer::from_text("Ord");
        apply_text_edits(&mut buffer, &[edit(0, 0, 0, 3, "Order")]);
        assert_eq!(buffer.to_string(), "Order");
    }

    #[test]
    fn additional_edits_apply_and_cursor_shifts_below_inserted_lines() {
        // Completion at the bottom plus a `use` import inserted at the top:
        // both land, and the caret follows the completion, shifted down by the
        // line the import added.
        let mut buffer = TextBuffer::from_text("<?php\n\nOrd");
        let edits = vec![
            edit(2, 0, 2, 3, "Order"),             // the completion (line 2)
            edit(1, 0, 1, 0, "use App\\Order;\n"), // import added above (line 1)
        ];
        let cursor = apply_text_edits(&mut buffer, &edits).expect("cursor");
        assert_eq!(buffer.to_string(), "<?php\nuse App\\Order;\n\nOrder");
        // Completion was on line 2; the import added one line above it -> line 3.
        assert_eq!((cursor.line, cursor.column), (3, 5));
    }
}

#[cfg(test)]
mod completion_trigger_tests {
    use super::char_triggers_completion;

    #[test]
    fn server_trigger_characters_take_precedence() {
        // phpactor-style triggers: `->` (the `>`) and `::` (the `:`).
        let triggers = vec![">".to_string(), ":".to_string()];
        assert!(char_triggers_completion('>', &triggers));
        assert!(char_triggers_completion(':', &triggers));
        // `.` is a fallback trigger but NOT advertised here, so it must not fire.
        assert!(!char_triggers_completion('.', &triggers));
        assert!(!char_triggers_completion('a', &triggers));
    }

    #[test]
    fn falls_back_when_server_advertises_none() {
        let none: Vec<String> = Vec::new();
        assert!(char_triggers_completion('.', &none));
        assert!(char_triggers_completion(':', &none));
        assert!(!char_triggers_completion('>', &none));
        assert!(!char_triggers_completion('a', &none));
    }
}
