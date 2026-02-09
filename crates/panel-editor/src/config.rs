//! Editor configuration and information types.

use std::path::PathBuf;
use termide_config::EditorKeybindings;

/// Editor mode configuration
#[derive(Debug, Clone)]
pub struct EditorConfig {
    /// Whether syntax highlighting is enabled
    pub syntax_highlighting: bool,
    /// Read-only mode
    pub read_only: bool,
    /// Automatic line wrapping by window width
    pub word_wrap: bool,
    /// Enable Vim mode
    pub vim_mode: bool,
    /// Tab size (number of spaces)
    pub tab_size: usize,
    /// Initial directory for new buffers (used in SaveAs dialog)
    pub initial_directory: Option<PathBuf>,
    /// Keyboard shortcuts configuration
    pub keybindings: EditorKeybindings,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            syntax_highlighting: true,
            read_only: false,
            word_wrap: true,
            vim_mode: false,
            tab_size: 4,
            initial_directory: None,
            keybindings: EditorKeybindings::default(),
        }
    }
}

impl EditorConfig {
    /// Create configuration for view mode (without editing)
    pub fn view_only() -> Self {
        Self {
            syntax_highlighting: true,
            read_only: true,
            word_wrap: true,
            vim_mode: false,
            tab_size: 4,
            initial_directory: None,
            keybindings: EditorKeybindings::default(),
        }
    }
}

/// Editor information for status bar
#[derive(Debug, Clone)]
pub struct EditorInfo {
    pub line: usize,                    // Current line (1-based)
    pub column: usize,                  // Current column (1-based)
    pub tab_size: usize,                // Tab size
    pub encoding: String,               // Encoding (UTF-8)
    pub line_ending: String,            // Line ending type ("LF" or "CRLF")
    pub file_type: String,              // File type / syntax language
    pub read_only: bool,                // Read-only mode
    pub syntax_highlighting: bool,      // Syntax highlighting enabled
    pub vim_mode: Option<&'static str>, // Vim mode indicator (NORMAL, INSERT, etc.)
}
