//! Line rendering for no-wrap mode.
//!
//! This module provides functions for rendering individual lines in the editor
//! when word wrap is disabled. Handles horizontal scrolling and syntax highlighting.

use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use termide_buffer::{Cursor, TextBuffer, Viewport};
use termide_git::{GitDiffCache, InlineChangeType};
use termide_highlight::LineHighlighter;
use termide_theme::Theme;

use super::{context::RenderContext, highlight_renderer, inline_diff};
use crate::git;

/// Render a single line in no-wrap mode.
///
/// Handles:
/// - Line number gutter with git status
/// - Syntax-highlighted content with horizontal scrolling
/// - Search matches, selection, and cursor line styling
/// - Background fill for cursor line
#[allow(clippy::too_many_arguments)] // Complex rendering requires many parameters
pub fn render_line_no_wrap<H: LineHighlighter>(
    buf: &mut Buffer,
    area: Rect,
    row: usize,
    line_idx: usize,
    line_text: &str,
    is_cursor_line: bool,
    text_style: Style,
    cursor_line_style: Style,
    git_diff_cache: &Option<GitDiffCache>,
    show_git_diff: bool,
    theme: &Theme,
    line_number_width: u16,
    content_width: usize,
    left_column: usize,
    syntax_highlighting_enabled: bool,
    highlight_cache: &mut H,
    render_context: &RenderContext,
    search_match_style: Style,
    current_match_style: Style,
    selection_style: Style,
) {
    let style = if is_cursor_line {
        cursor_line_style
    } else {
        text_style
    };

    // Render line number gutter with git status
    render_line_gutter(
        buf,
        area,
        row,
        line_idx,
        git_diff_cache,
        show_git_diff,
        theme,
    );

    // Render line content with horizontal scrolling
    render_line_content_horizontal_scroll(
        buf,
        area,
        row,
        line_idx,
        line_text,
        is_cursor_line,
        style,
        line_number_width,
        content_width,
        left_column,
        syntax_highlighting_enabled,
        highlight_cache,
        render_context,
        search_match_style,
        current_match_style,
        selection_style,
        theme,
        git_diff_cache,
        show_git_diff,
    );

    // Fill remainder of line with cursor line background
    if is_cursor_line {
        // Calculate visual line width (including deleted text from inline diff)
        let visual_line_width = if show_git_diff {
            git_diff_cache
                .as_ref()
                .and_then(|cache| cache.get_inline_diff(line_idx, line_text))
                .map(|changes| line_text.width() + inline_diff::calculate_deleted_width(&changes))
                .unwrap_or_else(|| line_text.width())
        } else {
            line_text.width()
        };

        fill_line_remainder(
            buf,
            area,
            row,
            visual_line_width,
            line_number_width,
            content_width,
            left_column,
            cursor_line_style,
        );
    }
}

/// Render line number gutter with git status markers.
fn render_line_gutter(
    buf: &mut Buffer,
    area: Rect,
    row: usize,
    line_idx: usize,
    git_diff_cache: &Option<GitDiffCache>,
    show_git_diff: bool,
    theme: &Theme,
) {
    let git_info = git::get_git_line_info(line_idx, git_diff_cache, show_git_diff, theme);

    // Render line number (4 chars) + status marker (1 char)
    let line_num_style = Style::default().fg(git_info.status_color);
    let line_num_part = format!("{:>4}{}", line_idx + 1, git_info.status_marker);

    for (i, ch) in line_num_part.chars().enumerate() {
        let x = area.x + i as u16;
        let y = area.y + row as u16;
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char(ch);
            cell.set_style(line_num_style);
        }
    }

    // Render space after marker (deletion markers are now virtual lines)
    let x = area.x + 5;
    let y = area.y + row as u16;
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_char(' ');
        cell.set_style(line_num_style);
    }
}

