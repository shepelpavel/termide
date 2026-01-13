use anyhow::{Context, Result};
use similar::TextDiff;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;

/// Result of async git diff load operation
#[derive(Debug)]
pub struct GitDiffAsyncResult {
    /// File path this result is for
    pub file_path: PathBuf,
    /// Original content from HEAD (None if error or file not in HEAD)
    pub original_content: Option<String>,
}

/// Load original content from HEAD in a background thread
/// Returns a receiver that will receive the result
pub fn load_original_async(file_path: PathBuf) -> mpsc::Receiver<GitDiffAsyncResult> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let original_content = load_original_from_head_sync(&file_path);
        let _ = tx.send(GitDiffAsyncResult {
            file_path,
            original_content,
        });
    });

    rx
}

/// Synchronous function to load original content from HEAD
/// Extracted for use in background thread
fn load_original_from_head_sync(file_path: &std::path::Path) -> Option<String> {
    // Get git root
    let git_root_output = Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .current_dir(file_path.parent().unwrap_or(std::path::Path::new("/")))
        .output()
        .ok()?;

    if !git_root_output.status.success() {
        return None;
    }

    let git_root = String::from_utf8(git_root_output.stdout)
        .ok()?
        .trim()
        .to_string();
    let git_root_path = std::path::Path::new(&git_root);

    // Get relative path
    let relative_path = file_path.strip_prefix(git_root_path).ok()?;

    // Get content from HEAD
    let output = Command::new("git")
        .arg("show")
        .arg(format!("HEAD:{}", relative_path.display()))
        .current_dir(&git_root)
        .output()
        .ok()?;

    if !output.status.success() {
        // File not in HEAD (new file)
        return Some(String::new());
    }

    String::from_utf8(output.stdout).ok()
}

/// Git diff status for a line in a file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStatus {
    /// Line unchanged from HEAD
    Unchanged,
    /// Line added (not in HEAD)
    Added,
    /// Line modified (changed from HEAD)
    Modified,
    /// Lines deleted after this line
    DeletedAfter,
}

/// Type of inline (character-level) change within a line
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineChangeType {
    /// Text unchanged
    Unchanged,
    /// Text deleted (from original, show with red background)
    Deleted,
    /// Text inserted (in current, show with green background)
    Inserted,
}

/// A segment of inline change within a modified line
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineChange {
    /// Type of change
    pub change_type: InlineChangeType,
    /// The text content
    pub text: String,
}

/// Compute character-level diff between two lines
/// Returns a list of InlineChange segments
pub fn compute_inline_diff(original: &str, current: &str) -> Vec<InlineChange> {
    use similar::ChangeTag;

    let diff = TextDiff::from_chars(original, current);
    let mut changes = Vec::new();

    // Collect all changes, merging consecutive same-type changes
    let mut current_type: Option<InlineChangeType> = None;
    let mut current_text = String::new();

    for change in diff.iter_all_changes() {
        let change_type = match change.tag() {
            ChangeTag::Equal => InlineChangeType::Unchanged,
            ChangeTag::Delete => InlineChangeType::Deleted,
            ChangeTag::Insert => InlineChangeType::Inserted,
        };

        if Some(change_type) == current_type {
            // Same type, append to current segment
            current_text.push_str(change.value());
        } else {
            // Different type, save previous segment and start new one
            if let Some(ct) = current_type {
                if !current_text.is_empty() {
                    changes.push(InlineChange {
                        change_type: ct,
                        text: std::mem::take(&mut current_text),
                    });
                }
            }
            current_type = Some(change_type);
            current_text = change.value().to_string();
        }
    }

    // Don't forget the last segment
    if let Some(ct) = current_type {
        if !current_text.is_empty() {
            changes.push(InlineChange {
                change_type: ct,
                text: current_text,
            });
        }
    }

    changes
}

/// Cache for git diff results for a single file
#[derive(Debug, Clone)]
pub struct GitDiffCache {
    /// File path this diff is for
    file_path: PathBuf,
    /// Map of line number (0-based) to status
    line_statuses: HashMap<usize, LineStatus>,
    /// Map of line numbers to count of deleted lines after them (line_idx -> deletion_count)
    deleted_after_lines: HashMap<usize, usize>,
    /// Mapping of current line index -> original line index (for Modified lines)
    modified_line_mapping: HashMap<usize, usize>,
    /// Timestamp when diff was last fetched
    last_updated: std::time::Instant,
    /// Original content from HEAD (for in-memory diff)
    original_content: Option<String>,
}

