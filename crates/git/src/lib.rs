//! Git integration for termide.
//!
//! Provides git status, diff information, and repository utilities.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::sync::OnceLock;

// Internal modules
pub(crate) mod command;
mod commits;
mod files;
mod operations;
mod repo_manager;
mod utils;

// Public submodules
pub mod diff;

// Re-export from internal modules
pub use commits::{
    get_commit_details, get_commit_diff, get_file_diff, get_file_diff_stats, get_log,
    get_log_with_graph, CommitDetails, CommitInfo, DiffStats,
};
pub use files::{get_staged_files, get_unstaged_files, StagedFile, UnstagedFile};
pub use operations::{
    commit, fetch, init_repo, pull, push, revert_file, stage_all, stage_file, stage_files,
    unstage_all, unstage_file, unstage_files,
};
pub use repo_manager::RepoManager;
pub use utils::{truncate_left, truncate_path_left, truncate_right, truncate_to_width};

// Re-export diff types
pub use diff::{
    compute_inline_diff, load_original_async, GitDiffAsyncResult, GitDiffCache, InlineChange,
    InlineChangeType, LineStatus,
};

// Import command helpers for use in this module
use command::{git_command, git_command_stdout, run_git_simple};

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

/// Find the top-level repository root, skipping submodules.
///
/// Submodules have `.git` as a file (not directory) containing `gitdir: ...`.
/// This function continues searching upward until it finds a repository
/// with `.git` as a directory (the actual root repo).
pub fn find_toplevel_repo(path: &Path) -> Option<PathBuf> {
    let mut current = path;

    loop {
        let git_path = current.join(".git");
        if git_path.exists() {
            // If .git is a directory (not a file), this is the top-level repo
            if git_path.is_dir() {
                return Some(current.to_path_buf());
            }
            // Otherwise it's a submodule (.git is a file), continue searching up
        }
        current = current.parent()?;
    }
}

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
    // Use -c core.quotepath=false to show non-ASCII characters properly
    let mut status_map = HashMap::new();
    let mut ignored_files = HashSet::new();

    if let Some(stdout) = git_command_stdout(
        &repo_root,
        &[
            "-c",
            "core.quotepath=false",
            "status",
            "--porcelain=v1",
            "--ignored",
        ],
    ) {
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

    // Pre-compute directories with changes for O(1) lookup
    let dirs_with_changes: HashSet<PathBuf> = status_map
        .iter()
        .filter(|(_, status)| **status != GitStatus::Unmodified && **status != GitStatus::Ignored)
        .flat_map(|(path, _)| {
            // Collect all ancestor directories of this path
            let mut ancestors = Vec::new();
            let mut current = path.as_path();
            while let Some(parent) = current.parent() {
                if parent.as_os_str().is_empty() {
                    break;
                }
                ancestors.push(parent.to_path_buf());
                current = parent;
            }
            ancestors
        })
        .collect();

    Some(GitStatusCache {
        status_map,
        ignored_files,
        relative_path,
        dirs_with_changes,
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
    /// Pre-computed set of directories that contain changes (for O(1) lookup)
    dirs_with_changes: HashSet<PathBuf>,
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

        // O(1) lookup instead of O(n) iteration
        self.dirs_with_changes.contains(&full_dir)
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
    ///
    /// Uses path component comparison to avoid string allocations in hot path.
    pub fn is_path_in_ignored(&self, relative_path: &Path) -> bool {
        // Check if exact path is ignored
        if self.ignored_files.contains(relative_path) {
            return true;
        }

        // Check if any ancestor is ignored (path is inside ignored directory)
        let mut ancestor = relative_path;
        while let Some(parent) = ancestor.parent() {
            if !parent.as_os_str().is_empty() && self.ignored_files.contains(parent) {
                return true;
            }
            ancestor = parent;
        }

        false
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
/// Optimized to use minimal git process spawns (2 instead of 6).
pub fn get_repo_status(repo_path: &Path, item_path: &Path) -> Option<GitRepoStatus> {
    if !is_available() {
        return None;
    }

    let git_work_dir = if item_path.is_file() {
        item_path.parent().unwrap_or(repo_path)
    } else {
        item_path
    };

    // Single call to get repo root (also validates we're in a git repo)
    let repo_root_str = git_command_stdout(git_work_dir, &["rev-parse", "--show-toplevel"])?;
    let repo_root = PathBuf::from(repo_root_str.trim());

    let relative_path = item_path.strip_prefix(&repo_root).ok()?;
    let is_repo_root = relative_path.as_os_str().is_empty();
    let git_path_str = if is_repo_root {
        ".".to_string()
    } else {
        relative_path.to_string_lossy().to_string()
    };

    // Single git status call with branch info and ignored files
    // Output format:
    //   ## branch...origin/branch [ahead N, behind M]
    //   !! ignored/file
    //    M modified/file
    // Use -c core.quotepath=false to show non-ASCII characters properly
    let status_output = git_command_stdout(
        &repo_root,
        &[
            "-c",
            "core.quotepath=false",
            "status",
            "--porcelain=v1",
            "-b",
            "--ignored",
            "--",
            &git_path_str,
        ],
    )
    .unwrap_or_default();

    let (ahead, behind, uncommitted_changes, is_ignored) =
        parse_git_status_output(&status_output, is_repo_root);

    Some(GitRepoStatus {
        uncommitted_changes,
        ahead,
        behind,
        is_ignored,
    })
}

/// Parse git status --porcelain=v1 -b --ignored output.
/// Returns (ahead, behind, uncommitted_changes, is_ignored).
fn parse_git_status_output(output: &str, is_repo_root: bool) -> (usize, usize, usize, bool) {
    let mut ahead = 0;
    let mut behind = 0;
    let mut uncommitted_changes = 0;
    let mut is_ignored = false;

    for line in output.lines() {
        if line.starts_with("## ") {
            // Parse branch line: "## main...origin/main [ahead 2, behind 1]"
            if let Some(bracket_start) = line.find('[') {
                let tracking_info = &line[bracket_start..];
                // Parse ahead count
                if let Some(ahead_pos) = tracking_info.find("ahead ") {
                    let start = ahead_pos + 6;
                    let end = tracking_info[start..]
                        .find(|c: char| !c.is_ascii_digit())
                        .map(|i| start + i)
                        .unwrap_or(tracking_info.len());
                    ahead = tracking_info[start..end].parse().unwrap_or(0);
                }
                // Parse behind count
                if let Some(behind_pos) = tracking_info.find("behind ") {
                    let start = behind_pos + 7;
                    let end = tracking_info[start..]
                        .find(|c: char| !c.is_ascii_digit())
                        .map(|i| start + i)
                        .unwrap_or(tracking_info.len());
                    behind = tracking_info[start..end].parse().unwrap_or(0);
                }
            }
        } else if line.starts_with("!! ") {
            // Ignored file - only count if not repo root
            if !is_repo_root {
                is_ignored = true;
            }
        } else if line.len() >= 2 && !line.starts_with("## ") {
            // Any other status line is an uncommitted change
            uncommitted_changes += 1;
        }
    }

    (ahead, behind, uncommitted_changes, is_ignored)
}

/// Get current branch name
pub fn get_current_branch(repo: &Path) -> Option<String> {
    git_command_stdout(repo, &["rev-parse", "--abbrev-ref", "HEAD"]).map(|s| s.trim().to_string())
}

/// Get list of all local branches
pub fn get_branches(repo: &Path) -> Vec<String> {
    git_command_stdout(repo, &["branch", "--format=%(refname:short)"])
        .map(|s| s.lines().map(|l| l.to_string()).collect())
        .unwrap_or_default()
}

/// Get all branches (local and remote-tracking).
/// Remote branches are included only if there's no local branch with the same name.
/// For example, if "main" exists locally, "origin/main" is not included.
pub fn get_all_branches(repo: &Path) -> Vec<String> {
    // Get local branches first
    let local_branches: Vec<String> = get_branches(repo);

    // Get remote branches
    let remote_branches: Vec<String> =
        git_command_stdout(repo, &["branch", "-r", "--format=%(refname:short)"])
            .map(|s| {
                s.lines()
                    .map(|l| l.to_string())
                    // Filter out HEAD pointer (e.g., "origin/HEAD")
                    .filter(|b| !b.ends_with("/HEAD"))
                    .collect()
            })
            .unwrap_or_default();

    // Combine: local branches + remote branches that don't have a local equivalent
    let mut result = local_branches.clone();
    for remote in remote_branches {
        // Extract branch name after "origin/" (or other remote name)
        if let Some(branch_name) = remote.split('/').nth(1) {
            // Only add if no local branch with this name exists
            if !local_branches.iter().any(|local| local == branch_name) {
                result.push(remote);
            }
        }
    }
    result
}

/// Switch to a different branch.
/// If branch looks like a remote branch (contains '/'), use --track to create a local tracking branch.
pub fn checkout_branch(repo: &Path, branch: &str) -> Result<(), String> {
    // If branch looks like a remote branch (contains '/'), use --track
    let args: Vec<&str> = if branch.contains('/') {
        vec!["checkout", "--track", branch]
    } else {
        vec!["checkout", branch]
    };
    run_git_simple(
        repo,
        &args,
        &format!("Failed to checkout branch: {}", branch),
    )
}

/// Find all git repositories under a root directory up to max_depth
pub fn find_all_repos(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    find_repos_recursive(root, 0, max_depth, &mut repos);
    repos
}

/// Find repositories based on a list of paths.
///
/// For each path:
/// - Searches UP to find the repository root
/// - Searches DOWN (up to submodule_depth) to find submodules
///
/// Optimizations: deduplicates paths, removes nested paths, skips already-scanned repos.
pub fn find_repos_from_paths(paths: &[PathBuf], submodule_depth: usize) -> Vec<PathBuf> {
    use std::collections::HashSet;

    if paths.is_empty() {
        return Vec::new();
    }

    // Deduplicate and sort paths
    let mut unique_paths: Vec<PathBuf> = paths
        .iter()
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    unique_paths.sort();

    // Remove nested paths (keep only the shortest/parent paths)
    let filtered_paths = remove_nested_paths(&unique_paths);

    let mut repos = HashSet::new();
    let mut searched_roots = HashSet::new();

    for path in filtered_paths {
        // Search UP to find TOP-LEVEL repository root (not submodule)
        if let Some(repo_root) = find_toplevel_repo(&path) {
            // Skip if we already scanned this repo
            if searched_roots.contains(&repo_root) {
                continue;
            }
            searched_roots.insert(repo_root.clone());
            repos.insert(repo_root.clone());

            // Search DOWN for submodules
            let submodules = find_all_repos(&repo_root, submodule_depth);
            for submodule in submodules {
                repos.insert(submodule);
            }
        }
    }

    let mut result: Vec<PathBuf> = repos.into_iter().collect();
    result.sort();
    result
}

/// Remove paths that are nested inside other paths.
/// E.g., ["/repo", "/repo/src", "/repo/src/lib"] -> ["/repo"]
///
/// Optimized from O(n²) to O(n log n) by leveraging sorted order:
/// after sorting, a parent path always comes before its children,
/// so we only need to check against the last added path.
///
/// Note: Input must be pre-sorted. If not sorted, behavior is undefined.
fn remove_nested_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    if paths.is_empty() {
        return Vec::new();
    }

    // Input is expected to be pre-sorted (sorted by caller)
    let mut result = Vec::with_capacity(paths.len());

    for path in paths {
        // After sorting, parent always comes before child.
        // So we only need to check if current path is nested under the last added path.
        let is_nested = result
            .last()
            .is_some_and(|last: &PathBuf| path.starts_with(last));
        if !is_nested {
            result.push(path.clone());
        }
    }

    result
}

fn find_repos_recursive(dir: &Path, depth: usize, max_depth: usize, repos: &mut Vec<PathBuf>) {
    if depth > max_depth {
        return;
    }

    // Check if this directory is a git repo
    if !dir.join(".git").exists() {
        // Not a git repo, scan subdirectories
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if !name.starts_with('.') {
                            find_repos_recursive(&path, depth + 1, max_depth, repos);
                        }
                    }
                }
            }
        }
        return;
    }

    // This is a git repo
    repos.push(dir.to_path_buf());

    // Parse .gitmodules to find submodules
    let gitmodules_path = dir.join(".gitmodules");
    if gitmodules_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&gitmodules_path) {
            for line in content.lines() {
                let line = line.trim();
                if let Some(path_value) = line.strip_prefix("path = ") {
                    let submodule_path = dir.join(path_value.trim());
                    if submodule_path.join(".git").exists() {
                        // Recursively find submodules in this submodule
                        find_repos_recursive(&submodule_path, depth + 1, max_depth, repos);
                    }
                }
            }
        }
    }
}

