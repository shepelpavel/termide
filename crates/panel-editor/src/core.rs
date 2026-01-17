use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{buffer::Buffer, layout::Rect};
use std::any::Any;
use std::path::PathBuf;

use termide_buffer::{Cursor, SearchState, Selection, TextBuffer, Viewport};
use termide_config::Config;
use termide_core::{CommandResult, Panel, PanelCommand, PanelEvent, RenderContext, SessionPanel};
use termide_git::GitDiffCache;
use termide_i18n::t;
use termide_modal::{ActiveModal, InputModal, ReplaceModal, SearchModal};
use termide_state::PendingAction;
use termide_theme::Theme;
use termide_ui::ScrollBar;

use crate::{
    clipboard, completion_popup,
    config::*,
    constants, cursor, file_io, git, keyboard, rendering, search, selection,
    state::{FileState, GitIntegration, InputState, LspState, RenderingCache, SearchController},
    text_editing, word_wrap,
};

// Re-export LspManager for use in app integration
pub use termide_lsp::{CompletionTriggerKind, LspManager, ServerStatus};

/// Convert screen column to grapheme index, accounting for display widths.
///
/// Used for mouse click position conversion.
fn screen_col_to_grapheme_idx(text: &str, target_col: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    let mut col = 0;
    let mut last_idx = 0;
    for (idx, g) in text.graphemes(true).enumerate() {
        let w = g.width();
        if col + w > target_col {
            return idx;
        }
        col += w;
        last_idx = idx + 1; // Track grapheme count as we iterate
    }
    last_idx // Return count without re-iterating
}

/// Editor panel with syntax highlighting
pub struct Editor {
    // === Core editing state ===
    /// Editor mode configuration
    config: EditorConfig,
    /// Text buffer with Rope
    buffer: TextBuffer,
    /// Cursor
    cursor: Cursor,
    /// Text selection (if any)
    selection: Option<Selection>,
    /// Viewport for virtual scrolling
    viewport: Viewport,

    // === Grouped state ===
    /// File-related state (mtime, external changes, title)
    pub(crate) file_state: FileState,
    /// Search-related state
    pub(crate) search: SearchController,
    /// Git integration state
    pub(crate) git: GitIntegration,
    /// Rendering cache
    pub(crate) render_cache: RenderingCache,
    /// Input state (clicks, preferred column)
    pub(crate) input: InputState,
    /// LSP integration state
    pub(crate) lsp: LspState,

    // === UI state ===
    /// Modal window request
    modal_request: Option<(PendingAction, ActiveModal)>,
    /// Updated config after save (for applying in AppState)
    config_update: Option<Config>,
    /// Status message to display to user
    pub(crate) status_message: Option<String>,
    /// When true, viewport follows cursor. When false (after mouse scroll), viewport stays put.
    scroll_follows_cursor: bool,
}

impl Editor {
    /// Create new empty editor with default configuration
    pub fn new() -> Self {
        Self::with_config(EditorConfig::default())
    }

    /// Create new empty editor with specified configuration
    pub fn with_config(config: EditorConfig) -> Self {
        let mut file_state = FileState::new();
        file_state.initial_directory = config.initial_directory.clone();

        Self {
            config,
            buffer: TextBuffer::new(),
            cursor: Cursor::new(),
            selection: None,
            viewport: Viewport::default(),
            file_state,
            search: SearchController::new(),
            git: GitIntegration::new(),
            render_cache: RenderingCache::new(),
            input: InputState::new(),
            lsp: LspState::new(),
            modal_request: None,
            config_update: None,
            status_message: None,
            scroll_follows_cursor: true,
        }
    }

    /// Check if smart word wrapping should be used
    ///
    /// Smart wrapping is enabled when:
    /// - File size is below the configured threshold
    ///
    /// Smart wrap works for both code files (with syntax) and plain text files.
    fn should_use_smart_wrap(&self, config: &Config) -> bool {
        // Check file size threshold (for performance)
        let threshold_bytes = config.editor.large_file_threshold_mb * constants::MEGABYTE;
        if self.file_state.size > threshold_bytes {
            return false;
        }

        true
    }

    /// Get file path
    pub fn file_path(&self) -> Option<&std::path::Path> {
        self.buffer.file_path()
    }

    /// Get cached git repository root (returns None if not yet cached)
    pub fn cached_repo_root(&self) -> Option<Option<&PathBuf>> {
        self.git.cached_repo_root.as_ref().map(|opt| opt.as_ref())
    }

    /// Get or compute git repository root for this file
    /// Returns Some(path) if in a git repo, None otherwise
    pub fn get_or_compute_repo_root(&mut self) -> Option<&PathBuf> {
        if self.git.cached_repo_root.is_none() {
            // Compute and cache
            let repo_root = self.file_path().and_then(termide_git::find_repo_root);
            self.git.cached_repo_root = Some(repo_root);
        }
        self.git
            .cached_repo_root
            .as_ref()
            .and_then(|opt| opt.as_ref())
    }

    /// Get unsaved buffer filename (if this is a temporary unsaved buffer)
    pub fn unsaved_buffer_file(&self) -> Option<&str> {
        self.file_state.unsaved_buffer_file.as_deref()
    }

    /// Open file with specified configuration
    pub fn open_file_with_config(path: PathBuf, mut config: EditorConfig) -> Result<Self> {
        // Check file size before loading and get modification time
        let metadata = file_io::check_file_metadata(&path)?;
        let file_size = metadata.size;
        let file_mtime = metadata.mtime;

        let buffer = TextBuffer::from_file(&path)?;

        // Check file access rights for auto-detection of read-only
        if file_io::is_file_readonly(&path) {
            log::warn!("File detected as read-only: {}", path.display());
            config.read_only = true;
        }

        // Create file state
        let file_state = FileState::from_path(&path, file_mtime, file_size);

        // Create rendering cache and set syntax by file extension
        let mut render_cache = RenderingCache::new();
        if config.syntax_highlighting {
            render_cache.highlight.set_syntax_from_path(&path);
        }

        // Initialize git integration
        let mut git = GitIntegration::new();
        let mut cache = GitDiffCache::new(path.clone());
        match cache.update() {
            Ok(()) => {
                log::debug!(
                    "Editor: GitDiffCache initialized for {:?}, has {} statuses",
                    path,
                    cache.line_status_count()
                );
                git.diff_cache = Some(cache);
            }
            Err(e) => {
                log::warn!("Editor: GitDiffCache update failed for {:?}: {}", path, e);
            }
        }

        Ok(Self {
            config,
            buffer,
            cursor: Cursor::new(),
            selection: None,
            viewport: Viewport::default(),
            file_state,
            search: SearchController::new(),
            git,
            render_cache,
            input: InputState::new(),
            lsp: LspState::new(),
            modal_request: None,
            config_update: None,
            status_message: None,
            scroll_follows_cursor: true,
        })
    }

    /// Create editor with text (for displaying help, etc.)
    pub fn from_text(content: &str, title: String) -> Self {
        use ropey::Rope;

        // Create buffer directly through Rope
        let rope = Rope::from_str(content);

        let mut file_state = FileState::new();
        file_state.title = title;

        Self {
            config: EditorConfig::view_only(),
            buffer: TextBuffer::from_rope(rope),
            cursor: Cursor::new(),
            selection: None,
            viewport: Viewport::default(),
            file_state,
            search: SearchController::new(),
            git: GitIntegration::new(),
            render_cache: RenderingCache::new(),
            input: InputState::new(),
            lsp: LspState::new(),
            modal_request: None,
            config_update: None,
            status_message: None,
            scroll_follows_cursor: true,
        }
    }

    // =========================================================================
    // LSP Integration
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

    // === LSP Completion methods ===

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

    /// Save file
    /// Returns error if file was modified externally (use force_save() to override)
    pub fn save(&mut self) -> Result<()> {
        // Check for external modification conflict
        if self.file_state.external_change_detected {
            return Err(anyhow::anyhow!(
                "File was modified on disk. Use force save (Ctrl+Shift+S) to overwrite or reload (Ctrl+Shift+R) to discard changes."
            ));
        }

        // Check if this is a config file
        if let Some(path) = self.buffer.file_path().map(|p| p.to_path_buf()) {
            if Config::is_config_file(&path) {
                let path_str = path.display().to_string();
                // Validate config before saving
                let content = self.buffer.to_string();
                match Config::validate_content(&content) {
                    Ok(new_config) => {
                        // Save and set config update flag
                        self.buffer.save()?;
                        log::info!("Config file saved: {}", path_str);
                        self.config_update = Some(new_config);
                        // Update file modification time after successful save
                        self.file_state.mtime = file_io::get_file_mtime(&path);
                        self.file_state.external_change_detected = false;
                    }
                    Err(e) => {
                        log::error!("Save failed - config validation error: {}", e);
                        return Err(anyhow::anyhow!("Invalid config: {}", e));
                    }
                }
                return Ok(());
            }
        }

        self.buffer.save()?;

        if let Some(path) = self.buffer.file_path() {
            log::info!("File saved: {}", path.display());
            // Update file modification time after successful save
            self.file_state.mtime = file_io::get_file_mtime(path);
            self.file_state.external_change_detected = false;
        }

        // Update git diff after successful save
        self.update_git_diff();

        Ok(())
    }

    /// Insert text at the beginning of the buffer (for restoring unsaved buffers)
    pub fn insert_text(&mut self, text: &str) -> Result<()> {
        let cursor_at_start = Cursor::new();
        self.cursor = self.buffer.insert(&cursor_at_start, text)?;
        Ok(())
    }

    /// Set the unsaved buffer filename (for session restoration)
    pub fn set_unsaved_buffer_file(&mut self, filename: Option<String>) {
        self.file_state.unsaved_buffer_file = filename;
    }

    /// Update git diff cache for this file (async - non-blocking)
    ///
    /// Spawns a background thread to load original content from HEAD.
    /// The result will be applied on next tick via check_git_diff_receiver().
    pub fn update_git_diff(&mut self) {
        // Clone file path to avoid borrow conflict with git_diff_cache
        let file_path = self.file_path().map(|p| p.to_path_buf());
        if let Some(rx) = git::update_git_diff_async(&mut self.git.diff_cache, file_path.as_deref())
        {
            self.git.diff_receiver = Some(rx);
        }
    }

