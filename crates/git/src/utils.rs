//! Utility functions for git panels.

use unicode_width::UnicodeWidthChar;

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