/// Get repository name (folder name containing .git)
pub fn get_repo_name(repo: &Path) -> String {
    repo.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repository")
        .to_string()
}

/// Get ahead/behind counts relative to upstream
pub fn get_ahead_behind(repo: &Path) -> (usize, usize) {
    git_command_stdout(
        repo,
        &["rev-list", "--left-right", "--count", "@{u}...HEAD"],
    )
    .and_then(|s| {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() == 2 {
            let behind = parts[0].parse().unwrap_or(0);
            let ahead = parts[1].parse().unwrap_or(0);
            Some((ahead, behind))
        } else {
            None
        }
    })
    .unwrap_or_else(|| {
        // No upstream tracking branch — count all local commits as ahead
        let ahead = git_command_stdout(repo, &["rev-list", "--count", "HEAD"])
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        (ahead, 0)
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

    #[test]
    fn test_parse_git_status_branch_with_tracking() {
        let output = "## main...origin/main [ahead 2, behind 3]\n M file.rs\n";
        let (ahead, behind, changes, ignored) = parse_git_status_output(output, false);
        assert_eq!(ahead, 2);
        assert_eq!(behind, 3);
        assert_eq!(changes, 1);
        assert!(!ignored);
    }

    #[test]
    fn test_parse_git_status_ahead_only() {
        let output = "## feature...origin/feature [ahead 5]\n";
        let (ahead, behind, changes, _) = parse_git_status_output(output, false);
        assert_eq!(ahead, 5);
        assert_eq!(behind, 0);
        assert_eq!(changes, 0);
    }

    #[test]
    fn test_parse_git_status_behind_only() {
        let output = "## main...origin/main [behind 1]\n";
        let (ahead, behind, _, _) = parse_git_status_output(output, false);
        assert_eq!(ahead, 0);
        assert_eq!(behind, 1);
    }

    #[test]
    fn test_parse_git_status_ignored_files() {
        let output = "## main\n!! ignored.txt\n M changed.rs\n";
        let (_, _, changes, ignored) = parse_git_status_output(output, false);
        assert!(ignored);
        assert_eq!(changes, 1); // Only the M line, not the !! line
    }

    #[test]
    fn test_parse_git_status_repo_root_not_ignored() {
        let output = "## main\n!! some_ignored\n";
        let (_, _, _, ignored) = parse_git_status_output(output, true);
        assert!(!ignored); // Repo root cannot be ignored
    }

    #[test]
    fn test_parse_git_status_no_tracking() {
        let output = "## main\n M file.rs\n?? new.txt\n";
        let (ahead, behind, changes, _) = parse_git_status_output(output, false);
        assert_eq!(ahead, 0);
        assert_eq!(behind, 0);
        assert_eq!(changes, 2); // M and ?? lines
    }
}
