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
    regex::Regex::new(r#"(?:~|\.\.?)?/[^\s)>\]\}"'`<:*?|]+"#).expect("Path regex pattern is valid")
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
/// Returns (LinkType, start_row, start_col) if found.
pub fn detect_link_at_position(
    screen: &TerminalScreen,
    abs_row: usize,
    col: usize,
    cwd: &Path,
) -> Option<(LinkType, usize, usize)> {
    let cols = screen.cols;

    // Look back up to 5 lines to find where a wrapped link might have started
    let start_row = abs_row.saturating_sub(5);

    // Find the actual start row (first line that doesn't look like a continuation)
    let mut search_start = abs_row;
    for row in (start_row..abs_row).rev() {
        if let Some(line) = screen.get_line_by_absolute(row) {
            let line_text: String = line.iter().map(|c| c.ch).collect();
            let trimmed_len = line_text.trim_end().len();
            if trimmed_len >= cols {
                search_start = row;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    // Concatenate text from search_start through current row and forward
    let mut combined_text = String::new();
    let mut line_starts: Vec<(usize, usize)> = Vec::new();

    for row in search_start.. {
        if let Some(line) = screen.get_line_by_absolute(row) {
            line_starts.push((row, combined_text.len()));
            let line_text: String = line.iter().map(|c| c.ch).collect();
            let trimmed_len = line_text.trim_end().len();
            combined_text.push_str(&line_text);

            if row >= abs_row && trimmed_len < cols {
                break;
            }
            if row > abs_row + 5 {
                break;
            }
        } else {
            break;
        }
    }

    // Calculate cursor offset in combined text
    let cursor_offset = line_starts
        .iter()
        .find(|(row, _)| *row == abs_row)
        .map(|(_, offset)| offset + col)?;

    // Helper to find start row/col from offset
    let find_start_pos = |start_offset: usize| -> Option<(usize, usize)> {
        for (row, offset) in line_starts.iter().rev() {
            if start_offset >= *offset {
                return Some((*row, start_offset - offset));
            }
        }
        None
    };

    // Try to detect URL first (using cached regex)
    for m in URL_REGEX.find_iter(&combined_text) {
        if cursor_offset >= m.start() && cursor_offset < m.end() {
            if let Some((row, col)) = find_start_pos(m.start()) {
                return Some((LinkType::Url(m.as_str().to_string()), row, col));
            }
        }
    }

    // Try to detect file path (using cached regex)
    // Matches: /path/to/file, ./path, ../path, ~/path
    for m in PATH_REGEX.find_iter(&combined_text) {
        if cursor_offset >= m.start() && cursor_offset < m.end() {
            let path_str = m.as_str();
            // Expand ~ to home directory
            let expanded = if let Some(suffix) = path_str.strip_prefix('~') {
                if let Ok(home) = std::env::var("HOME") {
                    PathBuf::from(home).join(suffix.trim_start_matches('/'))
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
                if let Some((row, col)) = find_start_pos(m.start()) {
                    return Some((LinkType::FilePath(expanded), row, col));
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

/// Get the length of link text in characters
pub fn link_text_len(link: &LinkType) -> usize {
    link_text(link).chars().count()
}
