use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

use termide_buffer::{Cursor, LineEnding, Selection, TextBuffer, Viewport};
use termide_config::Config;
use termide_core::{HotkeyTable, PanelEvent};
use termide_git::GitDiffCache;
use termide_i18n::t;
use termide_modal::{ActiveModal, FindBar, SaveAsModal};
use termide_state::PendingAction;
use termide_vfs::VfsManager;

use crate::{
    config::*,
    constants, file_io,
    state::{FileState, GitIntegration, InputState, LspState, RenderingCache, SearchController},
    vim::VimState,
};

// Re-export LspManager for use in app integration
pub use termide_lsp::{CompletionTriggerKind, LspManager, ServerStatus};

// Editor methods are split across separate files for better organization
#[path = "editor_lsp.rs"]
mod editor_lsp;

#[path = "editor_movement.rs"]
mod editor_movement;

#[path = "editor_text.rs"]
mod editor_text;

#[path = "editor_search.rs"]
mod editor_search;

#[path = "editor_mouse.rs"]
mod editor_mouse;

#[path = "editor_viewport.rs"]
mod editor_viewport;

#[path = "editor_panel.rs"]
mod editor_panel;

#[path = "editor_rendering.rs"]
mod editor_rendering;

#[path = "editor_git.rs"]
mod editor_git;

#[path = "editor_file_io.rs"]
mod editor_file_io;