    /// Check and apply async git diff result if ready (called on each tick)
    ///
    /// Returns true if result was applied and needs_redraw should be set.
    pub fn check_git_diff_receiver(&mut self) -> bool {
        git::check_git_diff_receiver(&mut self.git.diff_receiver, &mut self.git.diff_cache)
    }

    /// Schedule git diff update with debounce (300ms delay)
    pub fn schedule_git_diff_update(&mut self) {
        if let Some(instant) = git::schedule_git_diff_update(&self.git.diff_cache) {
            self.git.update_pending = Some(instant);
        }
    }

    /// Check and apply pending git diff update if debounce time has passed
    pub fn check_pending_git_diff_update(&mut self) {
        let (updated, new_pending) = git::check_pending_git_diff_update(
            self.git.update_pending,
            &mut self.git.diff_cache,
            &self.buffer,
        );
        if updated {
            self.git.update_pending = new_pending;
        }
    }

    /// Check if the file was modified externally (outside of this editor)
    pub fn check_external_modification(&mut self) {
        if let Some(file_path) = self.buffer.file_path() {
            if file_io::was_modified_externally(file_path, self.file_state.mtime) {
                self.file_state.external_change_detected = true;
            }
        }
    }

    /// Check if external modification was detected
    pub fn has_external_change(&self) -> bool {
        self.file_state.external_change_detected
    }

    /// Check if buffer has unsaved modifications
    pub fn buffer_is_modified(&self) -> bool {
        self.buffer.is_modified()
    }

    /// Reload file from disk (discards local changes)
    pub fn reload_from_disk(&mut self) -> Result<()> {
        if let Some(path) = self.buffer.file_path().map(|p| p.to_path_buf()) {
            // Re-read the file
            self.buffer = TextBuffer::from_file(&path)?;

            // Update modification time
            self.file_state.mtime = file_io::get_file_mtime(&path);
            self.file_state.external_change_detected = false;

            // Reset cursor and selection
            self.cursor = Cursor::new();
            self.selection = None;

            // Update git diff
            self.update_git_diff();

            log::info!("File reloaded from disk: {}", path.display());
        }
        Ok(())
    }

    /// Force save (ignore external changes)
    pub fn force_save(&mut self) -> Result<()> {
        self.file_state.external_change_detected = false;
        self.save()
    }

    /// Get updated config (if config file was saved)
    pub fn take_config_update(&mut self) -> Option<Config> {
        self.config_update.take()
    }

    /// Save file as (Save As)
    pub fn save_file_as(&mut self, path: PathBuf) -> Result<()> {
        self.buffer.save_to(&path)?;
        log::info!("File saved as: {}", path.display());

        // Update title
        self.file_state.title = file_io::path_to_title(&path);

        Ok(())
    }

    /// Check if file has path (not unnamed)
    pub fn has_file_path(&self) -> bool {
        self.buffer.file_path().is_some()
    }

    /// Get editor information for status bar
    pub fn get_editor_info(&self) -> EditorInfo {
        // Determine file type by current syntax
        let file_type = self
            .render_cache
            .highlight
            .current_syntax()
            .map(Self::format_language_name)
            .unwrap_or("Plain Text")
            .to_string();

        EditorInfo {
            line: self.cursor.line + 1,     // 1-based
            column: self.cursor.column + 1, // 1-based
            tab_size: self.config.tab_size,
            encoding: "UTF-8".to_string(),
            file_type,
            read_only: self.config.read_only,
            syntax_highlighting: self.config.syntax_highlighting,
        }
    }

    /// Get disk space information for the file's storage device.
    pub fn get_disk_space_info(&self) -> Option<termide_system_monitor::DiskSpaceInfo> {
        self.file_path()
            .and_then(termide_system_monitor::get_disk_space_info)
    }

    // ===== LogViewer support methods =====

    /// Get immutable reference to buffer.
    pub fn buffer(&self) -> &TextBuffer {
        &self.buffer
    }

    /// Get mutable reference to buffer.
    pub fn buffer_mut(&mut self) -> &mut TextBuffer {
        &mut self.buffer
    }

