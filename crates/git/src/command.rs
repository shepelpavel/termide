//! Git command execution utilities.
//!
//! Internal helpers for executing git commands with proper error handling.

use std::path::Path;
use std::process::{Command, Output};

/// Execute a git command in the specified directory.
/// Returns None if the command fails or git is not available.
pub(crate) fn git_command(dir: &Path, args: &[&str]) -> Option<Output> {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()
        .filter(|output| output.status.success())
}

/// Execute a git command and return stdout as String.
pub(crate) fn git_command_stdout(dir: &Path, args: &[&str]) -> Option<String> {
    git_command(dir, args).and_then(|output| String::from_utf8(output.stdout).ok())
}

/// Run a simple git operation, returning Ok(()) on success or error message on failure.
pub(crate) fn run_git_simple(repo: &Path, args: &[&str], error_msg: &str) -> Result<(), String> {
    match git_command(repo, args) {
        Some(_) => Ok(()),
        None => Err(error_msg.to_string()),
    }
}

/// Run a git command capturing stderr for detailed error messages.
pub(crate) fn run_git_with_stderr(repo: &Path, args: &[&str], op_name: &str) -> Result<(), String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|e| format!("Failed to run git {}: {}", op_name, e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("{} failed: {}", op_name, stderr.trim()))
    }
}
