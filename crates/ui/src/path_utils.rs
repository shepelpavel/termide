//! Utilities for resolving file operation destination paths

use std::path::{Path, PathBuf};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Resolve destination path for a single file/directory operation
///
/// If destination is a directory, appends source filename to it.
/// Otherwise, uses destination as-is.
pub fn resolve_destination_path(source: &Path, destination: &Path) -> PathBuf {
    if destination.is_dir() {
        destination.join(source.file_name().unwrap_or_default())
    } else {
        destination.to_path_buf()
    }
}

/// Resolve destination path for batch operations
///
/// Handles special case where single source to non-directory destination
/// should use the destination name (rename operation).
pub fn resolve_batch_destination_path(
    source: &Path,
    destination: &Path,
    is_single_source: bool,
) -> PathBuf {
    if destination.is_dir() {
        // Destination is directory - append source filename
        destination.join(source.file_name().unwrap_or_default())
    } else if is_single_source {
        // Single file to non-directory - use destination as-is (rename)
        destination.to_path_buf()
    } else {
        // Multiple files to non-directory - append filename (fallback)
        destination.join(source.file_name().unwrap_or_default())
    }
}

/// Resolve destination path when applying rename pattern
///
/// If destination is a directory, joins new_name to it.
/// Otherwise, replaces filename in destination path with new_name.
pub fn resolve_rename_destination_path(destination: &Path, new_name: &str) -> PathBuf {
    if destination.is_dir() {
        destination.join(new_name)
    } else {
        destination.with_file_name(new_name)
    }
}

/// Extract file name from path as string slice
///
/// Returns "?" if the path has no file name or it's not valid UTF-8.
pub fn get_file_name_str(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("?")
}

/// Extract file name from path as String
///
/// Returns "?" if the path has no file name or it's not valid UTF-8.
pub fn get_file_name_string(path: &Path) -> String {
    get_file_name_str(path).to_string()
}

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

/// Truncate a string from the right with ellipsis suffix.
///
/// Keeps the leftmost (beginning) part of the string.
/// Respects Unicode character widths (e.g., CJK characters count as 2).
pub fn truncate_right(s: &str, max_width: usize) -> String {
    let current_width = s.width();
    if current_width <= max_width {
        return s.to_string();
    }

    let ellipsis = "…";
    let ellipsis_width = 1;
    let available = max_width.saturating_sub(ellipsis_width);

    // Take characters from the left until we reach available width
    let mut result = String::new();
    let mut width = 0;

    for c in s.chars() {
        let char_width = c.width().unwrap_or(0);
        if width + char_width > available {
            break;
        }
        result.push(c);
        width += char_width;
    }

    format!("{}{}", result, ellipsis)
}

/// Truncate a string from the left with ellipsis prefix.
///
/// Keeps the rightmost (most relevant) part of the string.
/// Respects Unicode character widths (e.g., CJK characters count as 2).
pub fn truncate_left(s: &str, max_width: usize) -> String {
    let current_width = s.width();
    if current_width <= max_width {
        return s.to_string();
    }

    let ellipsis = "…";
    let ellipsis_width = 1;
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
