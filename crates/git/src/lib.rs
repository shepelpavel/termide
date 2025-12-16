//! Git integration for termide.
//!
//! Provides git status, diff information, and repository utilities.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::mpsc;
use std::sync::OnceLock;

pub mod diff;

/// Execute a git command in the specified directory.
/// Returns None if the command fails or git is not available.
fn git_command(dir: &Path, args: &[&str]) -> Option<Output> {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()
        .filter(|output| output.status.success())
}

/// Execute a git command and return stdout as String.
fn git_command_stdout(dir: &Path, args: &[&str]) -> Option<String> {
    git_command(dir, args).and_then(|output| String::from_utf8(output.stdout).ok())
}

pub use diff::{load_original_async, GitDiffAsyncResult, GitDiffCache, LineStatus};

/// Get git status for a specific file relative to repo root.
pub fn file_status(repo_root: &Path, file_path: &Path) -> GitStatus {
    let relative = match file_path.strip_prefix(repo_root) {
        Ok(rel) => rel,
        Err(_) => return GitStatus::default(),
    };

    let relative_str = relative.to_string_lossy();

    // Check if file is ignored
    if git_command(repo_root, &["check-ignore", "-q", &relative_str]).is_some() {
        return GitStatus::Ignored;
    }

    // Get status
    if let Some(stdout) = git_command_stdout(
        repo_root,
        &["status", "--porcelain=v1", "--", &relative_str],
    ) {
        if let Some(line) = stdout.lines().next() {
            if line.len() >= 2 {
                return parse_status_code(&line[0..2]);
            }
        }
    }

    GitStatus::Unmodified
}

/// Parse git status porcelain code to GitStatus enum.
fn parse_status_code(code: &str) -> GitStatus {
    match code {
        "!!" => GitStatus::Ignored,
        " M" | "M " | "MM" => GitStatus::Modified,
        "A " | " A" | "AM" | "AA" => GitStatus::Added,
        " D" | "D " | "DD" => GitStatus::Deleted,
        "??" => GitStatus::Added,
        _ => GitStatus::Unmodified,
    }
}

/// Global flag for git availability on system.
static GIT_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// Git file status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GitStatus {
    #[default]
    Unmodified,
    Modified,
    Added,
    Deleted,
    Ignored,
}

/// Check if git is available on system.
pub fn is_available() -> bool {
    *GIT_AVAILABLE.get_or_init(|| {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    })
}

/// Alias for backward compatibility.
#[inline]
pub fn check_git_available() -> bool {
    is_available()
}

/// Find git repository root by walking up from a path.
pub fn find_repo_root(path: &Path) -> Option<PathBuf> {
    let mut current = path;
    loop {
        if current.join(".git").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

/// Get git status for directory (synchronous version for compatibility).
pub fn get_git_status(dir: &Path) -> Option<GitStatusCache> {
    if !is_available() {
        return None;
    }

    // Single git command to check repo and get root path
    let repo_root_str = git_command_stdout(dir, &["rev-parse", "--show-toplevel"])?;
    let repo_root = PathBuf::from(repo_root_str.trim());

    let relative_path = dir
        .strip_prefix(&repo_root)
        .unwrap_or(Path::new(""))
        .to_path_buf();

    // Single git status command - parse both status and ignored files
    let mut status_map = HashMap::new();
    let mut ignored_files = HashSet::new();

    if let Some(stdout) = git_command_stdout(&repo_root, &["status", "--porcelain=v1", "--ignored"])
    {
        for line in stdout.lines() {
            if line.len() < 4 {
                continue;
            }

            let status_code = &line[0..2];
            let file_path = &line[3..];

            let status = if status_code == "!!" {
                // Also add to ignored_files for parent directory checks
                ignored_files.insert(PathBuf::from(file_path));
                GitStatus::Ignored
            } else {
                match parse_status_code(status_code) {
                    GitStatus::Unmodified => continue,
                    s => s,
                }
            };

            status_map.insert(PathBuf::from(file_path), status);
        }
    }

    Some(GitStatusCache {
        status_map,
        ignored_files,
        relative_path,
    })
}

/// Result type for async git status loading.
pub struct GitStatusAsyncResult {
    /// Directory path this result is for
    pub dir: PathBuf,
    /// Git status cache (None if not a git repo or error)
    pub cache: Option<GitStatusCache>,
}

/// Load git status asynchronously in a background thread.
/// Returns a receiver that will receive the result when complete.
pub fn get_git_status_async(dir: PathBuf) -> mpsc::Receiver<GitStatusAsyncResult> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let cache = get_git_status(&dir);
        let _ = tx.send(GitStatusAsyncResult { dir, cache });
    });
    rx
}

/// Git status cache for directory.
#[derive(Debug, Clone)]
pub struct GitStatusCache {
    status_map: HashMap<PathBuf, GitStatus>,
    ignored_files: HashSet<PathBuf>,
    relative_path: PathBuf,
}

impl GitStatusCache {
    fn is_parent_ignored(&self, path: &Path) -> bool {
        let mut current = path;
        while let Some(parent) = current.parent() {
            if self.ignored_files.contains(parent) {
                return true;
            }
            if let Some(&GitStatus::Ignored) = self.status_map.get(parent) {
                return true;
            }
            current = parent;
            if parent.as_os_str().is_empty() {
                break;
            }
        }
        false
    }