impl GitDiffCache {
    /// Create new git diff cache for a file
    pub fn new(file_path: PathBuf) -> Self {
        Self {
            file_path,
            line_statuses: HashMap::new(),
            deleted_after_lines: HashMap::new(),
            modified_line_mapping: HashMap::new(),
            last_updated: std::time::Instant::now(),
            original_content: None,
        }
    }

    /// Load original content from HEAD
    pub fn load_original_from_head(&mut self) -> Result<()> {
        log::debug!(
            "GitDiffCache::load_original_from_head for {:?}",
            self.file_path
        );

        // Get the directory containing the file to run git commands from
        let file_dir = self
            .file_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));

        // Convert absolute path to relative path from git root
        let git_root_output = Command::new("git")
            .arg("rev-parse")
            .arg("--show-toplevel")
            .current_dir(file_dir)
            .output()
            .context("Failed to get git root")?;

        if !git_root_output.status.success() {
            log::debug!("  git rev-parse failed - not a git repo");
            self.original_content = Some(String::new());
            return Ok(());
        }

        let git_root = String::from_utf8(git_root_output.stdout)
            .context("Failed to parse git root as UTF-8")?
            .trim()
            .to_string();
        let git_root_path = std::path::Path::new(&git_root);
        log::debug!("  git root: {:?}", git_root_path);

        // Get relative path from git root
        let relative_path = match self.file_path.strip_prefix(git_root_path) {
            Ok(p) => p,
            Err(e) => {
                log::debug!("  file not in git repo: {}", e);
                self.original_content = Some(String::new());
                return Ok(());
            }
        };
        log::debug!("  relative path: {:?}", relative_path);

        // Get file content from HEAD
        let output = Command::new("git")
            .arg("show")
            .arg(format!("HEAD:{}", relative_path.display()))
            .current_dir(&git_root)
            .output()
            .context("Failed to execute git show")?;

        if !output.status.success() {
            // File might be new (not in HEAD yet)
            log::debug!("  git show failed - file new or not tracked");
            self.original_content = Some(String::new());
            return Ok(());
        }

        let content =
            String::from_utf8(output.stdout).context("Failed to parse git show output as UTF-8")?;

        log::debug!("  loaded {} bytes from HEAD", content.len());
        self.original_content = Some(content);
        Ok(())
    }

    /// Update git diff by comparing buffer content with original from HEAD
    pub fn update_from_buffer(&mut self, current_content: &str) -> Result<()> {
        // Ensure we have original content loaded
        if self.original_content.is_none() {
            self.load_original_from_head()?;
        }

        let original = self
            .original_content
            .as_ref()
            .expect("original_content set by load_original_from_head above");

        // Compute diff using similar crate
        let diff = TextDiff::from_lines(original.as_str(), current_content);
        let result = compute_line_statuses_from_textdiff(&diff);

        // Use the computed results directly - they are the source of truth
        // TextDiff compares HEAD with current buffer, which correctly handles:
        // - Pure deletions (creates markers)
        // - Modified line deletions (creates markers)
        // - Restored lines via undo (removes markers)
        self.line_statuses = result.statuses;
        self.deleted_after_lines = result.deleted_after;
        self.modified_line_mapping = result.modified_mapping;
        self.last_updated = std::time::Instant::now();

        Ok(())
    }

    /// Update git diff by comparing file on disk with HEAD
    pub fn update(&mut self) -> Result<()> {
        log::debug!("GitDiffCache::update for {:?}", self.file_path);

        // Load original content from HEAD
        self.load_original_from_head()?;

        // If original content is empty (file not in HEAD or error), clear statuses
        let original = match self.original_content.as_ref() {
            Some(content) if !content.is_empty() => content,
            _ => {
                log::debug!("  original content empty - clearing statuses");
                self.line_statuses.clear();
                self.deleted_after_lines.clear();
                return Ok(());
            }
        };

        // Read current file content from disk
        let current_content = match std::fs::read_to_string(&self.file_path) {
            Ok(content) => content,
            Err(e) => {
                // File might not exist or can't be read
                log::debug!("  failed to read current file: {}", e);
                self.line_statuses.clear();
                self.deleted_after_lines.clear();
                return Ok(());
            }
        };

        log::debug!(
            "  comparing {} bytes original vs {} bytes current",
            original.len(),
            current_content.len()
        );

        // Use TextDiff for consistency with update_from_buffer()
        let diff = TextDiff::from_lines(original.as_str(), &current_content);
        let result = compute_line_statuses_from_textdiff(&diff);

        log::debug!(
            "  found {} changed lines, {} deletion markers",
            result.statuses.len(),
            result.deleted_after.len()
        );

        self.line_statuses = result.statuses;
        self.deleted_after_lines = result.deleted_after;
        self.modified_line_mapping = result.modified_mapping;
        self.last_updated = std::time::Instant::now();

        Ok(())
    }

    /// Get count of lines with statuses (for debugging)
    pub fn line_status_count(&self) -> usize {
        self.line_statuses.len()
    }

    /// Get status for a specific line (0-based index)
    pub fn get_line_status(&self, line: usize) -> LineStatus {
        self.line_statuses
            .get(&line)
            .copied()
            .unwrap_or(LineStatus::Unchanged)
    }

    /// Check if line has a deletion marker after it
    pub fn has_deletion_marker(&self, line: usize) -> bool {
        self.deleted_after_lines.contains_key(&line)
    }

    /// Get count of deleted lines after the given line
    pub fn get_deletion_count(&self, line: usize) -> usize {
        self.deleted_after_lines.get(&line).copied().unwrap_or(0)
    }

    /// Get total number of lines with deletion markers.
    ///
    /// This is O(1) - simply returns the number of entries in the map.
    /// Used for virtual line count calculation.
    pub fn deletion_marker_count(&self) -> usize {
        self.deleted_after_lines.len()
    }

    /// Get original line content from HEAD by original line index
    /// Returns None if original content is not loaded or line doesn't exist
    fn get_original_line_by_idx(&self, original_idx: usize) -> Option<&str> {
        let original = self.original_content.as_ref()?;
        original.lines().nth(original_idx)
    }

    /// Compute inline diff for a modified line
    /// Returns None if line is not modified or original is not available
    pub fn get_inline_diff(
        &self,
        line_idx: usize,
        current_text: &str,
    ) -> Option<Vec<InlineChange>> {
        // Only compute for modified lines
        if self.get_line_status(line_idx) != LineStatus::Modified {
            return None;
        }

        // Get the original line index from the mapping
        let original_idx = self.modified_line_mapping.get(&line_idx)?;
        let original_line = self.get_original_line_by_idx(*original_idx)?;
        Some(compute_inline_diff(original_line, current_text))
    }

    /// Apply async result and recompute diff
    /// Called when background thread completes loading original content
    pub fn apply_async_result(&mut self, async_result: GitDiffAsyncResult) {
        // Store original content
        self.original_content = async_result.original_content;

        // Recompute diff if we have original content
        let original = match self.original_content.as_ref() {
            Some(content) if !content.is_empty() => content,
            _ => {
                self.line_statuses.clear();
                self.deleted_after_lines.clear();
                self.modified_line_mapping.clear();
                return;
            }
        };

        // Read current file content from disk
        let current_content = match std::fs::read_to_string(&self.file_path) {
            Ok(content) => content,
            Err(_) => {
                self.line_statuses.clear();
                self.deleted_after_lines.clear();
                self.modified_line_mapping.clear();
                return;
            }
        };

        // Compute diff
        let diff = TextDiff::from_lines(original.as_str(), &current_content);
        let result = compute_line_statuses_from_textdiff(&diff);

        self.line_statuses = result.statuses;
        self.deleted_after_lines = result.deleted_after;
        self.modified_line_mapping = result.modified_mapping;
        self.last_updated = std::time::Instant::now();
    }
}

