//! Utility functions shared across termide crates.

use std::io::Read;
use std::path::Path;

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
