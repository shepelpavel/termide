//! Keyboard command handling for the editor.
//!
//! This module implements the Command Pattern for keyboard input, separating
//! key parsing from command execution for better testability and maintainability.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use termide_config::{matches_binding_or_default, matches_binding_or_defaults, EditorKeybindings};

/// Editor command representing a user action.
///
/// This enum captures all possible commands that can be triggered by keyboard input,
/// separating the concern of "what key was pressed" from "what action to perform".
#[derive(Debug, Clone, PartialEq)]
pub enum EditorCommand {
    // Navigation commands (clear selection and close search)
    MoveCursorUp,
    MoveCursorDown,
    MoveCursorLeft,
    MoveCursorRight,
    #[allow(dead_code)] // Mapped to MoveToVisualLineStart in from_key_event
    MoveToLineStart,
    #[allow(dead_code)] // Mapped to MoveToVisualLineEnd in from_key_event
    MoveToLineEnd,
    MoveToVisualLineStart,
    MoveToVisualLineEnd,
    PageUp,
    PageDown,
    MoveToDocumentStart,
    MoveToDocumentEnd,
    MoveWordForward,
    MoveWordBackward,

    // Navigation with selection (Shift modifier, closes search)
    MoveCursorUpWithSelection,
    MoveCursorDownWithSelection,
    MoveCursorLeftWithSelection,
    MoveCursorRightWithSelection,
    #[allow(dead_code)] // Mapped to MoveToVisualLineStartWithSelection in from_key_event
    MoveToLineStartWithSelection,
    #[allow(dead_code)] // Mapped to MoveToVisualLineEndWithSelection in from_key_event
    MoveToLineEndWithSelection,
    MoveToVisualLineStartWithSelection,
    MoveToVisualLineEndWithSelection,
    PageUpWithSelection,
    PageDownWithSelection,
    MoveToDocumentStartWithSelection,
    MoveToDocumentEndWithSelection,
    MoveWordForwardWithSelection,
    MoveWordBackwardWithSelection,
    MoveParagraphUp,
    MoveParagraphDown,
    MoveParagraphUpWithSelection,
    MoveParagraphDownWithSelection,

    // Text editing
    InsertChar(char),
    InsertTab,
    IndentLines,
    UnindentLines,
    InsertNewline,
    Backspace,
    Delete,

    // Undo/Redo
    Undo,
    Redo,

    // File operations
    Save,
    /// Save file with new name/path
    SaveAs,
    /// Force save (ignore external changes)
    #[allow(dead_code)]
    ForceSave,
    /// Reload file from disk (discard local changes)
    ReloadFromDisk,

    // Selection
    SelectAll,

    // Clipboard
    Copy,
    Cut,
    Paste,

    // Advanced editing
    DuplicateLine,
    ToggleComment,

    // Search
    StartSearch,
    SearchNext,
    SearchPrev,
    CloseSearch,
    SearchNextOrOpen,
    SearchPrevOrOpen,

    // Replace
    StartReplace,
    ReplaceNext,
    ReplaceAll,

    // LSP Completion
    /// Trigger completion popup (Ctrl+Space)
    TriggerCompletion,
    /// Accept selected completion (Enter/Tab when popup open)
    AcceptCompletion,
    /// Cancel completion popup (Escape)
    CancelCompletion,
    /// Select next completion item (Down arrow when popup open)
    NextCompletion,
    /// Select previous completion item (Up arrow when popup open)
    PrevCompletion,
    /// Filter completion with typed character
    FilterCompletion(char),
    /// Delete last filter character (Backspace when popup open)
    BackspaceCompletion,

    // Git
    /// Toggle inline blame annotation on the cursor line (no default key)
    ToggleBlame,

    // LSP Hover
    /// Show hover documentation (Ctrl+K)
    ShowHover,

    // LSP Go-to-Definition
    /// Go to definition (F12)
    GotoDefinition,

