//! Keyboard command handling for the file manager.
//!
//! This module implements the Command Pattern for keyboard input, separating
//! key parsing from command execution for better testability and maintainability.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use termide_config::{
    is_go_end, is_go_home, is_move_down, is_move_up, matches_binding_or_default,
    matches_binding_or_defaults, FileManagerKeybindings,
};

/// File manager command representing a user action.
///
/// This enum captures all possible commands that can be triggered by keyboard input,
/// separating the concern of "what key was pressed" from "what action to perform".
#[derive(Debug, Clone, PartialEq)]
pub enum FmCommand {
    // Navigation
    MoveUp,
    MoveDown,
    PageUp,
    PageDown,
    GoHome,
    GoEnd,
    Enter,
    GoParent,
    GoHomeDir,

    // Selection
    ToggleSelection,
    SelectAll,
    ClearSelection,
    MoveUpWithSelection,
    MoveDownWithSelection,
    PageUpWithSelection,
    PageDownWithSelection,
    SelectToHome,
    SelectToEnd,
    MoveUpWithToggle,
    MoveDownWithToggle,
    PageUpWithToggle,
    PageDownWithToggle,

    // File operations
    NewFile,
    NewDirectory,
    DeleteFiles,
    CopyFiles,
    MoveFiles,
    RenameFile,
    EditFile,
    ViewFile,
    OpenExternal,

    // Search
    Search,
    SearchContent,

    // Clipboard
    ClipboardCopy,
    ClipboardCut,
    ClipboardPaste,

    // Misc
    ShowFileInfo,
    Refresh,
    ToggleHidden,
    NextPanel,
    PrevPanel,
    /// Go to path/URL (Ctrl+G) - opens modal to enter path directly
    GoToPath,
    /// Open directory switcher modal (Ctrl+/)
    SwitchDirectory,
    /// Cancel pending VFS operation (Escape during connection)
    /// Note: Not yet mapped to any key, reserved for future use
    #[allow(dead_code)]
    CancelOperation,

    // Tree expand/collapse
    ExpandDir,
    CollapseDir,

    // No operation
    None,
}

