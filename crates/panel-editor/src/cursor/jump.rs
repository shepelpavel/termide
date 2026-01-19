//! Jump cursor movement operations (page up/down).
//!
//! This module provides page-based cursor movement operations.

use termide_buffer::{Cursor, TextBuffer};

/// Move cursor one page up.
///
/// If less than a page remains to the start, moves cursor to position (0, 0).
/// Returns (should_scroll_viewport, scroll_amount).
pub fn page_up(cursor: &mut Cursor, page_size: usize) -> (bool, usize) {
    if cursor.line < page_size {
        // Less than a page to start - move to beginning of file
        cursor.line = 0;
        cursor.column = 0;
        (true, page_size)
    } else {
        cursor.move_up(page_size);
        (true, page_size)
    }
}

/// Move cursor one page down.
///
/// If less than a page remains to the end, moves cursor to end of file.
/// Returns (should_scroll_viewport, scroll_amount).
pub fn page_down(cursor: &mut Cursor, buffer: &TextBuffer, page_size: usize) -> (bool, usize) {
    let max_line = buffer.line_count().saturating_sub(1);
    let remaining = max_line.saturating_sub(cursor.line);

    if remaining < page_size {
        // Less than a page to end - move to end of file
        cursor.line = max_line;
        cursor.column = buffer.line_len_graphemes(max_line);
        (true, page_size)
    } else {
        cursor.move_down(page_size, max_line);
        (true, page_size)
    }
}
