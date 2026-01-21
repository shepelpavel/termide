//! Shell detection and configuration utilities.
//!
//! This module provides functions for detecting available shells
//! and determining appropriate arguments for launching them.

/// Detect available shell on the system.
///
/// Checks in order:
/// 1. NixOS system shells (fish, zsh, bash)
/// 2. $SHELL environment variable
/// 3. Common shell paths (/usr/bin/fish, /usr/bin/zsh, /bin/bash, /bin/sh)
pub fn detect_shell() -> String {
    // On NixOS first check bash-interactive in system profile
    // (regular bash in nix store might be without readline)
    let nixos_shells = [
        "/run/current-system/sw/bin/fish",
        "/run/current-system/sw/bin/zsh",
        "/run/current-system/sw/bin/bash",
    ];
    for shell in nixos_shells {
        if std::path::Path::new(shell).exists() {
            return shell.to_string();
        }
    }

    // Then check $SHELL
    if let Ok(shell) = std::env::var("SHELL") {
        if std::path::Path::new(&shell).exists() {
            return shell;
        }
    }

    // Check popular shells on regular systems
    let shells = ["/usr/bin/fish", "/usr/bin/zsh", "/bin/bash", "/bin/sh"];
    for shell in shells {
        if std::path::Path::new(shell).exists() {
            return shell.to_string();
        }
    }

    "/bin/sh".to_string()
}

/// Get arguments for launching the shell.
///
/// Different shells require different arguments for proper interactive mode:
/// - fish: `-l` (login shell)
/// - zsh: `-l -i` (login + interactive)
/// - bash: no args (PTY makes it interactive automatically)
pub fn get_shell_args(shell_path: &str) -> Vec<&'static str> {
    let shell_name = std::path::Path::new(shell_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    match shell_name {
        "fish" => vec!["-l"],      // login shell
        "zsh" => vec!["-l", "-i"], // login + interactive
        "bash" => vec![],          // PTY will make it interactive automatically
        _ => vec![],               // no arguments
    }
}
