//! Git command execution utilities.
//!
//! Internal helpers for executing git commands with proper error handling.

use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Build a `git` command hardened so it can never block on or corrupt the
/// controlling terminal: git's interactive credential prompt is disabled and
/// stdin is detached. Used for local, non-network git invocations (status, log,
/// diff, blame, …).
fn hardened_git(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    cmd
}

/// Build a hardened `git` command for a NETWORK operation (fetch/pull/push).
///
/// Network operations can trigger ssh / credential prompts. Without hardening,
/// ssh opens `/dev/tty` directly to ask for a key passphrase and writes the
/// prompt straight over the TUI. Here stdin is detached, git's own prompt is
/// disabled, and ssh runs in `BatchMode` so a missing passphrase fails cleanly
/// instead of prompting — keys already loaded in ssh-agent keep working.
/// `stdout`/`stderr` are piped for the caller to capture; the command is
/// returned ready to `.spawn()`.
pub fn network_command(repo: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// Execute a git command in the specified directory.
/// Returns None if the command fails or git is not available.
pub(crate) fn git_command(dir: &Path, args: &[&str]) -> Option<Output> {
    hardened_git(dir, args)
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
    let output = hardened_git(repo, args)
        .output()
        .map_err(|e| format!("Failed to run git {}: {}", op_name, e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("{} failed: {}", op_name, stderr.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::network_command;
    use std::ffi::OsStr;
    use std::path::Path;

    // A network git command must never be able to prompt on the controlling
    // terminal (issue: SSH passphrase prompt drawn over the TUI). Verify the
    // hardening env is in place.
    #[test]
    fn network_command_disables_interactive_prompts() {
        let cmd = network_command(Path::new("/tmp"), &["fetch", "origin"]);

        let mut terminal_prompt = None;
        let mut ssh_command = None;
        for (k, v) in cmd.get_envs() {
            if k == OsStr::new("GIT_TERMINAL_PROMPT") {
                terminal_prompt = v.map(|s| s.to_string_lossy().into_owned());
            } else if k == OsStr::new("GIT_SSH_COMMAND") {
                ssh_command = v.map(|s| s.to_string_lossy().into_owned());
            }
        }

        assert_eq!(terminal_prompt.as_deref(), Some("0"));
        assert!(
            ssh_command
                .as_deref()
                .unwrap_or("")
                .contains("BatchMode=yes"),
            "GIT_SSH_COMMAND must run ssh in BatchMode, got {ssh_command:?}"
        );

        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["fetch", "origin"]);
    }
}
