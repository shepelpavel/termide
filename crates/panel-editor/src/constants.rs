//! Constants for the editor panel.

pub use termide_config::constants::MEGABYTE;

/// Maximum file size that can be opened in the editor (50 MB).
pub const MAX_EDITOR_FILE_SIZE: u64 = 50 * MEGABYTE;