    /// Get immutable reference to viewport.
    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    /// Get mutable reference to viewport.
    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.viewport
    }

    /// Set cursor to specific line (for log viewer scroll-to-end).
    pub fn set_cursor_line(&mut self, line: usize) {
        self.cursor.line = line.min(self.buffer.line_count().saturating_sub(1));
        self.cursor.column = 0;
    }

    /// Render with custom highlighter (for LogViewer).
    pub fn render_with_highlighter<H: termide_highlight::LineHighlighter>(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        config: &Config,
        highlighter: &mut H,
    ) {
        // Update viewport size
        let (content_width, content_height) =
            rendering::calculate_content_dimensions(area.width, area.height);

        self.render_cache.content_width = if self.config.word_wrap {
            content_width
        } else {
            0
        };
        self.render_cache.use_smart_wrap = false;

        self.viewport.resize(content_width, content_height);

        let use_smart_wrap = if self.config.word_wrap && content_width > 0 {
            self.should_use_smart_wrap(config)
        } else {
            false
        };
        self.render_cache.use_smart_wrap = use_smart_wrap;

        let virtual_lines_total = self.virtual_line_count(config);
        self.render_cache.virtual_line_count = virtual_lines_total;

        // Ensure cursor is visible (only when viewport follows cursor)
        if self.scroll_follows_cursor {
            if self.config.word_wrap && content_width > 0 {
                self.ensure_cursor_visible_word_wrap(content_height);
            } else {
                self.viewport
                    .ensure_cursor_visible(&self.cursor, virtual_lines_total);
            }
        }

        // Render with custom highlighter
        rendering::render_editor_content(
            buf,
            area,
            &self.buffer,
            &self.viewport,
            &self.cursor,
            &self.git.diff_cache,
            self.config.syntax_highlighting,
            highlighter,
            &self.search.state,
            &self.selection,
            theme,
            config.editor.show_git_diff,
            self.config.word_wrap,
            use_smart_wrap,
            content_width,
            content_height,
        );
    }

    /// Check if visual movement should be used (word wrap enabled and width cached).
    fn should_use_visual_movement(&self) -> bool {
        self.config.word_wrap && self.render_cache.content_width > 0
    }

    /// Ensure preferred column is set for vertical navigation.
    ///
    /// Sets preferred_column to visual offset within current visual row if not already set.
    /// Used by visual movement methods to maintain column across wrapped lines.
    fn ensure_preferred_column(&mut self) {
        if self.input.preferred_column.is_none() {
            // Calculate visual offset (position within current visual row)
            let visual_offset = if self.render_cache.content_width > 0 {
                if let Some(line_text) = self.buffer.line(self.cursor.line) {
                    use unicode_segmentation::UnicodeSegmentation;
                    let line_text = line_text.trim_end_matches('\n');
                    let line_len = line_text.graphemes(true).count();
                    let cursor_col = self.cursor.column.min(line_len);
                    let (_visual_rows, wrap_points) = word_wrap::get_line_wrap_points(
                        line_text,
                        self.render_cache.content_width,
                        self.render_cache.use_smart_wrap,
                    );
                    let current_visual_row =
                        wrap_points.iter().filter(|&&wp| wp <= cursor_col).count();
                    let visual_row_start = if current_visual_row == 0 {
                        0
                    } else if current_visual_row - 1 < wrap_points.len() {
                        wrap_points[current_visual_row - 1]
                    } else {
                        0
                    };
                    cursor_col.saturating_sub(visual_row_start)
                } else {
                    self.cursor.column
                }
            } else {
                self.cursor.column
            };
            self.input.preferred_column = Some(visual_offset);
        }
    }

    /// Convert syntax language name to human-readable display name.
    fn format_language_name(syntax_name: &str) -> &str {
        match syntax_name {
            "rust" => "Rust",
            "python" => "Python",
            "go" => "Go",
            "javascript" => "JavaScript",
            "typescript" => "TypeScript",
            "tsx" => "TSX",
            "c" => "C",
            "cpp" => "C++",
            "java" => "Java",
            "ruby" => "Ruby",
            "html" => "HTML",
            "css" => "CSS",
            "json" => "JSON",
            "toml" => "TOML",
            "yaml" => "YAML",
            "bash" => "Bash",
            "markdown" => "Markdown",
            _ => syntax_name,
        }
    }

    /// Move cursor up
    pub(crate) fn move_cursor_up(&mut self) {
        let maintain_preferred = cursor::physical::move_up(&mut self.cursor);
        if !maintain_preferred {
            self.input.preferred_column = None;
        }
        self.clamp_cursor();
    }

    /// Move cursor down
    pub(crate) fn move_cursor_down(&mut self) {
        let maintain_preferred = cursor::physical::move_down(&mut self.cursor, &self.buffer);
        if !maintain_preferred {
            self.input.preferred_column = None;
        }
        self.clamp_cursor();
    }

    /// Move cursor up by one visual line (accounting for word wrap)
    pub(crate) fn move_cursor_up_visual(&mut self) {
        if self.render_cache.content_width == 0 {
            self.move_cursor_up();
            return;
        }

        self.ensure_preferred_column();

        if let Some(new_cursor) = cursor::visual::move_up(
            &self.cursor,
            &self.buffer,
            self.input.preferred_column,
            self.render_cache.content_width,
            self.render_cache.use_smart_wrap,
        ) {
            self.cursor = new_cursor;
        }

        self.clamp_cursor();
    }

    /// Move cursor down by one visual line (accounting for word wrap)
    pub(crate) fn move_cursor_down_visual(&mut self) {
        if self.render_cache.content_width == 0 {
            self.move_cursor_down();
            return;
        }

        self.ensure_preferred_column();

        if let Some(new_cursor) = cursor::visual::move_down(
            &self.cursor,
            &self.buffer,
            self.input.preferred_column,
            self.render_cache.content_width,
            self.render_cache.use_smart_wrap,
        ) {
            self.cursor = new_cursor;
        }

        self.clamp_cursor();
    }

    /// Move cursor left
    pub(crate) fn move_cursor_left(&mut self) {
        let maintain_preferred = cursor::physical::move_left(&mut self.cursor, &self.buffer);
        if !maintain_preferred {
            self.input.preferred_column = None;
        }
    }

    /// Move cursor right
    pub(crate) fn move_cursor_right(&mut self) {
        let maintain_preferred = cursor::physical::move_right(&mut self.cursor, &self.buffer);
        if !maintain_preferred {
            self.input.preferred_column = None;
        }
        self.clamp_cursor();
    }

    /// Move cursor to start of line
    pub(crate) fn move_to_line_start(&mut self) {
        let maintain_preferred = cursor::physical::move_to_line_start(&mut self.cursor);
        if !maintain_preferred {
            self.input.preferred_column = None;
        }
    }

    /// Move cursor to end of line
    pub(crate) fn move_to_line_end(&mut self) {
        let maintain_preferred = cursor::physical::move_to_line_end(&mut self.cursor, &self.buffer);
        if !maintain_preferred {
            self.input.preferred_column = None;
        }
    }

    /// Move cursor to start of visual line (for wrapped lines)
    pub(crate) fn move_to_visual_line_start(&mut self) {
        // Reset preferred column on horizontal movement
        self.input.preferred_column = None;

        if self.render_cache.content_width == 0 {
            // No word wrap - fall back to physical line start
            self.move_to_line_start();
            return;
        }

        self.cursor.column = cursor::visual::move_to_visual_line_start(
            &self.cursor,
            &self.buffer,
            self.render_cache.content_width,
            self.render_cache.use_smart_wrap,
        );
    }

    /// Move cursor to end of visual line (for wrapped lines)
    pub(crate) fn move_to_visual_line_end(&mut self) {
        // Reset preferred column on horizontal movement
        self.input.preferred_column = None;

        if self.render_cache.content_width == 0 {
            // No word wrap - fall back to physical line end
            self.move_to_line_end();
            return;
        }

        self.cursor.column = cursor::visual::move_to_visual_line_end(
            &self.cursor,
            &self.buffer,
            self.render_cache.content_width,
            self.render_cache.use_smart_wrap,
        );
    }

    /// Move cursor page up
    pub(crate) fn page_up(&mut self) {
        let page_size = self.viewport.height;
        let (should_scroll, scroll_amount) = cursor::jump::page_up(&mut self.cursor, page_size);
        self.clamp_cursor();
        if should_scroll {
            self.viewport.scroll_up(scroll_amount);
        }
    }

    /// Move cursor page down
    pub(crate) fn page_down(&mut self) {
        let page_size = self.viewport.height;
        let (should_scroll, scroll_amount) =
            cursor::jump::page_down(&mut self.cursor, &self.buffer, page_size);
        self.clamp_cursor();
        if should_scroll {
            // Use cached virtual line count for viewport scroll (accounts for deletion markers)
            self.viewport
                .scroll_down(scroll_amount, self.render_cache.virtual_line_count);
        }
    }

    /// Move cursor page up by visual lines (accounting for word wrap)
    pub(crate) fn page_up_visual(&mut self) {
        if self.render_cache.content_width == 0 {
            // No word wrap - fall back to physical line movement
            self.page_up();
            return;
        }

        self.ensure_preferred_column();

        let page_size = self.viewport.height;
        self.cursor = cursor::visual::page_up(
            &self.cursor,
            &self.buffer,
            self.input.preferred_column,
            self.render_cache.content_width,
            self.render_cache.use_smart_wrap,
            page_size,
        );

        // Don't manually scroll viewport - let ensure_cursor_visible() handle it during rendering
        // This is correct because the viewport needs to track visual rows, not buffer lines
    }

    /// Move cursor page down by visual lines (accounting for word wrap)
    pub(crate) fn page_down_visual(&mut self) {
        if self.render_cache.content_width == 0 {
            // No word wrap - fall back to physical line movement
            self.page_down();
            return;
        }

        self.ensure_preferred_column();

        let page_size = self.viewport.height;
        self.cursor = cursor::visual::page_down(
            &self.cursor,
            &self.buffer,
            self.input.preferred_column,
            self.render_cache.content_width,
            self.render_cache.use_smart_wrap,
            page_size,
        );

        // Don't manually scroll viewport - let ensure_cursor_visible() handle it during rendering
        // This is correct because the viewport needs to track visual rows, not buffer lines
    }

    /// Move cursor to start of document
    pub(crate) fn move_to_document_start(&mut self) {
        let (new_cursor, should_scroll) = cursor::physical::move_to_document_start();
        self.cursor = new_cursor;
        if should_scroll {
            self.viewport.scroll_to_top();
        }
    }

    /// Move cursor to end of document
    pub(crate) fn move_to_document_end(&mut self) {
        let (new_cursor, should_scroll) = cursor::physical::move_to_document_end(&self.buffer);
        self.cursor = new_cursor;
        if should_scroll {
            // Use cached virtual line count for viewport scroll
            self.viewport
                .scroll_to_bottom(self.render_cache.virtual_line_count);
        }
    }

    /// Select all
    pub(crate) fn select_all(&mut self) {
        let (new_selection, new_cursor) = selection::select_all(&self.buffer);
        self.selection = Some(new_selection);
        self.cursor = new_cursor;
    }

    /// Start new selection or continue existing
    fn start_or_extend_selection(&mut self) {
        if let Some(new_selection) =
            selection::start_or_extend_selection(self.selection.as_ref(), self.cursor)
        {
            self.selection = Some(new_selection);
        }
    }

    /// Update active point of selection (after cursor movement)
    fn update_selection_active(&mut self) {
        selection::update_selection_active(&mut self.selection, self.cursor);
    }

    /// Get selected text
    fn get_selected_text(&self) -> Option<String> {
        selection::get_selected_text(&self.buffer, self.selection.as_ref())
    }

    /// Delete selected text
    fn delete_selection(&mut self) -> Result<()> {
        if let Some(new_cursor) =
            selection::delete_selection(&mut self.buffer, self.selection.as_ref())?
        {
            self.cursor = new_cursor;
            self.selection = None;
            self.input.preferred_column = None; // Reset preferred column on text edit

            // Invalidate highlighting cache
            selection::invalidate_cache_after_deletion(
                &mut self.render_cache.highlight,
                new_cursor.line,
                self.buffer.line_count(),
            );

            // Schedule git diff update
            self.schedule_git_diff_update();
        }
        Ok(())
    }

    /// Copy selected text to clipboard
    pub(crate) fn copy_to_clipboard(&mut self) -> Result<()> {
        let selected_text = self.get_selected_text();
        let result = clipboard::copy_to_clipboard(selected_text);
        self.status_message = Some(result.status_message);
        Ok(())
    }

    /// Cut selected text to clipboard
    pub(crate) fn cut_to_clipboard(&mut self) -> Result<()> {
        let selected_text = self.get_selected_text();
        let (result, should_delete) = clipboard::cut_to_clipboard(selected_text);
        self.status_message = Some(result.status_message);

        if should_delete {
            self.delete_selection()?;
        }
        Ok(())
    }

    /// Paste from clipboard
    pub fn paste_from_clipboard(&mut self) -> Result<()> {
        // Close search mode when editing begins
        self.close_search();

        // Delete selected text before pasting
        self.delete_selection()?;

        // Paste from clipboard using clipboard module
        if let Some((new_cursor, start_line, is_multiline)) =
            clipboard::paste_from_clipboard(&mut self.buffer, &self.cursor)?
        {
            self.cursor = new_cursor;
            self.input.preferred_column = None; // Reset preferred column on text edit
            self.clamp_cursor();

            // Invalidate highlighting cache and schedule git update
            self.invalidate_cache_after_edit(start_line, is_multiline);
        }
        Ok(())
    }

    /// Paste text directly (from bracketed paste event)
    pub fn paste_text(&mut self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }

        // Close search mode when editing begins
        self.close_search();

        // Delete selected text before pasting
        self.delete_selection()?;

        // Insert text at cursor position
        let start_line = self.cursor.line;
        let new_cursor = self.buffer.insert(&self.cursor, text)?;
        let is_multiline = text.contains('\n');

        self.cursor = new_cursor;
        self.input.preferred_column = None;
        self.clamp_cursor();

        // Invalidate highlighting cache and schedule git update
        self.invalidate_cache_after_edit(start_line, is_multiline);

        Ok(())
    }

    /// Duplicate current line or selected lines
    pub(crate) fn duplicate_line(&mut self) -> Result<()> {
        let result =
            text_editing::duplicate_line(&mut self.buffer, &self.cursor, self.selection.as_ref())?;

        self.cursor = result.new_cursor;
        self.input.preferred_column = None; // Reset preferred column on text edit
        self.clamp_cursor();

        // Clear selection
        self.selection = None;

        // Invalidate highlighting cache and schedule git update
        self.invalidate_cache_after_edit(result.start_line, result.is_multiline);

        Ok(())
    }

    /// Clamp cursor position to valid values
    fn clamp_cursor(&mut self) {
        cursor::physical::clamp_cursor(&mut self.cursor, &self.buffer);
    }

    /// Insert character at cursor position
    pub(crate) fn insert_char(&mut self, ch: char) -> Result<()> {
        // Close search mode when editing begins
        self.close_search();

        // Delete selected text before insertion
        self.delete_selection()?;

        let result = text_editing::insert_char(&mut self.buffer, &self.cursor, ch)?;
        self.cursor = result.new_cursor;
        self.input.preferred_column = None;
        self.clamp_cursor();

        // Track inserted character for auto-completion
        self.lsp.last_inserted_char = Some(ch);

        // Invalidate highlighting cache and schedule git update
        self.invalidate_cache_after_edit(result.start_line, result.is_multiline);

        Ok(())
    }

    /// Insert tab (spaces based on tab_size config)
    pub(crate) fn insert_tab(&mut self) -> Result<()> {
        // Close search mode when editing begins
        self.close_search();

        // Delete selected text before insertion
        self.delete_selection()?;

        // Insert tab_size spaces
        let spaces = " ".repeat(self.config.tab_size);
        for ch in spaces.chars() {
            let result = text_editing::insert_char(&mut self.buffer, &self.cursor, ch)?;
            self.cursor = result.new_cursor;
        }

        self.input.preferred_column = None;
        self.clamp_cursor();

        // Invalidate highlighting cache and schedule git update
        self.invalidate_cache_after_edit(self.cursor.line, false);

        Ok(())
    }

    /// Indent selected lines (or current line if no selection)
    pub(crate) fn indent_lines(&mut self) -> Result<()> {
        // Close search mode when editing begins
        self.close_search();

        let tab_size = self.config.tab_size;
        let indent = " ".repeat(tab_size);

        // Get line range from selection or current cursor
        let (start_line, end_line) = if let Some(ref sel) = self.selection {
            (sel.start().line, sel.end().line)
        } else {
            (self.cursor.line, self.cursor.line)
        };

        // Insert indent at the beginning of each line (iterate in reverse to avoid index shifts)
        for line_idx in (start_line..=end_line).rev() {
            let cursor_at_start = Cursor::at(line_idx, 0);
            self.buffer.insert(&cursor_at_start, &indent)?;
        }

        // Update cursor position
        self.cursor.column += tab_size;

        // Update selection positions if present
        if let Some(ref mut sel) = self.selection {
            sel.anchor.column += tab_size;
            sel.active.column += tab_size;
        }

        self.input.preferred_column = None;
        self.clamp_cursor();

        // Invalidate highlighting cache and schedule git update
        self.invalidate_cache_after_edit(start_line, true);
        self.schedule_git_diff_update();

        Ok(())
    }

    /// Unindent selected lines (or current line if no selection)
    pub(crate) fn unindent_lines(&mut self) -> Result<()> {
        // Close search mode when editing begins
        self.close_search();

        let tab_size = self.config.tab_size;

        // Get line range from selection or current cursor
        let (start_line, end_line) = if let Some(ref sel) = self.selection {
            (sel.start().line, sel.end().line)
        } else {
            (self.cursor.line, self.cursor.line)
        };

        // Track how many spaces were removed from each line for cursor adjustment
        let mut cursor_line_spaces_removed = 0;
        let mut anchor_line_spaces_removed = 0;
        let mut active_line_spaces_removed = 0;

        // Remove up to tab_size spaces from the beginning of each line
        for line_idx in (start_line..=end_line).rev() {
            if let Some(line) = self.buffer.line(line_idx) {
                // Count leading spaces (up to tab_size)
                let spaces_to_remove = line
                    .chars()
                    .take(tab_size)
                    .take_while(|c| *c == ' ')
                    .count();

                if spaces_to_remove > 0 {
                    let start = Cursor::at(line_idx, 0);
                    let end = Cursor::at(line_idx, spaces_to_remove);
                    self.buffer.delete_range(&start, &end)?;

                    // Track spaces removed for cursor/selection adjustment
                    if line_idx == self.cursor.line {
                        cursor_line_spaces_removed = spaces_to_remove;
                    }
                    if let Some(ref sel) = self.selection {
                        if line_idx == sel.anchor.line {
                            anchor_line_spaces_removed = spaces_to_remove;
                        }
                        if line_idx == sel.active.line {
                            active_line_spaces_removed = spaces_to_remove;
                        }
                    }
                }
            }
        }

        // Update cursor position (subtract removed spaces, but don't go below 0)
        self.cursor.column = self
            .cursor
            .column
            .saturating_sub(cursor_line_spaces_removed);

        // Update selection positions if present
        if let Some(ref mut sel) = self.selection {
            sel.anchor.column = sel.anchor.column.saturating_sub(anchor_line_spaces_removed);
            sel.active.column = sel.active.column.saturating_sub(active_line_spaces_removed);
        }

        self.input.preferred_column = None;
        self.clamp_cursor();

        // Invalidate highlighting cache and schedule git update
        self.invalidate_cache_after_edit(start_line, true);
        self.schedule_git_diff_update();

        Ok(())
    }

    /// Insert newline
    pub(crate) fn insert_newline(&mut self) -> Result<()> {
        // Close search mode when editing begins
        self.close_search();

        // Delete selected text before insertion
        self.delete_selection()?;

        let result = text_editing::insert_newline(&mut self.buffer, &self.cursor)?;
        self.cursor = result.new_cursor;
        self.input.preferred_column = None; // Reset preferred column on text edit
        self.clamp_cursor();

        // Invalidate highlighting cache and schedule git update
        self.invalidate_cache_after_edit(result.start_line, result.is_multiline);

        Ok(())
    }

    /// Delete character (backspace)
    pub(crate) fn backspace(&mut self) -> Result<()> {
        if let Some(result) = text_editing::backspace(&mut self.buffer, &self.cursor)? {
            self.cursor = result.new_cursor;
            self.input.preferred_column = None; // Reset preferred column on text edit
            self.clamp_cursor();

            // Invalidate highlighting cache and schedule git update
            self.invalidate_cache_after_edit(result.start_line, result.is_multiline);
        }
        Ok(())
    }

    /// Delete character (delete)
    pub(crate) fn delete(&mut self) -> Result<()> {
        if let Some(result) = text_editing::delete_char(&mut self.buffer, &self.cursor)? {
            self.input.preferred_column = None; // Reset preferred column on text edit
                                                // Invalidate highlighting cache and schedule git update
            self.invalidate_cache_after_edit(result.start_line, result.is_multiline);
        }
        Ok(())
    }

    /// Ensure cursor is visible when word wrap is enabled.
    /// This is more complex than the standard ensure_cursor_visible because we need
    /// to work with visual rows, not physical lines.
    fn ensure_cursor_visible_word_wrap(&mut self, content_height: usize) {
        if content_height == 0 || self.render_cache.content_width == 0 {
            return;
        }

        // First, handle the case where cursor is above viewport (physical line check)
        if self.cursor.line < self.viewport.top_line {
            self.viewport.top_line = self.cursor.line;
        }

        // Calculate the visual row of the cursor relative to viewport.top_line
        let cursor_visual_row = word_wrap::calculate_visual_row_for_cursor(
            &self.buffer,
            self.cursor.line,
            self.cursor.column,
            self.viewport.top_line,
            self.render_cache.content_width,
            self.config.word_wrap,
            self.render_cache.use_smart_wrap,
        );

        // If cursor is below the visible area, scroll down
        if cursor_visual_row >= content_height {
            // We need to increase top_line until cursor fits in view
            // Iterate: increase top_line and recalculate cursor_visual_row
            while self.viewport.top_line < self.cursor.line {
                self.viewport.top_line += 1;

                let new_visual_row = word_wrap::calculate_visual_row_for_cursor(
                    &self.buffer,
                    self.cursor.line,
                    self.cursor.column,
                    self.viewport.top_line,
                    self.render_cache.content_width,
                    self.config.word_wrap,
                    self.render_cache.use_smart_wrap,
                );

                // Stop when cursor is at the bottom of viewport
                if new_visual_row < content_height {
                    break;
                }
            }

            // Edge case: cursor line itself is longer than viewport height
            // In this case, ensure the visual row containing cursor is visible
            if self.viewport.top_line == self.cursor.line {
                // The cursor is on a line that starts at top_line
                // But the cursor column might be on a wrapped visual row
                // We've already done what we can - the line is at the top
            }
        }

        // Also handle horizontal scroll for non-word-wrap scenarios
        // (word wrap shouldn't need horizontal scroll, but just in case)
        if self.cursor.column < self.viewport.left_column {
            self.viewport.left_column = self.cursor.column;
        } else if self.cursor.column >= self.viewport.right_column() {
            self.viewport.left_column = self.cursor.column.saturating_sub(self.viewport.width - 1);
        }
    }

    /// Get the total count of virtual lines (real buffer lines + deletion marker lines + word wrap)
    /// This is used for viewport calculations to account for deletion markers and word wrapping
    fn virtual_line_count(&self, config: &Config) -> usize {
        // If word wrap is enabled, count visual rows instead of buffer lines
        if self.should_use_visual_movement() {
            // Use calculate_total_visual_rows which accounts for word wrapping
            let total_visual_rows = word_wrap::calculate_total_visual_rows(
                &self.buffer,
                self.render_cache.content_width,
                self.config.word_wrap,
                self.render_cache.use_smart_wrap,
            );

            // Add deletion markers if git diff is shown (O(1) lookup)
            if config.editor.show_git_diff {
                if let Some(git_diff) = &self.git.diff_cache {
                    return total_visual_rows + git_diff.deletion_marker_count();
                }
            }

            return total_visual_rows;
        }

        // No word wrap - use old logic with buffer lines + deletion markers
        let buffer_line_count = self.buffer.line_count();
        let deletion_marker_count = if config.editor.show_git_diff {
            self.git
                .diff_cache
                .as_ref()
                .map(|cache| cache.deletion_marker_count())
                .unwrap_or(0)
        } else {
            0
        };

        buffer_line_count + deletion_marker_count
    }

    /// Render editor content
    fn render_content(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        config: &Config,
        is_focused: bool,
        border_right_x: Option<u16>,
    ) {
        // Update viewport size (subtract space for line numbers)
        let (content_width, content_height) =
            rendering::calculate_content_dimensions(area.width, area.height);

        // Cache content width for visual line navigation
        self.render_cache.content_width = if self.config.word_wrap {
            content_width
        } else {
            0 // Set to 0 when word wrap is disabled to trigger fallback behavior
        };

        // Initially set smart wrap to false (will be updated later if word wrap is enabled)
        self.render_cache.use_smart_wrap = false;

        self.viewport.resize(content_width, content_height);

        // Determine smart wrap setting early (needed for ensure_cursor_visible_word_wrap)
        let use_smart_wrap = if self.config.word_wrap && content_width > 0 {
            self.should_use_smart_wrap(config)
        } else {
            false
        };
        self.render_cache.use_smart_wrap = use_smart_wrap;

        // Compute and cache virtual line count for viewport calculations
        let virtual_lines_total = self.virtual_line_count(config);
        self.render_cache.virtual_line_count = virtual_lines_total;

        // Ensure cursor is visible (only when viewport follows cursor)
        if self.scroll_follows_cursor {
            if self.config.word_wrap && content_width > 0 {
                // Word wrap mode: use visual row-aware scrolling
                self.ensure_cursor_visible_word_wrap(content_height);
            } else {
                // Standard mode: use physical line scrolling
                self.viewport
                    .ensure_cursor_visible(&self.cursor, virtual_lines_total);
            }
        }

        // Delegate to rendering orchestrator
        rendering::render_editor_content(
            buf,
            area,
            &self.buffer,
            &self.viewport,
            &self.cursor,
            &self.git.diff_cache,
            self.config.syntax_highlighting,
            &mut self.render_cache.highlight,
            &self.search.state,
            &self.selection,
            theme,
            config.editor.show_git_diff,
            self.config.word_wrap,
            use_smart_wrap,
            content_width,
            content_height,
        );

        // Render scrollbar on the right border
        if let Some(border_x) = border_right_x {
            let theme_colors = termide_core::ThemeColors::from(theme);
            ScrollBar::render(
                buf,
                border_x,
                area.y,
                area.height,
                self.viewport.top_line,
                content_height,
                virtual_lines_total,
                &theme_colors,
                is_focused,
            );
        }

        // Render completion popup if active
        if let Some(ref popup) = self.lsp.completion_popup {
            use unicode_width::UnicodeWidthStr;

            // Only render if cursor is in visible area
            if self.cursor.line >= self.viewport.top_line
                && self.cursor.line < self.viewport.top_line + content_height
            {
                // Calculate cursor screen position
                let line_number_width = rendering::LINE_NUMBER_WIDTH as u16;
                let content_x = area.x + 1 + line_number_width; // +1 for border

                // Calculate cursor X position within the line
                // Calculate display width up to cursor column
                let cursor_screen_col: usize = self
                    .buffer
                    .line(self.cursor.line)
                    .map(|line| {
                        line.chars()
                            .take(self.cursor.column)
                            .map(|c| c.to_string().width())
                            .sum()
                    })
                    .unwrap_or(0);

                let cursor_x = content_x + cursor_screen_col as u16;
                let cursor_y = area.y + 1 + (self.cursor.line - self.viewport.top_line) as u16;

                // Render popup within editor area only and store rect for mouse hit testing
                self.lsp.popup_rect = popup.render(buf, area, cursor_x, cursor_y, theme);
            } else {
                self.lsp.popup_rect = None;
            }
        } else {
            self.lsp.popup_rect = None;
        }
    }

    /// Start search
    pub fn start_search(&mut self, query: String, case_sensitive: bool) {
        let mut search_state = SearchState::new(query, case_sensitive);

        // Perform search throughout document
        self.perform_search(&mut search_state);

        // Find closest match to current cursor
        search_state.find_closest_match(&self.cursor);

        // Move cursor to end of match and create selection
        if let Some(match_cursor) = search_state.current_match_cursor() {
            let query_len = search_state.query.chars().count();
            let (selection, end_cursor) = search::get_match_selection(match_cursor, query_len);
            self.cursor = end_cursor;
            self.selection = Some(selection);
        }

        self.search.state = Some(search_state);
    }

    /// Perform search in document
    fn perform_search(&self, search_state: &mut SearchState) {
        search::perform_search(&self.buffer, search_state);
    }

    /// Go to next match
    pub fn search_next(&mut self) {
        if let Some(ref mut search_state) = self.search.state {
            search_state.next_match();
            if let Some(match_cursor) = search_state.current_match_cursor() {
                let query_len = search_state.query.chars().count();
                let (selection, end_cursor) = search::get_match_selection(match_cursor, query_len);
                self.cursor = end_cursor;
                self.selection = Some(selection);
            }
        }
    }

    /// Go to previous match
    pub fn search_prev(&mut self) {
        if let Some(ref mut search_state) = self.search.state {
            search_state.prev_match();
            if let Some(match_cursor) = search_state.current_match_cursor() {
                let query_len = search_state.query.chars().count();
                let (selection, end_cursor) = search::get_match_selection(match_cursor, query_len);
                self.cursor = end_cursor;
                self.selection = Some(selection);
            }
        }
    }

    /// Close search
    pub fn close_search(&mut self) {
        // Preserve the last search/replace query before closing
        if let Some(ref search) = self.search.state {
            if let Some(ref replace_with) = search.replace_with {
                // This is a replace operation - save to replace history
                self.search.last_replace_find = Some(search.query.clone());
                self.search.last_replace_with = Some(replace_with.clone());
            } else {
                // This is a search operation - save to search history
                self.search.last_query = Some(search.query.clone());
            }
        }
        self.search.state = None;
    }

    /// Get search match information (current index, total count)
    pub fn get_search_match_info(&self) -> Option<(usize, usize)> {
        if let Some(ref search) = self.search.state {
            let current = search.current_match.unwrap_or(0);
            let total = search.matches.len();
            Some((current, total))
        } else {
            None
        }
    }

    /// Start search with replace
    pub fn start_replace(&mut self, query: String, replace_with: String, case_sensitive: bool) {
        let mut search_state = SearchState::new_with_replace(query, replace_with, case_sensitive);

        // Perform search throughout document
        self.perform_search(&mut search_state);

        // Find closest match to current cursor
        search_state.find_closest_match(&self.cursor);

        // Move cursor to first match and create selection
        if let Some(match_cursor) = search_state.current_match_cursor() {
            let query_len = search_state.query.chars().count();
            let (selection, end_cursor) = search::get_match_selection(match_cursor, query_len);
            self.cursor = end_cursor;
            self.selection = Some(selection);
        }

        self.search.state = Some(search_state);
    }

    /// Update replace_with value in active search state without rebuilding search
    pub fn update_replace_with(&mut self, replace_with: String) {
        if let Some(ref mut search) = self.search.state {
            search.replace_with = Some(replace_with);
        }
    }

    /// Replace current match
    pub fn replace_current(&mut self) -> Result<()> {
        // Collect data from search_state
        let (match_cursor, replace_with, query_len) =
            if let Some(ref search_state) = self.search.state {
                if let (Some(replace_with), Some(idx)) =
                    (&search_state.replace_with, search_state.current_match)
                {
                    if let Some(match_cursor) = search_state.matches.get(idx).cloned() {
                        (match_cursor, replace_with.clone(), search_state.query.len())
                    } else {
                        return Ok(());
                    }
                } else {
                    return Ok(());
                }
            } else {
                return Ok(());
            };

        // Perform replacement
        let result =
            search::replace_at_position(&mut self.buffer, &match_cursor, query_len, &replace_with)?;
        self.cursor = result.new_cursor;

        // Invalidate highlighting cache for changed line
        self.render_cache
            .highlight
            .invalidate_line(result.start_line);

        // Update search_state
        if let Some(ref mut search_state) = self.search.state {
            if let Some(idx) = search_state.current_match {
                // Remove this match from list
                search_state.matches.remove(idx);

                // Update positions of remaining matches on the same line after replacement point
                search::update_match_positions_after_replace(
                    &mut search_state.matches,
                    &match_cursor,
                    query_len,
                    replace_with.len(),
                );

                // Update current match index
                if search_state.matches.is_empty() {
                    search_state.current_match = None;
                } else if idx >= search_state.matches.len() {
                    search_state.current_match = Some(search_state.matches.len() - 1);
                }

                // Move cursor to next match and create selection
                if let Some(match_cursor) = search_state.current_match_cursor() {
                    let query_len = search_state.query.chars().count();
                    let (selection, end_cursor) =
                        search::get_match_selection(match_cursor, query_len);
                    self.cursor = end_cursor;
                    self.selection = Some(selection);
                }
            }
        }

        // Schedule git diff update
        self.schedule_git_diff_update();

        Ok(())
    }

    /// Replace all matches
    pub fn replace_all(&mut self) -> Result<usize> {
        // Use take() instead of clone() to avoid allocation
        let Some(search_state) = self.search.state.take() else {
            return Ok(0);
        };

        let Some(replace_with) = &search_state.replace_with else {
            // Restore state if no replace_with
            self.search.state = Some(search_state);
            return Ok(0);
        };

        // Perform all replacements
        let count = search::replace_all_matches(
            &mut self.buffer,
            &search_state.matches,
            search_state.query.len(),
            replace_with,
        )?;

        // Invalidate highlighting cache for all affected lines
        for match_cursor in &search_state.matches {
            self.render_cache
                .highlight
                .invalidate_line(match_cursor.line);
        }

        // Schedule git diff update
        self.schedule_git_diff_update();

        Ok(count)
    }

    /// Prepare for navigation: close search, clear selection, and close completion popup.
    fn prepare_for_navigation(&mut self) {
        self.close_search();
        self.selection = None;
        // Close completion popup on cursor movement
        self.lsp.completion_popup = None;
    }

    /// Prepare for navigation with selection: close search, start/extend selection, and close completion popup.
    fn prepare_for_navigation_with_selection(&mut self) {
        self.close_search();
        self.start_or_extend_selection();
        // Close completion popup on cursor movement
        self.lsp.completion_popup = None;
    }

    /// Handle backspace/delete key with selection awareness.
    ///
    /// If selection exists and is not empty, deletes the selection.
    /// Otherwise, clears selection and performs the specified delete operation.
    pub(crate) fn handle_delete_key<F>(&mut self, delete_fn: F) -> Result<()>
    where
        F: FnOnce(&mut Self) -> Result<()>,
    {
        self.close_search();

        if self
            .selection
            .as_ref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        {
            self.delete_selection()?;
        } else {
            self.selection = None;
            delete_fn(self)?;
        }
        Ok(())
    }

    /// Invalidate syntax highlighting cache after text edit and schedule git diff update.
    ///
    /// If the edit is multiline, invalidates all lines from start_line to end of buffer.
    /// Otherwise, invalidates only the single changed line.
    fn invalidate_cache_after_edit(&mut self, start_line: usize, is_multiline: bool) {
        if is_multiline {
            self.render_cache
                .highlight
                .invalidate_range(start_line, self.buffer.line_count());
        } else {
            self.render_cache.highlight.invalidate_line(start_line);
        }
        self.schedule_git_diff_update();
        // Mark for LSP notification
        self.mark_lsp_changed();
    }

    /// Handle undo/redo operation with unified logic.
    ///
    /// Performs the specified buffer operation (undo or redo), updates cursor position,
    /// invalidates cache, and schedules git diff update.
    pub(crate) fn handle_undo_redo<F>(&mut self, operation: F) -> Result<()>
    where
        F: FnOnce(&mut TextBuffer) -> Result<Option<Cursor>>,
    {
        self.close_search();

        if let Some(new_cursor) = operation(&mut self.buffer)? {
            self.cursor = new_cursor;
            self.clamp_cursor();
            // Invalidate entire highlighting cache after undo/redo
            self.render_cache
                .highlight
                .invalidate_range(0, self.buffer.line_count());
            // Schedule git diff update
            self.schedule_git_diff_update();
            // Mark for LSP notification
            self.mark_lsp_changed();
        }
        Ok(())
    }

    /// Open search modal, optionally restoring and executing previous query.
    ///
    /// If active search exists, restores its state. Otherwise, if a previous query
    /// exists and execute_search is true, executes it immediately.
    pub(crate) fn open_search_modal(&mut self, execute_search: bool) {
        let mut search_modal = SearchModal::new("");

        // Restore active search state if it exists
        if let Some(ref search_state) = self.search.state {
            search_modal.set_input(search_state.query.clone());
            if let Some((current, total)) = self.get_search_match_info() {
                search_modal.set_match_info(current, total);
            }
        }
        // If there's a saved query but no active search
        else if let Some(ref query) = self.search.last_query {
            search_modal.set_input(query.clone());

            if execute_search {
                // Execute search immediately
                self.start_search(query.clone(), false);

                // Update match info in modal
                if let Some((current, total)) = self.get_search_match_info() {
                    search_modal.set_match_info(current, total);
                }
            }
        }

        self.modal_request = Some((
            PendingAction::Search,
            ActiveModal::Search(Box::new(search_modal)),
        ));
    }

    /// Execute navigation with visual/physical mode selection.
    ///
    /// Prepares for navigation, then calls visual_fn if word wrap is enabled,
    /// otherwise calls physical_fn.
    pub(crate) fn navigate<FV, FP>(&mut self, visual_fn: FV, physical_fn: FP)
    where
        FV: FnOnce(&mut Self),
        FP: FnOnce(&mut Self),
    {
        self.prepare_for_navigation();
        if self.should_use_visual_movement() {
            visual_fn(self);
        } else {
            physical_fn(self);
        }
    }

    /// Execute navigation with selection, using visual/physical mode.
    ///
    /// Prepares for navigation with selection, calls visual_fn if word wrap enabled,
    /// otherwise calls physical_fn, then updates selection.
    pub(crate) fn navigate_with_selection<FV, FP>(&mut self, visual_fn: FV, physical_fn: FP)
    where
        FV: FnOnce(&mut Self),
        FP: FnOnce(&mut Self),
    {
        self.prepare_for_navigation_with_selection();
        if self.should_use_visual_movement() {
            visual_fn(self);
        } else {
            physical_fn(self);
        }
        self.update_selection_active();
    }

    /// Execute simple navigation (no visual/physical choice).
    ///
    /// Prepares for navigation and calls the movement function.
    /// Use for movements that don't have visual/physical variants (e.g., Left, Right).
    pub(crate) fn navigate_simple<F>(&mut self, movement_fn: F)
    where
        F: FnOnce(&mut Self),
    {
        self.prepare_for_navigation();
        movement_fn(self);
    }

    /// Execute simple navigation with selection (no visual/physical choice).
    ///
    /// Prepares for navigation with selection, calls movement function, then updates selection.
    /// Use for movements that don't have visual/physical variants (e.g., Shift+Left, Shift+Right).
    pub(crate) fn navigate_with_selection_simple<F>(&mut self, movement_fn: F)
    where
        F: FnOnce(&mut Self),
    {
        self.prepare_for_navigation_with_selection();
        movement_fn(self);
        self.update_selection_active();
    }

    /// Go to next search match, or open search modal if no active search.
    pub(crate) fn search_next_or_open(&mut self) {
        if self.search.state.is_some() {
            self.search_next();
        } else {
            self.open_search_modal(true);
        }
    }

    /// Go to previous search match, or open search modal if no active search.
    pub(crate) fn search_prev_or_open(&mut self) {
        if self.search.state.is_some() {
            self.search_prev();
        } else {
            self.open_search_modal(true);
        }
    }

    /// Handle save command - either save to existing path or open "Save As" modal
    pub(crate) fn handle_save(&mut self) -> Result<()> {
        if self.buffer.file_path().is_some() {
            // File has path - save normally
            self.save()
        } else {
            // File has no path - open "Save As" dialog
            self.handle_save_as()
        }
    }

    /// Open "Save As" modal for saving file with a new name
    pub(crate) fn handle_save_as(&mut self) -> Result<()> {
        // Priority: initial_directory > file_path parent > CWD > home
        let directory = self
            .file_state
            .initial_directory
            .clone()
            .or_else(|| {
                self.file_path()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            })
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));

        // Предложить полный путь с именем файла по умолчанию
        let default_path = directory.join("untitled.txt");
        let default_value = default_path.display().to_string();

        let modal = InputModal::with_default(t().modal_save_as_title(), "", default_value);
        let action = PendingAction::SaveFileAs {
            panel_index: 0, // will be updated in app.rs
            directory,
        };
        self.modal_request = Some((action, ActiveModal::Input(Box::new(modal))));
        Ok(())
    }

    /// Open replace modal with previous find/replace text restored
    pub(crate) fn handle_start_replace(&mut self) {
        let mut replace_modal = ReplaceModal::new();

        // Restore previous find/replace text if available
        if let Some(ref find) = self.search.last_replace_find {
            replace_modal.set_find_input(find.clone());
        }
        if let Some(ref replace) = self.search.last_replace_with {
            replace_modal.set_replace_input(replace.clone());
        }

        // If there's saved find text - execute search immediately
        if let Some(ref find) = self.search.last_replace_find {
            let replace_with = self.search.last_replace_with.clone().unwrap_or_default();
            self.start_replace(find.clone(), replace_with, false);

            // Update match info in modal
            if let Some((current, total)) = self.get_search_match_info() {
                replace_modal.set_match_info(current, total);
            }
        }

        self.modal_request = Some((
            PendingAction::Replace,
            ActiveModal::Replace(Box::new(replace_modal)),
        ));
    }
}

