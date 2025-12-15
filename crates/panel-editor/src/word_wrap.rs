//! Word wrap calculations for editor content.
//!
//! This module provides utilities for calculating line wrapping in the editor,
//! including smart wrapping (breaking at word boundaries) and hard wrapping
//! (breaking at fixed column width).

use termide_buffer::{calculate_wrap_point, TextBuffer};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Calculate wrap points for a single line of text.
///
/// Returns (visual_row_count, wrap_points) where wrap_points contains
/// the grapheme indices where each new visual line starts.
///
/// Uses display width and grapheme clusters for proper Unicode handling.
/// This function iterates exactly like rendering does to ensure consistency.
pub fn get_line_wrap_points(
    line_text: &str,
    content_width: usize,
    use_smart_wrap: bool,
) -> (usize, Vec<usize>) {
    if content_width == 0 {
        return (1, Vec::new());
    }

    // Check display width, not char/grapheme count
    let display_width = line_text.width();
    if display_width == 0 {
        return (1, Vec::new());
    }

    if display_width <= content_width {
        return (1, Vec::new()); // No wrapping needed
    }

    // Iterate exactly like rendering does to ensure wrap points match
    let graphemes: Vec<&str> = line_text.graphemes(true).collect();
    let line_len = graphemes.len();
    let mut wrap_points = Vec::new();
    let mut grapheme_offset = 0;

    while grapheme_offset < line_len {
        let chunk_end = if use_smart_wrap {
            calculate_wrap_point(&graphemes, grapheme_offset, content_width, line_len)
        } else {
            calculate_simple_wrap_point(&graphemes, grapheme_offset, content_width)
        };

        // Push chunk_end as start of NEXT visual line (not grapheme_offset!)
        if chunk_end > grapheme_offset && chunk_end < line_len {
            wrap_points.push(chunk_end);
        }

        // Prevent infinite loop
        if chunk_end == grapheme_offset {
            grapheme_offset += 1;
        } else {
            grapheme_offset = chunk_end;
        }
    }

    (wrap_points.len() + 1, wrap_points)
}

/// Calculate simple wrap point for a single visual line (iterative version).
///
/// Returns the grapheme index where the visual line ends.
/// This mirrors the logic in wrap_rendering.rs for consistency.
fn calculate_simple_wrap_point(graphemes: &[&str], start: usize, max_width: usize) -> usize {
    let mut display_width = 0;

    for (i, grapheme) in graphemes.iter().enumerate().skip(start) {
        let grapheme_width = grapheme.width();

        if display_width + grapheme_width > max_width {
            return i;
        }

        display_width += grapheme_width;
    }

    graphemes.len()
}

/// Calculate the visual row index for a cursor position.
///
/// Returns the visual row index from viewport.top_line.
/// This accounts for word wrapping - a single buffer line may span multiple visual rows.
///
/// # Parameters
/// - `buffer`: The text buffer
/// - `cursor_line`: Current cursor line (buffer coordinates)
/// - `cursor_col`: Current cursor column (buffer coordinates)
/// - `viewport_top`: Top line of viewport (buffer coordinates)
/// - `content_width`: Width of content area for wrapping
/// - `word_wrap_enabled`: Whether word wrap is enabled
/// - `use_smart_wrap`: Whether to use smart wrapping
#[allow(dead_code)] // May be used in future phases
pub fn calculate_visual_row_for_cursor(
    buffer: &TextBuffer,
    cursor_line: usize,
    cursor_col: usize,
    viewport_top: usize,
    content_width: usize,
    word_wrap_enabled: bool,
    use_smart_wrap: bool,
) -> usize {
    if content_width == 0 || !word_wrap_enabled {
        // No word wrap - visual row is just buffer line offset from top
        return cursor_line.saturating_sub(viewport_top);
    }

    let mut visual_row = 0;
    let mut line_idx = viewport_top;

    // Count visual rows from viewport top to cursor line
    while line_idx < cursor_line && line_idx < buffer.line_count() {
        if let Some(line_text) = buffer.line(line_idx) {
            let line_text = line_text.trim_end_matches('\n');
            let (line_visual_rows, _) =
                get_line_wrap_points(line_text, content_width, use_smart_wrap);
            visual_row += line_visual_rows;
        } else {
            visual_row += 1; // Empty line = 1 visual row
        }
        line_idx += 1;
    }

    // Now add the visual row within the cursor's line
    if let Some(line_text) = buffer.line(cursor_line) {
        let line_text = line_text.trim_end_matches('\n');
        let (_line_visual_rows, wrap_points) =
            get_line_wrap_points(line_text, content_width, use_smart_wrap);

        // Find which visual row within this line the cursor is on
        let cursor_col_clamped = cursor_col.min(line_text.graphemes(true).count());
        let row_within_line = wrap_points
            .iter()
            .filter(|&&wp| wp <= cursor_col_clamped)
            .count();
        visual_row += row_within_line;
    }

    visual_row
}

