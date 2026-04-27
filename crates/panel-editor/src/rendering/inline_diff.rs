//! Inline diff rendering for word-level change highlighting.
//!
//! This module provides functions for building visual lines that display
//! inline differences between original and current text, showing both
//! deleted (red) and inserted (green) text segments.

use ratatui::style::{Color, Modifier, Style};

use termide_git::{InlineChange, InlineChangeType};

/// Segment of visual line with its change type (borrows text from InlineChange).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualSegment<'a> {
    pub text: &'a str,
    pub change_type: InlineChangeType,
}

/// Build a visual line from inline changes.
///
/// Converts inline diff changes into segments suitable for rendering.
/// Deleted text is included in the visual output (will be rendered
/// at the position where it was deleted).
///
/// # Returns
/// Vec of segments in display order, including both deleted and current text.
pub(crate) fn build_visual_line(inline_changes: &[InlineChange]) -> Vec<VisualSegment<'_>> {
    inline_changes
        .iter()
        .map(|change| VisualSegment {
            text: &change.text,
            change_type: change.change_type,
        })
        .collect()
}

/// Apply diff styles to visual segments.
///
/// Merges syntax highlighting styles with diff-specific styling:
/// - Unchanged: keeps original syntax style
/// - Deleted: red background with dimmed foreground, optionally strikethrough
/// - Inserted: green background, preserving syntax foreground color
///
/// # Arguments
/// - `visual_segments` - segments from `build_visual_line()`
/// - `deleted_bg` - background color for deleted text (typically error/red)
/// - `inserted_bg` - background color for inserted text (typically success/green)
/// - `base_fg` - default foreground color for text
pub fn apply_diff_style(
    change_type: InlineChangeType,
    base_style: Style,
    deleted_bg: Color,
    inserted_bg: Color,
) -> Style {
    match change_type {
        InlineChangeType::Unchanged => base_style,
        InlineChangeType::Deleted => {
            // Deleted text: red background, dimmed foreground
            Style::default()
                .bg(deleted_bg)
                .fg(Color::Rgb(180, 140, 140)) // Dimmed red-ish text
                .add_modifier(Modifier::CROSSED_OUT)
        }
        InlineChangeType::Inserted => {
            // Inserted text: green background, keep syntax fg if available
            let fg = base_style.fg.unwrap_or(Color::White);
            Style::default().bg(inserted_bg).fg(fg)
        }
    }
}

/// Calculate extra visual width added by deleted text.
///
/// Deleted text is shown visually but doesn't exist in the buffer,
/// so we need to account for this when calculating positions.
pub fn calculate_deleted_width(inline_changes: &[InlineChange]) -> usize {
    use unicode_width::UnicodeWidthStr;

    inline_changes
        .iter()
        .filter(|c| c.change_type == InlineChangeType::Deleted)
        .map(|c| c.text.width())
        .sum()
}

/// Convert buffer column to visual column.
///
/// Accounts for deleted text that appears before the given buffer position.
/// Deleted text is rendered but doesn't exist in the buffer.
pub fn buffer_to_visual_col(buffer_col: usize, inline_changes: &[InlineChange]) -> usize {
    use unicode_width::UnicodeWidthStr;

    let mut visual_col = 0;
    let mut buffer_pos = 0;

    for change in inline_changes {
        let text_width = change.text.width();

        match change.change_type {
            InlineChangeType::Deleted => {
                // Deleted text adds to visual but not buffer
                visual_col += text_width;
            }
            InlineChangeType::Unchanged | InlineChangeType::Inserted => {
                // Check if target is within this segment
                if buffer_pos + text_width > buffer_col {
                    // Target is in this segment
                    visual_col += buffer_col - buffer_pos;
                    return visual_col;
                }
                buffer_pos += text_width;
                visual_col += text_width;
            }
        }
    }

    visual_col
}