impl Panel for Editor {
    fn name(&self) -> &'static str {
        "editor"
    }

    fn title(&self) -> String {
        const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

        let modified = if self.buffer.is_modified() { "*" } else { "" };

        let external_change = if self.file_state.external_change_detected {
            " [changed on disk]"
        } else {
            ""
        };

        let search_info = if let Some(ref search) = self.search.state {
            if search.is_active() {
                let current = search.current_match.map(|i| i + 1).unwrap_or(0);
                let total = search.match_count();
                if total > 0 {
                    format!(" [{}]", t().editor_search_match_info(current, total))
                } else {
                    format!(" [{}]", t().editor_search_no_matches())
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // LSP loading spinner
        let lsp_indicator = if self.lsp.server_loading {
            let frame_idx = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                / 100) as usize
                % SPINNER_FRAMES.len();
            format!("{} ", SPINNER_FRAMES[frame_idx])
        } else {
            String::new()
        };

        // LSP status text (shown after filename)
        let lsp_status = self
            .lsp
            .server_status_text
            .as_ref()
            .map(|s| format!(" ({})", s))
            .unwrap_or_default();

        format!(
            "{}{}{}{}{}{}",
            lsp_indicator,
            self.file_state.title,
            modified,
            lsp_status,
            external_change,
            search_info
        )
    }

    fn prepare_render(&mut self, theme: &Theme, config: &Config) {
        self.render_cache.theme = *theme;
        self.render_cache.config = config.clone();

        // Sync EditorConfig with global Config.editor settings
        // This ensures runtime config changes are applied to the editor
        self.config.word_wrap = config.editor.word_wrap;
        self.config.tab_size = config.editor.tab_size;

        // Sync highlight cache with theme's light/dark mode and default foreground color
        self.render_cache
            .highlight
            .set_light_theme(theme.is_light_theme());
        self.render_cache.highlight.set_default_fg(theme.fg);
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
        // Update cached theme and config from render context
        // Note: We'll need to convert from PanelConfig/ThemeColors to our internal types
        // Use cached theme and config (updated by app layer before rendering)
        let theme = self.render_cache.theme;
        let config = self.render_cache.config.clone();
        self.render_content(
            area,
            buf,
            &theme,
            &config,
            ctx.is_focused,
            ctx.border_right_x,
        );
    }

    fn handle_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        // Any keyboard input should make viewport follow cursor again
        self.scroll_follows_cursor = true;

        // Note: Key translation should be done at app level before calling handle_key
        // If you need translation, call translate_hotkey from termide-core or keyboard module

        let command = keyboard::EditorCommand::from_key_event(
            key,
            self.config.read_only,
            self.search.state.is_some(),
            self.selection.is_some(),
            self.lsp.completion_popup.is_some(),
            &self.config.keybindings,
        );

        // Collect events from internal state
        let mut events = Vec::new();

        // Execute command and handle errors
        if let Err(e) = command.execute(self) {
            events.push(PanelEvent::SetStatusMessage {
                message: e.to_string(),
                is_error: true,
            });
        }

        // Convert status_message to event and take it (removes from legacy field)
        if let Some(message) = self.status_message.take() {
            events.push(PanelEvent::SetStatusMessage {
                message,
                is_error: false,
            });
        }

        events
    }

    fn handle_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        panel_area: Rect,
    ) -> Vec<PanelEvent> {
        use crossterm::event::{MouseButton, MouseEventKind};

        // Handle completion popup mouse interactions first
        if let Some(popup_rect) = self.lsp.popup_rect {
            let in_popup = mouse.column >= popup_rect.x
                && mouse.column < popup_rect.x + popup_rect.width
                && mouse.row >= popup_rect.y
                && mouse.row < popup_rect.y + popup_rect.height;

            match mouse.kind {
                MouseEventKind::ScrollUp if in_popup => {
                    if let Some(ref mut popup) = self.lsp.completion_popup {
                        popup.scroll_up(3);
                    }
                    return vec![];
                }
                MouseEventKind::ScrollDown if in_popup => {
                    if let Some(ref mut popup) = self.lsp.completion_popup {
                        popup.scroll_down(3);
                    }
                    return vec![];
                }
                MouseEventKind::Down(MouseButton::Left) if in_popup => {
                    // Click inside popup - select and accept
                    let row = (mouse.row - popup_rect.y) as usize;
                    if let Some(ref mut popup) = self.lsp.completion_popup {
                        popup.select_at_row(row);
                    }
                    // Accept the selected completion
                    self.accept_completion();
                    self.lsp.popup_rect = None;
                    // Skip the following MouseUp to prevent cursor jump
                    self.input.click_tracker.skip_next_up = true;
                    return vec![];
                }
                MouseEventKind::Down(MouseButton::Left) if !in_popup => {
                    // Click outside popup - close it and continue with normal handling
                    self.lsp.completion_popup = None;
                    self.lsp.popup_rect = None;
                    // Fall through to normal mouse handling
                }
                _ => {}
            }
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.viewport.scroll_up(3);
                self.scroll_follows_cursor = false;
                return vec![];
            }
            MouseEventKind::ScrollDown => {
                self.viewport
                    .scroll_down(3, self.render_cache.virtual_line_count);
                self.scroll_follows_cursor = false;
                return vec![];
            }
            _ => {}
        }

        let inner = Rect {
            x: panel_area.x + 1,
            y: panel_area.y + 1,
            width: panel_area.width.saturating_sub(2),
            height: panel_area.height.saturating_sub(2),
        };

        let line_number_width = rendering::LINE_NUMBER_WIDTH as u16;
        let content_x = inner.x + line_number_width;
        let content_y = inner.y;
        let content_width = inner.width.saturating_sub(line_number_width);
        let content_height = inner.height;

        if mouse.column < content_x || mouse.column >= content_x + content_width {
            return vec![];
        }
        if mouse.row < content_y || mouse.row >= content_y + content_height {
            return vec![];
        }

        let rel_x = (mouse.column - content_x) as usize;
        let rel_y = (mouse.row - content_y) as usize;

        let (buffer_line, wrapped_offset, chunk_end) = if self.config.word_wrap {
            word_wrap::visual_row_to_buffer_position(
                &self.buffer,
                rel_y,
                self.viewport.top_line,
                content_width as usize,
                self.render_cache.use_smart_wrap,
            )
        } else {
            let line_len = self
                .buffer
                .line(self.viewport.top_line + rel_y)
                .map(|s| {
                    use unicode_segmentation::UnicodeSegmentation;
                    s.trim_end_matches('\n').graphemes(true).count()
                })
                .unwrap_or(0);
            (self.viewport.top_line + rel_y, 0, line_len)
        };

        let max_line = self.buffer.line_count().saturating_sub(1);
        let target_line = buffer_line.min(max_line);

        // Get line text for screen→grapheme conversion
        let line_text = self
            .buffer
            .line(target_line)
            .map(|s| s.trim_end_matches('\n').to_string())
            .unwrap_or_default();

        // Convert screen column to grapheme index
        let buffer_col = if self.config.word_wrap {
            // wrapped_offset is grapheme index where this visual line starts
            // chunk_end is grapheme index where this visual line ends (exclusive)
            // rel_x is screen column within this visual line
            // Get only the text for this visual line and convert rel_x to grapheme offset
            use unicode_segmentation::UnicodeSegmentation;
            let visual_line_len = chunk_end.saturating_sub(wrapped_offset);
            let segment: String = line_text
                .graphemes(true)
                .skip(wrapped_offset)
                .take(visual_line_len)
                .collect();
            let grapheme_in_segment = screen_col_to_grapheme_idx(&segment, rel_x);
            wrapped_offset + grapheme_in_segment
        } else {
            // Without wrap: convert absolute screen col to grapheme idx
            screen_col_to_grapheme_idx(&line_text, self.viewport.left_column + rel_x)
        };

        let line_len = self.buffer.line_len_graphemes(target_line);
        let target_col = buffer_col.min(line_len);

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.scroll_follows_cursor = true;
                self.close_search();

                if self
                    .input
                    .click_tracker
                    .is_double_click(target_line, target_col)
                {
                    let temp_cursor = Cursor::at(target_line, target_col);
                    if let Some((new_selection, new_cursor)) =
                        selection::select_word(&self.buffer, &temp_cursor)
                    {
                        self.selection = Some(new_selection);
                        self.cursor = new_cursor;
                        self.input.click_tracker.skip_next_up = true;
                    }
                    self.input.click_tracker.reset();
                } else {
                    self.cursor = Cursor::at(target_line, target_col);
                    self.selection = Some(Selection::new(self.cursor, self.cursor));
                    self.input
                        .click_tracker
                        .record_click(target_line, target_col);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.scroll_follows_cursor = true;
                self.cursor = Cursor::at(target_line, target_col);
                if let Some(ref mut selection) = self.selection {
                    selection.active = self.cursor;
                }
                self.viewport
                    .ensure_cursor_visible(&self.cursor, self.render_cache.virtual_line_count);
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.scroll_follows_cursor = true;
                if self.input.click_tracker.skip_next_up {
                    self.input.click_tracker.skip_next_up = false;
                    return vec![];
                }
                self.cursor = Cursor::at(target_line, target_col);
                if let Some(ref mut selection) = self.selection {
                    selection.active = self.cursor;
                    if selection.is_empty() {
                        self.selection = None;
                    }
                }
            }
            _ => {}
        }

        vec![]
    }

    fn handle_command(&mut self, cmd: PanelCommand<'_>) -> CommandResult {
        match cmd {
            PanelCommand::GetRepoRoot => {
                let repo_root = self.get_or_compute_repo_root().cloned();
                CommandResult::RepoRoot(repo_root)
            }
            PanelCommand::OnGitUpdate { repo_paths } => {
                if let Some(file_path) = self.file_path() {
                    // Check if any updated repo contains this file
                    if repo_paths.iter().any(|repo| file_path.starts_with(repo)) {
                        self.update_git_diff();
                        return CommandResult::NeedsRedraw(true);
                    }
                }
                CommandResult::NeedsRedraw(false)
            }
            PanelCommand::CheckPendingGitDiff => {
                self.check_pending_git_diff_update();
                CommandResult::None
            }
            PanelCommand::CheckGitDiffReceiver => {
                let needs_redraw = self.check_git_diff_receiver();
                CommandResult::NeedsRedraw(needs_redraw)
            }
            PanelCommand::CheckExternalModification => {
                self.check_external_modification();
                CommandResult::None
            }
            PanelCommand::GetFsWatchInfo => {
                // For Editor, return file path info for watcher registration
                let file_path = self.file_path().map(|p| p.to_path_buf());
                if let Some(ref file_path) = file_path {
                    let repo_root = self.get_or_compute_repo_root().cloned();
                    let current_path = file_path
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| PathBuf::from("/"));
                    CommandResult::FsWatchInfo {
                        watched_root: repo_root,
                        current_path,
                        is_git_repo: self
                            .git
                            .cached_repo_root
                            .as_ref()
                            .is_some_and(|r| r.is_some()),
                    }
                } else {
                    CommandResult::None
                }
            }
            PanelCommand::OnFsUpdate { changed_path } => {
                if let Some(file_path) = self.file_path() {
                    // Check for exact file match or directory containing the file
                    let file_changed =
                        changed_path == file_path || changed_path.parent() == file_path.parent();

                    if file_changed {
                        self.update_git_diff();
                        self.check_external_modification();
                        return CommandResult::NeedsRedraw(true);
                    }
                }
                CommandResult::NeedsRedraw(false)
            }
            PanelCommand::Reload => {
                if self.reload_from_disk().is_ok() {
                    CommandResult::NeedsRedraw(true)
                } else {
                    CommandResult::NeedsRedraw(false)
                }
            }
            PanelCommand::GetModificationStatus => CommandResult::ModificationStatus {
                is_modified: self.buffer.is_modified(),
                has_external_change: self.file_state.external_change_detected,
            },
            PanelCommand::Save => match self.save() {
                Ok(_) => CommandResult::SaveResult {
                    success: true,
                    error: None,
                },
                Err(e) => CommandResult::SaveResult {
                    success: false,
                    error: Some(e.to_string()),
                },
            },
            PanelCommand::CloseWithoutSaving => {
                // Clear external change flag - the panel is being closed without saving
                self.file_state.external_change_detected = false;
                // Note: buffer.modified stays true but caller handles closing directly
                CommandResult::None
            }
            // Commands not applicable to Editor
            PanelCommand::SetFsWatchRoot { .. }
            | PanelCommand::Resize { .. }
            | PanelCommand::RefreshDirectory
            | PanelCommand::SetGitOperationInProgress { .. } => CommandResult::None,
        }
    }

    fn needs_close_confirmation(&self) -> Option<String> {
        if self.buffer.is_modified() {
            Some("File has unsaved changes. Close anyway?".to_string())
        } else if self.file_state.external_change_detected {
            Some("File changed on disk. Close anyway?".to_string())
        } else {
            None
        }
    }

    fn captures_escape(&self) -> bool {
        // Capture Escape when search is active or completion popup is open
        self.search.state.is_some() || self.lsp.completion_popup.is_some()
    }

    fn to_session(&self, session_dir: &std::path::Path) -> Option<SessionPanel> {
        if let Some(path) = self.file_path() {
            // Named file - save path
            Some(SessionPanel::Editor {
                path: Some(path.to_path_buf()),
                unsaved_buffer_file: None,
            })
        } else if self.buffer_is_modified() {
            // Unnamed buffer with unsaved content - save to session dir
            let filename = self
                .unsaved_buffer_file()
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    format!(
                        "unsaved-{}.txt",
                        chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")
                    )
                });

            let content = self.buffer.text();
            if content.trim().is_empty() {
                return None; // Don't save empty buffers
            }

            // Save content to session directory
            if let Err(e) = termide_session::save_unsaved_buffer(session_dir, &filename, &content) {
                eprintln!("Warning: Failed to save unsaved buffer: {}", e);
                return None;
            }

            Some(SessionPanel::Editor {
                path: None,
                unsaved_buffer_file: Some(filename),
            })
        } else {
            // Unnamed buffer without changes - don't save
            None
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn get_working_directory(&self) -> Option<PathBuf> {
        self.file_path()
            .and_then(|p| p.parent().map(|parent| parent.to_path_buf()))
    }
}

