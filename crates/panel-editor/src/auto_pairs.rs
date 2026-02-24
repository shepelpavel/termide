//! Auto-closing bracket and quote pairs.
//!
//! Provides matching pairs for brackets and quotes, with context-aware
//! logic for when to auto-close (e.g., don't close apostrophes mid-word).

/// Bracket and quote pairs: (opening, closing).
pub const PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\'')];

/// Get the closing character for an opening character.
pub fn closing_char(open: char) -> Option<char> {
    PAIRS.iter().find(|(o, _)| *o == open).map(|(_, c)| *c)
}

/// Check if a character is a closing bracket/quote from a known pair.
pub fn is_closing(ch: char) -> bool {
    PAIRS.iter().any(|(_, c)| *c == ch)
}

/// Check if a character is a quote character (same open and close).
fn is_quote(ch: char) -> bool {
    ch == '"' || ch == '\''
}

/// Determine whether a quote should be auto-closed in the current context.
///
/// Don't auto-close after alphanumeric characters to avoid interfering
/// with apostrophes in words like "it's", "don't".
pub fn should_auto_close(ch: char, char_before: Option<char>) -> bool {
    if is_quote(ch) {
        // Don't auto-close quote after alphanumeric (handles apostrophes)
        !char_before.is_some_and(|c| c.is_alphanumeric())
    } else {
        true
    }
}

/// Check if the opening and closing characters form a known pair.
pub fn is_matching_pair(open: char, close: char) -> bool {
    PAIRS.iter().any(|(o, c)| *o == open && *c == close)
}
