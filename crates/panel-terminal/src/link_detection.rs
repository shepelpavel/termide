//! Link detection for terminal (URLs and file paths).
//!
//! Detects and highlights clickable links in terminal output.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::terminal::TerminalScreen;

/// Cached regex for URL detection in terminal (compiled once, used many times)
pub static URL_REGEX: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?:https?|ftp)://[^\s)>\]\}"'`<]+"#).expect("URL regex pattern is valid")
});

/// Cached regex for file path detection in terminal (compiled once, used many times)
pub static PATH_REGEX: LazyLock<regex::Regex> = LazyLock::new(|| {
    // Match Unix paths: /path, ./path, ../path, ~/path
    // Match Windows paths: C:\path, C:/path, \\server\share
    regex::Regex::new(
        r#"(?:[A-Za-z]:[/\\][^\s)>\]\}"'`<:*?|]*|\\\\[^\s)>\]\}"'`<:*?|]+|(?:~|\.\.?)?/[^\s)>\]\}"'`<:*?|]+)"#
    ).expect("Path regex pattern is valid")
});

/// Type of detected link in terminal
#[derive(Clone, Debug, PartialEq)]
pub enum LinkType {
    /// HTTP/HTTPS/FTP URL
    Url(String),
    /// Local file path (resolved to absolute)
    FilePath(PathBuf),
}

/// Highlight segment: (abs_row, start_col, end_col)
pub type HighlightSegment = (usize, usize, usize);

/// Detect link (URL or file path) at given position.
/// Returns (LinkType, start_row, start_col, display_len) if found.
/// `display_len` is the length of the matched text on screen (in cells),
/// which may differ from `link_text().len()` for resolved file paths.
pub fn detect_link_at_position(
    screen: &TerminalScreen,
    abs_row: usize,
    col: usize,
    cwd: &Path,
) -> Option<(LinkType, usize, usize, usize)> {
    let cols = screen.cols;

    // Look back up to 5 lines to find where a wrapped link might have started
    let start_row = abs_row.saturating_sub(5);

    // Find the actual start row (first line that doesn't look like a continuation)
    let mut search_start = abs_row;
    for row in (start_row..abs_row).rev() {
        if let Some(line) = screen.get_line_by_absolute(row) {
            // Use cell count (not byte count) to check if line fills terminal width
            let trimmed_cell_len = line
                .iter()
                .rposition(|c| c.ch != ' ' && c.ch != '\0')
                .map_or(0, |p| p + 1);
            if trimmed_cell_len >= cols {
                search_start = row;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    // Concatenate text from search_start through current row and forward.
    // Track char offsets (= cell offsets) for each line, since regex returns
    // byte offsets that don't match cell positions for non-ASCII content.
    let mut combined_text = String::new();
    let mut line_starts: Vec<(usize, usize)> = Vec::new(); // (row, char_offset)
    let mut char_count: usize = 0;

    for row in search_start.. {
        if let Some(line) = screen.get_line_by_absolute(row) {
            line_starts.push((row, char_count));
            let line_text: String = line.iter().map(|c| c.ch).collect();
            char_count += line.len(); // cell count = char count (one char per cell)
            let trimmed_cell_len = line
                .iter()
                .rposition(|c| c.ch != ' ' && c.ch != '\0')
                .map_or(0, |p| p + 1);
            combined_text.push_str(&line_text);

            if row >= abs_row && trimmed_cell_len < cols {
                break;
            }
            if row > abs_row + 5 {
                break;
            }
        } else {
            break;
        }
    }

    // Calculate cursor offset in char/cell units
    let cursor_char_offset = line_starts
        .iter()
        .find(|(row, _)| *row == abs_row)
        .map(|(_, char_offset)| char_offset + col)?;

    // Convert regex byte offset to char offset (handles multi-byte chars)
    let byte_to_char =
        |byte_offset: usize| -> usize { combined_text[..byte_offset].chars().count() };

    // Helper to find start row/col from char offset
    let find_start_pos = |char_offset: usize| -> Option<(usize, usize)> {
        for (row, offset) in line_starts.iter().rev() {
            if char_offset >= *offset {
                return Some((*row, char_offset - offset));
            }
        }
        None
    };

    // Try to detect URL first (using cached regex)
    for m in URL_REGEX.find_iter(&combined_text) {
        let match_start = byte_to_char(m.start());
        let match_end = byte_to_char(m.end());
        if cursor_char_offset >= match_start && cursor_char_offset < match_end {
            let display_len = match_end - match_start;
            if let Some((row, col)) = find_start_pos(match_start) {
                return Some((LinkType::Url(m.as_str().to_string()), row, col, display_len));
            }
        }
    }

    // Try to detect file path (using cached regex)
    // Matches: /path/to/file, ./path, ../path, ~/path
    for m in PATH_REGEX.find_iter(&combined_text) {
        let match_start = byte_to_char(m.start());
        let match_end = byte_to_char(m.end());
        if cursor_char_offset >= match_start && cursor_char_offset < match_end {
            let path_str = m.as_str();
            // Expand ~ to home directory
            let expanded = if let Some(suffix) = path_str.strip_prefix('~') {
                if let Some(home) = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .ok()
                {
                    PathBuf::from(home).join(suffix.trim_start_matches(['/', '\\']))
                } else {
                    PathBuf::from(path_str)
                }
            } else if path_str.starts_with('/') {
                PathBuf::from(path_str)
            } else {
                // Relative path - resolve against cwd
                cwd.join(path_str)
            };

            // Check if path exists
            if expanded.exists() {
                let display_len = match_end - match_start;
                if let Some((row, col)) = find_start_pos(match_start) {
                    return Some((LinkType::FilePath(expanded), row, col, display_len));
                }
            }
        }
    }

    None
}

/// Build highlight segments for multi-line link.
pub fn build_link_segments(
    text_len: usize,
    start_row: usize,
    start_col: usize,
    cols: usize,
) -> Vec<HighlightSegment> {
    let mut segments = Vec::new();
    let mut remaining = text_len;
    let mut current_row = start_row;
    let mut current_col = start_col;

    while remaining > 0 {
        let available = cols.saturating_sub(current_col);
        let segment_len = remaining.min(available);

        if segment_len > 0 {
            segments.push((current_row, current_col, current_col + segment_len));
        }

        remaining = remaining.saturating_sub(segment_len);
        current_row += 1;
        current_col = 0;
    }

    segments
}

/// Get the text representation of a link for display/copying
pub fn link_text(link: &LinkType) -> String {
    match link {
        LinkType::Url(url) => url.clone(),
        LinkType::FilePath(path) => path.display().to_string(),
    }
}