// Additional methods used by app layer (not part of Panel trait)
impl Editor {
    /// Take modal window request (if any).
    pub fn take_modal_request(&mut self) -> Option<(PendingAction, ActiveModal)> {
        self.modal_request.take()
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use termide_core::{CommandResult, Panel, PanelCommand};

    fn create_editor_with_content(content: &str) -> (Editor, NamedTempFile) {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "{}", content).unwrap();
        let editor =
            Editor::open_file_with_config(file.path().to_path_buf(), EditorConfig::default())
                .unwrap();
        (editor, file)
    }

    #[test]
    fn test_handle_command_get_modification_status_new_editor() {
        let mut editor = Editor::new();
        let result = editor.handle_command(PanelCommand::GetModificationStatus);

        if let CommandResult::ModificationStatus {
            is_modified,
            has_external_change,
        } = result
        {
            assert!(!is_modified);
            assert!(!has_external_change);
        } else {
            panic!("Expected ModificationStatus result");
        }
    }

    #[test]
    fn test_handle_command_get_modification_status_after_edit() {
        let (mut editor, _file) = create_editor_with_content("hello");

        // Insert text to modify buffer
        let _ = editor.insert_char('x');

        let result = editor.handle_command(PanelCommand::GetModificationStatus);
        if let CommandResult::ModificationStatus {
            is_modified,
            has_external_change,
        } = result
        {
            assert!(is_modified);
            assert!(!has_external_change);
        } else {
            panic!("Expected ModificationStatus result");
        }
    }