/// Render line content with horizontal scrolling.
#[allow(clippy::too_many_arguments)]
fn render_line_content_horizontal_scroll<H: LineHighlighter>(
    buf: &mut Buffer,
    area: Rect,
    row: usize,
    line_idx: usize,
    line_text: &str,
    is_cursor_line: bool,
    style: Style,
    line_number_width: u16,
    content_width: usize,
    left_column: usize,
    syntax_highlighting_enabled: bool,
    highlight_cache: &mut H,
    render_context: &RenderContext,
    search_match_style: Style,
    current_match_style: Style,
    selection_style: Style,
    theme: &Theme,
    git_diff_cache: &Option<GitDiffCache>,
    show_git_diff: bool,
) {
    // Check if this line has inline diff (modified line with original content)
    let inline_changes = if show_git_diff {
        git_diff_cache
            .as_ref()
            .and_then(|cache| cache.get_inline_diff(line_idx, line_text))
    } else {
        None
    };

    if let Some(ref changes) = inline_changes {
        // Render with inline diff highlighting
        render_line_with_inline_diff(
            buf,
            area,
            row,
            line_idx,
            changes,
            is_cursor_line,
            style,
            line_number_width,
            content_width,
            left_column,
            syntax_highlighting_enabled,
            highlight_cache,
            render_context,
            search_match_style,
            current_match_style,
            selection_style,
            theme,
        );
    } else {
        // Regular rendering without inline diff
        render_line_regular(
            buf,
            area,
            row,
            line_idx,
            line_text,
            is_cursor_line,
            style,
            line_number_width,
            content_width,
            left_column,
            syntax_highlighting_enabled,
            highlight_cache,
            render_context,
            search_match_style,
            current_match_style,
            selection_style,
            theme,
        );
    }
}

/// Render line content without inline diff (regular mode).
#[allow(clippy::too_many_arguments)]
fn render_line_regular<H: LineHighlighter>(
    buf: &mut Buffer,
    area: Rect,
    row: usize,
    line_idx: usize,
    line_text: &str,
    is_cursor_line: bool,
    style: Style,
    line_number_width: u16,
    content_width: usize,
    left_column: usize,
    syntax_highlighting_enabled: bool,
    highlight_cache: &mut H,
    render_context: &RenderContext,
    search_match_style: Style,
    current_match_style: Style,
    selection_style: Style,
    theme: &Theme,
) {
    // Get syntax highlighting segments
    let segments = if syntax_highlighting_enabled && highlight_cache.has_syntax() {
        highlight_cache.get_line_segments(line_idx, line_text)
    } else {
        // No syntax highlighting - use single segment
        &[(line_text.to_string(), style)][..]
    };

    // Render segments with horizontal scrolling
    // Using graphemes instead of chars to properly handle combining characters (Hindi, etc.)
    let mut col_offset = 0;
    let mut grapheme_idx = 0; // Grapheme index for selection/search matching
    for (segment_text, segment_style) in segments {
        for grapheme in segment_text.graphemes(true) {
            // Get display width of grapheme cluster
            let grapheme_width = grapheme.width();

            // Skip zero-width graphemes
            if grapheme_width == 0 {
                grapheme_idx += 1;
                continue;
            }

            if col_offset >= left_column && col_offset < left_column + content_width {
                let x = area.x + line_number_width + (col_offset - left_column) as u16;
                let y = area.y + row as u16;

                if x < area.x + area.width && y < area.y + area.height {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        // Use set_symbol for proper grapheme cluster handling
                        cell.set_symbol(grapheme);

                        // Determine final style using highlight renderer
                        let final_style = highlight_renderer::determine_cell_style(
                            line_idx,
                            grapheme_idx,
                            *segment_style,
                            is_cursor_line,
                            render_context,
                            search_match_style,
                            current_match_style,
                            selection_style,
                            theme.accented_bg,
                        );
                        cell.set_style(final_style);
                    }
                }
            }
            col_offset += grapheme_width;
            grapheme_idx += 1;
        }
    }
}