#[cfg(test)]
#[path = "core_tests.rs"]
mod core_tests;

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
    /// VFS manager for remote file operations
    pub(crate) vfs_manager: Option<Arc<VfsManager>>,

    // === UI state ===
    /// Inline find/replace bar, docked at the bottom of the panel while open.
    /// Replaces the floating search/replace modals for the editor.
    pub(crate) find_bar: Option<FindBar>,
    /// Modal window request
    modal_request: Option<(PendingAction, ActiveModal)>,
    /// Pending upload operation (for regular Ctrl+S saves of remote files)
    /// Contains (temp_path, remote_path, vfs_manager) for app to create upload via OperationManager
    pub(crate) pending_upload: Option<(
        PathBuf,
        termide_vfs::VfsPath,
        std::sync::Arc<termide_vfs::VfsManager>,
    )>,
    /// Pending remote file open operation (for async downloads)
    pub(crate) pending_remote_open: Option<crate::remote::PendingRemoteOpen>,
    /// Updated config after save (for applying in AppState)
    config_update: Option<Config>,
    /// Status message to display to user
    pub(crate) status_message: Option<String>,
    /// When true, viewport follows cursor. When false (after mouse scroll), viewport stays put.
    scroll_follows_cursor: bool,

    // === Vim mode state ===
    /// Vim mode state (None if Vim mode is disabled)
    pub(crate) vim: Option<VimState>,

    // === Outline symbol navigation ===
    /// Sorted line positions of structural symbols (from outline) for Ctrl+Up/Down navigation.
    /// When empty, paragraph navigation falls back to blank lines.
    symbol_lines: Vec<usize>,

    // === Stale-on-collapse optimization ===
    /// Whether panel is stale (collapsed, skipping background work)
    is_stale: bool,

    /// Hotkey table for configurable keyboard shortcuts
    hotkeys: HotkeyTable,
    /// Pointer of the last Arc<Config> used to build hotkeys (skip rebuild when unchanged)
    last_config_ptr: usize,

    /// Per-editor tab_size override set at runtime (e.g. from the status bar
    /// Tab indicator modal). When `Some`, it wins over `config.editor.tab_size`
    /// on every `prepare_render` so the global config resync doesn't clobber
    /// it. `None` means "follow the global setting".
    tab_size_override: Option<usize>,
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

        // Initialize Vim state if vim mode is enabled
        let vim = if config.vim_mode {
            Some(VimState::new())
        } else {
            None
        };

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
            vfs_manager: None,
            find_bar: None,
            modal_request: None,
            pending_upload: None,
            pending_remote_open: None,
            config_update: None,
            status_message: None,
            scroll_follows_cursor: true,
            vim,
            symbol_lines: Vec::new(),
            is_stale: false,
            hotkeys: HotkeyTable::default(),
            last_config_ptr: 0,
            tab_size_override: None,
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

    /// Check if Vim mode is enabled
    pub fn vim_mode_enabled(&self) -> bool {
        self.vim.is_some()
    }

    /// Get Vim mode display string for status bar (e.g., "NORMAL", "INSERT")
    /// Returns None if Vim mode is disabled
    pub fn vim_mode_display(&self) -> Option<&'static str> {
        self.vim.as_ref().map(|v| v.mode.display())
    }

    /// Get mutable reference to Vim state
    pub fn vim_state_mut(&mut self) -> Option<&mut VimState> {
        self.vim.as_mut()
    }

    /// Get reference to Vim state
    pub fn vim_state(&self) -> Option<&VimState> {
        self.vim.as_ref()
    }

    /// Execute a Vim key result and return any panel events
    fn execute_vim_result(&mut self, result: crate::vim::VimKeyResult) -> Option<Vec<PanelEvent>> {
        use crate::vim::{
            motions::execute_motion, operators, InsertPosition, PanelDirection, VimKeyResult,
            VimMode,
        };
        use termide_core::VimPanelDirection;

        let version_before = self.buffer.edit_version();
        let mut events = Vec::new();

        match result {
            VimKeyResult::Motion { motion, count } => {
                let viewport_height = self.viewport.height;
                let content_width = self.render_cache.content_width;
                let new_cursor = execute_motion(
                    motion,
                    &self.cursor,
                    &self.buffer,
                    count,
                    viewport_height,
                    content_width,
                    true, // use_smart_wrap
                );
                self.cursor = new_cursor;
                // Clear selection on normal mode motion
                self.selection = None;
            }
            VimKeyResult::MotionWithSelection { motion, count } => {
                let viewport_height = self.viewport.height;
                let content_width = self.render_cache.content_width;
                let new_cursor = execute_motion(
                    motion,
                    &self.cursor,
                    &self.buffer,
                    count,
                    viewport_height,
                    content_width,
                    true, // use_smart_wrap
                );

                // Update selection
                if let Some(vim) = &self.vim {
                    if let Some(anchor) = vim.visual_anchor {
                        let selection = termide_buffer::Selection::new(anchor, new_cursor);
                        self.selection = Some(selection);
                    }
                }
                self.cursor = new_cursor;
            }
            VimKeyResult::OperatorMotion {
                operator,
                motion,
                count,
            } => {
                let viewport_height = self.viewport.height;
                let content_width = self.render_cache.content_width;
                let start = self.cursor;
                let end = execute_motion(
                    motion,
                    &self.cursor,
                    &self.buffer,
                    count,
                    viewport_height,
                    content_width,
                    true, // use_smart_wrap
                );

                if let Some(vim) = self.vim.as_mut() {
                    if let Ok(op_result) = operators::execute_operator(
                        operator,
                        start,
                        end,
                        &mut self.buffer,
                        vim,
                        false,
                    ) {
                        self.cursor = op_result.cursor;
                        if op_result.enter_insert {
                            vim.enter_insert();
                        }
                    }
                }
                self.selection = None;
            }
            VimKeyResult::LinewiseOperator { operator, count } => {
                let start_line = self.cursor.line;
                let end_line =
                    (start_line + count - 1).min(self.buffer.line_count().saturating_sub(1));

                if let Some(vim) = self.vim.as_mut() {
                    if let Ok(op_result) = operators::execute_linewise_operator(
                        operator,
                        start_line,
                        end_line,
                        &mut self.buffer,
                        vim,
                    ) {
                        self.cursor = op_result.cursor;
                        if op_result.enter_insert {
                            vim.enter_insert();
                        }
                    }
                }
                self.selection = None;
            }
            VimKeyResult::VisualOperator { operator } => {
                if let (Some(selection), Some(vim)) = (self.selection.as_ref(), self.vim.as_mut()) {
                    let start = selection.start();
                    let end = selection.end();
                    let linewise = vim.mode == VimMode::VisualLine;

                    if let Ok(op_result) = operators::execute_operator(
                        operator,
                        start,
                        end,
                        &mut self.buffer,
                        vim,
                        linewise,
                    ) {
                        self.cursor = op_result.cursor;
                        if op_result.enter_insert {
                            vim.enter_insert();
                        } else {
                            vim.exit_to_normal();
                        }
                    }
                }
                self.selection = None;
            }
            VimKeyResult::EnterInsert(position) => {
                // Position cursor based on insert position
                match position {
                    InsertPosition::BeforeCursor => {
                        // Cursor stays where it is
                    }
                    InsertPosition::AfterCursor => {
                        let line_len = self.buffer.line_len_graphemes(self.cursor.line);
                        if self.cursor.column < line_len {
                            self.cursor.column += 1;
                        }
                    }
                    InsertPosition::LineStart => {
                        // Move to first non-blank
                        if let Some(line) = self.buffer.line(self.cursor.line) {
                            use unicode_segmentation::UnicodeSegmentation;
                            let line = line.trim_end_matches('\n');
                            let first_non_blank = line
                                .graphemes(true)
                                .position(|g| !g.chars().all(|c| c.is_whitespace()))
                                .unwrap_or(0);
                            self.cursor.column = first_non_blank;
                        }
                    }
                    InsertPosition::LineEnd => {
                        let line_len = self.buffer.line_len_graphemes(self.cursor.line);
                        self.cursor.column = line_len;
                    }
                    InsertPosition::NewLineBelow => {
                        // Insert new line below and position cursor
                        let line_len = self.buffer.line_len_graphemes(self.cursor.line);
                        self.cursor.column = line_len;
                        let _ = self.buffer.insert(&self.cursor, "\n");
                        self.cursor.line += 1;
                        self.cursor.column = 0;
                    }
                    InsertPosition::NewLineAbove => {
                        // Insert new line above and position cursor
                        self.cursor.column = 0;
                        let _ = self.buffer.insert(&self.cursor, "\n");
                        // Cursor stays on the new (now previous) line
                    }
                }
                if let Some(vim) = self.vim.as_mut() {
                    vim.enter_insert();
                }
            }
            VimKeyResult::ExitToNormal => {
                // Move cursor back one position when exiting insert mode
                if self.cursor.column > 0 {
                    self.cursor.column -= 1;
                }
                self.selection = None;
            }
            VimKeyResult::StartVisual => {
                if let Some(vim) = self.vim.as_mut() {
                    vim.enter_visual(self.cursor);
                    // Start selection at current cursor
                    self.selection = Some(termide_buffer::Selection::new(self.cursor, self.cursor));
                }
            }
            VimKeyResult::StartVisualLine => {
                if let Some(vim) = self.vim.as_mut() {
                    vim.enter_visual_line(self.cursor);
                    // Select the entire line
                    let line_start = termide_buffer::Cursor::at(self.cursor.line, 0);
                    let line_end_col = self.buffer.line_len_graphemes(self.cursor.line);
                    let line_end = termide_buffer::Cursor::at(self.cursor.line, line_end_col);
                    self.selection = Some(termide_buffer::Selection::new(line_start, line_end));
                }
            }
            VimKeyResult::DeleteChar { count } => {
                for _ in 0..count {
                    if let Some(vim) = self.vim.as_mut() {
                        if let Ok(Some(deleted)) =
                            operators::delete_char(&mut self.buffer, &self.cursor)
                        {
                            vim.yank(deleted, false);
                        }
                    }
                }
            }
            VimKeyResult::Paste { after, count } => {
                if let Some(vim) = &self.vim {
                    if let Some(text) = vim.get_register() {
                        let linewise = vim.is_register_linewise();
                        for _ in 0..count {
                            if linewise {
                                // Linewise paste - insert on new line
                                let paste_line = if after {
                                    self.cursor.line + 1
                                } else {
                                    self.cursor.line
                                };
                                let insert_cursor = termide_buffer::Cursor::at(paste_line, 0);
                                // Need to handle insertion at end of document
                                if paste_line >= self.buffer.line_count() {
                                    let last_line_len = self
                                        .buffer
                                        .line_len_graphemes(self.buffer.line_count() - 1);
                                    let end_cursor = termide_buffer::Cursor::at(
                                        self.buffer.line_count() - 1,
                                        last_line_len,
                                    );
                                    let mut text_with_newline = String::from("\n");
                                    text_with_newline.push_str(text.trim_end_matches('\n'));
                                    let _ = self.buffer.insert(&end_cursor, &text_with_newline);
                                } else {
                                    let _ = self.buffer.insert(&insert_cursor, text);
                                }
                                self.cursor.line = paste_line;
                                // Move to first non-blank
                                if let Some(line) = self.buffer.line(self.cursor.line) {
                                    use unicode_segmentation::UnicodeSegmentation;
                                    let line = line.trim_end_matches('\n');
                                    let first_non_blank = line
                                        .graphemes(true)
                                        .position(|g| !g.chars().all(|c| c.is_whitespace()))
                                        .unwrap_or(0);
                                    self.cursor.column = first_non_blank;
                                }
                            } else {
                                // Charwise paste
                                let insert_cursor = if after {
                                    termide_buffer::Cursor::at(
                                        self.cursor.line,
                                        self.cursor.column + 1,
                                    )
                                } else {
                                    self.cursor
                                };
                                if let Ok(new_cursor) = self.buffer.insert(&insert_cursor, text) {
                                    self.cursor = new_cursor;
                                    // Position cursor on last char of pasted text
                                    if self.cursor.column > 0 {
                                        self.cursor.column -= 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            VimKeyResult::Undo => {
                if let Ok(Some(new_cursor)) = self.buffer.undo() {
                    self.cursor = new_cursor;
                }
            }
            VimKeyResult::Redo => {
                if let Ok(Some(new_cursor)) = self.buffer.redo() {
                    self.cursor = new_cursor;
                }
            }
            VimKeyResult::PanelNavigation(direction) => {
                let vim_direction = match direction {
                    PanelDirection::Left => VimPanelDirection::Left,
                    PanelDirection::Down => VimPanelDirection::Down,
                    PanelDirection::Up => VimPanelDirection::Up,
                    PanelDirection::Right => VimPanelDirection::Right,
                };
                events.push(PanelEvent::VimPanelNavigation {
                    direction: vim_direction,
                });
            }
            VimKeyResult::Consumed | VimKeyResult::PassThrough | VimKeyResult::Unhandled => {
                // These are handled in handle_key
            }
        }

        // Ensure cursor is valid after operations
        self.clamp_cursor();

        // Catch-all: invalidate wrap cache if buffer was modified by any VIM operation
        if self.buffer.edit_version() != version_before {
            self.render_cache.invalidate_wrap_cache();
            self.render_cache
                .highlight
                .invalidate_range(0, self.buffer.line_count());
            self.schedule_git_diff_update();
            self.mark_lsp_changed();
        }

        if events.is_empty() {
            None
        } else {
            Some(events)
        }
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
                git.diff_cache = Some(cache);
            }
            Err(e) => {
                log::warn!("Editor: GitDiffCache update failed for {:?}: {}", path, e);
            }
        }

        // Start blame loading immediately (blame is enabled by default)
        if let Some(repo_root) = termide_git::find_repo_root(&path) {
            git.start_blame(&repo_root, &path);
        }

        // Initialize Vim state if vim mode is enabled
        let vim = if config.vim_mode {
            Some(VimState::new())
        } else {
            None
        };

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
            vfs_manager: None,
            find_bar: None,
            modal_request: None,
            pending_upload: None,
            pending_remote_open: None,
            config_update: None,
            status_message: None,
            scroll_follows_cursor: true,
            vim,
            symbol_lines: Vec::new(),
            is_stale: false,
            hotkeys: HotkeyTable::default(),
            last_config_ptr: 0,
            tab_size_override: None,
        })
    }

    /// Create editor with text (for displaying help, etc.)
    pub fn from_text(content: &str, title: String) -> Self {
        use ropey::Rope;

        // Create buffer directly through Rope
        let rope = Rope::from_str(content);

        let mut file_state = FileState::new();
        file_state.title = title;

        // view_only mode doesn't have vim enabled
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
            vfs_manager: None,
            find_bar: None,
            modal_request: None,
            pending_upload: None,
            pending_remote_open: None,
            config_update: None,
            status_message: None,
            scroll_follows_cursor: true,
            vim: None, // view_only mode doesn't have vim
            symbol_lines: Vec::new(),
            is_stale: false,
            hotkeys: HotkeyTable::default(),
            last_config_ptr: 0,
            tab_size_override: None,
        }
    }

    /// Set the file state (for remote file handling)
    pub fn set_file_state(&mut self, file_state: FileState) {
        self.file_state = file_state;
    }

    /// Set the VFS manager (for remote file saves)
    pub fn set_vfs_manager(&mut self, vfs_manager: Arc<VfsManager>) {
        self.vfs_manager = Some(vfs_manager);
    }

    /// Set outline symbol line positions for Ctrl+Up/Down navigation.
    pub fn set_symbol_lines(&mut self, lines: Vec<usize>) {
        self.symbol_lines = lines;
    }

    /// Insert text at the beginning of the buffer (for restoring unsaved buffers)
    pub fn insert_text(&mut self, text: &str) -> Result<()> {
        let cursor_at_start = Cursor::new();
        self.cursor = self.buffer.insert(&cursor_at_start, text)?;
        self.invalidate_cache_after_edit(0, text.contains('\n'));
        Ok(())
    }

    /// Set the unsaved buffer filename (for session restoration)
    pub fn set_unsaved_buffer_file(&mut self, filename: Option<String>) {
        self.file_state.unsaved_buffer_file = filename;
    }

    /// Assign a filename to this unsaved buffer if it doesn't have one yet.
    /// Called before session save so that to_session() has a stable name.
    pub fn ensure_unsaved_buffer_file(&mut self) {
        if self.file_path().is_none()
            && self.buffer_is_modified()
            && self.file_state.unsaved_buffer_file.is_none()
        {
            self.file_state.unsaved_buffer_file =
                Some(termide_session::generate_unsaved_filename());
        }
    }

    /// Check if buffer has unsaved modifications
    pub fn buffer_is_modified(&self) -> bool {
        self.buffer.is_modified()
    }

    /// Get updated config (if config file was saved)
    pub fn take_config_update(&mut self) -> Option<Config> {
        self.config_update.take()
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
            line_ending: match self.buffer.line_ending() {
                LineEnding::LF => "LF".to_string(),
                LineEnding::CRLF => "CRLF".to_string(),
            },
            file_type,
            read_only: self.config.read_only,
            syntax_highlighting: self.config.syntax_highlighting,
            vim_mode: self.vim_mode_display(),
        }
    }

    /// Get disk space information for the file's storage device.
    pub fn get_disk_space_info(&self) -> Option<termide_system_monitor::DiskSpaceInfo> {
        self.file_path()
            .and_then(termide_system_monitor::get_disk_space_info)
    }

    // ===== LogViewer support methods =====

    /// Get current cursor line (0-based).
    pub fn cursor_line(&self) -> usize {
        self.cursor.line
    }

    /// Get all buffer text as a string.
    pub fn content_string(&self) -> String {
        self.buffer.text()
    }

    /// Monotonic edit version counter (delegates to buffer).
    pub fn edit_version(&self) -> u64 {
        self.buffer.edit_version()
    }

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

    /// Scroll to end of document (word-wrap aware).
    /// Used by JournalPanel for auto-scroll functionality.
    pub fn scroll_to_document_end(&mut self) {
        let last_line = self.buffer.line_count().saturating_sub(1);
        self.cursor.line = last_line;
        self.cursor.column = 0;
        self.scroll_follows_cursor = true;
    }

    /// Go to specific position (for go-to-definition, outline navigation, etc.).
    /// Places the target line at the top of the viewport.
    pub fn goto_position(&mut self, line: usize, column: usize) {
        let max_line = self.buffer.line_count().saturating_sub(1);
        let target_line = line.min(max_line);

        let line_len = self.buffer.line_len_graphemes(target_line);
        let target_col = column.min(line_len);

        self.cursor = Cursor::at(target_line, target_col);
        self.selection = None;

        // Place the target line at the top of the viewport
        self.viewport.top_line = target_line;
        self.viewport.top_visual_row_offset = 0;
        self.scroll_follows_cursor = true;
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

    /// Invalidate syntax highlighting and wrap caches after text edit and schedule git diff update.
    ///
    /// If the edit is multiline, invalidates all lines from start_line to end of buffer.
    /// Otherwise, invalidates only the single changed line.
    pub(crate) fn invalidate_cache_after_edit(&mut self, start_line: usize, is_multiline: bool) {
        if is_multiline {
            self.render_cache
                .highlight
                .invalidate_range(start_line, self.buffer.line_count());
            // Invalidate wrap cache for all lines from start_line onwards
            self.render_cache.invalidate_wrap_range(start_line);
        } else {
            self.render_cache.highlight.invalidate_line(start_line);
            // Invalidate wrap cache for just this line
            self.render_cache.invalidate_wrap_line(start_line);
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
            // Invalidate wrap cache - undo/redo can affect any lines
            self.render_cache.invalidate_wrap_cache();
            // Schedule git diff update
            self.schedule_git_diff_update();
            // Mark for LSP notification
            self.mark_lsp_changed();
        }
        Ok(())
    }

    /// Open the inline find bar (find-only). The `_execute_search` flag is
    /// retained for its callers; the bar always runs the seeded query.
    pub(crate) fn open_search_modal(&mut self, _execute_search: bool) {
        self.open_find_bar(false);
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
    /// Returns Some((temp_path, remote_path, vfs_manager)) for remote files (async upload via OperationManager), None for local files
    pub(crate) fn handle_save(
        &mut self,
    ) -> Result<
        Option<(
            PathBuf,
            termide_vfs::VfsPath,
            std::sync::Arc<termide_vfs::VfsManager>,
        )>,
    > {
        if self.buffer.file_path().is_some() {
            // File has path - save normally
            self.save()
        } else {
            // File has no path - open "Save As" dialog
            self.handle_save_as()?;
            Ok(None)
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

        let modal = SaveAsModal::new(t().modal_save_as_title(), default_value);
        let action = PendingAction::SaveFileAs { directory };
        self.modal_request = Some((action, ActiveModal::SaveAs(Box::new(modal))));
        Ok(())
    }

    /// Open the inline find/replace bar (find + replace fields).
    pub(crate) fn handle_start_replace(&mut self) {
        self.open_find_bar(true);
    }
}

// Additional methods used by app layer (not part of Panel trait)
impl Editor {
    /// Take modal window request (if any).
    pub fn take_modal_request(&mut self) -> Option<(PendingAction, ActiveModal)> {
        self.modal_request.take()
    }

    /// Take pending upload operation (if any).
    /// Returns (temp_path, remote_path, vfs_manager) for app to create upload via OperationManager
    pub fn take_pending_upload(
        &mut self,
    ) -> Option<(
        PathBuf,
        termide_vfs::VfsPath,
        std::sync::Arc<termide_vfs::VfsManager>,
    )> {
        self.pending_upload.take()
    }

    /// Set pending upload operation (called by keyboard handler).
    pub(crate) fn set_pending_upload(
        &mut self,
        upload: (
            PathBuf,
            termide_vfs::VfsPath,
            std::sync::Arc<termide_vfs::VfsManager>,
        ),
    ) {
        self.pending_upload = Some(upload);
    }

    /// Take pending remote open operation (if any).
    pub fn take_pending_remote_open(&mut self) -> Option<crate::remote::PendingRemoteOpen> {
        self.pending_remote_open.take()
    }

    /// Set pending remote open operation.
    pub fn set_pending_remote_open(&mut self, pending: crate::remote::PendingRemoteOpen) {
        self.pending_remote_open = Some(pending);
    }

    /// Get the per-editor tab_size override, if any.
    pub fn tab_size_override(&self) -> Option<usize> {
        self.tab_size_override
    }

    /// Set (or clear with `None`) the per-editor tab_size override.
    /// Applied on the next `prepare_render`.
    pub fn set_tab_size_override(&mut self, v: Option<usize>) {
        self.tab_size_override = v;
    }
}

/// Build HotkeyTable for the editor from config.
pub(crate) fn build_editor_hotkey_table(config: &Config) -> HotkeyTable {
    let mut t = HotkeyTable::new();
    let kb = &config.editor.keybindings;

    // File operations
    t.insert("save", &kb.save);
    t.insert("save_as", &kb.save_as);
    t.insert("reload", &kb.reload);

    // Undo/Redo
    t.insert("undo", &kb.undo);
    t.insert("redo", &kb.redo);

    // Search & Replace
    t.insert("search", &kb.search);
    t.insert("search_next", &kb.search_next);
    t.insert("search_prev", &kb.search_prev);
    t.insert("replace", &kb.replace);
    t.insert("replace_current", &kb.replace_current);
    t.insert("replace_all", &kb.replace_all);

    // Selection
    t.insert("select_all", &kb.select_all);

    // Clipboard
    t.insert("copy", &kb.copy);
    t.insert("cut", &kb.cut);
    t.insert("paste", &kb.paste);

    // Advanced editing
    t.insert("duplicate_line", &kb.duplicate_line);
    t.insert("delete_line", &kb.delete_line);
    t.insert("toggle_comment", &kb.toggle_comment);

    // LSP
    t.insert("trigger_completion", &kb.trigger_completion);
    t.insert("show_hover", &kb.show_hover);
    t.insert("goto_definition", &kb.goto_definition);
    t.insert("find_references", &kb.find_references);
    t.insert("rename_symbol", &kb.rename_symbol);
    t.insert("code_action", &kb.code_action);

    t
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        // Cleanup remote temp file if present
        if let Some(temp_path) = self.file_state.temp_local_path() {
            // Safety check - only cleanup files in our temp directory
            if let Some(parent) = temp_path.parent() {
                if parent.ends_with("termide-remote-edit") && temp_path.exists() {
                    if let Err(e) = std::fs::remove_file(temp_path) {
                        log::warn!("Failed to cleanup temp file {}: {}", temp_path.display(), e);
                    }
                }
            }
        }
    }
}
