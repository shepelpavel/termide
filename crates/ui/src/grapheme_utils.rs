//! Grapheme-level text utilities for search result rendering.
//!
//! Shared helpers for truncating and windowing text with proper
//! Unicode grapheme cluster awareness.

use unicode_segmentation::UnicodeSegmentation;

/// Truncate text from the start to fit within `max_chars` graphemes.
/// Adds "…" prefix if truncated.
pub fn truncate_from_start(line: &str, max_chars: usize) -> String {
    let graphemes: Vec<&str> = line.graphemes(true).collect();
    if graphemes.len() > max_chars {
        let truncated: String = graphemes[..max_chars.saturating_sub(1)].concat();
        format!("{}…", truncated)
    } else {
        line.to_string()
    }
}

/// Convert byte positions to grapheme indices.
pub fn byte_to_grapheme_indices(line: &str, byte_start: usize, byte_end: usize) -> (usize, usize) {
    let mut grapheme_start = 0;
    let mut grapheme_end = 0;
    let mut byte_pos = 0;

    for (idx, grapheme) in line.graphemes(true).enumerate() {
        if byte_pos <= byte_start {
            grapheme_start = idx;
        }
        byte_pos += grapheme.len();
        if byte_pos <= byte_end {
            grapheme_end = idx + 1;
        }
    }

    (grapheme_start, grapheme_end)
}

/// Prepare a matched line for display — center the match if the line is too long.
///
/// Returns `(display_text, match_start_grapheme, match_end_grapheme)`.
pub fn prepare_matched_line(
    line: &str,
    match_start: usize,
    match_end: usize,
    max_width: usize,
) -> (String, usize, usize) {
    let graphemes: Vec<&str> = line.graphemes(true).collect();
    let (match_start_g, match_end_g) = byte_to_grapheme_indices(line, match_start, match_end);

    if graphemes.len() <= max_width {
        return (line.to_string(), match_start_g, match_end_g);
    }

    let ellipsis_len = 1;
    let match_center = (match_start_g + match_end_g) / 2;
    let available_for_text = max_width.saturating_sub(ellipsis_len * 2);
    let half_window = available_for_text / 2;
    let window_start = match_center.saturating_sub(half_window).min(match_start_g);
    let window_end = (window_start + available_for_text).min(graphemes.len());
    let window_start = if window_end == graphemes.len() {
        graphemes.len().saturating_sub(available_for_text)
    } else {
        window_start
    };

    let needs_left_ellipsis = window_start > 0;
    let needs_right_ellipsis = window_end < graphemes.len();

    let mut result = String::new();
    let mut new_match_start = match_start_g.saturating_sub(window_start);
    let mut new_match_end = match_end_g.saturating_sub(window_start);

    if needs_left_ellipsis {
        result.push('…');
        new_match_start += ellipsis_len;
        new_match_end += ellipsis_len;
    }

    result.push_str(&graphemes[window_start..window_end].concat());

    if needs_right_ellipsis {
        result.push('…');
    }

    let result_len = result.graphemes(true).count();
    (result, new_match_start, new_match_end.min(result_len))
}