    #[test]
    fn test_handle_command_save_new_editor() {
        let mut editor = Editor::new();
        // New editor without file path should fail to save
        let result = editor.handle_command(PanelCommand::Save);

        if let CommandResult::SaveResult { success, error } = result {
            assert!(!success);
            assert!(error.is_some());
        } else {
            panic!("Expected SaveResult");
        }
    }

    #[test]
    fn test_handle_command_save_with_file() {
        let (mut editor, _file) = create_editor_with_content("original");

        // Modify and save
        let _ = editor.insert_char('!');
        let result = editor.handle_command(PanelCommand::Save);

        if let CommandResult::SaveResult { success, error } = result {
            assert!(success);
            assert!(error.is_none());
        } else {
            panic!("Expected SaveResult");
        }

        // Check modification status after save
        let result = editor.handle_command(PanelCommand::GetModificationStatus);
        if let CommandResult::ModificationStatus { is_modified, .. } = result {
            assert!(!is_modified);
        }
    }

    #[test]
    fn test_handle_command_reload() {
        let (mut editor, mut file) = create_editor_with_content("original");

        // Modify file externally
        write!(file, "modified content").unwrap();

        let result = editor.handle_command(PanelCommand::Reload);
        assert!(result.needs_redraw());
    }

    #[test]
    fn test_handle_command_close_without_saving() {
        let (mut editor, _file) = create_editor_with_content("hello");
        editor.file_state.external_change_detected = true;

        let result = editor.handle_command(PanelCommand::CloseWithoutSaving);
        assert!(matches!(result, CommandResult::None));

        // External change flag should be cleared
        assert!(!editor.file_state.external_change_detected);
    }