    pub fn get_status(&self, file_name: &str) -> GitStatus {
        let full_path = if self.relative_path.as_os_str().is_empty() {
            PathBuf::from(file_name)
        } else {
            self.relative_path.join(file_name)
        };

        if self.ignored_files.contains(&full_path) {
            return GitStatus::Ignored;
        }

        if let Some(&status) = self.status_map.get(&full_path) {
            return status;
        }

        if self.is_parent_ignored(&full_path) {
            return GitStatus::Ignored;
        }

        GitStatus::Unmodified
    }

    pub fn is_ignored(&self, file_name: &str) -> bool {
        let full_path = if self.relative_path.as_os_str().is_empty() {
            PathBuf::from(file_name)
        } else {
            self.relative_path.join(file_name)
        };
        self.ignored_files.contains(&full_path)
    }

    pub fn has_changes_in_directory(&self, dir_name: &str) -> bool {
        let full_dir = if self.relative_path.as_os_str().is_empty() {
            PathBuf::from(dir_name)
        } else {
            self.relative_path.join(dir_name)
        };

        let dir_prefix = format!("{}/", full_dir.display());

        self.status_map.iter().any(|(path, status)| {
            if let Some(path_str) = path.to_str() {
                path_str.starts_with(&dir_prefix)
                    && *status != GitStatus::Unmodified
                    && *status != GitStatus::Ignored
            } else {
                false
            }
        })
    }

    pub fn get_directory_status(&self, dir_name: &str) -> GitStatus {
        let full_path = if self.relative_path.as_os_str().is_empty() {
            PathBuf::from(dir_name)
        } else {
            self.relative_path.join(dir_name)
        };

        if let Some(&status) = self.status_map.get(&full_path) {
            if status != GitStatus::Unmodified {
                return status;
            }
        }

        if self.is_parent_ignored(&full_path) {
            return GitStatus::Ignored;
        }

        if self.has_changes_in_directory(dir_name) {
            return GitStatus::Modified;
        }

        GitStatus::Unmodified
    }

    pub fn get_deleted_files(&self) -> Vec<String> {
        self.status_map
            .iter()
            .filter(|(path, status)| {
                **status == GitStatus::Deleted
                    && path
                        .parent()
                        .map(|p| p == self.relative_path)
                        .unwrap_or(self.relative_path.as_os_str().is_empty())
            })
            .filter_map(|(path, _)| path.file_name()?.to_str().map(String::from))
            .collect()
    }

    /// Check if path (relative to repo root) is ignored or inside an ignored directory.
    pub fn is_path_in_ignored(&self, relative_path: &Path) -> bool {
        let path_str = relative_path.to_string_lossy();

        self.ignored_files.iter().any(|ignored| {
            let ignored_str = ignored.to_string_lossy();
            // Normalize: remove trailing slash for comparison
            let ignored_normalized = ignored_str.trim_end_matches('/');

            // Exact match (file or directory name)
            if path_str == ignored_normalized {
                return true;
            }

            // Check if path is inside ignored directory
            let prefix = format!("{}/", ignored_normalized);
            path_str.starts_with(&prefix)
        })
    }
}

/// Git repository status information.
#[derive(Debug, Clone, Copy)]
pub struct GitRepoStatus {
    pub uncommitted_changes: usize,
    pub ahead: usize,
    pub behind: usize,
    pub is_ignored: bool,
}

/// Get git repository status for a specific file or directory.
pub fn get_repo_status(repo_path: &Path, item_path: &Path) -> Option<GitRepoStatus> {
    if !is_available() {
        return None;
    }

    let git_work_dir = if item_path.is_file() {
        item_path.parent().unwrap_or(repo_path)
    } else {
        item_path
    };

    // Check if inside git work tree
    git_command(git_work_dir, &["rev-parse", "--is-inside-work-tree"])?;

    let repo_root_str = git_command_stdout(git_work_dir, &["rev-parse", "--show-toplevel"])?;
    let repo_root = PathBuf::from(repo_root_str.trim());

    let relative_path = item_path.strip_prefix(&repo_root).ok()?;
    let is_repo_root = relative_path.as_os_str().is_empty();
    let git_path_str = if is_repo_root {
        ".".to_string()
    } else {
        relative_path.to_string_lossy().to_string()
    };

    // Repo root cannot be ignored within itself; for other paths check git status
    let is_ignored = if is_repo_root {
        false
    } else {
        git_command_stdout(
            &repo_root,
            &["status", "--porcelain", "--ignored", "--", &git_path_str],
        )
        .map(|stdout| stdout.lines().any(|line| line.starts_with("!! ")))
        .unwrap_or(false)
    };

    let uncommitted_changes =
        git_command_stdout(&repo_root, &["status", "--porcelain", "--", &git_path_str])
            .map(|stdout| stdout.lines().filter(|l| !l.starts_with("!!")).count())
            .unwrap_or(0);

    let ahead = git_command_stdout(&repo_root, &["rev-list", "--count", "@{upstream}..HEAD"])
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    let behind = git_command_stdout(&repo_root, &["rev-list", "--count", "HEAD..@{upstream}"])
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    Some(GitRepoStatus {
        uncommitted_changes,
        ahead,
        behind,
        is_ignored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_repo_root() {
        let current = std::env::current_dir().unwrap();
        if let Some(root) = find_repo_root(&current) {
            assert!(root.join(".git").exists());
        }
    }
}
