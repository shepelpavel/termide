//! Git operations for staging, committing, and syncing.
//!
//! Provides high-level git operations like stage, unstage, commit, push, pull.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::command::{run_git_simple, run_git_with_stderr};

/// Stage a file (add to index)
pub fn stage_file(repo: &Path, file: &Path) -> Result<(), String> {
    let file_str = file.to_string_lossy();
    run_git_simple(
        repo,
        &["add", &file_str],
        &format!("Failed to stage file: {}", file_str),
    )
}

/// Stage multiple files
pub fn stage_files(repo: &Path, files: &[PathBuf]) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }

    let mut args = vec!["add", "--"];
    let file_strs: Vec<String> = files
        .iter()
        .map(|f| f.to_string_lossy().to_string())
        .collect();
    args.extend(file_strs.iter().map(|s| s.as_str()));

    run_git_simple(repo, &args, "Failed to stage files")
}

/// Unstage a file (remove from index)
pub fn unstage_file(repo: &Path, file: &Path) -> Result<(), String> {
    let file_str = file.to_string_lossy();
    run_git_simple(
        repo,
        &["reset", "HEAD", "--", &file_str],
        &format!("Failed to unstage file: {}", file_str),
    )
}

/// Unstage multiple files
pub fn unstage_files(repo: &Path, files: &[PathBuf]) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }

    let mut args = vec!["reset", "HEAD", "--"];
    let file_strs: Vec<String> = files
        .iter()
        .map(|f| f.to_string_lossy().to_string())
        .collect();
    args.extend(file_strs.iter().map(|s| s.as_str()));

    run_git_simple(repo, &args, "Failed to unstage files")
}

/// Stage all changes
pub fn stage_all(repo: &Path) -> Result<(), String> {
    run_git_simple(repo, &["add", "-A"], "Failed to stage all files")
}

/// Unstage all changes
pub fn unstage_all(repo: &Path) -> Result<(), String> {
    run_git_simple(repo, &["reset", "HEAD"], "Failed to unstage all files")
}

/// Create a commit
pub fn commit(repo: &Path, message: &str) -> Result<String, String> {
    let output = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(repo)
        .output()
        .map_err(|e| format!("Failed to run git commit: {}", e))?;

    if output.status.success() {
        // Extract commit hash from output
        let stdout = String::from_utf8_lossy(&output.stdout);
        let hash = stdout
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().last())
            .map(|s| s.trim_matches(|c| c == '[' || c == ']').to_string())
            .unwrap_or_default();
        Ok(hash)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Commit failed: {}", stderr.trim()))
    }
}

/// Revert changes in a file (restore from HEAD)
pub fn revert_file(repo: &Path, file: &Path) -> Result<(), String> {
    let file_str = file.to_string_lossy();
    run_git_simple(
        repo,
        &["checkout", "--", &file_str],
        &format!("Failed to revert file: {}", file_str),
    )
}

/// Push to remote
pub fn push(repo: &Path) -> Result<(), String> {
    run_git_with_stderr(repo, &["push"], "push")
}

/// Pull from remote
pub fn pull(repo: &Path) -> Result<(), String> {
    run_git_with_stderr(repo, &["pull"], "pull")
}

/// Initialize a new git repository
pub fn init_repo(path: &Path) -> Result<(), String> {
    run_git_with_stderr(path, &["init"], "init")
}
