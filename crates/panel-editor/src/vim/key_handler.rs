//! Vim key event handling.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use termide_keyboard::cyrillic_to_latin_opt;

use super::mode::VimMode;
use super::motions::VimMotion;
use super::operators::VimOperator;
use super::state::VimState;
use super::PanelDirection;

/// Translate Cyrillic characters to Latin for vim commands.
fn translate_cyrillic_key(key: KeyEvent) -> KeyEvent {
    if let KeyCode::Char(c) = key.code {
        if let Some(latin) = cyrillic_to_latin_opt(c) {
            return KeyEvent::new(KeyCode::Char(latin), key.modifiers);
        }
    }
    key
}

/// Result of handling a Vim key event.
#[derive(Debug, Clone)]
pub enum VimKeyResult {
    /// No action needed, key was consumed by Vim state.
    Consumed,
    /// Execute a motion (cursor movement).
    Motion { motion: VimMotion, count: usize },
    /// Execute a motion with selection (visual mode).
    MotionWithSelection { motion: VimMotion, count: usize },
    /// Execute an operator with a motion.
    OperatorMotion {
        operator: VimOperator,
        motion: VimMotion,
        count: usize,
    },
    /// Execute a linewise operator (dd, yy, cc).
    LinewiseOperator { operator: VimOperator, count: usize },
    /// Execute operator on visual selection.
    VisualOperator { operator: VimOperator },
    /// Enter insert mode at position.
    EnterInsert(InsertPosition),
    /// Exit to normal mode.
    ExitToNormal,
    /// Start visual mode.
    StartVisual,
    /// Start visual line mode.
    StartVisualLine,
    /// Delete character under cursor (x).
    DeleteChar { count: usize },
    /// Paste from register.
    Paste { after: bool, count: usize },
    /// Undo.
    Undo,
    /// Redo.
    Redo,
    /// Panel navigation (Ctrl+w h/j/k/l).
    PanelNavigation(PanelDirection),
    /// Pass through to standard editor (for insert mode).
    PassThrough,
    /// Key not recognized.
    Unhandled,
}

/// Position for entering insert mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertPosition {
    /// Insert before cursor (i).
    BeforeCursor,
    /// Insert after cursor (a).
    AfterCursor,
    /// Insert at line start (I).
    LineStart,
    /// Insert at line end (A).
    LineEnd,
    /// Open new line below (o).
    NewLineBelow,
    /// Open new line above (O).
    NewLineAbove,
}

/// Handle a key event in Vim mode.
///
/// # Arguments
/// * `state` - Current Vim state
/// * `key` - The key event to handle
///
/// # Returns
/// Result indicating what action to take
pub fn handle_vim_key(state: &mut VimState, key: KeyEvent) -> VimKeyResult {
    match state.mode {
        VimMode::Normal => handle_normal_mode(state, key),
        VimMode::Insert => handle_insert_mode(state, key),
        VimMode::Visual => handle_visual_mode(state, key),
        VimMode::VisualLine => handle_visual_line_mode(state, key),
    }
}