impl FmCommand {
    /// Parse a KeyEvent into an FmCommand.
    ///
    /// This function handles all FM keys including F-keys and global actions.
    ///
    /// # Arguments
    ///
    /// * `key` - The key event to parse (should already be translated via translate_hotkey)
    /// * `keybindings` - Configurable keybindings from config (only panel-specific ones remain)
    /// * `vim_mode` - Whether vim mode is enabled (adds j/k/g/G navigation)
    pub fn from_key_event(
        key: KeyEvent,
        keybindings: &FileManagerKeybindings,
        vim_mode: bool,
    ) -> Self {
        // =================================================================
        // Configurable panel-specific keybindings
        // =================================================================

        // Go to home directory (~)
        if matches_binding_or_default(
            &keybindings.go_home,
            &key,
            KeyCode::Char('~'),
            KeyModifiers::NONE,
        ) {
            return Self::GoHomeDir;
        }

        // Content search (Ctrl+Shift+F)
        if matches_binding_or_default(
            &keybindings.search_content,
            &key,
            KeyCode::Char('F'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ) {
            return Self::SearchContent;
        }

        // Go to path (Ctrl+G) - enter path/URL directly
        if key.code == KeyCode::Char('g') && key.modifiers == KeyModifiers::CONTROL {
            return Self::GoToPath;
        }

        // Switch directory - open directory switcher modal (Ctrl+/)
        if matches_binding_or_default(
            &keybindings.switch_directory,
            &key,
            KeyCode::Char('/'),
            KeyModifiers::CONTROL,
        ) {
            return Self::SwitchDirectory;
        }

        // =================================================================
        // F-key universal actions
        // =================================================================
        match (key.code, key.modifiers) {
            (KeyCode::F(2), KeyModifiers::NONE) => return Self::RenameFile,
            (KeyCode::F(3), KeyModifiers::NONE) => return Self::ViewFile,
            (KeyCode::F(4), KeyModifiers::NONE) => return Self::EditFile,
            (KeyCode::F(5), KeyModifiers::NONE) => return Self::CopyFiles,
            (KeyCode::F(6), KeyModifiers::NONE) => return Self::MoveFiles,
            (KeyCode::F(7), KeyModifiers::NONE) => return Self::NewDirectory,
            (KeyCode::F(8), KeyModifiers::NONE) => return Self::DeleteFiles,
            (KeyCode::F(12), KeyModifiers::NONE) => return Self::ShowFileInfo,
            _ => {}
        }

        // Ctrl+S → rename (same as F2)
        if key.code == KeyCode::Char('s') && key.modifiers == KeyModifiers::CONTROL {
            return Self::RenameFile;
        }
        // Ctrl+N → new directory (same as F7)
        if key.code == KeyCode::Char('n') && key.modifiers == KeyModifiers::CONTROL {
            return Self::NewDirectory;
        }

        // Ctrl+F → search
        if key.code == KeyCode::Char('f') && key.modifiers == KeyModifiers::CONTROL {
            return Self::Search;
        }
        // Ctrl+R → refresh
        if key.code == KeyCode::Char('r') && key.modifiers == KeyModifiers::CONTROL {
            return Self::Refresh;
        }
        // Ctrl+A → select all
        if key.code == KeyCode::Char('a') && key.modifiers == KeyModifiers::CONTROL {
            return Self::SelectAll;
        }
        // Ctrl+C → clipboard copy
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            return Self::ClipboardCopy;
        }
        // Ctrl+X → clipboard cut
        if key.code == KeyCode::Char('x') && key.modifiers == KeyModifiers::CONTROL {
            return Self::ClipboardCut;
        }
        // Ctrl+V → clipboard paste
        if key.code == KeyCode::Char('v') && key.modifiers == KeyModifiers::CONTROL {
            return Self::ClipboardPaste;
        }
        // Insert → toggle selection
        if key.code == KeyCode::Insert && key.modifiers.is_empty() {
            return Self::ToggleSelection;
        }
        // Space → show file info
        if key.code == KeyCode::Char(' ') && key.modifiers.is_empty() {
            return Self::ShowFileInfo;
        }
        // Backspace → go parent
        if key.code == KeyCode::Backspace && key.modifiers.is_empty() {
            return Self::GoParent;
        }

        // =================================================================
        // Hardcoded letter shortcuts (FM-specific, no config needed)
        // =================================================================

        // These are simple letter keys that only make sense in the file manager.
        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
            match key.code {
                // File operations
                KeyCode::Char('c') | KeyCode::Char('C') => return Self::CopyFiles,
                KeyCode::Char('m') | KeyCode::Char('M') => return Self::MoveFiles,
                KeyCode::Char('v') | KeyCode::Char('V') => return Self::ViewFile,
                KeyCode::Char('e') | KeyCode::Char('E') => return Self::EditFile,
                KeyCode::Char('r') | KeyCode::Char('R') => return Self::RenameFile,
                KeyCode::Char('d') | KeyCode::Char('D') => return Self::NewDirectory,
                KeyCode::Char('f') | KeyCode::Char('F') => return Self::NewFile,
                // Toggle hidden files (.)
                KeyCode::Char('.') => {
                    if matches_binding_or_default(
                        &keybindings.toggle_hidden,
                        &key,
                        KeyCode::Char('.'),
                        KeyModifiers::NONE,
                    ) {
                        return Self::ToggleHidden;
                    }
                }
                // Open external (o/O) — Ctrl+Enter also handled via config
                KeyCode::Char('o') | KeyCode::Char('O') => return Self::OpenExternal,
                _ => {}
            }
        }

        // Open external via Ctrl+Enter (configurable)
        if matches_binding_or_defaults(
            &keybindings.open_external,
            &key,
            &[(KeyCode::Enter, KeyModifiers::CONTROL)],
        ) {
            return Self::OpenExternal;
        }