    // LSP Find References
    /// Find all references (Shift+F12)
    FindReferences,

    // LSP Rename Symbol
    /// Rename symbol at cursor (F2)
    RenameSymbol,

    // No operation (for unhandled keys)
    None,
}

impl EditorCommand {
    /// Parse a KeyEvent into an EditorCommand.
    ///
    /// This function encapsulates all keyboard shortcuts and their modifiers,
    /// making it easy to see all bindings in one place and test them independently.
    ///
    /// # Arguments
    ///
    /// * `key` - The key event to parse (should already be translated via translate_hotkey)
    /// * `read_only` - Whether the editor is in read-only mode
    /// * `has_search` - Whether there's an active search
    /// * `has_selection` - Whether there's an active text selection
    /// * `has_completion` - Whether completion popup is open
    /// * `keybindings` - Configurable keybindings from config
    pub fn from_key_event(
        key: KeyEvent,
        read_only: bool,
        has_search: bool,
        has_selection: bool,
        has_completion: bool,
        keybindings: &EditorKeybindings,
    ) -> Self {
        // When completion popup is open, intercept navigation keys
        if has_completion {
            match (key.code, key.modifiers) {
                // Navigation within completion popup
                (KeyCode::Up, KeyModifiers::NONE) => return Self::PrevCompletion,
                (KeyCode::Down, KeyModifiers::NONE) => return Self::NextCompletion,

                // Accept completion
                (KeyCode::Enter, KeyModifiers::NONE) => return Self::AcceptCompletion,
                (KeyCode::Tab, KeyModifiers::NONE) => return Self::AcceptCompletion,

                // Cancel completion
                (KeyCode::Esc, KeyModifiers::NONE) => return Self::CancelCompletion,

                // Filter completion with typed characters
                (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT)
                    if !key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    return Self::FilterCompletion(ch)
                }

                // Backspace removes filter character
                (KeyCode::Backspace, KeyModifiers::NONE) => return Self::BackspaceCompletion,

                // Other keys close completion and proceed normally
                _ => {}
            }
        }
        // Check configurable bindings first (order matters for conflicts)
        // File operations
        // Note: Save (Ctrl+S) is now handled by the global normalizer via handle_action.
        // The raw key still arrives here when forwarded, so we keep the match.
        if !read_only && key.code == KeyCode::Char('s') && key.modifiers == KeyModifiers::CONTROL {
            return Self::Save;
        }
        if !read_only
            && matches_binding_or_default(
                &keybindings.save_as,
                &key,
                KeyCode::Char('S'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            )
        {
            return Self::SaveAs;
        }
        if matches_binding_or_default(
            &keybindings.reload,
            &key,
            KeyCode::Char('R'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ) {
            return Self::ReloadFromDisk;
        }

        // Undo/Redo — now handled globally, but raw keys still arrive when forwarded
        if !read_only && key.code == KeyCode::Char('z') && key.modifiers == KeyModifiers::CONTROL {
            return Self::Undo;
        }
        if !read_only
            && ((key.code == KeyCode::Char('y') && key.modifiers == KeyModifiers::CONTROL)
                || (key.code == KeyCode::Char('z')
                    && key.modifiers == KeyModifiers::CONTROL | KeyModifiers::SHIFT))
        {
            return Self::Redo;
        }

        // Search & Replace — Search (Ctrl+F) now handled globally, raw key forwarded
        if key.code == KeyCode::Char('f') && key.modifiers == KeyModifiers::CONTROL {
            return Self::StartSearch;
        }
        if matches_binding_or_default(
            &keybindings.search_next,
            &key,
            KeyCode::F(3),
            KeyModifiers::NONE,
        ) {
            return Self::SearchNextOrOpen;
        }
        if matches_binding_or_default(
            &keybindings.search_prev,
            &key,
            KeyCode::F(3),
            KeyModifiers::SHIFT,
        ) {
            return Self::SearchPrevOrOpen;
        }
        if !read_only
            && matches_binding_or_default(
                &keybindings.replace,
                &key,
                KeyCode::Char('h'),
                KeyModifiers::CONTROL,
            )
        {
            return Self::StartReplace;
        }
        if !read_only
            && matches_binding_or_default(
                &keybindings.replace_all,
                &key,
                KeyCode::Char('r'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            )
        {
            return Self::ReplaceAll;
        }
        if !read_only
            && matches_binding_or_default(
                &keybindings.replace_current,
                &key,
                KeyCode::Char('r'),
                KeyModifiers::CONTROL,
            )
        {
            return Self::ReplaceNext;
        }

        // Selection — SelectAll (Ctrl+A) now handled globally, raw key forwarded
        if key.code == KeyCode::Char('a') && key.modifiers == KeyModifiers::CONTROL {
            return Self::SelectAll;
        }

        // Clipboard — Copy/Cut/Paste (Ctrl+C/X/V) now handled globally, raw keys forwarded.
        // Keep secondary bindings (Ctrl+Insert, Shift+Delete, Shift+Insert) as hardcoded.
        {
            let is_copy = matches!(key.code, KeyCode::Char('c') | KeyCode::Insert)
                && key.modifiers == KeyModifiers::CONTROL
                || matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
                    && key.modifiers == KeyModifiers::CONTROL.union(KeyModifiers::SHIFT);
            if is_copy {
                return Self::Copy;
            }
        }

        if !read_only
            && matches!(
                (key.code, key.modifiers),
                (KeyCode::Char('x'), KeyModifiers::CONTROL)
                    | (KeyCode::Delete, KeyModifiers::SHIFT)
            )
        {
            return Self::Cut;
        }

        if !read_only
            && matches!(
                (key.code, key.modifiers),
                (KeyCode::Char('v'), KeyModifiers::CONTROL)
                    | (KeyCode::Insert, KeyModifiers::SHIFT)
            )
        {
            return Self::Paste;
        }
        // Also handle Ctrl+Shift+V
        if !read_only
            && matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V'))
            && key.modifiers == KeyModifiers::CONTROL.union(KeyModifiers::SHIFT)
        {
            return Self::Paste;
        }

        // Advanced editing
        if !read_only
            && matches_binding_or_default(
                &keybindings.duplicate_line,
                &key,
                KeyCode::Char('d'),
                KeyModifiers::CONTROL,
            )
        {
            return Self::DuplicateLine;
        }
        if !read_only
            && matches_binding_or_default(
                &keybindings.toggle_comment,
                &key,
                KeyCode::Char('/'),
                KeyModifiers::CONTROL,
            )
        {
            return Self::ToggleComment;
        }

        // LSP Completion trigger (configurable, default Ctrl+.)
        if matches_binding_or_default(
            &keybindings.trigger_completion,
            &key,
            KeyCode::Char('.'),
            KeyModifiers::CONTROL,
        ) {
            return Self::TriggerCompletion;
        }

        // Toggle blame (no default — configure via [editor.keybindings] show_blame)
        if let Some(ref binding) = keybindings.show_blame {
            if binding.matches(&key) {
                return Self::ToggleBlame;
            }
        }

        // LSP Hover (configurable, default Ctrl+K)
        if matches_binding_or_default(
            &keybindings.show_hover,
            &key,
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        ) {
            return Self::ShowHover;
        }

        // LSP Go-to-Definition (configurable, default F12)
        if matches_binding_or_default(
            &keybindings.goto_definition,
            &key,
            KeyCode::F(12),
            KeyModifiers::NONE,
        ) {
            return Self::GotoDefinition;
        }

        // LSP Find References (configurable, default Shift+F12)
        // F(24) is the fallback for terminals (e.g. gnome-terminal/VTE) that encode
        // Shift+F12 as F24 instead of F12+SHIFT.
        if matches_binding_or_defaults(
            &keybindings.find_references,
            &key,
            &[
                (KeyCode::F(12), KeyModifiers::SHIFT),
                (KeyCode::F(24), KeyModifiers::NONE),
            ],
        ) {
            return Self::FindReferences;
        }

        // LSP Rename Symbol (configurable, default F4)
        // F4 = EditItem globally, but Editor intercepts it for rename_symbol
        if matches_binding_or_default(
            &keybindings.rename_symbol,
            &key,
            KeyCode::F(4),
            KeyModifiers::NONE,
        ) {
            return Self::RenameSymbol;
        }

        // Non-configurable bindings (navigation, basic editing)
        match (key.code, key.modifiers) {
            // Navigation (clears selection and closes search)
            (KeyCode::Up, KeyModifiers::NONE) => Self::MoveCursorUp,
            (KeyCode::Down, KeyModifiers::NONE) => Self::MoveCursorDown,
            (KeyCode::Left, KeyModifiers::NONE) => Self::MoveCursorLeft,
            (KeyCode::Right, KeyModifiers::NONE) => Self::MoveCursorRight,
            (KeyCode::Home, KeyModifiers::NONE) => Self::MoveToVisualLineStart,
            (KeyCode::End, KeyModifiers::NONE) => Self::MoveToVisualLineEnd,
            (KeyCode::PageUp, KeyModifiers::NONE) => Self::PageUp,
            (KeyCode::PageDown, KeyModifiers::NONE) => Self::PageDown,
            (KeyCode::Home, KeyModifiers::CONTROL) => Self::MoveToDocumentStart,
            (KeyCode::End, KeyModifiers::CONTROL) => Self::MoveToDocumentEnd,
            (KeyCode::Left, KeyModifiers::CONTROL) => Self::MoveWordBackward,
            (KeyCode::Right, KeyModifiers::CONTROL) => Self::MoveWordForward,
            (KeyCode::Up, KeyModifiers::CONTROL) => Self::MoveParagraphUp,
            (KeyCode::Down, KeyModifiers::CONTROL) => Self::MoveParagraphDown,

            // Navigation with selection (Shift) - closes search
            (KeyCode::Up, KeyModifiers::SHIFT) => Self::MoveCursorUpWithSelection,
            (KeyCode::Down, KeyModifiers::SHIFT) => Self::MoveCursorDownWithSelection,
            (KeyCode::Left, KeyModifiers::SHIFT) => Self::MoveCursorLeftWithSelection,
            (KeyCode::Right, KeyModifiers::SHIFT) => Self::MoveCursorRightWithSelection,
            (KeyCode::Left, mods)
                if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
            {
                Self::MoveWordBackwardWithSelection
            }
            (KeyCode::Right, mods)
                if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
            {
                Self::MoveWordForwardWithSelection
            }
            (KeyCode::Up, mods)
                if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
            {
                Self::MoveParagraphUpWithSelection
            }
            (KeyCode::Down, mods)
                if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
            {
                Self::MoveParagraphDownWithSelection
            }
            (KeyCode::Home, mods)
                if mods.contains(KeyModifiers::SHIFT) && !mods.contains(KeyModifiers::CONTROL) =>
            {
                Self::MoveToVisualLineStartWithSelection
            }
            (KeyCode::End, mods)
                if mods.contains(KeyModifiers::SHIFT) && !mods.contains(KeyModifiers::CONTROL) =>
            {
                Self::MoveToVisualLineEndWithSelection
            }
            (KeyCode::PageUp, mods)
                if mods.contains(KeyModifiers::SHIFT) && !mods.contains(KeyModifiers::CONTROL) =>
            {
                Self::PageUpWithSelection
            }
            (KeyCode::PageDown, mods)
                if mods.contains(KeyModifiers::SHIFT) && !mods.contains(KeyModifiers::CONTROL) =>
            {
                Self::PageDownWithSelection
            }
            (KeyCode::Home, mods)
                if mods.contains(KeyModifiers::SHIFT) && mods.contains(KeyModifiers::CONTROL) =>
            {
                Self::MoveToDocumentStartWithSelection
            }
            (KeyCode::End, mods)
                if mods.contains(KeyModifiers::SHIFT) && mods.contains(KeyModifiers::CONTROL) =>
            {
                Self::MoveToDocumentEndWithSelection
            }

            // Editing (only if not read-only)
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT)
                if !read_only && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                Self::InsertChar(ch)
            }
            (KeyCode::Enter, KeyModifiers::NONE) if !read_only => Self::InsertNewline,
            (KeyCode::Backspace, KeyModifiers::NONE) if !read_only => Self::Backspace,
            (KeyCode::Delete, KeyModifiers::NONE) if !read_only => Self::Delete,

            // Esc - close search
            (KeyCode::Esc, KeyModifiers::NONE) if has_search => Self::CloseSearch,

            // Tab - next match (when search is active), indent lines (with selection), or insert tab
            (KeyCode::Tab, KeyModifiers::NONE) if has_search => Self::SearchNext,
            (KeyCode::Tab, KeyModifiers::NONE) if !read_only && has_selection => Self::IndentLines,
            (KeyCode::Tab, KeyModifiers::NONE) if !read_only => Self::InsertTab,

            // Shift+Tab - previous match (when search is active), or unindent lines
            (KeyCode::BackTab, _) if has_search => Self::SearchPrev,
            (KeyCode::BackTab, _) if !read_only => Self::UnindentLines,

            // Default - no operation
            _ => Self::None,
        }
    }