/// Handle key in normal mode.
fn handle_normal_mode(state: &mut VimState, key: KeyEvent) -> VimKeyResult {
    // Translate Cyrillic to Latin for vim commands
    let key = translate_cyrillic_key(key);

    // Check for Ctrl+w prefix for panel navigation
    if !state.partial_keys.is_empty() && state.partial_keys[0] == '\x17' {
        // Ctrl+W was pressed, waiting for h/j/k/l
        state.partial_keys.clear();
        return match key.code {
            KeyCode::Char('h') => VimKeyResult::PanelNavigation(PanelDirection::Left),
            KeyCode::Char('j') => VimKeyResult::PanelNavigation(PanelDirection::Down),
            KeyCode::Char('k') => VimKeyResult::PanelNavigation(PanelDirection::Up),
            KeyCode::Char('l') => VimKeyResult::PanelNavigation(PanelDirection::Right),
            _ => VimKeyResult::Consumed,
        };
    }

    // Check for 'g' prefix
    if !state.partial_keys.is_empty() && state.partial_keys[0] == 'g' {
        state.partial_keys.clear();
        return match key.code {
            KeyCode::Char('g') => {
                // gg - go to document start (or line if count specified)
                let count = state.effective_count();
                state.clear_pending();
                if count > 1 {
                    VimKeyResult::Motion {
                        motion: VimMotion::GoToLine(count),
                        count: 1,
                    }
                } else {
                    VimKeyResult::Motion {
                        motion: VimMotion::DocumentStart,
                        count: 1,
                    }
                }
            }
            // gj - visual line down (respects word wrap)
            KeyCode::Char('j') => {
                let count = state.effective_count();
                state.clear_pending();
                VimKeyResult::Motion {
                    motion: VimMotion::VisualDown,
                    count,
                }
            }
            // gk - visual line up (respects word wrap)
            KeyCode::Char('k') => {
                let count = state.effective_count();
                state.clear_pending();
                VimKeyResult::Motion {
                    motion: VimMotion::VisualUp,
                    count,
                }
            }
            _ => VimKeyResult::Consumed,
        };
    }

    // Handle Ctrl+W (start panel navigation sequence)
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('w') => {
                state.push_partial_key('\x17'); // Ctrl+W marker
                return VimKeyResult::Consumed;
            }
            KeyCode::Char('u') => {
                let count = state.effective_count();
                state.clear_pending();
                return VimKeyResult::Motion {
                    motion: VimMotion::HalfPageUp,
                    count,
                };
            }
            KeyCode::Char('d') => {
                let count = state.effective_count();
                state.clear_pending();
                return VimKeyResult::Motion {
                    motion: VimMotion::HalfPageDown,
                    count,
                };
            }
            KeyCode::Char('r') => {
                state.clear_pending();
                return VimKeyResult::Redo;
            }
            _ => {
                state.clear_pending();
                return VimKeyResult::PassThrough;
            }
        }
    }

    match key.code {
        // Count prefix (1-9, or 0 after other digits)
        KeyCode::Char(ch @ '0'..='9') => {
            if state.accumulate_count(ch) {
                VimKeyResult::Consumed
            } else {
                // '0' at start means line start
                VimKeyResult::Motion {
                    motion: VimMotion::LineStart,
                    count: 1,
                }
            }
        }

        // Basic motions
        KeyCode::Char('h') | KeyCode::Left => {
            let count = state.effective_count();
            if let Some(op) = state.take_pending_operator() {
                state.clear_pending();
                VimKeyResult::OperatorMotion {
                    operator: op,
                    motion: VimMotion::Left,
                    count,
                }
            } else {
                state.clear_pending();
                VimKeyResult::Motion {
                    motion: VimMotion::Left,
                    count,
                }
            }
        }
        // j - logical line down (by buffer lines)
        KeyCode::Char('j') => {
            let count = state.effective_count();
            if let Some(op) = state.take_pending_operator() {
                state.clear_pending();
                // j with operator is linewise
                VimKeyResult::LinewiseOperator {
                    operator: op,
                    count: count + 1, // Current line + count lines down
                }
            } else {
                state.clear_pending();
                VimKeyResult::Motion {
                    motion: VimMotion::Down,
                    count,
                }
            }
        }
        // Down arrow - visual line down (respects word wrap)
        KeyCode::Down => {
            let count = state.effective_count();
            if let Some(op) = state.take_pending_operator() {
                state.clear_pending();
                VimKeyResult::LinewiseOperator {
                    operator: op,
                    count: count + 1,
                }
            } else {
                state.clear_pending();
                VimKeyResult::Motion {
                    motion: VimMotion::VisualDown,
                    count,
                }
            }
        }
        // k - logical line up (by buffer lines)
        KeyCode::Char('k') => {
            let count = state.effective_count();
            if let Some(op) = state.take_pending_operator() {
                state.clear_pending();
                // k with operator is linewise
                VimKeyResult::LinewiseOperator {
                    operator: op,
                    count: count + 1, // Current line + count lines up
                }
            } else {
                state.clear_pending();
                VimKeyResult::Motion {
                    motion: VimMotion::Up,
                    count,
                }
            }
        }
        // Up arrow - visual line up (respects word wrap)
        KeyCode::Up => {
            let count = state.effective_count();
            if let Some(op) = state.take_pending_operator() {
                state.clear_pending();
                VimKeyResult::LinewiseOperator {
                    operator: op,
                    count: count + 1,
                }
            } else {
                state.clear_pending();
                VimKeyResult::Motion {
                    motion: VimMotion::VisualUp,
                    count,
                }
            }
        }
        KeyCode::Char('l') | KeyCode::Right => {
            let count = state.effective_count();
            if let Some(op) = state.take_pending_operator() {
                state.clear_pending();
                VimKeyResult::OperatorMotion {
                    operator: op,
                    motion: VimMotion::Right,
                    count,
                }
            } else {
                state.clear_pending();
                VimKeyResult::Motion {
                    motion: VimMotion::Right,
                    count,
                }
            }
        }

        // Word motions
        KeyCode::Char('w') => {
            let count = state.effective_count();
            if let Some(op) = state.take_pending_operator() {
                state.clear_pending();
                VimKeyResult::OperatorMotion {
                    operator: op,
                    motion: VimMotion::WordForward,
                    count,
                }
            } else {
                state.clear_pending();
                VimKeyResult::Motion {
                    motion: VimMotion::WordForward,
                    count,
                }
            }
        }
        KeyCode::Char('b') => {
            let count = state.effective_count();
            if let Some(op) = state.take_pending_operator() {
                state.clear_pending();
                VimKeyResult::OperatorMotion {
                    operator: op,
                    motion: VimMotion::WordBackward,
                    count,
                }
            } else {
                state.clear_pending();
                VimKeyResult::Motion {
                    motion: VimMotion::WordBackward,
                    count,
                }
            }
        }
        KeyCode::Char('e') => {
            let count = state.effective_count();
            if let Some(op) = state.take_pending_operator() {
                state.clear_pending();
                VimKeyResult::OperatorMotion {
                    operator: op,
                    motion: VimMotion::WordEnd,
                    count,
                }
            } else {
                state.clear_pending();
                VimKeyResult::Motion {
                    motion: VimMotion::WordEnd,
                    count,
                }
            }
        }

        // Line position motions
        KeyCode::Char('^') => {
            let count = state.effective_count();
            if let Some(op) = state.take_pending_operator() {
                state.clear_pending();
                VimKeyResult::OperatorMotion {
                    operator: op,
                    motion: VimMotion::FirstNonBlank,
                    count,
                }
            } else {
                state.clear_pending();
                VimKeyResult::Motion {
                    motion: VimMotion::FirstNonBlank,
                    count,
                }
            }
        }
        KeyCode::Char('$') | KeyCode::End => {
            let count = state.effective_count();
            if let Some(op) = state.take_pending_operator() {
                state.clear_pending();
                VimKeyResult::OperatorMotion {
                    operator: op,
                    motion: VimMotion::LineEnd,
                    count,
                }
            } else {
                state.clear_pending();
                VimKeyResult::Motion {
                    motion: VimMotion::LineEnd,
                    count,
                }
            }
        }
        KeyCode::Home => {
            state.clear_pending();
            VimKeyResult::Motion {
                motion: VimMotion::LineStart,
                count: 1,
            }
        }

        // Document motions
        KeyCode::Char('g') => {
            state.push_partial_key('g');
            VimKeyResult::Consumed
        }
        KeyCode::Char('G') => {
            let count = state.count;
            state.clear_pending();
            if let Some(line_num) = count {
                VimKeyResult::Motion {
                    motion: VimMotion::GoToLine(line_num),
                    count: 1,
                }
            } else {
                VimKeyResult::Motion {
                    motion: VimMotion::DocumentEnd,
                    count: 1,
                }
            }
        }

        // Operators
        KeyCode::Char('d') => {
            if state.pending_operator == Some(VimOperator::Delete) {
                // dd - delete line
                let count = state.effective_count();
                state.clear_pending();
                VimKeyResult::LinewiseOperator {
                    operator: VimOperator::Delete,
                    count,
                }
            } else {
                state.set_pending_operator(VimOperator::Delete);
                VimKeyResult::Consumed
            }
        }
        KeyCode::Char('y') => {
            if state.pending_operator == Some(VimOperator::Yank) {
                // yy - yank line
                let count = state.effective_count();
                state.clear_pending();
                VimKeyResult::LinewiseOperator {
                    operator: VimOperator::Yank,
                    count,
                }
            } else {
                state.set_pending_operator(VimOperator::Yank);
                VimKeyResult::Consumed
            }
        }
        KeyCode::Char('c') => {
            if state.pending_operator == Some(VimOperator::Change) {
                // cc - change line
                let count = state.effective_count();
                state.clear_pending();
                VimKeyResult::LinewiseOperator {
                    operator: VimOperator::Change,
                    count,
                }
            } else {
                state.set_pending_operator(VimOperator::Change);
                VimKeyResult::Consumed
            }
        }

        // Delete character (x)
        KeyCode::Char('x') => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::DeleteChar { count }
        }

        // Paste
        KeyCode::Char('p') => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::Paste { after: true, count }
        }
        KeyCode::Char('P') => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::Paste {
                after: false,
                count,
            }
        }

        // Undo
        KeyCode::Char('u') => {
            state.clear_pending();
            VimKeyResult::Undo
        }

        // Insert mode entry
        KeyCode::Char('i') => {
            state.clear_pending();
            VimKeyResult::EnterInsert(InsertPosition::BeforeCursor)
        }
        KeyCode::Char('a') => {
            state.clear_pending();
            VimKeyResult::EnterInsert(InsertPosition::AfterCursor)
        }
        KeyCode::Char('I') => {
            state.clear_pending();
            VimKeyResult::EnterInsert(InsertPosition::LineStart)
        }
        KeyCode::Char('A') => {
            state.clear_pending();
            VimKeyResult::EnterInsert(InsertPosition::LineEnd)
        }
        KeyCode::Char('o') => {
            state.clear_pending();
            VimKeyResult::EnterInsert(InsertPosition::NewLineBelow)
        }
        KeyCode::Char('O') => {
            state.clear_pending();
            VimKeyResult::EnterInsert(InsertPosition::NewLineAbove)
        }

        // Visual mode entry
        KeyCode::Char('v') => {
            state.clear_pending();
            VimKeyResult::StartVisual
        }
        KeyCode::Char('V') => {
            state.clear_pending();
            VimKeyResult::StartVisualLine
        }

        // Escape clears pending operations
        KeyCode::Esc => {
            state.clear_pending();
            VimKeyResult::Consumed
        }

        // F-keys pass through (F3/Shift+F3 for search next/prev, etc.)
        KeyCode::F(_) => {
            state.clear_pending();
            VimKeyResult::PassThrough
        }

        _ => {
            state.clear_pending();
            VimKeyResult::Unhandled
        }
    }
}