        // Delete files (Delete key — F8 also handled above)
        if matches!(key.code, KeyCode::Delete) && key.modifiers.is_empty() {
            return Self::DeleteFiles;
        }

        // =================================================================
        // Non-configurable bindings (navigation, basic keys)
        // =================================================================

        // Vim-aware navigation (j/k/g/G when vim_mode is enabled)
        if is_move_down(&key, vim_mode) {
            return Self::MoveDown;
        }
        if is_move_up(&key, vim_mode) {
            return Self::MoveUp;
        }
        if is_go_home(&key, vim_mode) {
            return Self::GoHome;
        }
        if is_go_end(&key, vim_mode) {
            return Self::GoEnd;
        }

        match (key.code, key.modifiers) {
            // Selection with Shift
            (KeyCode::Down, KeyModifiers::SHIFT) => Self::MoveDownWithSelection,
            (KeyCode::Up, KeyModifiers::SHIFT) => Self::MoveUpWithSelection,
            (KeyCode::PageDown, KeyModifiers::SHIFT) => Self::PageDownWithSelection,
            (KeyCode::PageUp, KeyModifiers::SHIFT) => Self::PageUpWithSelection,
            (KeyCode::Home, KeyModifiers::SHIFT) => Self::SelectToHome,
            (KeyCode::End, KeyModifiers::SHIFT) => Self::SelectToEnd,

            // Toggle selection with Ctrl
            (KeyCode::Down, KeyModifiers::CONTROL) => Self::MoveDownWithToggle,
            (KeyCode::Up, KeyModifiers::CONTROL) => Self::MoveUpWithToggle,
            (KeyCode::PageDown, KeyModifiers::CONTROL) => Self::PageDownWithToggle,
            (KeyCode::PageUp, KeyModifiers::CONTROL) => Self::PageUpWithToggle,

            // Tree expand/collapse
            (KeyCode::Right, KeyModifiers::NONE) => Self::ExpandDir,
            (KeyCode::Left, KeyModifiers::NONE) => Self::CollapseDir,
            (KeyCode::Char('l'), KeyModifiers::NONE) if vim_mode => Self::ExpandDir,
            (KeyCode::Char('h'), KeyModifiers::NONE) if vim_mode => Self::CollapseDir,

            // Regular navigation (arrows-only, vim handled above)
            (KeyCode::PageUp, KeyModifiers::NONE) => Self::PageUp,
            (KeyCode::PageDown, KeyModifiers::NONE) => Self::PageDown,
            (KeyCode::Enter, KeyModifiers::NONE) => Self::Enter,
            (KeyCode::Esc, KeyModifiers::NONE) => Self::ClearSelection,

            // Backspace with any modifiers (go to parent)
            (KeyCode::Backspace, mods) if mods != KeyModifiers::NONE => Self::GoParent,

            // Panel navigation
            (KeyCode::Tab, KeyModifiers::NONE) => Self::NextPanel,
            (KeyCode::BackTab, _) => Self::PrevPanel,

            _ => Self::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn default_keybindings() -> FileManagerKeybindings {
        FileManagerKeybindings::default()
    }

    #[test]
    fn test_navigation_keys() {
        let kb = default_keybindings();

        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Up, KeyModifiers::NONE), &kb, false),
            FmCommand::MoveUp
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Down, KeyModifiers::NONE), &kb, false),
            FmCommand::MoveDown
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::PageUp, KeyModifiers::NONE), &kb, false),
            FmCommand::PageUp
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::PageDown, KeyModifiers::NONE), &kb, false),
            FmCommand::PageDown
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Home, KeyModifiers::NONE), &kb, false),
            FmCommand::GoHome
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::End, KeyModifiers::NONE), &kb, false),
            FmCommand::GoEnd
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Enter, KeyModifiers::NONE), &kb, false),
            FmCommand::Enter
        );
    }

    #[test]
    fn test_tree_expand_collapse_keys() {
        let kb = default_keybindings();

        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Right, KeyModifiers::NONE), &kb, false),
            FmCommand::ExpandDir
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Left, KeyModifiers::NONE), &kb, false),
            FmCommand::CollapseDir
        );

        // Vim mode: l/h for expand/collapse
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('l'), KeyModifiers::NONE), &kb, true),
            FmCommand::ExpandDir
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('h'), KeyModifiers::NONE), &kb, true),
            FmCommand::CollapseDir
        );
    }

    #[test]
    fn test_vim_navigation_keys() {
        let kb = default_keybindings();

        // Vim keys should not work when vim_mode is false
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('j'), KeyModifiers::NONE), &kb, false),
            FmCommand::None
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('k'), KeyModifiers::NONE), &kb, false),
            FmCommand::None
        );

        // Vim keys should work when vim_mode is true
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('j'), KeyModifiers::NONE), &kb, true),
            FmCommand::MoveDown
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('k'), KeyModifiers::NONE), &kb, true),
            FmCommand::MoveUp
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('g'), KeyModifiers::NONE), &kb, true),
            FmCommand::GoHome
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('G'), KeyModifiers::SHIFT), &kb, true),
            FmCommand::GoEnd
        );
    }

    #[test]
    fn test_selection_keys() {
        let kb = default_keybindings();

        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Down, KeyModifiers::SHIFT), &kb, false),
            FmCommand::MoveDownWithSelection
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Up, KeyModifiers::SHIFT), &kb, false),
            FmCommand::MoveUpWithSelection
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Esc, KeyModifiers::NONE), &kb, false),
            FmCommand::ClearSelection
        );
    }

    #[test]
    fn test_file_operations() {
        let kb = default_keybindings();

        // Letter shortcuts (handled by from_key_event via Other)
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('v'), KeyModifiers::NONE), &kb, false),
            FmCommand::ViewFile
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('e'), KeyModifiers::NONE), &kb, false),
            FmCommand::EditFile
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('r'), KeyModifiers::NONE), &kb, false),
            FmCommand::RenameFile
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('c'), KeyModifiers::NONE), &kb, false),
            FmCommand::CopyFiles
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('m'), KeyModifiers::NONE), &kb, false),
            FmCommand::MoveFiles
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('d'), KeyModifiers::NONE), &kb, false),
            FmCommand::NewDirectory
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Delete, KeyModifiers::NONE), &kb, false),
            FmCommand::DeleteFiles
        );

        // F-keys (F2-F8, F12) are now handled directly by from_key_event.
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::F(2), KeyModifiers::NONE), &kb, false),
            FmCommand::RenameFile
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::F(4), KeyModifiers::NONE), &kb, false),
            FmCommand::EditFile
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::F(5), KeyModifiers::NONE), &kb, false),
            FmCommand::CopyFiles
        );
    }

    #[test]
    fn test_panel_navigation() {
        let kb = default_keybindings();

        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Tab, KeyModifiers::NONE), &kb, false),
            FmCommand::NextPanel
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::BackTab, KeyModifiers::SHIFT), &kb, false),
            FmCommand::PrevPanel
        );
    }

    #[test]
    fn test_toggle_hidden() {
        let kb = default_keybindings();

        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('.'), KeyModifiers::NONE), &kb, false),
            FmCommand::ToggleHidden
        );
    }

    #[test]
    fn test_search_keys() {
        let kb = default_keybindings();

        // Ctrl+Shift+F → content search
        assert_eq!(
            FmCommand::from_key_event(
                key(
                    KeyCode::Char('F'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT
                ),
                &kb,
                false
            ),
            FmCommand::SearchContent
        );
    }

    #[test]
    fn test_f12_returns_show_file_info() {
        let kb = default_keybindings();

        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::F(12), KeyModifiers::NONE), &kb, false),
            FmCommand::ShowFileInfo
        );
    }
}
