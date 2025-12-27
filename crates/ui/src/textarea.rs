//! Multi-line text area handler with cursor management and selection support.
//!
//! Extends the concept of TextInput for multi-line text editing with:
//! - Multiple lines storage
//! - 2D cursor navigation (row, col)
//! - Multi-line selection
//! - Clipboard support for multi-line text
//! - Undo/Redo history

/// Position in the text area (row, column in characters)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CursorPos {
    pub row: usize,
    pub col: usize,
}

impl CursorPos {
    pub fn new(row: usize, col: usize) -> Self {
        Self { row, col }
    }
}

impl PartialOrd for CursorPos {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CursorPos {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.row.cmp(&other.row) {
            std::cmp::Ordering::Equal => self.col.cmp(&other.col),
            ord => ord,
        }
    }
}

/// Multi-line text area handler with selection and undo support.
#[derive(Debug, Clone)]
pub struct TextArea {
    /// Lines of text
    lines: Vec<String>,
    /// Current cursor position (active point)
    cursor: CursorPos,
    /// Selection anchor (None = no selection)
    selection_anchor: Option<CursorPos>,
    /// Undo history: (lines, cursor)
    undo_stack: Vec<(Vec<String>, CursorPos)>,
    /// Redo history: (lines, cursor)
    redo_stack: Vec<(Vec<String>, CursorPos)>,
    /// Scroll offset for vertical scrolling
    scroll_offset: usize,
}

impl Default for TextArea {
    fn default() -> Self {
        Self::new()
    }
}