/// Result of computing line statuses from diff
struct DiffResult {
    statuses: HashMap<usize, LineStatus>,
    deleted_after: HashMap<usize, usize>,
    modified_mapping: HashMap<usize, usize>,
}

/// Compute line statuses from TextDiff (similar crate)
fn compute_line_statuses_from_textdiff<'a>(diff: &TextDiff<'a, 'a, 'a, str>) -> DiffResult {
    let mut statuses = HashMap::new();
    let mut deleted_after = HashMap::new();
    let mut modified_mapping = HashMap::new();

    // Process changes tracking both old and new line indices
    let changes: Vec<_> = diff.iter_all_changes().collect();
    let mut i = 0;
    let mut new_idx = 0;
    let mut old_idx = 0;

    while i < changes.len() {
        use similar::ChangeTag;

        match changes[i].tag() {
            ChangeTag::Equal => {
                // Unchanged line - increment both counters
                new_idx += 1;
                old_idx += 1;
                i += 1;
            }
            ChangeTag::Delete => {
                // Check if immediately followed by Insert (indicating modification)
                if i + 1 < changes.len() && changes[i + 1].tag() == ChangeTag::Insert {
                    // Modification: pair this Delete with the next Insert
                    statuses.insert(new_idx, LineStatus::Modified);
                    // Store mapping: new_idx -> old_idx
                    modified_mapping.insert(new_idx, old_idx);
                    new_idx += 1;
                    old_idx += 1;
                    i += 2; // Skip both Delete and Insert
                } else {
                    // Count consecutive deletions
                    let mut deletion_count = 0;
                    while i < changes.len() && changes[i].tag() == ChangeTag::Delete {
                        deletion_count += 1;
                        old_idx += 1;
                        i += 1;
                    }

                    // Place deletion marker after previous line with deletion count
                    let marker_line_idx = if new_idx > 0 { new_idx - 1 } else { 0 };
                    deleted_after.insert(marker_line_idx, deletion_count);

                    // new_idx stays the same (no new lines added)
                }
            }
            ChangeTag::Insert => {
                // Pure insertion - only new index increments
                statuses.insert(new_idx, LineStatus::Added);
                new_idx += 1;
                i += 1;
            }
        }
    }

    DiffResult {
        statuses,
        deleted_after,
        modified_mapping,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_line_statuses_from_textdiff_added() {
        let original = "line1\nline2\nline3\n";
        let current = "line1\nline2\nnew line\nline3\n";

        let diff = similar::TextDiff::from_lines(original, current);
        let result = compute_line_statuses_from_textdiff(&diff);

        // Line at index 2 should be Added (0-indexed: "new line")
        assert_eq!(result.statuses.get(&2), Some(&LineStatus::Added));
        // No deletions
        assert!(result.deleted_after.is_empty());
    }

    #[test]
    fn test_compute_line_statuses_from_textdiff_modified() {
        let original = "line1\noriginal line\nline3\n";
        let current = "line1\nmodified line\nline3\n";

        let diff = similar::TextDiff::from_lines(original, current);
        let result = compute_line_statuses_from_textdiff(&diff);

        // Line at index 1 should be Modified (0-indexed)
        assert_eq!(result.statuses.get(&1), Some(&LineStatus::Modified));
        assert!(result.deleted_after.is_empty());
        // Mapping should show new_idx 1 -> old_idx 1
        assert_eq!(result.modified_mapping.get(&1), Some(&1));
    }

    #[test]
    fn test_compute_line_statuses_from_textdiff_deleted() {
        let original = "line1\ndeleted line\nline3\n";
        let current = "line1\nline3\n";

        let diff = similar::TextDiff::from_lines(original, current);
        let result = compute_line_statuses_from_textdiff(&diff);

        // Deletion marker should be after line 0 (where line1 is)
        assert!(result.deleted_after.contains_key(&0));
        assert_eq!(result.deleted_after.get(&0), Some(&1)); // 1 line deleted
        assert!(result.statuses.is_empty()); // No modifications or additions
    }

    #[test]
    fn test_compute_line_statuses_from_textdiff_multiple_deletions() {
        let original = "line1\ndeleted1\ndeleted2\ndeleted3\nline5\n";
        let current = "line1\nline5\n";

        let diff = similar::TextDiff::from_lines(original, current);
        let result = compute_line_statuses_from_textdiff(&diff);

        // 3 lines deleted after line 0
        assert!(result.deleted_after.contains_key(&0));
        assert_eq!(result.deleted_after.get(&0), Some(&3));
        assert!(result.statuses.is_empty()); // No modifications or additions
    }

    #[test]
    fn test_compute_line_statuses_from_textdiff_mixed_changes() {
        let original = "line1\nold\nline3\n";
        let current = "line1\nnew\nnew2\nline3\n";

        let diff = similar::TextDiff::from_lines(original, current);
        let result = compute_line_statuses_from_textdiff(&diff);

        // Line 1 modified (old -> new), line 2 added (new2)
        assert_eq!(result.statuses.get(&1), Some(&LineStatus::Modified));
        assert_eq!(result.statuses.get(&2), Some(&LineStatus::Added));
        assert!(result.deleted_after.is_empty());
        // Modified line mapping: new_idx 1 -> old_idx 1
        assert_eq!(result.modified_mapping.get(&1), Some(&1));
    }

    #[test]
    fn test_modified_line_mapping_preserves_index() {
        // Test that mapping works correctly when lines are modified in place
        let original = "line1\noriginal line 2\nline3\noriginal line 4\n";
        let current = "line1\nmodified line 2\nline3\nmodified line 4\n";

        let diff = similar::TextDiff::from_lines(original, current);
        let result = compute_line_statuses_from_textdiff(&diff);

        // Lines 1 and 3 are modified (0-indexed)
        assert_eq!(result.statuses.get(&1), Some(&LineStatus::Modified));
        assert_eq!(result.statuses.get(&3), Some(&LineStatus::Modified));
        // Mapping should preserve indices since no lines added/deleted
        assert_eq!(result.modified_mapping.get(&1), Some(&1));
        assert_eq!(result.modified_mapping.get(&3), Some(&3));
    }

    #[test]
    fn test_compute_inline_diff_insertion() {
        let original = "Hello world";
        let current = "Hello beautiful world";

        let changes = compute_inline_diff(original, current);

        // Should be: Unchanged("Hello "), Inserted("beautiful "), Unchanged("world")
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].change_type, InlineChangeType::Unchanged);
        assert_eq!(changes[0].text, "Hello ");
        assert_eq!(changes[1].change_type, InlineChangeType::Inserted);
        assert_eq!(changes[1].text, "beautiful ");
        assert_eq!(changes[2].change_type, InlineChangeType::Unchanged);
        assert_eq!(changes[2].text, "world");
    }

    #[test]
    fn test_compute_inline_diff_deletion() {
        let original = "Hello beautiful world";
        let current = "Hello world";

        let changes = compute_inline_diff(original, current);

        // Should be: Unchanged("Hello "), Deleted("beautiful "), Unchanged("world")
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].change_type, InlineChangeType::Unchanged);
        assert_eq!(changes[0].text, "Hello ");
        assert_eq!(changes[1].change_type, InlineChangeType::Deleted);
        assert_eq!(changes[1].text, "beautiful ");
        assert_eq!(changes[2].change_type, InlineChangeType::Unchanged);
        assert_eq!(changes[2].text, "world");
    }

    #[test]
    fn test_compute_inline_diff_replacement() {
        let original = "let x = 5";
        let current = "let x = 10";

        let changes = compute_inline_diff(original, current);

        // Should have: Unchanged("let x = "), Deleted("5"), Inserted("10")
        assert!(changes.len() >= 3);
        // Find the deleted and inserted parts
        let deleted: Vec<_> = changes
            .iter()
            .filter(|c| c.change_type == InlineChangeType::Deleted)
            .collect();
        let inserted: Vec<_> = changes
            .iter()
            .filter(|c| c.change_type == InlineChangeType::Inserted)
            .collect();

        assert!(!deleted.is_empty());
        assert!(!inserted.is_empty());
    }

    #[test]
    fn test_compute_inline_diff_same_text() {
        let text = "Hello world";

        let changes = compute_inline_diff(text, text);

        // Should be single unchanged segment
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, InlineChangeType::Unchanged);
        assert_eq!(changes[0].text, text);
    }
}
