//! Utility functions shared across termide crates.

use std::io::Read;
use std::path::Path;

/// Replace home directory prefix with `~` for display.
/// E.g. "/home/user/projects/foo" → "~/projects/foo"
pub fn shorten_home_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.display().to_string();
        if let Some(rest) = path.strip_prefix(&home_str) {
            return format!("~{rest}");
        }
    }
    path.to_string()
}

/// Check if file appears to be binary (has null bytes in first 8KB).
///
/// This is a heuristic: text files rarely contain null bytes,
/// while binary files (executables, images, etc.) typically do.
pub fn is_binary_file(path: &Path) -> bool {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut reader = std::io::BufReader::new(file);
    let mut buffer = [0u8; 8192];
    let bytes_read = match reader.read(&mut buffer) {
        Ok(n) => n,
        Err(_) => return false,
    };
    buffer[..bytes_read].contains(&0)
}