/// Convert visual column to buffer column.
///
/// Accounts for deleted text when mapping visual position to buffer position.
pub fn visual_to_buffer_col(visual_col: usize, inline_changes: &[InlineChange]) -> usize {
    use unicode_width::UnicodeWidthStr;

    let mut current_visual = 0;
    let mut buffer_col = 0;

    for change in inline_changes {
        let text_width = change.text.width();

        if current_visual + text_width > visual_col {
            // Target is within this segment
            let offset = visual_col - current_visual;
            return match change.change_type {
                InlineChangeType::Deleted => buffer_col, // Clicking on deleted = position after
                _ => buffer_col + offset,
            };
        }

        current_visual += text_width;
        if change.change_type != InlineChangeType::Deleted {
            buffer_col += text_width;
        }
    }

    buffer_col
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_change(text: &str, change_type: InlineChangeType) -> InlineChange {
        InlineChange {
            text: text.to_string(),
            change_type,
        }
    }

    #[test]
    fn test_build_visual_line() {
        let changes = vec![
            make_change("Hello ", InlineChangeType::Unchanged),
            make_change("world", InlineChangeType::Deleted),
            make_change("beautiful world", InlineChangeType::Inserted),
        ];

        let segments = build_visual_line(&changes);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].text, "Hello ");
        assert_eq!(segments[1].text, "world");
        assert_eq!(segments[2].text, "beautiful world");
    }

    #[test]
    fn test_calculate_deleted_width() {
        let changes = vec![
            make_change("Hello ", InlineChangeType::Unchanged),
            make_change("world", InlineChangeType::Deleted),
            make_change("beautiful world", InlineChangeType::Inserted),
        ];

        assert_eq!(calculate_deleted_width(&changes), 5); // "world" = 5 chars
    }

    #[test]
    fn test_buffer_to_visual_col() {
        // "Hello " -> "Hello beautiful "
        // Changes: "Hello "(unchanged) + ""(deleted) + "beautiful "(inserted)
        let changes = vec![
            make_change("Hello ", InlineChangeType::Unchanged),
            make_change("old", InlineChangeType::Deleted), // 3 chars deleted
            make_change("new", InlineChangeType::Inserted),
        ];

        // Buffer col 0 -> visual col 0 (at "H")
        assert_eq!(buffer_to_visual_col(0, &changes), 0);

        // Buffer col 6 -> visual col 9 (after "Hello " and "old" deleted)
        assert_eq!(buffer_to_visual_col(6, &changes), 9);
    }

    #[test]
    fn test_visual_to_buffer_col() {
        let changes = vec![
            make_change("Hello ", InlineChangeType::Unchanged),
            make_change("old", InlineChangeType::Deleted),
            make_change("new", InlineChangeType::Inserted),
        ];

        // Visual col 0 -> buffer col 0
        assert_eq!(visual_to_buffer_col(0, &changes), 0);

        // Visual col in deleted region -> buffer col after unchanged
        assert_eq!(visual_to_buffer_col(7, &changes), 6);
    }

    #[test]
    fn test_apply_diff_style_unchanged() {
        let base = Style::default().fg(Color::Cyan);
        let result = apply_diff_style(InlineChangeType::Unchanged, base, Color::Red, Color::Green);
        assert_eq!(result, base);
    }

    #[test]
    fn test_apply_diff_style_deleted() {
        let base = Style::default().fg(Color::White);
        let result = apply_diff_style(InlineChangeType::Deleted, base, Color::Red, Color::Green);
        assert_eq!(result.bg, Some(Color::Red));
        assert!(result.add_modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn test_apply_diff_style_inserted() {
        let base = Style::default().fg(Color::Cyan);
        let result = apply_diff_style(InlineChangeType::Inserted, base, Color::Red, Color::Green);
        assert_eq!(result.bg, Some(Color::Green));
        assert_eq!(result.fg, Some(Color::Cyan)); // Preserves syntax color
    }
}