    #[test]
    fn test_handle_command_not_applicable() {
        let mut editor = Editor::new();

        // Commands not applicable to Editor should return None
        let result = editor.handle_command(PanelCommand::Resize { rows: 24, cols: 80 });
        assert!(matches!(result, CommandResult::None));

        let result = editor.handle_command(PanelCommand::RefreshDirectory);
        assert!(matches!(result, CommandResult::None));

        let result = editor.handle_command(PanelCommand::SetFsWatchRoot {
            root: None,
            is_git_repo: false,
        });
        assert!(matches!(result, CommandResult::None));
    }

    #[test]
    fn test_editor_panel_trait_title() {
        let editor = Editor::new();
        assert_eq!(editor.title(), "Untitled");

        let (editor, _file) = create_editor_with_content("test");
        // Title should be the filename
        assert!(editor.title().ends_with(".tmp") || !editor.title().is_empty());
    }

    #[test]
    fn test_editor_panel_trait_needs_close_confirmation() {
        let editor = Editor::new();
        // New unmodified editor doesn't need confirmation
        assert!(editor.needs_close_confirmation().is_none());

        let (mut editor, _file) = create_editor_with_content("hello");
        let _ = editor.insert_char('x');
        // Modified editor needs confirmation
        assert!(editor.needs_close_confirmation().is_some());
    }

