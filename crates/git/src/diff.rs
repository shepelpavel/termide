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

/// Cache for git diff results for a single file
#[derive(Debug, Clone)]
pub struct GitDiffCache {
    /// File path this diff is for
    file_path: PathBuf,
    /// Map of line number (0-based) to status
    line_statuses: HashMap<usize, LineStatus>,
    /// Map of line numbers to count of deleted lines after them (line_idx -> deletion_count)
    deleted_after_lines: HashMap<usize, usize>,
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
            last_updated: std::time::Instant::now(),
            original_content: None,
        }
    }

    /// Load original content from HEAD
    pub fn load_original_from_head(&mut self) -> Result<()> {
        // Convert absolute path to relative path from git root
        let git_root_output = Command::new("git")
            .arg("rev-parse")
            .arg("--show-toplevel")
            .output()
            .context("Failed to get git root")?;

        if !git_root_output.status.success() {
            self.original_content = Some(String::new());
            return Ok(());
        }

        let git_root = String::from_utf8(git_root_output.stdout)
            .context("Failed to parse git root as UTF-8")?
            .trim()
            .to_string();
        let git_root_path = std::path::Path::new(&git_root);

        // Get relative path from git root
        let relative_path = self
            .file_path
            .strip_prefix(git_root_path)
            .context("File is not within git repository")?;

        // Get file content from HEAD
        let output = Command::new("git")
            .arg("show")
            .arg(format!("HEAD:{}", relative_path.display()))
            .output()
            .context("Failed to execute git show")?;

        if !output.status.success() {
            // File might be new (not in HEAD yet)
            self.original_content = Some(String::new());
            return Ok(());
        }

        let content =
            String::from_utf8(output.stdout).context("Failed to parse git show output as UTF-8")?;

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
        let (statuses, deleted_after) = compute_line_statuses_from_textdiff(&diff);

        // Use the computed results directly - they are the source of truth
        // TextDiff compares HEAD with current buffer, which correctly handles:
        // - Pure deletions (creates markers)
        // - Modified line deletions (creates markers)
        // - Restored lines via undo (removes markers)
        self.line_statuses = statuses;
        self.deleted_after_lines = deleted_after;
        self.last_updated = std::time::Instant::now();

        Ok(())
    }

    /// Update git diff by comparing file on disk with HEAD
    pub fn update(&mut self) -> Result<()> {
        // Load original content from HEAD
        self.load_original_from_head()?;

        // If original content is empty (file not in HEAD or error), clear statuses
        let original = match self.original_content.as_ref() {
            Some(content) if !content.is_empty() => content,
            _ => {
                self.line_statuses.clear();
                self.deleted_after_lines.clear();
                return Ok(());
            }
        };

        // Read current file content from disk
        let current_content = match std::fs::read_to_string(&self.file_path) {
            Ok(content) => content,
            Err(_) => {
                // File might not exist or can't be read
                self.line_statuses.clear();
                self.deleted_after_lines.clear();
                return Ok(());
            }
        };

        // Use TextDiff for consistency with update_from_buffer()
        let diff = TextDiff::from_lines(original.as_str(), &current_content);
        let (statuses, deleted_after) = compute_line_statuses_from_textdiff(&diff);

        self.line_statuses = statuses;
        self.deleted_after_lines = deleted_after;
        self.last_updated = std::time::Instant::now();

        Ok(())
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

    /// Apply async result and recompute diff
    /// Called when background thread completes loading original content
    pub fn apply_async_result(&mut self, result: GitDiffAsyncResult) {
        // Store original content
        self.original_content = result.original_content;

        // Recompute diff if we have original content
        let original = match self.original_content.as_ref() {
            Some(content) if !content.is_empty() => content,
            _ => {
                self.line_statuses.clear();
                self.deleted_after_lines.clear();
                return;
            }
        };

        // Read current file content from disk
        let current_content = match std::fs::read_to_string(&self.file_path) {
            Ok(content) => content,
            Err(_) => {
                self.line_statuses.clear();
                self.deleted_after_lines.clear();
                return;
            }
        };

        // Compute diff
        let diff = TextDiff::from_lines(original.as_str(), &current_content);
        let (statuses, deleted_after) = compute_line_statuses_from_textdiff(&diff);

        self.line_statuses = statuses;
        self.deleted_after_lines = deleted_after;
        self.last_updated = std::time::Instant::now();
    }
}

