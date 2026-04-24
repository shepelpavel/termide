//! Syntax highlighting and text styling.
//!
//! This module provides functions for determining the final visual style of each
//! character based on syntax highlighting, selection, search matches, cursor position,
//! and diagnostic underlines.

use lsp_types::DiagnosticSeverity;
use ratatui::style::{Color, Modifier, Style};

use super::context::RenderContext;

/// Determine the final style for a cell at the given position.
///
/// Applies styling priority:
/// 1. Current search match (highest priority)
/// 2. Regular search match
/// 3. Text selection
/// 4. Cursor line (base style with accented background)
/// 5. Diagnostic underline (applied on top of base style)
/// 6. Base syntax highlighting style
#[allow(clippy::too_many_arguments)] // Logical grouping of styling parameters
pub fn determine_cell_style(
    line: usize,
    column: usize,
    base_style: Style,
    is_cursor_line: bool,
    render_context: &RenderContext,
    search_match_style: Style,
    current_match_style: Style,
    selection_style: Style,
    cursor_line_bg: Color,
    error_color: Color,
    warning_color: Color,
) -> Style {
    // Check if this is a search match (O(1) HashMap lookup)
    let match_idx = render_context
        .search_match_map
        .get(&(line, column))
        .copied();

    // Check if this character is in selection (inline comparison avoids Cursor allocation)
    let is_selected = if let Some((sel_start, sel_end)) = &render_context.selection_range {
        (line > sel_start.line || (line == sel_start.line && column >= sel_start.column))
            && (line < sel_end.line || (line == sel_end.line && column < sel_end.column))
    } else {
        false
    };

    // Check for diagnostic at this position
    let diagnostic_severity = render_context.diagnostic_severity_at(line, column);

    // Determine final style based on priority
    let mut result = if let Some(idx) = match_idx {
        // Search match - highest priority
        if Some(idx) == render_context.current_match_idx {
            current_match_style
        } else {
            search_match_style
        }
    } else if is_selected {
        // Selected text
        selection_style
    } else if is_cursor_line {
        // Cursor line (but not search match or selection)
        base_style.bg(cursor_line_bg)
    } else {
        // Regular syntax highlighting
        base_style
    };

    // Apply diagnostic underline if present (lower priority than search/selection)
    if let Some(severity) = diagnostic_severity {
        let underline_color = match severity {
            DiagnosticSeverity::ERROR => error_color,
            DiagnosticSeverity::WARNING => warning_color,
            _ => warning_color, // Use warning color for INFO/HINT
        };
        result = result
            .add_modifier(Modifier::UNDERLINED)
            .underline_color(underline_color);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style};
    use std::collections::HashMap;
    use termide_buffer::Cursor;

    fn create_test_context(
        search_matches: Vec<(usize, usize)>,
        current_match_idx: Option<usize>,
        selection_range: Option<(Cursor, Cursor)>,
    ) -> RenderContext {
        let mut search_match_map = HashMap::new();
        for (idx, (line, col)) in search_matches.iter().enumerate() {
            search_match_map.insert((*line, *col), idx);
        }

        RenderContext {
            search_match_map,
            current_match_idx,
            selection_range,
            cursor_viewport_pos: None,
            diagnostic_line_severity: HashMap::new(),
            diagnostic_ranges: HashMap::new(),
        }
    }

    #[test]
    fn test_current_search_match_priority() {
        let context = create_test_context(vec![(0, 5)], Some(0), None);

        let base_style = Style::default().fg(Color::White);
        let search_match_style = Style::default().bg(Color::Yellow);
        let current_match_style = Style::default().bg(Color::Green);
        let selection_style = Style::default().bg(Color::Blue);

        let result = determine_cell_style(
            0,
            5,
            base_style,
            false,
            &context,
            search_match_style,
            current_match_style,
            selection_style,
            Color::DarkGray,
            Color::Red,
            Color::Yellow,
        );

        assert_eq!(result.bg, Some(Color::Green)); // Current match has highest priority
    }

    #[test]
    fn test_regular_search_match_priority() {
        let context = create_test_context(vec![(0, 5)], Some(999), None);

        let base_style = Style::default().fg(Color::White);
        let search_match_style = Style::default().bg(Color::Yellow);
        let current_match_style = Style::default().bg(Color::Green);
        let selection_style = Style::default().bg(Color::Blue);

        let result = determine_cell_style(
            0,
            5,
            base_style,
            false,
            &context,
            search_match_style,
            current_match_style,
            selection_style,
            Color::DarkGray,
            Color::Red,
            Color::Yellow,
        );

        assert_eq!(result.bg, Some(Color::Yellow)); // Regular search match
    }

    #[test]
    fn test_selection_priority() {
        let sel_start = Cursor::at(0, 3);
        let sel_end = Cursor::at(0, 8);
        let context = create_test_context(vec![], None, Some((sel_start, sel_end)));

        let base_style = Style::default().fg(Color::White);
        let search_match_style = Style::default().bg(Color::Yellow);
        let current_match_style = Style::default().bg(Color::Green);
        let selection_style = Style::default().bg(Color::Blue);

        let result = determine_cell_style(
            0,
            5,
            base_style,
            false,
            &context,
            search_match_style,
            current_match_style,
            selection_style,
            Color::DarkGray,
            Color::Red,
            Color::Yellow,
        );

        assert_eq!(result.bg, Some(Color::Blue)); // Selection style
    }

    #[test]
    fn test_cursor_line_priority() {
        let context = create_test_context(vec![], None, None);

        let base_style = Style::default().fg(Color::White).bg(Color::Black);
        let search_match_style = Style::default().bg(Color::Yellow);
        let current_match_style = Style::default().bg(Color::Green);
        let selection_style = Style::default().bg(Color::Blue);

        let result = determine_cell_style(
            0,
            5,
            base_style,
            true, // is_cursor_line = true
            &context,
            search_match_style,
            current_match_style,
            selection_style,
            Color::DarkGray,
            Color::Red,
            Color::Yellow,
        );

        assert_eq!(result.bg, Some(Color::DarkGray)); // Cursor line bg
        assert_eq!(result.fg, Some(Color::White)); // Preserves base fg
    }

    #[test]
    fn test_base_style_fallback() {
        let context = create_test_context(vec![], None, None);

        let base_style = Style::default().fg(Color::Cyan).bg(Color::Black);
        let search_match_style = Style::default().bg(Color::Yellow);
        let current_match_style = Style::default().bg(Color::Green);
        let selection_style = Style::default().bg(Color::Blue);

        let result = determine_cell_style(
            0,
            5,
            base_style,
            false,
            &context,
            search_match_style,
            current_match_style,
            selection_style,
            Color::DarkGray,
            Color::Red,
            Color::Yellow,
        );

        assert_eq!(result, base_style); // Returns base style unchanged
    }
}