/// Render line content with inline diff highlighting.
///
/// Shows deleted text with red background and inserted text with green background.
/// Visual line is longer than buffer line due to deleted text being shown.
#[allow(clippy::too_many_arguments)]
fn render_line_with_inline_diff<H: LineHighlighter>(
    buf: &mut Buffer,
    area: Rect,
    row: usize,
    line_idx: usize,
    inline_changes: &[termide_git::InlineChange],
    is_cursor_line: bool,
    style: Style,
    line_number_width: u16,
    content_width: usize,
    left_column: usize,
    syntax_highlighting_enabled: bool,
    highlight_cache: &mut H,
    render_context: &RenderContext,
    search_match_style: Style,
    current_match_style: Style,
    selection_style: Style,
    theme: &Theme,
) {
    // Build visual segments from inline changes
    let visual_segments = inline_diff::build_visual_line(inline_changes);

    // Build the current line text (excluding deleted text) for syntax highlighting
    let current_text: String = inline_changes
        .iter()
        .filter(|c| c.change_type != InlineChangeType::Deleted)
        .map(|c| c.text.as_str())
        .collect();

    // Get syntax highlighting for the current text
    let syntax_segments = if syntax_highlighting_enabled && highlight_cache.has_syntax() {
        highlight_cache.get_line_segments(line_idx, &current_text)
    } else {
        &[(current_text.clone(), style)][..]
    };

    // Build a map of buffer position -> syntax style
    let mut syntax_styles: Vec<(usize, Style)> = Vec::new();
    let mut buf_pos = 0;
    for (seg_text, seg_style) in syntax_segments {
        let seg_len = seg_text.graphemes(true).count();
        for _ in 0..seg_len {
            syntax_styles.push((buf_pos, *seg_style));
            buf_pos += 1;
        }
    }

    // Render visual segments
    let mut visual_col = 0;
    let mut buffer_grapheme_idx = 0; // For selection/search matching (buffer coordinates)

    for segment in &visual_segments {
        let change_type = segment.change_type;

        for grapheme in segment.text.graphemes(true) {
            let grapheme_width = grapheme.width();

            if grapheme_width == 0 {
                if change_type != InlineChangeType::Deleted {
                    buffer_grapheme_idx += 1;
                }
                continue;
            }

            // Check if visible in viewport
            if visual_col >= left_column && visual_col < left_column + content_width {
                let x = area.x + line_number_width + (visual_col - left_column) as u16;
                let y = area.y + row as u16;

                if x < area.x + area.width && y < area.y + area.height {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_symbol(grapheme);

                        // Get base style from syntax highlighting (for non-deleted text)
                        let base_style = if change_type == InlineChangeType::Deleted {
                            style // Use default style for deleted text
                        } else {
                            syntax_styles
                                .get(buffer_grapheme_idx)
                                .map(|(_, s)| *s)
                                .unwrap_or(style)
                        };

                        // Apply diff styling
                        let diff_style = inline_diff::apply_diff_style(
                            change_type,
                            base_style,
                            theme.error,   // deleted bg
                            theme.success, // inserted bg
                        );

                        // For non-deleted text, also check selection/search highlights
                        let final_style = if change_type == InlineChangeType::Deleted {
                            // Deleted text: use diff style directly (no search/selection)
                            diff_style
                        } else {
                            // Check if this position has search/selection override
                            let highlight_style = highlight_renderer::determine_cell_style(
                                line_idx,
                                buffer_grapheme_idx,
                                diff_style,
                                is_cursor_line,
                                render_context,
                                search_match_style,
                                current_match_style,
                                selection_style,
                                theme.accented_bg,
                            );
                            // If unchanged and no highlight, apply diff style if inserted
                            if change_type == InlineChangeType::Inserted
                                && highlight_style == diff_style
                            {
                                diff_style
                            } else if change_type == InlineChangeType::Unchanged {
                                // For unchanged text in a modified line, check cursor line bg
                                if is_cursor_line && highlight_style == base_style {
                                    base_style.bg(theme.accented_bg)
                                } else {
                                    highlight_style
                                }
                            } else {
                                highlight_style
                            }
                        };

                        cell.set_style(final_style);
                    }
                }
            }

            visual_col += grapheme_width;
            if change_type != InlineChangeType::Deleted {
                buffer_grapheme_idx += 1;
            }
        }
    }
}