/// Compute line statuses from TextDiff (similar crate)
fn compute_line_statuses_from_textdiff<'a>(
    diff: &TextDiff<'a, 'a, 'a, str>,
) -> (HashMap<usize, LineStatus>, HashMap<usize, usize>) {
    let mut statuses = HashMap::new();
    let mut deleted_after = HashMap::new();
    let mut new_line_idx = 0;

    for change in diff.iter_all_changes() {
        use similar::ChangeTag;

        match change.tag() {
            ChangeTag::Equal => {
                // Unchanged line - just increment counter
                new_line_idx += 1;
            }
            ChangeTag::Insert => {
                // Added line
                statuses.insert(new_line_idx, LineStatus::Added);
                new_line_idx += 1;
            }
            ChangeTag::Delete => {
                // Delete tags will be processed in the second pass
                // to distinguish between modifications and pure deletions
            }
        }
    }

    // Second pass: identify modified lines and count consecutive deletions
    // Process Delete and Insert pairwise:
    // - Delete immediately followed by Insert = Modification (1:1 pairing)
    // - Consecutive Deletes NOT followed by Insert = Pure deletions (count them)
    // - Insert NOT preceded by Delete = Pure addition (already handled in first pass)
    let changes: Vec<_> = diff.iter_all_changes().collect();
    let mut i = 0;
    let mut new_idx = 0;

    while i < changes.len() {
        use similar::ChangeTag;

        match changes[i].tag() {
            ChangeTag::Equal => {
                new_idx += 1;
                i += 1;
            }
            ChangeTag::Delete => {
                // Check if immediately followed by Insert (indicating modification)
                if i + 1 < changes.len() && changes[i + 1].tag() == ChangeTag::Insert {
                    // Modification: pair this Delete with the next Insert
                    statuses.insert(new_idx, LineStatus::Modified);
                    new_idx += 1;
                    i += 2; // Skip both Delete and Insert
                } else {
                    // Count consecutive deletions
                    let mut deletion_count = 0;
                    while i < changes.len() && changes[i].tag() == ChangeTag::Delete {
                        deletion_count += 1;
                        i += 1;
                    }

                    // Place deletion marker after previous line with deletion count
                    let marker_line_idx = if new_idx > 0 { new_idx - 1 } else { 0 };
                    deleted_after.insert(marker_line_idx, deletion_count);

                    // new_idx stays the same (no new lines added)
                }
            }
            ChangeTag::Insert => {
                // Pure insertion (already marked as Added in first pass)
                new_idx += 1;
                i += 1;
            }
        }
    }

    (statuses, deleted_after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_line_statuses_from_textdiff_added() {
        let original = "line1\nline2\nline3\n";
        let current = "line1\nline2\nnew line\nline3\n";

        let diff = similar::TextDiff::from_lines(original, current);
        let (statuses, deleted_after) = compute_line_statuses_from_textdiff(&diff);

        // Line at index 2 should be Added (0-indexed: "new line")
        assert_eq!(statuses.get(&2), Some(&LineStatus::Added));
        // No deletions
        assert!(deleted_after.is_empty());
    }

    #[test]
    fn test_compute_line_statuses_from_textdiff_modified() {
        let original = "line1\noriginal line\nline3\n";
        let current = "line1\nmodified line\nline3\n";

        let diff = similar::TextDiff::from_lines(original, current);
        let (statuses, deleted_after) = compute_line_statuses_from_textdiff(&diff);

        // Line at index 1 should be Modified (0-indexed)
        assert_eq!(statuses.get(&1), Some(&LineStatus::Modified));
        assert!(deleted_after.is_empty());
    }

    #[test]
    fn test_compute_line_statuses_from_textdiff_deleted() {
        let original = "line1\ndeleted line\nline3\n";
        let current = "line1\nline3\n";

        let diff = similar::TextDiff::from_lines(original, current);
        let (statuses, deleted_after) = compute_line_statuses_from_textdiff(&diff);

        // Deletion marker should be after line 0 (where line1 is)
        assert!(deleted_after.contains_key(&0));
        assert_eq!(deleted_after.get(&0), Some(&1)); // 1 line deleted
    }

    #[test]
    fn test_compute_line_statuses_from_textdiff_multiple_deletions() {
        let original = "line1\ndeleted1\ndeleted2\ndeleted3\nline5\n";
        let current = "line1\nline5\n";

        let diff = similar::TextDiff::from_lines(original, current);
        let (statuses, deleted_after) = compute_line_statuses_from_textdiff(&diff);

        // 3 lines deleted after line 0
        assert!(deleted_after.contains_key(&0));
        assert_eq!(deleted_after.get(&0), Some(&3));
        assert!(statuses.is_empty()); // No modifications or additions
    }

    #[test]
    fn test_compute_line_statuses_from_textdiff_mixed_changes() {
        let original = "line1\nold\nline3\n";
        let current = "line1\nnew\nnew2\nline3\n";

        let diff = similar::TextDiff::from_lines(original, current);
        let (statuses, deleted_after) = compute_line_statuses_from_textdiff(&diff);

        // Line 1 modified (old -> new), line 2 added (new2)
        assert_eq!(statuses.get(&1), Some(&LineStatus::Modified));
        assert_eq!(statuses.get(&2), Some(&LineStatus::Added));
        assert!(deleted_after.is_empty());
    }
}
