//! Utility functions for git panels.
//!
//! Re-exports common text utilities from termide-ui.

pub use termide_ui::path_utils::{truncate_left, truncate_right, truncate_to_width};

/// Alias for backward compatibility.
pub use truncate_left as truncate_path_left;