impl TextArea {
    /// Create a new empty text area.
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: CursorPos::default(),
            selection_anchor: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            scroll_offset: 0,
        }
    }

    /// Create a text area with initial text.
    pub fn with_text(text: &str) -> Self {
        let lines: Vec<String> = if text.is_empty() {
            vec![String::new()]
        } else {
            text.lines().map(String::from).collect()
        };
        let row = lines.len().saturating_sub(1);
        let col = lines.last().map(|l| l.chars().count()).unwrap_or(0);

        Self {
            lines,
            cursor: CursorPos::new(row, col),
            selection_anchor: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            scroll_offset: 0,
        }
    }

    // === Getters ===

    /// Get all lines.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Get the full text content.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Get cursor position.
    pub fn cursor(&self) -> CursorPos {
        self.cursor
    }

    /// Get scroll offset.
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Set scroll offset.
    pub fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset.min(self.lines.len().saturating_sub(1));
    }

    /// Get number of lines.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Check if text is empty.
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// Get current line.
    fn current_line(&self) -> &str {
        self.lines
            .get(self.cursor.row)
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// Get current line length in characters.
    fn current_line_len(&self) -> usize {
        self.current_line().chars().count()
    }

    // === Undo/Redo ===

    fn save_undo_state(&mut self) {
        // Don't save if state hasn't changed
        if let Some((last_lines, _)) = self.undo_stack.last() {
            if last_lines == &self.lines {
                return;
            }
        }
        self.undo_stack.push((self.lines.clone(), self.cursor));
        self.redo_stack.clear();

        const MAX_UNDO_HISTORY: usize = 100;
        if self.undo_stack.len() > MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
    }

    /// Undo last change.
    pub fn undo(&mut self) -> bool {
        if let Some((lines, cursor)) = self.undo_stack.pop() {
            self.redo_stack.push((self.lines.clone(), self.cursor));
            self.lines = lines;
            self.cursor = cursor;
            self.selection_anchor = None;
            true
        } else {
            false
        }
    }

    /// Redo last undone change.
    pub fn redo(&mut self) -> bool {
        if let Some((lines, cursor)) = self.redo_stack.pop() {
            self.undo_stack.push((self.lines.clone(), self.cursor));
            self.lines = lines;
            self.cursor = cursor;
            self.selection_anchor = None;
            true
        } else {
            false
        }
    }

    // === Selection ===

    /// Check if has selection.
    pub fn has_selection(&self) -> bool {
        self.selection_anchor
            .is_some_and(|anchor| anchor != self.cursor)
    }

    /// Get selection range (start, end) positions.
    pub fn selection_range(&self) -> Option<(CursorPos, CursorPos)> {
        self.selection_anchor.map(|anchor| {
            if anchor <= self.cursor {
                (anchor, self.cursor)
            } else {
                (self.cursor, anchor)
            }
        })
    }

    /// Get selected text.
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        if start == end {
            return None;
        }

        if start.row == end.row {
            // Single line selection
            let line = &self.lines[start.row];
            let start_byte = char_to_byte_index(line, start.col);
            let end_byte = char_to_byte_index(line, end.col);
            Some(line[start_byte..end_byte].to_string())
        } else {
            // Multi-line selection
            let mut result = String::new();

            // First line (from start.col to end)
            let first_line = &self.lines[start.row];
            let start_byte = char_to_byte_index(first_line, start.col);
            result.push_str(&first_line[start_byte..]);

            // Middle lines (complete)
            for row in (start.row + 1)..end.row {
                result.push('\n');
                result.push_str(&self.lines[row]);
            }

            // Last line (from start to end.col)
            result.push('\n');
            let last_line = &self.lines[end.row];
            let end_byte = char_to_byte_index(last_line, end.col);
            result.push_str(&last_line[..end_byte]);

            Some(result)
        }
    }

    /// Select all text.
    pub fn select_all(&mut self) {
        if !self.is_empty() {
            self.selection_anchor = Some(CursorPos::new(0, 0));
            let last_row = self.lines.len() - 1;
            let last_col = self.lines[last_row].chars().count();
            self.cursor = CursorPos::new(last_row, last_col);
        }
    }

    /// Start selection at current position.
    pub fn start_selection(&mut self) {
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor);
        }
    }

    /// Clear selection.
    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }

    /// Delete selected text.
    pub fn delete_selection(&mut self) -> bool {
        if let Some((start, end)) = self.selection_range() {
            if start != end {
                self.save_undo_state();
                self.delete_selection_internal();
                return true;
            }
        }
        self.selection_anchor = None;
        false
    }

    fn delete_selection_internal(&mut self) {
        let Some((start, end)) = self.selection_range() else {
            return;
        };
        if start == end {
            self.selection_anchor = None;
            return;
        }

        if start.row == end.row {
            // Single line deletion
            let line = &mut self.lines[start.row];
            let start_byte = char_to_byte_index(line, start.col);
            let end_byte = char_to_byte_index(line, end.col);
            line.replace_range(start_byte..end_byte, "");
        } else {
            // Multi-line deletion
            let first_line = &self.lines[start.row];
            let start_byte = char_to_byte_index(first_line, start.col);
            let first_part = first_line[..start_byte].to_string();

            let last_line = &self.lines[end.row];
            let end_byte = char_to_byte_index(last_line, end.col);
            let last_part = last_line[end_byte..].to_string();

            // Combine first and last parts
            self.lines[start.row] = first_part + &last_part;

            // Remove middle and last lines
            self.lines.drain((start.row + 1)..=end.row);
        }

        self.cursor = start;
        self.selection_anchor = None;
    }

    // === Text modification ===

    /// Insert a character at cursor.
    pub fn insert(&mut self, c: char) {
        if c == '\n' {
            self.insert_newline();
            return;
        }

        self.save_undo_state();
        self.delete_selection_internal();

        let line = &mut self.lines[self.cursor.row];
        let byte_idx = char_to_byte_index(line, self.cursor.col);
        line.insert(byte_idx, c);
        self.cursor.col += 1;
    }

    /// Insert a string at cursor (handles multi-line).
    pub fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }

        self.save_undo_state();
        self.delete_selection_internal();

        let mut lines_to_insert: Vec<&str> = s.split('\n').collect();

        if lines_to_insert.len() == 1 {
            // Single line insert
            let line = &mut self.lines[self.cursor.row];
            let byte_idx = char_to_byte_index(line, self.cursor.col);
            line.insert_str(byte_idx, s);
            self.cursor.col += s.chars().count();
        } else {
            // Multi-line insert
            let current_line = &self.lines[self.cursor.row];
            let byte_idx = char_to_byte_index(current_line, self.cursor.col);
            let before = current_line[..byte_idx].to_string();
            let after = current_line[byte_idx..].to_string();

            // First line: before + first part of inserted text
            let first_insert = lines_to_insert.remove(0);
            self.lines[self.cursor.row] = before + first_insert;

            // Last line: last part of inserted text + after
            let last_insert = lines_to_insert.pop().unwrap_or("");
            let last_line = last_insert.to_string() + &after;

            // Insert middle lines and last line
            let insert_row = self.cursor.row + 1;
            for (i, line) in lines_to_insert.iter().enumerate() {
                self.lines.insert(insert_row + i, line.to_string());
            }
            self.lines
                .insert(insert_row + lines_to_insert.len(), last_line);

            // Update cursor
            self.cursor.row += lines_to_insert.len() + 1;
            self.cursor.col = last_insert.chars().count();
        }
    }

    /// Insert a newline (Enter key).
    pub fn insert_newline(&mut self) {
        self.save_undo_state();
        self.delete_selection_internal();

        let line = &self.lines[self.cursor.row];
        let byte_idx = char_to_byte_index(line, self.cursor.col);
        let after = line[byte_idx..].to_string();
        self.lines[self.cursor.row].truncate(byte_idx);

        self.cursor.row += 1;
        self.cursor.col = 0;
        self.lines.insert(self.cursor.row, after);
    }

    /// Delete character before cursor (Backspace).
    pub fn backspace(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }

        if self.cursor.col > 0 {
            self.save_undo_state();
            self.cursor.col -= 1;
            let line = &mut self.lines[self.cursor.row];
            let byte_idx = char_to_byte_index(line, self.cursor.col);
            line.remove(byte_idx);
            true
        } else if self.cursor.row > 0 {
            // Join with previous line
            self.save_undo_state();
            let current_line = self.lines.remove(self.cursor.row);
            self.cursor.row -= 1;
            self.cursor.col = self.lines[self.cursor.row].chars().count();
            self.lines[self.cursor.row].push_str(&current_line);
            true
        } else {
            false
        }
    }

    /// Delete character at cursor (Delete key).
    pub fn delete(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }

        let line_len = self.current_line_len();
        if self.cursor.col < line_len {
            self.save_undo_state();
            let line = &mut self.lines[self.cursor.row];
            let byte_idx = char_to_byte_index(line, self.cursor.col);
            line.remove(byte_idx);
            true
        } else if self.cursor.row + 1 < self.lines.len() {
            // Join with next line
            self.save_undo_state();
            let next_line = self.lines.remove(self.cursor.row + 1);
            self.lines[self.cursor.row].push_str(&next_line);
            true
        } else {
            false
        }
    }

    // === Navigation ===

    /// Move cursor left.
    pub fn move_left(&mut self) -> bool {
        self.clear_selection();
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
            true
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.col = self.current_line_len();
            true
        } else {
            false
        }
    }

    /// Move cursor right.
    pub fn move_right(&mut self) -> bool {
        self.clear_selection();
        let line_len = self.current_line_len();
        if self.cursor.col < line_len {
            self.cursor.col += 1;
            true
        } else if self.cursor.row + 1 < self.lines.len() {
            self.cursor.row += 1;
            self.cursor.col = 0;
            true
        } else {
            false
        }
    }

    /// Move cursor up.
    pub fn move_up(&mut self) -> bool {
        self.clear_selection();
        if self.cursor.row > 0 {
            self.cursor.row -= 1;
            let line_len = self.current_line_len();
            self.cursor.col = self.cursor.col.min(line_len);
            true
        } else {
            false
        }
    }

    /// Move cursor down.
    pub fn move_down(&mut self) -> bool {
        self.clear_selection();
        if self.cursor.row + 1 < self.lines.len() {
            self.cursor.row += 1;
            let line_len = self.current_line_len();
            self.cursor.col = self.cursor.col.min(line_len);
            true
        } else {
            false
        }
    }

    /// Move to start of line (Home).
    pub fn move_home(&mut self) {
        self.clear_selection();
        self.cursor.col = 0;
    }

    /// Move to end of line (End).
    pub fn move_end(&mut self) {
        self.clear_selection();
        self.cursor.col = self.current_line_len();
    }

    /// Move to start of text (Ctrl+Home).
    pub fn move_to_start(&mut self) {
        self.clear_selection();
        self.cursor = CursorPos::default();
    }

    /// Move to end of text (Ctrl+End).
    pub fn move_to_end(&mut self) {
        self.clear_selection();
        self.cursor.row = self.lines.len().saturating_sub(1);
        self.cursor.col = self.current_line_len();
    }

    // === Selection movement ===

    /// Move left with selection.
    pub fn move_left_with_selection(&mut self) -> bool {
        self.start_selection();
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
            true
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.col = self.current_line_len();
            true
        } else {
            false
        }
    }

    /// Move right with selection.
    pub fn move_right_with_selection(&mut self) -> bool {
        self.start_selection();
        let line_len = self.current_line_len();
        if self.cursor.col < line_len {
            self.cursor.col += 1;
            true
        } else if self.cursor.row + 1 < self.lines.len() {
            self.cursor.row += 1;
            self.cursor.col = 0;
            true
        } else {
            false
        }
    }

    /// Move up with selection.
    pub fn move_up_with_selection(&mut self) -> bool {
        self.start_selection();
        if self.cursor.row > 0 {
            self.cursor.row -= 1;
            let line_len = self.current_line_len();
            self.cursor.col = self.cursor.col.min(line_len);
            true
        } else {
            false
        }
    }

    /// Move down with selection.
    pub fn move_down_with_selection(&mut self) -> bool {
        self.start_selection();
        if self.cursor.row + 1 < self.lines.len() {
            self.cursor.row += 1;
            let line_len = self.current_line_len();
            self.cursor.col = self.cursor.col.min(line_len);
            true
        } else {
            false
        }
    }

    /// Move to home with selection.
    pub fn move_home_with_selection(&mut self) {
        self.start_selection();
        self.cursor.col = 0;
    }

    /// Move to end with selection.
    pub fn move_end_with_selection(&mut self) {
        self.start_selection();
        self.cursor.col = self.current_line_len();
    }

    // === Scrolling ===

    /// Ensure cursor is visible within given height.
    pub fn ensure_cursor_visible(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
        if self.cursor.row < self.scroll_offset {
            self.scroll_offset = self.cursor.row;
        } else if self.cursor.row >= self.scroll_offset + visible_height {
            self.scroll_offset = self.cursor.row - visible_height + 1;
        }
    }

    /// Set cursor position directly (for mouse clicks).
    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.clear_selection();
        self.cursor.row = row.min(self.lines.len().saturating_sub(1));
        self.cursor.col = col.min(self.current_line_len());
    }
}

