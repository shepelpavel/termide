//! Word boundary detection for cursor navigation.
//!
//! Provides character classification and word boundary finding functions
//! used by both normal mode (Ctrl+Left/Right) and Vim mode word motions.

/// Character type for word boundary detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharType {
    Word,
    Punctuation,
    Space,
}

/// Determine character type for word motion.
pub(crate) fn char_type(grapheme: &str) -> CharType {
    if let Some(ch) = grapheme.chars().next() {
        if ch.is_whitespace() {
            CharType::Space
        } else if ch.is_alphanumeric() || ch == '_' {
            CharType::Word
        } else {
            CharType::Punctuation
        }
    } else {
        CharType::Space
    }
}

/// Find next word start position on current line.
///
/// Returns the column index of the next word start, or None if no word start
/// exists after `start_col` on this line.
pub(crate) fn find_next_word_start(graphemes: &[&str], start_col: usize) -> Option<usize> {
    if start_col >= graphemes.len() {
        return None;
    }

    let mut pos = start_col;
    let start_type = char_type(graphemes.get(pos).copied().unwrap_or(" "));

    // Skip current word
    while pos < graphemes.len() && char_type(graphemes[pos]) == start_type {
        pos += 1;
    }

    // Skip whitespace
    while pos < graphemes.len() && char_type(graphemes[pos]) == CharType::Space {
        pos += 1;
    }

    if pos < graphemes.len() {
        Some(pos)
    } else {
        None
    }
}

/// Find previous word start position on current line.
///
/// Returns the column index of the previous word start, or None if no word start
/// exists before `start_col` on this line.
pub fn find_prev_word_start(graphemes: &[&str], start_col: usize) -> Option<usize> {
    if start_col == 0 || graphemes.is_empty() {
        return None;
    }

    let mut pos = start_col.min(graphemes.len()) - 1;

    // Skip whitespace before current position
    while pos > 0 && char_type(graphemes[pos]) == CharType::Space {
        pos -= 1;
    }

    if pos == 0 {
        return Some(0);
    }

    let word_type = char_type(graphemes[pos]);

    // Move back to start of word
    while pos > 0 && char_type(graphemes[pos - 1]) == word_type {
        pos -= 1;
    }

    Some(pos)
}
