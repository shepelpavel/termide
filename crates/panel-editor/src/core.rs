use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{buffer::Buffer, layout::Rect};
use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;

use termide_buffer::{Cursor, LineEnding, Selection, TextBuffer, Viewport};
use termide_config::Config;
use termide_core::{
    CommandResult, Panel, PanelCommand, PanelEvent, RenderContext, SessionPanel, WidthPreference,
};
use termide_git::GitDiffCache;
use termide_i18n::t;
use termide_modal::{ActiveModal, ReplaceModal, SaveAsModal, SearchModal};
use termide_state::PendingAction;
use termide_theme::Theme;
use termide_ui::ScrollBar;
use termide_vfs::VfsManager;

use crate::{
    config::*,
    constants, file_io, git, keyboard, rendering,
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
            modal_request: None,
            pending_upload: None,
            pending_remote_open: None,
            config_update: None,
            status_message: None,
            scroll_follows_cursor: true,
            vim,
            symbol_lines: Vec::new(),
            is_stale: false,
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
            modal_request: None,
            pending_upload: None,
            pending_remote_open: None,
            config_update: None,
            status_message: None,
            scroll_follows_cursor: true,
            vim,
            symbol_lines: Vec::new(),
            is_stale: false,
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
            modal_request: None,
            pending_upload: None,
            pending_remote_open: None,
            config_update: None,
            status_message: None,
            scroll_follows_cursor: true,
            vim: None, // view_only mode doesn't have vim
            symbol_lines: Vec::new(),
            is_stale: false,
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

    /// Save file
    /// Returns error if file was modified externally (use force_save() to override)
    /// Returns Some((temp_path, remote_path, vfs_manager)) for remote files (async upload via OperationManager), None for local files
    pub fn save(
        &mut self,
    ) -> Result<
        Option<(
            PathBuf,
            termide_vfs::VfsPath,
            std::sync::Arc<termide_vfs::VfsManager>,
        )>,
    > {
        // Check for external modification conflict
        if self.file_state.external_change_detected {
            return Err(anyhow::anyhow!(
                "File was modified on disk. Use force save (Ctrl+Shift+S) to overwrite or reload (Ctrl+Shift+R) to discard changes."
            ));
        }

        // Handle remote file saves
        if self.file_state.is_remote() {
            let vfs_manager = self
                .vfs_manager
                .clone()
                .ok_or_else(|| anyhow::anyhow!("No VFS manager for remote file"))?;

            // Save to local temp file first
            self.buffer.save()?;

            // Get remote path and temp path
            let remote_path = self
                .file_state
                .remote_path()
                .ok_or_else(|| anyhow::anyhow!("No remote path"))?
                .clone();
            let temp_path = self
                .file_state
                .temp_local_path()
                .ok_or_else(|| anyhow::anyhow!("No temp path"))?
                .to_path_buf();

            log::info!(
                "Remote file save requested: {}",
                remote_path.to_url_string()
            );

            // Return info for async upload via OperationManager
            // Note: mtime and external_change_detected will be updated when upload completes
            return Ok(Some((temp_path, remote_path, vfs_manager)));
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
                return Ok(None); // Config file saved locally
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

        Ok(None) // Local file saved
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

    /// Toggle inline blame annotation for the current file (Alt+B).
    pub fn toggle_blame(&mut self) {
        let repo = self.get_or_compute_repo_root().cloned();
        let file = self.file_path().map(|p| p.to_path_buf());
        if let (Some(repo), Some(file)) = (repo, file) {
            self.git.toggle_blame(&repo, &file);
        }
    }

    /// Check if the file was modified externally (outside of this editor)
    pub fn check_external_modification(&mut self) {
        // Skip check for remote files - temp file changes don't indicate external edits
        if self.file_state.is_remote() {
            return;
        }

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

            // Invalidate rendering cache so new content is displayed immediately
            self.render_cache.invalidate_wrap_cache();
            self.render_cache
                .highlight
                .invalidate_range(0, self.buffer.line_count());

            log::info!("File reloaded from disk: {}", path.display());
        }
        Ok(())
    }

    /// Force save (ignore external changes)
    /// Returns Some((temp_path, remote_path, vfs_manager)) for remote files (async upload), None for local files
    pub fn force_save(
        &mut self,
    ) -> Result<
        Option<(
            PathBuf,
            termide_vfs::VfsPath,
            std::sync::Arc<termide_vfs::VfsManager>,
        )>,
    > {
        self.file_state.external_change_detected = false;
        self.save()
    }

    /// Get updated config (if config file was saved)
    pub fn take_config_update(&mut self) -> Option<Config> {
        self.config_update.take()
    }

    /// Update file modification time (for remote file uploads)
    pub fn update_file_mtime(&mut self, mtime: Option<std::time::SystemTime>) {
        self.file_state.mtime = mtime;
        if self.file_state.is_remote() {
            self.file_state.update_remote_mtime(mtime);
        }
    }

    /// Clear external change detected flag (after successful remote upload)
    pub fn clear_external_change_detected(&mut self) {
        self.file_state.external_change_detected = false;
    }

    /// Set upload state for remote files
    pub fn set_uploading(&mut self, uploading: bool) {
        self.file_state.set_uploading(uploading);
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

        let effective_width = if self.config.word_wrap {
            content_width
        } else {
            0
        };
        let use_smart_wrap = if self.config.word_wrap && content_width > 0 {
            self.should_use_smart_wrap(config)
        } else {
            false
        };

        // Update wrap settings BEFORE building cumulative cache
        // This ensures cache is invalidated if width changed
        self.render_cache
            .update_wrap_settings(effective_width, use_smart_wrap);
        self.render_cache.content_height = content_height;

        self.viewport.resize(content_width, content_height);

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
            &self.lsp.diagnostics,
            theme,
            config.editor.show_git_diff,
            self.config.word_wrap,
            use_smart_wrap,
            content_width,
            content_height,
        );

        // Blame annotation overlay on the cursor line
        self.render_blame_annotation(buf, area, content_width, content_height, theme);
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

    /// Render inline blame annotation on the cursor line (VS Code style).
    ///
    /// Draws dim text after the code content on the active line only.
    /// Does nothing if blame is disabled, data is not loaded, or there is no room.
    fn render_blame_annotation(
        &self,
        buf: &mut Buffer,
        area: Rect,
        content_width: usize,
        content_height: usize,
        theme: &Theme,
    ) {
        let entry = match self.git.blame_for_line(self.cursor.line) {
            Some(e) => e,
            None => return,
        };

        // Check cursor line is in the visible viewport
        if self.cursor.line < self.viewport.top_line
            || self.cursor.line >= self.viewport.top_line + content_height
        {
            return;
        }

        // Compute cursor screen row
        let cursor_screen_row = if self.config.word_wrap {
            let top_visual = self
                .render_cache
                .get_visual_row_for_line(self.viewport.top_line)
                .unwrap_or(0);
            let line_visual = self
                .render_cache
                .get_visual_row_for_line(self.cursor.line)
                .unwrap_or(0);
            line_visual.saturating_sub(top_visual)
        } else {
            self.cursor.line - self.viewport.top_line
        };

        let line_number_width = rendering::LINE_NUMBER_WIDTH as u16;
        // area is already the inner rect (borders stripped by the app layer before calling render)
        let content_x = area.x + line_number_width;
        let line_y = area.y + cursor_screen_row as u16;

        // Compute the visual width of the cursor line's content that is visible
        let line_visual_width: usize = self
            .buffer
            .line(self.cursor.line)
            .map(|l| {
                use unicode_width::UnicodeWidthChar;
                l.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>()
            })
            .unwrap_or(0);
        let visible_code_width = line_visual_width
            .saturating_sub(self.viewport.left_column)
            .min(content_width);
        let ann_x = content_x + visible_code_width as u16;

        // Need at least 12 columns for the annotation to be useful
        let right_edge = area.x + area.width; // area is inner rect, no border to subtract
        let available = right_edge.saturating_sub(ann_x) as usize;
        if available < 12 {
            return;
        }

        let annotation = entry.inline_text();
        let truncated = termide_git::truncate_right(&annotation, available);
        let style = ratatui::style::Style::default().fg(theme.disabled);
        buf.set_string(ann_x, line_y, &truncated, style);
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

        let effective_width = if self.config.word_wrap {
            content_width
        } else {
            0 // Set to 0 when word wrap is disabled to trigger fallback behavior
        };
        let use_smart_wrap = if self.config.word_wrap && content_width > 0 {
            self.should_use_smart_wrap(config)
        } else {
            false
        };

        // Update wrap settings BEFORE building cumulative cache
        // This ensures cache is invalidated if width changed
        self.render_cache
            .update_wrap_settings(effective_width, use_smart_wrap);
        self.render_cache.content_height = content_height;

        self.viewport.resize(content_width, content_height);

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
            &self.lsp.diagnostics,
            theme,
            config.editor.show_git_diff,
            self.config.word_wrap,
            use_smart_wrap,
            content_width,
            content_height,
        );

        // Blame annotation overlay on the cursor line
        self.render_blame_annotation(buf, area, content_width, content_height, theme);

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
            use unicode_width::UnicodeWidthChar;

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
                            .map(|c| c.width().unwrap_or(0))
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

        // Render hover popup if active
        if let Some(ref popup) = self.lsp.hover_popup {
            // Use stored mouse position for popup placement
            if let Some((mouse_x, mouse_y)) = self.lsp.last_mouse_position {
                self.lsp.hover_popup_rect = popup.render(buf, area, mouse_x, mouse_y, theme);
            } else {
                self.lsp.hover_popup_rect = None;
            }
        } else {
            self.lsp.hover_popup_rect = None;
        }

        // Render color preview popup if active
        if let Some(ref preview) = self.lsp.color_preview {
            preview.render(buf, area);
        }
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
    fn invalidate_cache_after_edit(&mut self, start_line: usize, is_multiline: bool) {
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

    /// Open search modal, optionally restoring and executing previous query.
    ///
    /// If active search exists, restores its state. Otherwise, if a previous query
    /// exists and execute_search is true, executes it immediately.
    pub(crate) fn open_search_modal(&mut self, execute_search: bool) {
        let mut search_modal = SearchModal::new(termide_core::SearchMode::Text);

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

    fn width_preference(&self) -> WidthPreference {
        WidthPreference::PreferWide
    }

    fn title(&self) -> String {
        use termide_config::constants::spinner_frame;

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

        // Upload spinner (takes priority over LSP spinner)
        let upload_indicator = if self.file_state.uploading {
            format!("{} ", spinner_frame())
        } else {
            String::new()
        };

        // LSP loading spinner (only if not uploading)
        let lsp_indicator = if !self.file_state.uploading && self.lsp.server_loading {
            format!("{} ", spinner_frame())
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
            "{}{}{}{}{}{}{}",
            upload_indicator,
            lsp_indicator,
            self.file_state.title,
            modified,
            lsp_status,
            external_change,
            search_info
        )
    }

    fn prepare_render(&mut self, theme: &Theme, config: Arc<Config>) {
        self.render_cache.theme = *theme;
        self.render_cache.config = config.clone();

        // Sync EditorConfig with global Config.editor settings
        // This ensures runtime config changes are applied to the editor
        self.config.word_wrap = config.editor.word_wrap;
        self.config.tab_size = config.editor.tab_size;
        self.config.auto_indent = config.editor.auto_indent;
        self.config.auto_close_brackets = config.editor.auto_close_brackets;

        // Sync highlight cache with theme's light/dark mode and default foreground color
        self.render_cache
            .highlight
            .set_light_theme(theme.is_light_theme());
        self.render_cache.highlight.set_default_fg(theme.fg);
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
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

        // Close hover popups on any key press
        // For Escape, just close the popup and don't process further if one was open
        let had_popup = self.lsp.hover_popup.is_some() || self.lsp.completion_popup.is_some();
        self.close_hover_popup();

        if had_popup && key.code == crossterm::event::KeyCode::Esc {
            // Close completion popup if open
            if self.lsp.completion_popup.is_some() {
                self.cancel_completion();
            }
            // Just close the popup, don't trigger other Escape actions
            return vec![];
        }

        // Note: Key translation should be done at app level before calling handle_key
        // If you need translation, call translate_hotkey from termide-core or keyboard module

        // Collect events from internal state
        let mut events = Vec::new();

        // Handle Vim mode if enabled
        if let Some(ref mut vim_state) = self.vim {
            use crate::vim::{handle_vim_key, VimKeyResult};

            let result = handle_vim_key(vim_state, key);

            match result {
                VimKeyResult::Consumed => return events,
                VimKeyResult::PassThrough => {
                    // In insert mode, fall through to normal editor handling
                }
                VimKeyResult::Unhandled => {
                    // Key not recognized by vim in NORMAL/VISUAL mode - ignore it
                    return events;
                }
                _ => {
                    // Execute vim action
                    if let Some(panel_events) = self.execute_vim_result(result) {
                        events.extend(panel_events);
                    }
                    // Convert status_message to event
                    if let Some(message) = self.status_message.take() {
                        events.push(PanelEvent::SetStatusMessage {
                            message,
                            is_error: false,
                        });
                    }
                    return events;
                }
            }
        }

        let command = keyboard::EditorCommand::from_key_event(
            key,
            self.config.read_only,
            self.search.state.is_some(),
            self.selection.is_some(),
            self.lsp.completion_popup.is_some(),
            &self.config.keybindings,
        );

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
        self.handle_mouse_event(mouse, panel_area)
    }

    fn handle_scroll(&mut self, delta: i32, _panel_area: Rect) -> Vec<PanelEvent> {
        let lines = delta.unsigned_abs() as usize * 3; // 3 lines per scroll unit
        if delta < 0 {
            // Scroll up - check popups first
            if let Some(ref mut popup) = self.lsp.completion_popup {
                popup.scroll_up(lines);
                return vec![];
            }
            if let Some(ref mut popup) = self.lsp.hover_popup {
                popup.scroll_up(lines);
                return vec![];
            }
            // No popup - scroll editor by visual rows (accounts for word wrap)
            self.scroll_visual_rows_up(lines);
        } else {
            // Scroll down - check popups first
            if let Some(ref mut popup) = self.lsp.completion_popup {
                popup.scroll_down(lines);
                return vec![];
            }
            if let Some(ref mut popup) = self.lsp.hover_popup {
                popup.scroll_down(lines);
                return vec![];
            }
            // No popup - scroll editor by visual rows (accounts for word wrap)
            self.scroll_visual_rows_down(lines);
        }
        self.scroll_follows_cursor = false;
        vec![]
    }

    fn tick(&mut self) -> Vec<PanelEvent> {
        // Skip background work when panel is collapsed (stale)
        if self.is_stale {
            return vec![];
        }

        // Handle auto-scroll during selection drag
        if self.tick_auto_scroll() {
            return vec![PanelEvent::NeedsRedraw];
        }

        // Keep redrawing while spinner is animating (upload or LSP loading)
        if self.file_state.uploading || self.lsp.server_loading {
            return vec![PanelEvent::NeedsRedraw];
        }

        // Check if async blame data arrived
        if self.git.poll_blame() {
            return vec![PanelEvent::NeedsRedraw];
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
                Ok(_upload_op) => {
                    // TODO: Handle async upload operation for remote files
                    // For now, remote saves will show progress via VFS status messages
                    CommandResult::SaveResult {
                        success: true,
                        error: None,
                    }
                }
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
            PanelCommand::MarkStale => {
                self.is_stale = true;
                CommandResult::None
            }
            PanelCommand::RefreshIfStale => {
                if self.is_stale {
                    self.is_stale = false;
                    self.check_external_modification();
                    self.update_git_diff();
                    CommandResult::NeedsRedraw(true)
                } else {
                    CommandResult::None
                }
            }
            // Commands not applicable to Editor
            PanelCommand::SetFsWatchRoot { .. }
            | PanelCommand::Resize { .. }
            | PanelCommand::RefreshDirectory
            | PanelCommand::SetGitOperationInProgress { .. }
            | PanelCommand::UpdateRepoPaths { .. } => CommandResult::None,
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
        // Capture Escape when search is active, popups are open, or vim is in INSERT mode
        self.search.state.is_some()
            || self.lsp.completion_popup.is_some()
            || self.lsp.hover_popup.is_some()
            || self
                .vim
                .as_ref()
                .map(|v| v.mode.is_insert())
                .unwrap_or(false)
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
            // ensure_unsaved_buffer_file() must be called before to_session()
            let filename = self.unsaved_buffer_file()?.to_string();

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