    /// Execute this command on the given editor.
    ///
    /// This method performs the actual action associated with the command.
    /// Most commands delegate to existing methods on Editor, keeping the
    /// business logic in one place.
    ///
    /// # Arguments
    ///
    /// * `editor` - The editor to execute the command on
    ///
    /// # Returns
    ///
    /// Ok(()) if the command executed successfully, or an error if something went wrong.
    pub fn execute(self, editor: &mut super::Editor) -> Result<()> {
        use super::Editor;

        match self {
            // Navigation (clears selection and closes search)
            Self::MoveCursorUp => {
                editor.navigate(Editor::move_cursor_up_visual, Editor::move_cursor_up);
                Ok(())
            }
            Self::MoveCursorDown => {
                editor.navigate(Editor::move_cursor_down_visual, Editor::move_cursor_down);
                Ok(())
            }
            Self::MoveCursorLeft => {
                editor.navigate_simple(Editor::move_cursor_left);
                Ok(())
            }
            Self::MoveCursorRight => {
                editor.navigate_simple(Editor::move_cursor_right);
                Ok(())
            }
            Self::MoveToLineStart => {
                editor.navigate(
                    Editor::move_to_visual_line_start,
                    Editor::move_to_line_start,
                );
                Ok(())
            }
            Self::MoveToLineEnd => {
                editor.navigate(Editor::move_to_visual_line_end, Editor::move_to_line_end);
                Ok(())
            }
            Self::MoveToVisualLineStart => {
                editor.navigate(
                    Editor::move_to_visual_line_start,
                    Editor::move_to_line_start,
                );
                Ok(())
            }
            Self::MoveToVisualLineEnd => {
                editor.navigate(Editor::move_to_visual_line_end, Editor::move_to_line_end);
                Ok(())
            }
            Self::PageUp => {
                editor.navigate(Editor::page_up_visual, Editor::page_up);
                Ok(())
            }
            Self::PageDown => {
                editor.navigate(Editor::page_down_visual, Editor::page_down);
                Ok(())
            }
            Self::MoveToDocumentStart => {
                editor.navigate_simple(Editor::move_to_document_start);
                Ok(())
            }
            Self::MoveToDocumentEnd => {
                editor.navigate_simple(Editor::move_to_document_end);
                Ok(())
            }
            Self::MoveWordForward => {
                editor.navigate_simple(Editor::move_word_forward);
                Ok(())
            }
            Self::MoveWordBackward => {
                editor.navigate_simple(Editor::move_word_backward);
                Ok(())
            }
            Self::MoveParagraphUp => {
                editor.navigate_simple(Editor::move_paragraph_up);
                Ok(())
            }
            Self::MoveParagraphDown => {
                editor.navigate_simple(Editor::move_paragraph_down);
                Ok(())
            }

            // Navigation with selection
            Self::MoveCursorUpWithSelection => {
                editor
                    .navigate_with_selection(Editor::move_cursor_up_visual, Editor::move_cursor_up);
                Ok(())
            }
            Self::MoveCursorDownWithSelection => {
                editor.navigate_with_selection(
                    Editor::move_cursor_down_visual,
                    Editor::move_cursor_down,
                );
                Ok(())
            }
            Self::MoveCursorLeftWithSelection => {
                editor.navigate_with_selection_simple(Editor::move_cursor_left);
                Ok(())
            }
            Self::MoveCursorRightWithSelection => {
                editor.navigate_with_selection_simple(Editor::move_cursor_right);
                Ok(())
            }
            Self::MoveToLineStartWithSelection => {
                editor.navigate_with_selection(
                    Editor::move_to_visual_line_start,
                    Editor::move_to_line_start,
                );
                Ok(())
            }
            Self::MoveToLineEndWithSelection => {
                editor.navigate_with_selection(
                    Editor::move_to_visual_line_end,
                    Editor::move_to_line_end,
                );
                Ok(())
            }
            Self::MoveToVisualLineStartWithSelection => {
                editor.navigate_with_selection(
                    Editor::move_to_visual_line_start,
                    Editor::move_to_line_start,
                );
                Ok(())
            }
            Self::MoveToVisualLineEndWithSelection => {
                editor.navigate_with_selection(
                    Editor::move_to_visual_line_end,
                    Editor::move_to_line_end,
                );
                Ok(())
            }
            Self::PageUpWithSelection => {
                editor.navigate_with_selection(Editor::page_up_visual, Editor::page_up);
                Ok(())
            }
            Self::PageDownWithSelection => {
                editor.navigate_with_selection(Editor::page_down_visual, Editor::page_down);
                Ok(())
            }
            Self::MoveToDocumentStartWithSelection => {
                editor.navigate_with_selection_simple(Editor::move_to_document_start);
                Ok(())
            }
            Self::MoveToDocumentEndWithSelection => {
                editor.navigate_with_selection_simple(Editor::move_to_document_end);
                Ok(())
            }
            Self::MoveWordForwardWithSelection => {
                editor.navigate_with_selection_simple(Editor::move_word_forward);
                Ok(())
            }
            Self::MoveWordBackwardWithSelection => {
                editor.navigate_with_selection_simple(Editor::move_word_backward);
                Ok(())
            }
            Self::MoveParagraphUpWithSelection => {
                editor.navigate_with_selection_simple(Editor::move_paragraph_up);
                Ok(())
            }
            Self::MoveParagraphDownWithSelection => {
                editor.navigate_with_selection_simple(Editor::move_paragraph_down);
                Ok(())
            }

            // Text editing
            Self::InsertChar(ch) => editor.insert_char(ch),
            Self::InsertTab => editor.insert_tab(),
            Self::IndentLines => editor.indent_lines(),
            Self::UnindentLines => editor.unindent_lines(),
            Self::InsertNewline => editor.insert_newline(),
            Self::Backspace => editor.handle_delete_key(|e| e.backspace()),
            Self::Delete => editor.handle_delete_key(|e| e.delete()),

            // Undo/Redo
            Self::Undo => editor.handle_undo_redo(|buf| buf.undo()),
            Self::Redo => editor.handle_undo_redo(|buf| buf.redo()),

            // File operations - Save requires special handling for SaveAs modal
            Self::Save => {
                match editor.handle_save() {
                    Ok(Some(upload)) => {
                        // Remote file - store upload operation for app layer to process
                        editor.set_pending_upload(upload);
                        Ok(())
                    }
                    Ok(None) => {
                        // Local file - saved synchronously, no upload needed
                        Ok(())
                    }
                    Err(e) => {
                        // Error already shown in editor
                        Err(e)
                    }
                }
            }
            Self::SaveAs => {
                // This shouldn't be reached from key parsing, but included for completeness
                editor.handle_save_as()
            }
            Self::ForceSave => {
                match editor.force_save() {
                    Err(e) => {
                        editor.status_message = Some(format!("Force save failed: {}", e));
                        Err(e)
                    }
                    Ok(_upload_op) => {
                        // Discard upload operation - async handling not implemented in keyboard shortcuts yet
                        editor.status_message = Some("File force saved".to_string());
                        Ok(())
                    }
                }
            }
            Self::ReloadFromDisk => {
                if let Err(e) = editor.reload_from_disk() {
                    editor.status_message = Some(format!("Reload failed: {}", e));
                } else {
                    editor.status_message = Some("File reloaded from disk".to_string());
                }
                Ok(())
            }

            // Selection
            Self::SelectAll => {
                editor.select_all();
                Ok(())
            }

            // Clipboard
            Self::Copy => editor.copy_to_clipboard(),
            Self::Cut => editor.cut_to_clipboard(),
            Self::Paste => editor.paste_from_clipboard(),

            // Advanced editing
            Self::DuplicateLine => editor.duplicate_line(),
            Self::ToggleComment => editor.toggle_comment(),

            // Search
            Self::StartSearch => {
                editor.open_search_modal(true);
                Ok(())
            }
            Self::SearchNext => {
                editor.search_next();
                Ok(())
            }
            Self::SearchPrev => {
                editor.search_prev();
                Ok(())
            }
            Self::CloseSearch => {
                if editor.search.state.is_some() {
                    editor.close_search();
                }
                Ok(())
            }
            Self::SearchNextOrOpen => {
                editor.search_next_or_open();
                Ok(())
            }
            Self::SearchPrevOrOpen => {
                editor.search_prev_or_open();
                Ok(())
            }

            // Replace
            Self::StartReplace => {
                editor.handle_start_replace();
                Ok(())
            }
            Self::ReplaceNext => editor.replace_current(),
            Self::ReplaceAll => match editor.replace_all() {
                Ok(count) => {
                    editor.status_message = Some(format!(
                        "Replaced {} occurrence{}",
                        count,
                        if count == 1 { "" } else { "s" }
                    ));
                    Ok(())
                }
                Err(e) => Err(e),
            },

            // LSP Completion
            Self::TriggerCompletion => {
                editor.trigger_completion();
                Ok(())
            }
            Self::AcceptCompletion => {
                editor.accept_completion();
                Ok(())
            }
            Self::CancelCompletion => {
                editor.cancel_completion();
                Ok(())
            }
            Self::NextCompletion => {
                editor.next_completion();
                Ok(())
            }
            Self::PrevCompletion => {
                editor.prev_completion();
                Ok(())
            }
            Self::FilterCompletion(ch) => {
                editor.filter_completion(ch);
                Ok(())
            }
            Self::BackspaceCompletion => {
                editor.backspace_completion();
                Ok(())
            }
            Self::ToggleBlame => {
                editor.toggle_blame();
                Ok(())
            }
            Self::ShowHover => {
                editor.request_hover_at_cursor();
                Ok(())
            }
            Self::GotoDefinition => {
                editor.request_definition_at_cursor();
                Ok(())
            }
            Self::FindReferences => {
                editor.request_references_at_cursor();
                Ok(())
            }
            Self::RenameSymbol => {
                editor.request_rename_at_cursor();
                Ok(())
            }

            // No operation
            Self::None => Ok(()),
        }
    }
}