/// Convert character position to byte index in a string.
fn char_to_byte_index(s: &str, char_pos: usize) -> usize {
    s.char_indices()
        .nth(char_pos)
        .map(|(idx, _)| idx)
        .unwrap_or(s.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_textarea() {
        let ta = TextArea::new();
        assert_eq!(ta.lines().len(), 1);
        assert_eq!(ta.lines()[0], "");
        assert!(ta.is_empty());
    }

    #[test]
    fn test_with_text() {
        let ta = TextArea::with_text("hello\nworld");
        assert_eq!(ta.lines().len(), 2);
        assert_eq!(ta.lines()[0], "hello");
        assert_eq!(ta.lines()[1], "world");
        assert_eq!(ta.cursor(), CursorPos::new(1, 5));
    }

    #[test]
    fn test_insert_char() {
        let mut ta = TextArea::new();
        ta.insert('a');
        ta.insert('b');
        assert_eq!(ta.text(), "ab");
        assert_eq!(ta.cursor(), CursorPos::new(0, 2));
    }

    #[test]
    fn test_insert_newline() {
        let mut ta = TextArea::with_text("hello");
        ta.set_cursor(0, 2);
        ta.insert_newline();
        assert_eq!(ta.lines().len(), 2);
        assert_eq!(ta.lines()[0], "he");
        assert_eq!(ta.lines()[1], "llo");
    }

    #[test]
    fn test_backspace_join_lines() {
        let mut ta = TextArea::with_text("hello\nworld");
        ta.set_cursor(1, 0);
        ta.backspace();
        assert_eq!(ta.lines().len(), 1);
        assert_eq!(ta.text(), "helloworld");
    }

    #[test]
    fn test_navigation() {
        let mut ta = TextArea::with_text("ab\ncd");
        ta.set_cursor(0, 0);

        ta.move_right();
        assert_eq!(ta.cursor(), CursorPos::new(0, 1));

        ta.move_down();
        assert_eq!(ta.cursor(), CursorPos::new(1, 1));

        ta.move_left();
        assert_eq!(ta.cursor(), CursorPos::new(1, 0));

        ta.move_up();
        assert_eq!(ta.cursor(), CursorPos::new(0, 0));
    }

    #[test]
    fn test_selection() {
        let mut ta = TextArea::with_text("hello");
        ta.set_cursor(0, 0);
        ta.select_all();
        assert!(ta.has_selection());
        assert_eq!(ta.selected_text(), Some("hello".to_string()));
    }
}
