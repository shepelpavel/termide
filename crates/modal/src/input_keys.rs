//! Common input key handling for modal text fields.
//!
//! Provides unified text input handling with selection support:
//! - Shift+arrows: character-by-character selection
//! - Shift+Home/End: select to start/end
//! - Ctrl+arrows: word-by-word navigation
//! - Ctrl+Shift+arrows: word-by-word selection
//! - Ctrl+A: select all
//! - Ctrl+C: copy
//! - Ctrl+X: cut
//! - Ctrl+V: paste
//! - Ctrl+Z: undo
//! - Ctrl+Y / Ctrl+Shift+Z: redo

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::TextInputHandler;

/// Result of input key handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKeyResult {
    /// Key was handled by input handler.
    Handled,
    /// Key was not handled - should be processed by modal.
    NotHandled,
    /// Text was modified (for modals that need to react to changes).
    TextModified,
}

/// Handle common text input keys.
///
/// This function handles:
/// - Selection: Shift+Left/Right/Home/End
/// - Word navigation: Ctrl+Left/Right
/// - Word selection: Ctrl+Shift+Left/Right
/// - Clipboard: Ctrl+A/C/X/V
/// - Undo/Redo: Ctrl+Z/Y
/// - Basic: Left/Right/Home/End/Backspace/Delete/character input
///
/// Returns `InputKeyResult::Handled` if the key was processed,
/// `InputKeyResult::TextModified` if text was changed,
/// `InputKeyResult::NotHandled` if the modal should handle it.
pub fn handle_input_key(input: &mut TextInputHandler, key: KeyEvent) -> InputKeyResult {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match (key.code, ctrl, shift) {
        // === Selection with Shift ===

        // Shift+Left: select one char left
        (KeyCode::Left, false, true) => {
            input.move_left_with_selection();
            InputKeyResult::Handled
        }
        // Shift+Right: select one char right
        (KeyCode::Right, false, true) => {
            input.move_right_with_selection();
            InputKeyResult::Handled
        }
        // Shift+Home: select to start
        (KeyCode::Home, false, true) => {
            input.move_home_with_selection();
            InputKeyResult::Handled
        }
        // Shift+End: select to end
        (KeyCode::End, false, true) => {
            input.move_end_with_selection();
            InputKeyResult::Handled
        }

        // === Word navigation with Ctrl ===

        // Ctrl+Left: move word left
        (KeyCode::Left, true, false) => {
            input.move_word_left();
            InputKeyResult::Handled
        }
        // Ctrl+Right: move word right
        (KeyCode::Right, true, false) => {
            input.move_word_right();
            InputKeyResult::Handled
        }
        // Ctrl+Shift+Left: select word left
        (KeyCode::Left, true, true) => {
            input.move_word_left_with_selection();
            InputKeyResult::Handled
        }
        // Ctrl+Shift+Right: select word right
        (KeyCode::Right, true, true) => {
            input.move_word_right_with_selection();
            InputKeyResult::Handled
        }

        // === Ctrl+A: Select all ===
        (KeyCode::Char('a'), true, false) => {
            input.select_all();
            InputKeyResult::Handled
        }

        // === Clipboard operations ===

        // Ctrl+C: Copy
        (KeyCode::Char('c'), true, false) => {
            if let Some(text) = input.selected_text() {
                let _ = termide_clipboard::copy(text);
            }
            InputKeyResult::Handled
        }
        // Ctrl+X: Cut
        (KeyCode::Char('x'), true, false) => {
            if let Some(text) = input.selected_text() {
                let _ = termide_clipboard::copy(text);
                input.delete_selection();
                return InputKeyResult::TextModified;
            }
            InputKeyResult::Handled
        }
        // Ctrl+V: Paste
        (KeyCode::Char('v'), true, false) => {
            if let Some(text) = termide_clipboard::paste() {
                input.paste(&text);
                return InputKeyResult::TextModified;
            }
            InputKeyResult::Handled
        }

        // === Undo/Redo ===

        // Ctrl+Z: Undo
        (KeyCode::Char('z'), true, false) => {
            if input.undo() {
                InputKeyResult::TextModified
            } else {
                InputKeyResult::Handled
            }
        }
        // Ctrl+Y or Ctrl+Shift+Z: Redo
        (KeyCode::Char('y'), true, false) | (KeyCode::Char('z'), true, true) => {
            if input.redo() {
                InputKeyResult::TextModified
            } else {
                InputKeyResult::Handled
            }
        }

        // === Basic navigation (without modifiers or with Shift only) ===

        // Left: move cursor left
        (KeyCode::Left, false, false) => {
            input.move_left();
            InputKeyResult::Handled
        }
        // Right: move cursor right
        (KeyCode::Right, false, false) => {
            input.move_right();
            InputKeyResult::Handled
        }
        // Home: move to start
        (KeyCode::Home, false, false) => {
            input.move_home();
            InputKeyResult::Handled
        }
        // End: move to end
        (KeyCode::End, false, false) => {
            input.move_end();
            InputKeyResult::Handled
        }

        // === Text modification ===

        // Backspace: delete backward
        (KeyCode::Backspace, false, false) => {
            if input.backspace() {
                InputKeyResult::TextModified
            } else {
                InputKeyResult::Handled
            }
        }
        // Delete: delete forward
        (KeyCode::Delete, false, false) => {
            if input.delete() {
                InputKeyResult::TextModified
            } else {
                InputKeyResult::Handled
            }
        }
        // Character input (without Ctrl)
        (KeyCode::Char(c), false, _) => {
            input.insert_char(c);
            InputKeyResult::TextModified
        }

        // Not handled - let modal process it
        _ => InputKeyResult::NotHandled,
    }
}