/// Handle key in insert mode.
fn handle_insert_mode(state: &mut VimState, key: KeyEvent) -> VimKeyResult {
    match key.code {
        KeyCode::Esc => {
            state.exit_to_normal();
            VimKeyResult::ExitToNormal
        }
        // All other keys pass through to standard editor
        _ => VimKeyResult::PassThrough,
    }
}

/// Handle key in visual mode.
fn handle_visual_mode(state: &mut VimState, key: KeyEvent) -> VimKeyResult {
    // Translate Cyrillic to Latin for vim commands
    let key = translate_cyrillic_key(key);

    // Check for Ctrl modifiers
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('u') => {
                let count = state.effective_count();
                state.clear_pending();
                return VimKeyResult::MotionWithSelection {
                    motion: VimMotion::HalfPageUp,
                    count,
                };
            }
            KeyCode::Char('d') => {
                let count = state.effective_count();
                state.clear_pending();
                return VimKeyResult::MotionWithSelection {
                    motion: VimMotion::HalfPageDown,
                    count,
                };
            }
            _ => {}
        }
    }

    match key.code {
        // Count prefix
        KeyCode::Char(ch @ '1'..='9') => {
            state.accumulate_count(ch);
            VimKeyResult::Consumed
        }
        KeyCode::Char(ch @ '0') => {
            if state.accumulate_count(ch) {
                VimKeyResult::Consumed
            } else {
                state.clear_pending();
                VimKeyResult::MotionWithSelection {
                    motion: VimMotion::LineStart,
                    count: 1,
                }
            }
        }

        // Motions extend selection
        KeyCode::Char('h') | KeyCode::Left => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::Left,
                count,
            }
        }
        // j - logical line down
        KeyCode::Char('j') => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::Down,
                count,
            }
        }
        // Down arrow - visual line down
        KeyCode::Down => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::VisualDown,
                count,
            }
        }
        // k - logical line up
        KeyCode::Char('k') => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::Up,
                count,
            }
        }
        // Up arrow - visual line up
        KeyCode::Up => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::VisualUp,
                count,
            }
        }
        KeyCode::Char('l') | KeyCode::Right => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::Right,
                count,
            }
        }
        KeyCode::Char('w') => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::WordForward,
                count,
            }
        }
        KeyCode::Char('b') => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::WordBackward,
                count,
            }
        }
        KeyCode::Char('e') => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::WordEnd,
                count,
            }
        }
        KeyCode::Char('^') => {
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::FirstNonBlank,
                count: 1,
            }
        }
        KeyCode::Char('$') | KeyCode::End => {
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::LineEnd,
                count: 1,
            }
        }
        KeyCode::Home => {
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::LineStart,
                count: 1,
            }
        }
        KeyCode::Char('G') => {
            let count = state.count;
            state.clear_pending();
            if let Some(line_num) = count {
                VimKeyResult::MotionWithSelection {
                    motion: VimMotion::GoToLine(line_num),
                    count: 1,
                }
            } else {
                VimKeyResult::MotionWithSelection {
                    motion: VimMotion::DocumentEnd,
                    count: 1,
                }
            }
        }

        // Operators on selection
        KeyCode::Char('d') => {
            state.clear_pending();
            VimKeyResult::VisualOperator {
                operator: VimOperator::Delete,
            }
        }
        KeyCode::Char('y') => {
            state.clear_pending();
            VimKeyResult::VisualOperator {
                operator: VimOperator::Yank,
            }
        }
        KeyCode::Char('c') => {
            state.clear_pending();
            VimKeyResult::VisualOperator {
                operator: VimOperator::Change,
            }
        }

        // Switch to visual line mode
        KeyCode::Char('V') => {
            state.clear_pending();
            VimKeyResult::StartVisualLine
        }

        // Escape exits visual mode
        KeyCode::Esc => {
            state.exit_to_normal();
            VimKeyResult::ExitToNormal
        }

        _ => VimKeyResult::Consumed,
    }
}