/// Calculate total number of visual rows in the entire buffer.
///
/// This accounts for word wrapping - returns total visual rows across all lines.
pub fn calculate_total_visual_rows(
    buffer: &TextBuffer,
    content_width: usize,
    word_wrap_enabled: bool,
    use_smart_wrap: bool,
) -> usize {
    if content_width == 0 || !word_wrap_enabled {
        // No word wrap - just return buffer line count
        return buffer.line_count();
    }

    let mut total_visual_rows = 0;

    for line_idx in 0..buffer.line_count() {
        if let Some(line_text) = buffer.line(line_idx) {
            let line_text = line_text.trim_end_matches('\n');
            let (line_visual_rows, _) =
                get_line_wrap_points(line_text, content_width, use_smart_wrap);
            total_visual_rows += line_visual_rows;
        } else {
            total_visual_rows += 1; // Empty line = 1 visual row
        }
    }

    total_visual_rows
}

/// Convert visual row to buffer position accounting for word wrap.
///
/// Returns (buffer_line, column_offset, chunk_end) for the given visual row.
/// - `buffer_line`: The buffer line index
/// - `column_offset`: Grapheme index where this visual line starts
/// - `chunk_end`: Grapheme index where this visual line ends (exclusive)
///
/// # Parameters
/// - `buffer`: The text buffer
/// - `visual_row`: Visual row index relative to viewport
/// - `viewport_top`: Top line of viewport (buffer coordinates)
/// - `content_width`: Width of content area for wrapping
/// - `use_smart_wrap`: Whether to use smart wrapping
///
/// This function iterates through wrap points exactly like rendering does,
/// ensuring perfect consistency between displayed text and mouse positions.
pub fn visual_row_to_buffer_position(
    buffer: &TextBuffer,
    visual_row: usize,
    viewport_top: usize,
    content_width: usize,
    use_smart_wrap: bool,
) -> (usize, usize, usize) {
    if content_width == 0 {
        let line_len = buffer
            .line(viewport_top + visual_row)
            .map(|s| s.trim_end_matches('\n').graphemes(true).count())
            .unwrap_or(0);
        return (viewport_top + visual_row, 0, line_len);
    }

    let mut current_visual_row = 0;
    let mut line_idx = viewport_top;

    while line_idx < buffer.line_count() {
        if let Some(line_text) = buffer.line(line_idx) {
            let line_text = line_text.trim_end_matches('\n');
            let graphemes: Vec<&str> = line_text.graphemes(true).collect();
            let line_len = graphemes.len();

            // Handle empty lines (1 visual row)
            if line_len == 0 {
                if current_visual_row == visual_row {
                    return (line_idx, 0, 0);
                }
                current_visual_row += 1;
                line_idx += 1;
                continue;
            }

            // Iterate exactly like rendering does
            let mut grapheme_offset = 0;

            while grapheme_offset < line_len {
                let chunk_end = if use_smart_wrap {
                    calculate_wrap_point(&graphemes, grapheme_offset, content_width, line_len)
                } else {
                    calculate_simple_wrap_point(&graphemes, grapheme_offset, content_width)
                };

                if current_visual_row == visual_row {
                    // Found the target visual row
                    return (line_idx, grapheme_offset, chunk_end);
                }

                current_visual_row += 1;

                // Safety: prevent infinite loop
                if chunk_end == grapheme_offset {
                    grapheme_offset += 1;
                } else {
                    grapheme_offset = chunk_end;
                }
            }
        } else {
            // If line doesn't exist, treat as empty (1 visual row)
            if current_visual_row == visual_row {
                return (line_idx, 0, 0);
            }
            current_visual_row += 1;
        }

        line_idx += 1;
    }

    // If we've exhausted all lines, return the last line
    let last_line = buffer.line_count().saturating_sub(1);
    let last_line_len = buffer
        .line(last_line)
        .map(|s| s.trim_end_matches('\n').graphemes(true).count())
        .unwrap_or(0);
    (last_line, 0, last_line_len)
}