    // === Large file handling tests ===

    fn create_large_file(line_count: usize) -> (Editor, NamedTempFile) {
        let mut file = NamedTempFile::new().unwrap();
        for i in 0..line_count {
            writeln!(
                file,
                "Line {}: content with some text for testing large file behavior",
                i + 1
            )
            .unwrap();
        }
        file.flush().unwrap();
        let editor =
            Editor::open_file_with_config(file.path().to_path_buf(), EditorConfig::default())
                .unwrap();
        (editor, file)
    }

    #[test]
    fn test_large_file_load_10k_lines() {
        let (editor, _file) = create_large_file(10_000);
        // writeln! adds trailing newline, so we get one extra empty line
        assert!(editor.buffer.line_count() >= 10_000);
        assert_eq!(editor.cursor.line, 0);
        assert_eq!(editor.cursor.column, 0);
    }

    #[test]
    fn test_large_file_viewport_navigation() {
        let (mut editor, _file) = create_large_file(10_000);
        editor.viewport.resize(80, 24);

        // Initial state
        assert_eq!(editor.viewport().top_line, 0);
        assert!(editor.viewport().is_line_visible(0));
        assert!(!editor.viewport().is_line_visible(30));

        // Navigate to middle of file
        editor.set_cursor_line(4999);
        editor
            .viewport
            .ensure_cursor_visible(&editor.cursor, editor.buffer.line_count());
        assert!(editor.viewport().is_cursor_visible(&editor.cursor));
        assert_eq!(editor.cursor.line, 4999);

        // Navigate to end
        editor.set_cursor_line(9999);
        editor
            .viewport
            .ensure_cursor_visible(&editor.cursor, editor.buffer.line_count());
        assert_eq!(editor.cursor.line, 9999);
        assert!(editor.viewport().is_cursor_visible(&editor.cursor));
    }

    #[test]
    fn test_large_file_cursor_movement() {
        let (mut editor, _file) = create_large_file(10_000);
        editor.viewport.resize(80, 24);

        // Move down page by page
        for _ in 0..100 {
            editor.page_down();
        }
        // Should be around line 2400+ (100 pages * ~24 lines)
        assert!(editor.cursor.line > 2000);

        // Move to end
        editor.move_to_document_end();
        // Should be at last line (buffer may have trailing empty line)
        assert_eq!(editor.cursor.line, editor.buffer.line_count() - 1);

        // Move to start
        editor.move_to_document_start();
        assert_eq!(editor.cursor.line, 0);
    }

    #[test]
    fn test_large_file_edit_at_various_positions() {
        let (mut editor, _file) = create_large_file(1_000);
        editor.viewport.resize(80, 24);

        // Edit at beginning
        let _ = editor.insert_char('A');
        assert_eq!(editor.buffer.line(0).unwrap().chars().next().unwrap(), 'A');

        // Edit at middle
        editor.set_cursor_line(499);
        let _ = editor.insert_char('M');
        assert!(editor.buffer.line(499).unwrap().starts_with('M'));

        // Edit at end
        editor.set_cursor_line(999);
        let _ = editor.insert_char('Z');
        assert!(editor.buffer.line(999).unwrap().starts_with('Z'));

        // Verify buffer is modified
        assert!(editor.buffer.is_modified());
    }

    #[test]
    fn test_large_file_undo_redo() {
        let (mut editor, _file) = create_large_file(1_000);

        // Make several edits
        let _ = editor.insert_char('X');
        editor.set_cursor_line(499);
        let _ = editor.insert_char('Y');
        editor.set_cursor_line(999);
        let _ = editor.insert_char('Z');

        // Undo all
        let _ = editor.buffer.undo();
        let _ = editor.buffer.undo();
        let _ = editor.buffer.undo();

        // Buffer should not be modified after full undo
        // (assuming we undid all changes)
        let first_line = editor.buffer.line(0).unwrap();
        assert!(first_line.starts_with("Line 1:"));
    }

    #[test]
    fn test_large_file_search() {
        let (mut editor, _file) = create_large_file(1_000);

        // Search for a line in the middle
        editor.start_search("Line 500:".to_string(), false);
        editor.search_next();

        // Cursor should move to line 500 (0-indexed: line 499)
        assert_eq!(editor.cursor.line, 499);
    }

    #[test]
    fn test_large_file_scroll_performance() {
        let (mut editor, _file) = create_large_file(50_000);
        editor.viewport.resize(80, 24);

        // Rapid scrolling should be efficient
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            editor.viewport.scroll_down(10, editor.buffer.line_count());
        }
        let scroll_time = start.elapsed();

        // Should complete in reasonable time (< 100ms for 50K lines)
        assert!(
            scroll_time.as_millis() < 100,
            "Scrolling took too long: {:?}",
            scroll_time
        );

        // Verify we actually scrolled
        assert!(editor.viewport().top_line > 0);
    }
}
