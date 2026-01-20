//! Git file status types and functions.
//!
//! Provides types and functions for working with staged and unstaged files.

use std::path::{Path, PathBuf};

use crate::command::git_command_stdout;

/// Staged file information
#[derive(Debug, Clone)]
pub struct StagedFile {
    /// Path relative to repo root
    pub path: PathBuf,
    /// Status code (M=modified, A=added, D=deleted, R=renamed)
    pub status: char,
}

/// Unstaged file information
#[derive(Debug, Clone)]
pub struct UnstagedFile {
    /// Path relative to repo root
    pub path: PathBuf,
    /// Status code (M=modified, D=deleted)
    pub status: char,
    /// Is this an untracked file
    pub untracked: bool,
}

/// Get staged files (files in index ready for commit)
pub fn get_staged_files(repo: &Path) -> Vec<StagedFile> {
    let mut result = Vec::new();

    // Use -c core.quotepath=false to show non-ASCII characters properly
    if let Some(stdout) = git_command_stdout(
        repo,
        &[
            "-c",
            "core.quotepath=false",
            "diff",
            "--cached",
            "--name-status",
        ],
    ) {
        for line in stdout.lines() {
            if let Some((status, path)) = line.split_once('\t') {
                if let Some(status_char) = status.chars().next() {
                    result.push(StagedFile {
                        path: PathBuf::from(path),
                        status: status_char,
                    });
                }
            }
        }
    }

    result
}

/// Get unstaged files (modified files not in index) and untracked files
pub fn get_unstaged_files(repo: &Path) -> Vec<UnstagedFile> {
    let mut result = Vec::new();

    // Get modified but not staged files
    // Use -c core.quotepath=false to show non-ASCII characters properly
    if let Some(stdout) = git_command_stdout(
        repo,
        &["-c", "core.quotepath=false", "diff", "--name-status"],
    ) {
        for line in stdout.lines() {
            if let Some((status, path)) = line.split_once('\t') {
                if let Some(status_char) = status.chars().next() {
                    result.push(UnstagedFile {
                        path: PathBuf::from(path),
                        status: status_char,
                        untracked: false,
                    });
                }
            }
        }
    }

    // Get untracked files
    // Use -c core.quotepath=false to show non-ASCII characters properly
    if let Some(stdout) = git_command_stdout(
        repo,
        &[
            "-c",
            "core.quotepath=false",
            "ls-files",
            "--others",
            "--exclude-standard",
        ],
    ) {
        for line in stdout.lines() {
            if !line.is_empty() {
                result.push(UnstagedFile {
                    path: PathBuf::from(line),
                    status: '?',
                    untracked: true,
                });
            }
        }
    }

    result
}
