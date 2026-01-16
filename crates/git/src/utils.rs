//! Utility functions for git panels.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Truncate a string to fit within a given display width.
///
/// Respects Unicode character widths (e.g., CJK characters count as 2).
pub fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut width = 0;

    for c in s.chars() {
        let char_width = c.width().unwrap_or(0);
        if width + char_width > max_width {
            break;
        }
        result.push(c);
        width += char_width;
    }

    result
}

/// Truncate a string from the left with ellipsis prefix.
///
/// Keeps the rightmost (most relevant) part of the string.
/// Respects Unicode character widths (e.g., CJK characters count as 2).
pub fn truncate_path_left(s: &str, max_width: usize) -> String {
    let current_width = s.width();
    if current_width <= max_width {
        return s.to_string();
    }

    let ellipsis = "...";
    let ellipsis_width = 3;
    let available = max_width.saturating_sub(ellipsis_width);

    // Take characters from the right until we reach available width
    let mut result = String::new();
    let mut width = 0;

    for c in s.chars().rev() {
        let char_width = c.width().unwrap_or(0);
        if width + char_width > available {
            break;
        }
        result.insert(0, c);
        width += char_width;
    }

    format!("{}{}", ellipsis, result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_ascii() {
        assert_eq!(truncate_to_width("hello world", 5), "hello");
        assert_eq!(truncate_to_width("hello", 10), "hello");
        assert_eq!(truncate_to_width("", 5), "");
    }

    #[test]
    fn test_truncate_unicode() {
        // CJK characters are width 2
        assert_eq!(truncate_to_width("日本語", 4), "日本");
        assert_eq!(truncate_to_width("日本語", 5), "日本");
        assert_eq!(truncate_to_width("日本語", 6), "日本語");
    }

    #[test]
    fn test_truncate_mixed() {
        assert_eq!(truncate_to_width("a日b本c", 4), "a日b");
    }
}