/// Handle key in visual line mode.
fn handle_visual_line_mode(state: &mut VimState, key: KeyEvent) -> VimKeyResult {
    // Translate Cyrillic to Latin for vim commands
    let key = translate_cyrillic_key(key);

    // Check for Ctrl modifiers
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('u') => {
                let count = state.effective_count();
                state.clear_pending();
                return VimKeyResult::MotionWithSelection {
                    motion: VimMotion::HalfPageUp,
                    count,
                };
            }
            KeyCode::Char('d') => {
                let count = state.effective_count();
                state.clear_pending();
                return VimKeyResult::MotionWithSelection {
                    motion: VimMotion::HalfPageDown,
                    count,
                };
            }
            _ => {}
        }
    }

    match key.code {
        // Count prefix
        KeyCode::Char(ch @ '1'..='9') => {
            state.accumulate_count(ch);
            VimKeyResult::Consumed
        }
        KeyCode::Char(ch @ '0') => {
            state.accumulate_count(ch);
            VimKeyResult::Consumed
        }

        // Vertical motions extend selection
        // j - logical line down
        KeyCode::Char('j') => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::Down,
                count,
            }
        }
        // Down arrow - visual line down
        KeyCode::Down => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::VisualDown,
                count,
            }
        }
        // k - logical line up
        KeyCode::Char('k') => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::Up,
                count,
            }
        }
        // Up arrow - visual line up
        KeyCode::Up => {
            let count = state.effective_count();
            state.clear_pending();
            VimKeyResult::MotionWithSelection {
                motion: VimMotion::VisualUp,
                count,
            }
        }
        KeyCode::Char('G') => {
            let count = state.count;
            state.clear_pending();
            if let Some(line_num) = count {
                VimKeyResult::MotionWithSelection {
                    motion: VimMotion::GoToLine(line_num),
                    count: 1,
                }
            } else {
                VimKeyResult::MotionWithSelection {
                    motion: VimMotion::DocumentEnd,
                    count: 1,
                }
            }
        }

        // Operators on selection (linewise)
        KeyCode::Char('d') => {
            state.clear_pending();
            VimKeyResult::VisualOperator {
                operator: VimOperator::Delete,
            }
        }
        KeyCode::Char('y') => {
            state.clear_pending();
            VimKeyResult::VisualOperator {
                operator: VimOperator::Yank,
            }
        }
        KeyCode::Char('c') => {
            state.clear_pending();
            VimKeyResult::VisualOperator {
                operator: VimOperator::Change,
            }
        }

        // Switch to character-wise visual mode
        KeyCode::Char('v') => {
            state.clear_pending();
            VimKeyResult::StartVisual
        }

        // Escape exits visual mode
        KeyCode::Esc => {
            state.exit_to_normal();
            VimKeyResult::ExitToNormal
        }

        _ => VimKeyResult::Consumed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_event(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_event_ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    #[test]
    fn test_basic_motions() {
        let mut state = VimState::new();

        // h motion
        let result = handle_vim_key(&mut state, key_event(KeyCode::Char('h')));
        assert!(matches!(
            result,
            VimKeyResult::Motion {
                motion: VimMotion::Left,
                count: 1
            }
        ));

        // j motion
        let result = handle_vim_key(&mut state, key_event(KeyCode::Char('j')));
        assert!(matches!(
            result,
            VimKeyResult::Motion {
                motion: VimMotion::Down,
                count: 1
            }
        ));
    }

    #[test]
    fn test_count_prefix() {
        let mut state = VimState::new();

        // 5j should move down 5 times
        handle_vim_key(&mut state, key_event(KeyCode::Char('5')));
        let result = handle_vim_key(&mut state, key_event(KeyCode::Char('j')));
        assert!(matches!(
            result,
            VimKeyResult::Motion {
                motion: VimMotion::Down,
                count: 5
            }
        ));
    }

    #[test]
    fn test_insert_mode_entry() {
        let mut state = VimState::new();

        let result = handle_vim_key(&mut state, key_event(KeyCode::Char('i')));
        assert!(matches!(
            result,
            VimKeyResult::EnterInsert(InsertPosition::BeforeCursor)
        ));

        state = VimState::new();
        let result = handle_vim_key(&mut state, key_event(KeyCode::Char('a')));
        assert!(matches!(
            result,
            VimKeyResult::EnterInsert(InsertPosition::AfterCursor)
        ));
    }

    #[test]
    fn test_insert_mode_escape() {
        let mut state = VimState::new();
        state.enter_insert();

        let result = handle_vim_key(&mut state, key_event(KeyCode::Esc));
        assert!(matches!(result, VimKeyResult::ExitToNormal));
        assert_eq!(state.mode, VimMode::Normal);
    }

    #[test]
    fn test_dd_delete_line() {
        let mut state = VimState::new();

        // First 'd' sets pending operator
        let result = handle_vim_key(&mut state, key_event(KeyCode::Char('d')));
        assert!(matches!(result, VimKeyResult::Consumed));
        assert_eq!(state.pending_operator, Some(VimOperator::Delete));

        // Second 'd' triggers linewise delete
        let result = handle_vim_key(&mut state, key_event(KeyCode::Char('d')));
        assert!(matches!(
            result,
            VimKeyResult::LinewiseOperator {
                operator: VimOperator::Delete,
                count: 1
            }
        ));
    }

    #[test]
    fn test_ctrl_u_d() {
        let mut state = VimState::new();

        let result = handle_vim_key(&mut state, key_event_ctrl('u'));
        assert!(matches!(
            result,
            VimKeyResult::Motion {
                motion: VimMotion::HalfPageUp,
                count: 1
            }
        ));

        let result = handle_vim_key(&mut state, key_event_ctrl('d'));
        assert!(matches!(
            result,
            VimKeyResult::Motion {
                motion: VimMotion::HalfPageDown,
                count: 1
            }
        ));
    }

    #[test]
    fn test_visual_mode() {
        let mut state = VimState::new();

        let result = handle_vim_key(&mut state, key_event(KeyCode::Char('v')));
        assert!(matches!(result, VimKeyResult::StartVisual));
    }

    #[test]
    fn test_gg_document_start() {
        let mut state = VimState::new();

        // First 'g'
        let result = handle_vim_key(&mut state, key_event(KeyCode::Char('g')));
        assert!(matches!(result, VimKeyResult::Consumed));

        // Second 'g'
        let result = handle_vim_key(&mut state, key_event(KeyCode::Char('g')));
        assert!(matches!(
            result,
            VimKeyResult::Motion {
                motion: VimMotion::DocumentStart,
                count: 1
            }
        ));
    }
}
