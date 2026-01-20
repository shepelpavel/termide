//! Git commit information and log functions.
//!
//! Provides types and functions for working with git commit history.

use std::path::Path;

use crate::command::git_command_stdout;

/// Commit information
#[derive(Debug, Clone)]
pub struct CommitInfo {
    /// Commit hash (short form)
    pub hash: String,
    /// Author name
    pub author: String,
    /// Commit date
    pub date: String,
    /// Commit message (first line)
    pub message: String,
    /// Graph line for display (if using --graph)
    pub graph: Option<String>,
    /// Refs pointing to this commit (HEAD, branches, tags)
    pub refs: Option<String>,
}

/// Get commit log
pub fn get_log(repo: &Path, count: usize) -> Vec<CommitInfo> {
    let count_str = count.to_string();
    // Format: hash, author, date, refs, message
    let format = "%h\t%an\t%ar\t%d\t%s";

    git_command_stdout(
        repo,
        &[
            "log",
            &format!("-{}", count_str),
            &format!("--format={}", format),
        ],
    )
    .map(|stdout| {
        stdout
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(5, '\t').collect();
                if parts.len() == 5 {
                    let refs = if parts[3].is_empty() {
                        None
                    } else {
                        Some(parts[3].trim().to_string())
                    };
                    Some(CommitInfo {
                        hash: parts[0].to_string(),
                        author: parts[1].to_string(),
                        date: parts[2].to_string(),
                        message: parts[4].to_string(),
                        graph: None,
                        refs,
                    })
                } else {
                    None
                }
            })
            .collect()
    })
    .unwrap_or_default()
}

/// Get commit log with graph
pub fn get_log_with_graph(repo: &Path, count: usize) -> Vec<CommitInfo> {
    let count_str = count.to_string();

    // Use a special format that includes graph and refs
    // Format: hash, author, date, refs, message
    git_command_stdout(
        repo,
        &[
            "log",
            &format!("-{}", count_str),
            "--graph",
            "--format=%h\t%an\t%ar\t%d\t%s",
        ],
    )
    .map(|stdout| {
        stdout
            .lines()
            .filter_map(|line| {
                // Graph lines start with *, |, /, \ or space
                // Find where the actual commit info starts
                let graph_end = line.find(|c: char| c.is_ascii_hexdigit()).unwrap_or(0);

                let graph = if graph_end > 0 {
                    Some(line[..graph_end].to_string())
                } else {
                    None
                };

                let info_part = &line[graph_end..];
                let parts: Vec<&str> = info_part.splitn(5, '\t').collect();

                if parts.len() == 5 {
                    let refs = if parts[3].is_empty() {
                        None
                    } else {
                        Some(parts[3].trim().to_string())
                    };
                    Some(CommitInfo {
                        hash: parts[0].to_string(),
                        author: parts[1].to_string(),
                        date: parts[2].to_string(),
                        message: parts[4].to_string(),
                        graph,
                        refs,
                    })
                } else if !info_part.trim().is_empty() {
                    // Handle graph-only lines (merge indicators)
                    Some(CommitInfo {
                        hash: String::new(),
                        author: String::new(),
                        date: String::new(),
                        message: String::new(),
                        graph,
                        refs: None,
                    })
                } else {
                    None
                }
            })
            .collect()
    })
    .unwrap_or_default()
}

/// Get diff for a specific commit (with full patch)
pub fn get_commit_diff(repo: &Path, hash: &str) -> Option<String> {
    git_command_stdout(repo, &["show", "--stat", "--patch", hash])
}

/// Get file diff (for diff viewer)
pub fn get_file_diff(repo: &Path, file: &Path, staged: bool) -> Option<String> {
    let file_str = file.to_string_lossy();
    if staged {
        git_command_stdout(repo, &["diff", "--cached", "--", &file_str])
    } else {
        git_command_stdout(repo, &["diff", "--", &file_str])
    }
}

/// Diff statistics for a file.
#[derive(Debug, Clone, Default)]
pub struct DiffStats {
    /// Number of lines added.
    pub additions: usize,
    /// Number of lines deleted.
    pub deletions: usize,
}

/// Get diff stats for a file (additions/deletions count).
pub fn get_file_diff_stats(repo: &Path, file: &Path, staged: bool) -> DiffStats {
    let file_str = file.to_string_lossy();
    let args: Vec<&str> = if staged {
        vec!["diff", "--cached", "--numstat", "--", &file_str]
    } else {
        vec!["diff", "--numstat", "--", &file_str]
    };

    let output = git_command_stdout(repo, &args);

    // Parse: "10\t5\tfilename" -> additions=10, deletions=5
    if let Some(text) = output {
        if let Some(line) = text.lines().next() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 2 {
                return DiffStats {
                    additions: parts[0].parse().unwrap_or(0),
                    deletions: parts[1].parse().unwrap_or(0),
                };
            }
        }
    }
    DiffStats::default()
}
