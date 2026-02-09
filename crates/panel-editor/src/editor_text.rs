//! Text editing methods for the Editor.
//!
//! This module contains text editing functionality including:
//! - Clipboard operations (copy, cut, paste)
//! - Character insertion and deletion
//! - Line operations (duplicate, indent, unindent)
//! - Newline insertion with auto-indentation

use anyhow::Result;
use termide_buffer::Cursor;

use crate::{clipboard, cursor, selection, text_editing};

use super::Editor;

impl Editor {
    // =========================================================================
    // Selection Helpers (Private)
    // =========================================================================

    /// Get selected text
    pub(crate) fn get_selected_text(&self) -> Option<String> {
        selection::get_selected_text(&self.buffer, self.selection.as_ref())
    }

    /// Delete selected text
    pub(crate) fn delete_selection(&mut self) -> Result<()> {
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

    // =========================================================================
    // Clipboard Operations
    // =========================================================================

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

        // Normalize line endings: some terminals (VTE/gnome-terminal) send \r
        // for newlines in bracketed paste instead of \n
        let text = text.replace("\r\n", "\n").replace('\r', "\n");

        // Close search mode when editing begins
        self.close_search();

        // Delete selected text before pasting
        self.delete_selection()?;

        // Insert text at cursor position
        let start_line = self.cursor.line;
        let new_cursor = self.buffer.insert(&self.cursor, &text)?;
        let is_multiline = text.contains('\n');

        self.cursor = new_cursor;
        self.input.preferred_column = None;
        self.clamp_cursor();

        // Invalidate highlighting cache and schedule git update
        self.invalidate_cache_after_edit(start_line, is_multiline);

        Ok(())
    }

    // =========================================================================
    // Character Operations
    // =========================================================================

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

    // =========================================================================
    // Line Operations
    // =========================================================================

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

    // =========================================================================
    // Cursor Helpers (Private)
    // =========================================================================

    /// Clamp cursor position to valid values
    pub(crate) fn clamp_cursor(&mut self) {
        cursor::physical::clamp_cursor(&mut self.cursor, &self.buffer);
    }
}