/// Fill remainder of line with cursor line background.
#[allow(clippy::too_many_arguments)] // Helper for render_line_no_wrap
fn fill_line_remainder(
    buf: &mut Buffer,
    area: Rect,
    row: usize,
    visual_line_width: usize,
    line_number_width: u16,
    content_width: usize,
    left_column: usize,
    cursor_line_style: Style,
) {
    for col in visual_line_width..content_width {
        if col >= left_column {
            let x = area.x + line_number_width + (col - left_column) as u16;
            let y = area.y + row as u16;

            if x < area.x + area.width && y < area.y + area.height {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_style(cursor_line_style);
                }
            }
        }
    }
}

/// Render editor content in no-wrap mode with virtual lines.
///
/// This is the main rendering function for no-wrap mode that handles:
/// - Virtual lines (real lines + deletion markers)
/// - Horizontal scrolling
/// - Cursor positioning accounting for virtual lines
#[allow(clippy::too_many_arguments)]
pub fn render_content_no_wrap<H: LineHighlighter>(
    buf: &mut Buffer,
    area: Rect,
    buffer: &TextBuffer,
    viewport: &Viewport,
    cursor: &Cursor,
    git_diff_cache: &Option<GitDiffCache>,
    show_git_diff: bool,
    syntax_highlighting_enabled: bool,
    highlight_cache: &mut H,
    render_context: &RenderContext,
    theme: &Theme,
    content_width: usize,
    content_height: usize,
    line_number_width: u16,
    text_style: Style,
    cursor_line_style: Style,
    search_match_style: Style,
    current_match_style: Style,
    selection_style: Style,
) {
    // Build list of virtual lines (real buffer lines + deletion markers)
    let virtual_lines = git::build_virtual_lines(buffer, git_diff_cache, show_git_diff);

    // Find index of first virtual line for viewport.top_line
    let start_virtual_idx = virtual_lines
        .iter()
        .position(|vline| matches!(vline, git::VirtualLine::Real(idx) if *idx >= viewport.top_line))
        .unwrap_or(virtual_lines.len());

    // Render visible virtual lines
    for row in 0..content_height {
        let virtual_idx = start_virtual_idx + row;

        if virtual_idx >= virtual_lines.len() {
            break;
        }

        let virtual_line = virtual_lines[virtual_idx];

        // Handle different types of virtual lines
        match virtual_line {
            git::VirtualLine::Real(line_idx) => {
                // Render real line - use line_cow for zero-copy when possible
                if let Some(line_text) = buffer.line_cow(line_idx) {
                    let line_text = line_text.trim_end_matches('\n');
                    let is_cursor_line = line_idx == cursor.line;

                    render_line_no_wrap(
                        buf,
                        area,
                        row,
                        line_idx,
                        line_text,
                        is_cursor_line,
                        text_style,
                        cursor_line_style,
                        git_diff_cache,
                        show_git_diff,
                        theme,
                        line_number_width,
                        content_width,
                        viewport.left_column,
                        syntax_highlighting_enabled,
                        highlight_cache,
                        render_context,
                        search_match_style,
                        current_match_style,
                        selection_style,
                    );
                }
            }
            git::VirtualLine::DeletionMarker(_after_line_idx, deletion_count) => {
                // Render deletion marker virtual line
                super::deletion_markers::render_deletion_marker(
                    buf,
                    area,
                    row,
                    deletion_count,
                    theme,
                    content_width,
                    line_number_width,
                );
            }
        }
    }

    // Render cursor accounting for virtual lines
    let cursor_virtual_idx = virtual_lines
        .iter()
        .position(|vline| matches!(vline, git::VirtualLine::Real(idx) if *idx == cursor.line));

    if let Some(cursor_virtual_idx) = cursor_virtual_idx {
        if cursor_virtual_idx >= start_virtual_idx {
            let viewport_row = cursor_virtual_idx - start_virtual_idx;

            if cursor.column >= viewport.left_column {
                let viewport_col = cursor.column - viewport.left_column;

                let cursor_x = area.x + line_number_width + viewport_col as u16;
                let cursor_y = area.y + viewport_row as u16;

                if viewport_col < content_width {
                    super::cursor_renderer::render_cursor_at(buf, cursor_x, cursor_y, area, theme);
                }
            }
        }
    }
}
