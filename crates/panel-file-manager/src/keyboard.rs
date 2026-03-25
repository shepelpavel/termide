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
    /// This function encapsulates all keyboard shortcuts and their modifiers,
    /// making it easy to see all bindings in one place and test them independently.
    ///
    /// # Arguments
    ///
    /// * `key` - The key event to parse (should already be translated via translate_hotkey)
    /// * `keybindings` - Configurable keybindings from config
    /// * `vim_mode` - Whether vim mode is enabled (adds j/k/g/G navigation)
    pub fn from_key_event(
        key: KeyEvent,
        keybindings: &FileManagerKeybindings,
        vim_mode: bool,
    ) -> Self {
        // =================================================================
        // Configurable keybindings (checked first)
        // =================================================================

        // Select all
        if matches_binding_or_default(
            &keybindings.select_all,
            &key,
            KeyCode::Char('a'),
            KeyModifiers::CONTROL,
        ) {
            return Self::SelectAll;
        }

        // Refresh
        if matches_binding_or_default(
            &keybindings.refresh,
            &key,
            KeyCode::Char('r'),
            KeyModifiers::CONTROL,
        ) {
            return Self::Refresh;
        }

        // Toggle selection
        if matches_binding_or_default(
            &keybindings.toggle_selection,
            &key,
            KeyCode::Insert,
            KeyModifiers::NONE,
        ) {
            return Self::ToggleSelection;
        }

        // Go to home directory
        if matches_binding_or_default(
            &keybindings.go_home,
            &key,
            KeyCode::Char('~'),
            KeyModifiers::NONE,
        ) {
            return Self::GoHomeDir;
        }

        // Go to parent directory
        if matches_binding_or_default(
            &keybindings.go_parent,
            &key,
            KeyCode::Backspace,
            KeyModifiers::NONE,
        ) {
            return Self::GoParent;
        }

        // New file (F, Ctrl+N)
        if matches_binding_or_defaults(
            &keybindings.new_file,
            &key,
            &[
                (KeyCode::Char('f'), KeyModifiers::NONE),
                (KeyCode::Char('F'), KeyModifiers::NONE),
                (KeyCode::Char('n'), KeyModifiers::CONTROL),
            ],
        ) {
            return Self::NewFile;
        }

        // File search (Ctrl+F)
        if matches_binding_or_default(
            &keybindings.search,
            &key,
            KeyCode::Char('f'),
            KeyModifiers::CONTROL,
        ) {
            return Self::Search;
        }

        // Content search
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

        // Switch directory - open directory switcher modal
        if matches_binding_or_default(
            &keybindings.switch_directory,
            &key,
            KeyCode::Char('/'),
            KeyModifiers::CONTROL,
        ) {
            return Self::SwitchDirectory;
        }

        // New directory (D, F7)
        if matches_binding_or_defaults(
            &keybindings.new_directory,
            &key,
            &[
                (KeyCode::Char('d'), KeyModifiers::NONE),
                (KeyCode::Char('D'), KeyModifiers::NONE),
                (KeyCode::F(7), KeyModifiers::NONE),
            ],
        ) {
            return Self::NewDirectory;
        }

        // Delete files (Delete, F8)
        if matches_binding_or_defaults(
            &keybindings.delete_files,
            &key,
            &[
                (KeyCode::Delete, KeyModifiers::NONE),
                (KeyCode::F(8), KeyModifiers::NONE),
            ],
        ) {
            return Self::DeleteFiles;
        }

        // Rename file (R, F2)
        if matches_binding_or_defaults(
            &keybindings.rename_file,
            &key,
            &[
                (KeyCode::Char('r'), KeyModifiers::NONE),
                (KeyCode::Char('R'), KeyModifiers::NONE),
                (KeyCode::F(2), KeyModifiers::NONE),
            ],
        ) {
            return Self::RenameFile;
        }

        // Edit file (E, F4)
        if matches_binding_or_defaults(
            &keybindings.edit_file,
            &key,
            &[
                (KeyCode::Char('e'), KeyModifiers::NONE),
                (KeyCode::Char('E'), KeyModifiers::NONE),
                (KeyCode::F(4), KeyModifiers::NONE),
            ],
        ) {
            return Self::EditFile;
        }

        // View file (V, F3)
        if matches_binding_or_defaults(
            &keybindings.view_file,
            &key,
            &[
                (KeyCode::Char('v'), KeyModifiers::NONE),
                (KeyCode::Char('V'), KeyModifiers::NONE),
                (KeyCode::F(3), KeyModifiers::NONE),
            ],
        ) {
            return Self::ViewFile;
        }

        // Open external (o, Ctrl+Enter)
        if matches_binding_or_defaults(
            &keybindings.open_external,
            &key,
            &[
                (KeyCode::Char('o'), KeyModifiers::NONE),
                (KeyCode::Char('O'), KeyModifiers::NONE),
                (KeyCode::Enter, KeyModifiers::CONTROL),
            ],
        ) {
            return Self::OpenExternal;
        }

        // Copy files (C, F5)
        if matches_binding_or_defaults(
            &keybindings.copy_files,
            &key,
            &[
                (KeyCode::Char('c'), KeyModifiers::NONE),
                (KeyCode::Char('C'), KeyModifiers::NONE),
                (KeyCode::F(5), KeyModifiers::NONE),
            ],
        ) {
            return Self::CopyFiles;
        }

        // Move files (M, F6)
        if matches_binding_or_defaults(
            &keybindings.move_files,
            &key,
            &[
                (KeyCode::Char('m'), KeyModifiers::NONE),
                (KeyCode::Char('M'), KeyModifiers::NONE),
                (KeyCode::F(6), KeyModifiers::NONE),
            ],
        ) {
            return Self::MoveFiles;
        }

        // Toggle hidden files
        if matches_binding_or_default(
            &keybindings.toggle_hidden,
            &key,
            KeyCode::Char('.'),
            KeyModifiers::NONE,
        ) {
            return Self::ToggleHidden;
        }

        // =================================================================
        // Non-configurable bindings (navigation, clipboard, basic keys)
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
            // Space - show file information
            (KeyCode::Char(' '), KeyModifiers::NONE) => Self::ShowFileInfo,

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

            // Clipboard
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => Self::ClipboardCopy,
            (KeyCode::Char('x'), KeyModifiers::CONTROL) => Self::ClipboardCut,
            (KeyCode::Char('v'), KeyModifiers::CONTROL) => Self::ClipboardPaste,

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
    fn test_clipboard_keys() {
        let kb = default_keybindings();

        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('c'), KeyModifiers::CONTROL), &kb, false),
            FmCommand::ClipboardCopy
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('x'), KeyModifiers::CONTROL), &kb, false),
            FmCommand::ClipboardCut
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('v'), KeyModifiers::CONTROL), &kb, false),
            FmCommand::ClipboardPaste
        );
    }

    #[test]
    fn test_file_operations() {
        let kb = default_keybindings();

        // View file (V, F3)
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('v'), KeyModifiers::NONE), &kb, false),
            FmCommand::ViewFile
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::F(3), KeyModifiers::NONE), &kb, false),
            FmCommand::ViewFile
        );

        // Edit file (E, F4)
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('e'), KeyModifiers::NONE), &kb, false),
            FmCommand::EditFile
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::F(4), KeyModifiers::NONE), &kb, false),
            FmCommand::EditFile
        );

        // Rename file (R, F2)
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('r'), KeyModifiers::NONE), &kb, false),
            FmCommand::RenameFile
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::F(2), KeyModifiers::NONE), &kb, false),
            FmCommand::RenameFile
        );

        // Copy files (C, F5)
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('c'), KeyModifiers::NONE), &kb, false),
            FmCommand::CopyFiles
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::F(5), KeyModifiers::NONE), &kb, false),
            FmCommand::CopyFiles
        );

        // Move files (M, F6)
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('m'), KeyModifiers::NONE), &kb, false),
            FmCommand::MoveFiles
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::F(6), KeyModifiers::NONE), &kb, false),
            FmCommand::MoveFiles
        );

        // New directory (D, F7)
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('d'), KeyModifiers::NONE), &kb, false),
            FmCommand::NewDirectory
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::F(7), KeyModifiers::NONE), &kb, false),
            FmCommand::NewDirectory
        );

        // Delete files (Delete, F8)
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Delete, KeyModifiers::NONE), &kb, false),
            FmCommand::DeleteFiles
        );
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::F(8), KeyModifiers::NONE), &kb, false),
            FmCommand::DeleteFiles
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

        // Ctrl+F → file search
        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::Char('f'), KeyModifiers::CONTROL), &kb, false),
            FmCommand::Search
        );

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
    fn test_unknown_key_returns_none() {
        let kb = default_keybindings();

        assert_eq!(
            FmCommand::from_key_event(key(KeyCode::F(12), KeyModifiers::NONE), &kb, false),
            FmCommand::None
        );
    }
}
