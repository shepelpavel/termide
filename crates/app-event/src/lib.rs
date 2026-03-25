//! Event handling traits and hotkey processing for termide.
//!
//! This crate provides:
//! - `HotkeyProcessor` trait for checking global hotkeys
//! - `KeyBinding` type for configurable hotkey mappings
//! - Default hotkey processor implementation
//!
//! # Architecture
//!
//! The hotkey processor converts keyboard events into `AppCommand`s,
//! which are then executed by the app orchestrator. This separation
//! enables isolated unit testing without requiring full app context.
//!
//! ```text
//! KeyEvent → HotkeyProcessor → Option<AppCommand> → App Orchestrator
//! ```

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use termide_app_core::{AppCommand, Direction, PanelType};
use termide_config::{latin_to_cyrillic, GlobalKeybindings, KeyBinding as ConfigKeyBinding};

// ============================================================================
// Key Binding Types
// ============================================================================

/// A key binding specification.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyBinding {
    /// The key code (e.g., Char('m'), Left, F1)
    pub code: KeyCode,
    /// Required modifiers (e.g., ALT, CTRL)
    pub modifiers: KeyModifiers,
}

impl KeyBinding {
    /// Create a new key binding.
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    /// Create an Alt+key binding.
    pub fn alt(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::ALT)
    }

    /// Create a key binding without modifiers.
    pub fn plain(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::NONE)
    }

    /// Check if a key event matches this binding.
    pub fn matches(&self, key: &KeyEvent) -> bool {
        self.code == key.code && key.modifiers.contains(self.modifiers)
    }
}

impl From<KeyEvent> for KeyBinding {
    fn from(event: KeyEvent) -> Self {
        Self {
            code: event.code,
            modifiers: event.modifiers,
        }
    }
}

// ============================================================================
// Hotkey Action Enum
// ============================================================================

/// Actions that can be triggered by hotkeys.
///
/// These are higher-level actions that get converted to AppCommands.
/// This allows for more semantic hotkey definitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyAction {
    // === Menu ===
    /// Toggle menu visibility
    ToggleMenu,

    // === Panel creation ===
    /// Open new file manager panel
    NewFileManager,
    /// Open new terminal panel
    NewTerminal,
    /// Open new editor panel
    NewEditor,
    /// Open journal panel
    NewJournal,
    /// Open help panel
    OpenHelp,
    /// Open preferences (config file)
    OpenPreferences,

    // === Navigation ===
    /// Navigate to previous group
    PrevGroup,
    /// Navigate to next group
    NextGroup,
    /// Navigate to previous panel in group
    PrevInGroup,
    /// Navigate to next panel in group
    NextInGroup,
    /// Go to panel by number (1-9)
    GoToPanel(usize),

    // === Panel management ===
    /// Close active panel
    ClosePanel,
    /// Toggle panel stacking
    ToggleStacking,
    /// Swap panel left
    SwapPanelLeft,
    /// Swap panel right
    SwapPanelRight,
    /// Move panel to first group
    MoveToFirst,
    /// Move panel to last group
    MoveToLast,
    /// Resize panel (delta in columns)
    ResizePanel(i16),

    // === Application ===
    /// Request quit (with confirmation if needed)
    RequestQuit,
    /// Open sessions modal
    OpenSessions,
    /// Open or focus Git Status panel
    OpenGitStatus,
    /// Open directory switcher modal
    OpenDirectorySwitcher,
    /// Open bookmark add dialog
    OpenBookmarkAdd,
    /// Open or focus the Outline panel
    OpenOutline,
    /// Open or focus the Diagnostics panel
    OpenDiagnostics,
    /// Open or focus the Git Log panel
    OpenGitLog,
    /// Open command palette
    OpenCommandPalette,
}

impl HotkeyAction {
    /// Convert action to AppCommand.
    ///
    /// Some actions require additional context and return None,
    /// indicating they need special handling by the orchestrator.
    pub fn to_command(&self) -> Option<AppCommand> {
        match self {
            // Navigation commands
            HotkeyAction::PrevGroup => Some(AppCommand::Navigate {
                direction: Direction::Prev,
            }),
            HotkeyAction::NextGroup => Some(AppCommand::Navigate {
                direction: Direction::Next,
            }),
            HotkeyAction::GoToPanel(n) => Some(AppCommand::Navigate {
                direction: Direction::Index(*n - 1),
            }),

            // Panel creation
            HotkeyAction::NewFileManager => Some(AppCommand::CreatePanel {
                panel_type: PanelType::FileManager { working_dir: None },
            }),
            HotkeyAction::NewTerminal => Some(AppCommand::CreatePanel {
                panel_type: PanelType::Terminal { cwd: None },
            }),
            HotkeyAction::NewEditor => Some(AppCommand::CreatePanel {
                panel_type: PanelType::Editor { file_path: None },
            }),
            HotkeyAction::NewJournal => Some(AppCommand::CreatePanel {
                panel_type: PanelType::Journal,
            }),
            HotkeyAction::OpenHelp => Some(AppCommand::CreatePanel {
                panel_type: PanelType::Help,
            }),

            // Panel management
            HotkeyAction::ClosePanel => Some(AppCommand::ClosePanel),
            HotkeyAction::RequestQuit => Some(AppCommand::Quit),

            // Actions that need special handling (return None)
            HotkeyAction::ToggleMenu
            | HotkeyAction::OpenPreferences
            | HotkeyAction::OpenSessions
            | HotkeyAction::OpenGitStatus
            | HotkeyAction::OpenOutline
            | HotkeyAction::OpenDiagnostics
            | HotkeyAction::OpenGitLog
            | HotkeyAction::OpenDirectorySwitcher
            | HotkeyAction::OpenBookmarkAdd
            | HotkeyAction::PrevInGroup
            | HotkeyAction::NextInGroup
            | HotkeyAction::ToggleStacking
            | HotkeyAction::SwapPanelLeft
            | HotkeyAction::SwapPanelRight
            | HotkeyAction::MoveToFirst
            | HotkeyAction::MoveToLast
            | HotkeyAction::ResizePanel(_)
            | HotkeyAction::OpenCommandPalette => None,
        }
    }
}

// ============================================================================
// Hotkey Processor Trait
// ============================================================================

/// Trait for processing global hotkeys.
///
/// Implementations check if a key event is a global hotkey and
/// return the corresponding action if so.
pub trait HotkeyProcessor {
    /// Check if key is a global hotkey.
    ///
    /// Returns the action if the key matches a hotkey binding,
    /// or None if it should be passed to the active panel.
    fn process_hotkey(&self, key: &KeyEvent) -> Option<HotkeyAction>;

    /// Check if Escape should close the panel.
    ///
    /// Returns true if Escape is not captured by the active panel
    /// and should trigger panel close.
    fn should_escape_close(&self, key: &KeyEvent, panel_captures_escape: bool) -> bool {
        key.code == KeyCode::Esc && key.modifiers.is_empty() && !panel_captures_escape
    }
}

// ============================================================================
// Default Hotkey Processor
// ============================================================================

/// Default hotkey processor with standard key bindings.
///
/// Uses Alt+key combinations for all hotkeys as per termide conventions.
#[derive(Debug, Clone)]
pub struct DefaultHotkeyProcessor {
    bindings: HashMap<KeyBinding, HotkeyAction>,
}

impl Default for DefaultHotkeyProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultHotkeyProcessor {
    /// Create a new processor with default bindings.
    pub fn new() -> Self {
        let mut bindings = HashMap::new();

        // Menu
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('m')),
            HotkeyAction::ToggleMenu,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('M')),
            HotkeyAction::ToggleMenu,
        );

        // Panel creation
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('f')),
            HotkeyAction::NewFileManager,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('F')),
            HotkeyAction::NewFileManager,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('t')),
            HotkeyAction::NewTerminal,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('T')),
            HotkeyAction::NewTerminal,
        );
        bindings.insert(KeyBinding::alt(KeyCode::Char('e')), HotkeyAction::NewEditor);
        bindings.insert(KeyBinding::alt(KeyCode::Char('E')), HotkeyAction::NewEditor);
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('l')),
            HotkeyAction::NewJournal,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('L')),
            HotkeyAction::NewJournal,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('p')),
            HotkeyAction::OpenPreferences,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('P')),
            HotkeyAction::OpenPreferences,
        );
        bindings.insert(KeyBinding::alt(KeyCode::Char('h')), HotkeyAction::OpenHelp);
        bindings.insert(KeyBinding::alt(KeyCode::Char('H')), HotkeyAction::OpenHelp);

        // Quit
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('q')),
            HotkeyAction::RequestQuit,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('Q')),
            HotkeyAction::RequestQuit,
        );

        // Close panel
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('x')),
            HotkeyAction::ClosePanel,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('X')),
            HotkeyAction::ClosePanel,
        );
        bindings.insert(KeyBinding::alt(KeyCode::Delete), HotkeyAction::ClosePanel);

        // Navigation - arrows
        bindings.insert(KeyBinding::alt(KeyCode::Left), HotkeyAction::PrevGroup);
        bindings.insert(KeyBinding::alt(KeyCode::Right), HotkeyAction::NextGroup);
        bindings.insert(KeyBinding::alt(KeyCode::Up), HotkeyAction::PrevInGroup);
        bindings.insert(KeyBinding::alt(KeyCode::Down), HotkeyAction::NextInGroup);

        // Navigation - alternative keys (vim-style WASD)
        bindings.insert(KeyBinding::alt(KeyCode::Char('a')), HotkeyAction::PrevGroup);
        bindings.insert(KeyBinding::alt(KeyCode::Char('A')), HotkeyAction::PrevGroup);
        bindings.insert(KeyBinding::alt(KeyCode::Char('d')), HotkeyAction::NextGroup);
        bindings.insert(KeyBinding::alt(KeyCode::Char('D')), HotkeyAction::NextGroup);
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('w')),
            HotkeyAction::PrevInGroup,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('W')),
            HotkeyAction::PrevInGroup,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('s')),
            HotkeyAction::NextInGroup,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('S')),
            HotkeyAction::NextInGroup,
        );

        // Sessions switcher (Alt+/ or Alt+. for Cyrillic layout)
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('/')),
            HotkeyAction::OpenSessions,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('?')),
            HotkeyAction::OpenSessions,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('.')),
            HotkeyAction::OpenSessions,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('>')),
            HotkeyAction::OpenSessions,
        );

        // Outline panel
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('o')),
            HotkeyAction::OpenOutline,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('O')),
            HotkeyAction::OpenOutline,
        );

        // Diagnostics panel
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('i')),
            HotkeyAction::OpenDiagnostics,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('I')),
            HotkeyAction::OpenDiagnostics,
        );

        // Git Log panel
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('c')),
            HotkeyAction::OpenGitLog,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('C')),
            HotkeyAction::OpenGitLog,
        );

        // Git Status panel
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('g')),
            HotkeyAction::OpenGitStatus,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('G')),
            HotkeyAction::OpenGitStatus,
        );

        // Directory Switcher (Ctrl+P)
        bindings.insert(
            KeyBinding::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
            HotkeyAction::OpenDirectorySwitcher,
        );
        bindings.insert(
            KeyBinding::new(KeyCode::Char('P'), KeyModifiers::CONTROL),
            HotkeyAction::OpenDirectorySwitcher,
        );

        // Bookmark Add (Ctrl+B)
        bindings.insert(
            KeyBinding::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            HotkeyAction::OpenBookmarkAdd,
        );
        bindings.insert(
            KeyBinding::new(KeyCode::Char('B'), KeyModifiers::CONTROL),
            HotkeyAction::OpenBookmarkAdd,
        );

        // Command Palette (Ctrl+Shift+P)
        bindings.insert(
            KeyBinding::new(
                KeyCode::Char('p'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            HotkeyAction::OpenCommandPalette,
        );
        bindings.insert(
            KeyBinding::new(
                KeyCode::Char('P'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            HotkeyAction::OpenCommandPalette,
        );

        // Panel management
        bindings.insert(
            KeyBinding::alt(KeyCode::Backspace),
            HotkeyAction::ToggleStacking,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::PageUp),
            HotkeyAction::SwapPanelLeft,
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::PageDown),
            HotkeyAction::SwapPanelRight,
        );
        bindings.insert(KeyBinding::alt(KeyCode::Home), HotkeyAction::MoveToFirst);
        bindings.insert(KeyBinding::alt(KeyCode::End), HotkeyAction::MoveToLast);

        // Resize
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('-')),
            HotkeyAction::ResizePanel(-1),
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('+')),
            HotkeyAction::ResizePanel(1),
        );
        bindings.insert(
            KeyBinding::alt(KeyCode::Char('=')),
            HotkeyAction::ResizePanel(1),
        );

        // Number keys for panel selection
        for i in 1..=9u8 {
            bindings.insert(
                KeyBinding::alt(KeyCode::Char((b'0' + i) as char)),
                HotkeyAction::GoToPanel(i as usize),
            );
        }

        Self { bindings }
    }

    /// Create a processor from configuration, falling back to defaults.
    pub fn from_config(config: &GlobalKeybindings) -> Self {
        let mut processor = Self::new();

        // Helper to add binding from config or keep default.
        // Also auto-generates Cyrillic variants for Latin character bindings.
        let add_binding =
            |processor: &mut Self, binding: &Option<ConfigKeyBinding>, action: HotkeyAction| {
                if let Some(kb) = binding {
                    // Remove existing bindings for this action
                    processor.bindings.retain(|_, a| a != &action);
                    // Add new bindings from config
                    for parsed in kb.parse() {
                        processor.bindings.insert(
                            KeyBinding::new(parsed.key, parsed.modifiers),
                            action.clone(),
                        );

                        // Add opposite case for letter keys (both 'm' and 'M' should work)
                        if let KeyCode::Char(c) = parsed.key {
                            if c.is_ascii_alphabetic() {
                                let opposite = if c.is_ascii_lowercase() {
                                    c.to_ascii_uppercase()
                                } else {
                                    c.to_ascii_lowercase()
                                };
                                processor.bindings.insert(
                                    KeyBinding::new(KeyCode::Char(opposite), parsed.modifiers),
                                    action.clone(),
                                );
                            }
                        }

                        // Auto-generate Cyrillic variant for Latin character bindings
                        if let KeyCode::Char(c) = parsed.key {
                            if let Some(cyrillic) = latin_to_cyrillic(c) {
                                // Add lowercase Cyrillic
                                processor.bindings.insert(
                                    KeyBinding::new(KeyCode::Char(cyrillic), parsed.modifiers),
                                    action.clone(),
                                );
                                // Add uppercase Cyrillic
                                if let Some(upper) = cyrillic.to_uppercase().next() {
                                    if upper != cyrillic {
                                        processor.bindings.insert(
                                            KeyBinding::new(KeyCode::Char(upper), parsed.modifiers),
                                            action.clone(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            };

        // Menu & UI
        add_binding(
            &mut processor,
            &config.toggle_menu,
            HotkeyAction::ToggleMenu,
        );

        // Panel creation
        add_binding(
            &mut processor,
            &config.new_file_manager,
            HotkeyAction::NewFileManager,
        );
        add_binding(
            &mut processor,
            &config.new_terminal,
            HotkeyAction::NewTerminal,
        );
        add_binding(&mut processor, &config.new_editor, HotkeyAction::NewEditor);
        add_binding(
            &mut processor,
            &config.new_journal,
            HotkeyAction::NewJournal,
        );
        add_binding(&mut processor, &config.open_help, HotkeyAction::OpenHelp);
        add_binding(
            &mut processor,
            &config.open_preferences,
            HotkeyAction::OpenPreferences,
        );
        add_binding(
            &mut processor,
            &config.open_sessions,
            HotkeyAction::OpenSessions,
        );
        add_binding(
            &mut processor,
            &config.open_git_status,
            HotkeyAction::OpenGitStatus,
        );
        add_binding(
            &mut processor,
            &config.open_directory_switcher,
            HotkeyAction::OpenDirectorySwitcher,
        );
        add_binding(
            &mut processor,
            &config.open_bookmark_add,
            HotkeyAction::OpenBookmarkAdd,
        );
        add_binding(
            &mut processor,
            &config.open_outline,
            HotkeyAction::OpenOutline,
        );
        add_binding(
            &mut processor,
            &config.open_diagnostics,
            HotkeyAction::OpenDiagnostics,
        );
        add_binding(
            &mut processor,
            &config.open_git_log,
            HotkeyAction::OpenGitLog,
        );

        // Panel management
        add_binding(
            &mut processor,
            &config.close_panel,
            HotkeyAction::ClosePanel,
        );
        add_binding(
            &mut processor,
            &config.toggle_stack,
            HotkeyAction::ToggleStacking,
        );
        add_binding(
            &mut processor,
            &config.swap_left,
            HotkeyAction::SwapPanelLeft,
        );
        add_binding(
            &mut processor,
            &config.swap_right,
            HotkeyAction::SwapPanelRight,
        );
        add_binding(
            &mut processor,
            &config.move_first,
            HotkeyAction::MoveToFirst,
        );
        add_binding(&mut processor, &config.move_last, HotkeyAction::MoveToLast);
        add_binding(
            &mut processor,
            &config.resize_smaller,
            HotkeyAction::ResizePanel(-1),
        );
        add_binding(
            &mut processor,
            &config.resize_larger,
            HotkeyAction::ResizePanel(1),
        );

        // Navigation
        add_binding(&mut processor, &config.prev_group, HotkeyAction::PrevGroup);
        add_binding(&mut processor, &config.next_group, HotkeyAction::NextGroup);
        add_binding(
            &mut processor,
            &config.prev_panel,
            HotkeyAction::PrevInGroup,
        );
        add_binding(
            &mut processor,
            &config.next_panel,
            HotkeyAction::NextInGroup,
        );

        // Panel selection by number
        add_binding(
            &mut processor,
            &config.goto_panel_1,
            HotkeyAction::GoToPanel(1),
        );
        add_binding(
            &mut processor,
            &config.goto_panel_2,
            HotkeyAction::GoToPanel(2),
        );
        add_binding(
            &mut processor,
            &config.goto_panel_3,
            HotkeyAction::GoToPanel(3),
        );
        add_binding(
            &mut processor,
            &config.goto_panel_4,
            HotkeyAction::GoToPanel(4),
        );
        add_binding(
            &mut processor,
            &config.goto_panel_5,
            HotkeyAction::GoToPanel(5),
        );
        add_binding(
            &mut processor,
            &config.goto_panel_6,
            HotkeyAction::GoToPanel(6),
        );
        add_binding(
            &mut processor,
            &config.goto_panel_7,
            HotkeyAction::GoToPanel(7),
        );
        add_binding(
            &mut processor,
            &config.goto_panel_8,
            HotkeyAction::GoToPanel(8),
        );
        add_binding(
            &mut processor,
            &config.goto_panel_9,
            HotkeyAction::GoToPanel(9),
        );

        // Application
        add_binding(&mut processor, &config.quit, HotkeyAction::RequestQuit);
        add_binding(
            &mut processor,
            &config.open_command_palette,
            HotkeyAction::OpenCommandPalette,
        );

        processor
    }

    /// Add or replace a hotkey binding.
    pub fn bind(&mut self, key: KeyBinding, action: HotkeyAction) {
        self.bindings.insert(key, action);
    }

    /// Remove a hotkey binding.
    pub fn unbind(&mut self, key: &KeyBinding) {
        self.bindings.remove(key);
    }

    /// Get all current bindings.
    pub fn bindings(&self) -> &HashMap<KeyBinding, HotkeyAction> {
        &self.bindings
    }
}

impl HotkeyProcessor for DefaultHotkeyProcessor {
    fn process_hotkey(&self, key: &KeyEvent) -> Option<HotkeyAction> {
        // Only process Alt+key or Ctrl+key combinations
        if !key.modifiers.contains(KeyModifiers::ALT)
            && !key.modifiers.contains(KeyModifiers::CONTROL)
        {
            return None;
        }

        let binding = KeyBinding::from(*key);
        self.bindings.get(&binding).cloned()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn key_event(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn alt_key(c: char) -> KeyEvent {
        key_event(KeyCode::Char(c), KeyModifiers::ALT)
    }

    fn ctrl_key(c: char) -> KeyEvent {
        key_event(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn test_key_binding_matches() {
        let binding = KeyBinding::alt(KeyCode::Char('m'));
        assert!(binding.matches(&alt_key('m')));
        assert!(!binding.matches(&alt_key('n')));
        assert!(!binding.matches(&key_event(KeyCode::Char('m'), KeyModifiers::NONE)));
    }

    #[test]
    fn test_default_processor_toggle_menu() {
        let processor = DefaultHotkeyProcessor::new();
        let action = processor.process_hotkey(&alt_key('m'));
        assert_eq!(action, Some(HotkeyAction::ToggleMenu));
    }

    #[test]
    fn test_default_processor_new_file_manager() {
        let processor = DefaultHotkeyProcessor::new();
        let action = processor.process_hotkey(&alt_key('f'));
        assert_eq!(action, Some(HotkeyAction::NewFileManager));
    }

    #[test]
    fn test_default_processor_new_terminal() {
        let processor = DefaultHotkeyProcessor::new();
        let action = processor.process_hotkey(&alt_key('t'));
        assert_eq!(action, Some(HotkeyAction::NewTerminal));
    }

    #[test]
    fn test_default_processor_quit() {
        let processor = DefaultHotkeyProcessor::new();
        let action = processor.process_hotkey(&alt_key('q'));
        assert_eq!(action, Some(HotkeyAction::RequestQuit));
    }

    #[test]
    fn test_default_processor_navigation() {
        let processor = DefaultHotkeyProcessor::new();

        // Arrow keys
        assert_eq!(
            processor.process_hotkey(&key_event(KeyCode::Left, KeyModifiers::ALT)),
            Some(HotkeyAction::PrevGroup)
        );
        assert_eq!(
            processor.process_hotkey(&key_event(KeyCode::Right, KeyModifiers::ALT)),
            Some(HotkeyAction::NextGroup)
        );
        assert_eq!(
            processor.process_hotkey(&key_event(KeyCode::Up, KeyModifiers::ALT)),
            Some(HotkeyAction::PrevInGroup)
        );
        assert_eq!(
            processor.process_hotkey(&key_event(KeyCode::Down, KeyModifiers::ALT)),
            Some(HotkeyAction::NextInGroup)
        );

        // WASD alternatives
        assert_eq!(
            processor.process_hotkey(&alt_key('a')),
            Some(HotkeyAction::PrevGroup)
        );
        assert_eq!(
            processor.process_hotkey(&alt_key('d')),
            Some(HotkeyAction::NextGroup)
        );
        assert_eq!(
            processor.process_hotkey(&alt_key('w')),
            Some(HotkeyAction::PrevInGroup)
        );
        assert_eq!(
            processor.process_hotkey(&alt_key('s')),
            Some(HotkeyAction::NextInGroup)
        );
    }

    #[test]
    fn test_default_processor_panel_numbers() {
        let processor = DefaultHotkeyProcessor::new();

        for i in 1..=9 {
            let action = processor.process_hotkey(&alt_key((b'0' + i) as char));
            assert_eq!(action, Some(HotkeyAction::GoToPanel(i as usize)));
        }
    }

    #[test]
    fn test_default_processor_resize() {
        let processor = DefaultHotkeyProcessor::new();

        assert_eq!(
            processor.process_hotkey(&alt_key('-')),
            Some(HotkeyAction::ResizePanel(-1))
        );
        assert_eq!(
            processor.process_hotkey(&alt_key('+')),
            Some(HotkeyAction::ResizePanel(1))
        );
        assert_eq!(
            processor.process_hotkey(&alt_key('=')),
            Some(HotkeyAction::ResizePanel(1))
        );
    }

    #[test]
    fn test_default_processor_non_modifier_keys() {
        let processor = DefaultHotkeyProcessor::new();

        // Keys without Alt or Ctrl modifiers should return None
        assert_eq!(
            processor.process_hotkey(&key_event(KeyCode::Char('m'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            processor.process_hotkey(&key_event(KeyCode::Char('m'), KeyModifiers::SHIFT)),
            None
        );
    }

    #[test]
    fn test_default_processor_ctrl_p_directory_switcher() {
        let processor = DefaultHotkeyProcessor::new();

        // Ctrl+P should open directory switcher (lowercase)
        assert_eq!(
            processor.process_hotkey(&ctrl_key('p')),
            Some(HotkeyAction::OpenDirectorySwitcher)
        );

        // Ctrl+P should also work with uppercase
        assert_eq!(
            processor.process_hotkey(&ctrl_key('P')),
            Some(HotkeyAction::OpenDirectorySwitcher)
        );
    }

    #[test]
    fn test_hotkey_action_to_command() {
        // Navigation
        assert!(matches!(
            HotkeyAction::PrevGroup.to_command(),
            Some(AppCommand::Navigate {
                direction: Direction::Prev
            })
        ));
        assert!(matches!(
            HotkeyAction::NextGroup.to_command(),
            Some(AppCommand::Navigate {
                direction: Direction::Next
            })
        ));

        // Panel creation
        assert!(matches!(
            HotkeyAction::NewFileManager.to_command(),
            Some(AppCommand::CreatePanel {
                panel_type: PanelType::FileManager { .. }
            })
        ));
        assert!(matches!(
            HotkeyAction::NewTerminal.to_command(),
            Some(AppCommand::CreatePanel {
                panel_type: PanelType::Terminal { .. }
            })
        ));

        // Close/Quit
        assert!(matches!(
            HotkeyAction::ClosePanel.to_command(),
            Some(AppCommand::ClosePanel)
        ));
        assert!(matches!(
            HotkeyAction::RequestQuit.to_command(),
            Some(AppCommand::Quit)
        ));

        // Actions that need special handling
        assert!(HotkeyAction::ToggleMenu.to_command().is_none());
        assert!(HotkeyAction::OpenPreferences.to_command().is_none());
    }

    #[test]
    fn test_escape_close() {
        let processor = DefaultHotkeyProcessor::new();

        // Escape without modifiers, panel doesn't capture
        assert!(processor.should_escape_close(&key_event(KeyCode::Esc, KeyModifiers::NONE), false));

        // Escape with modifiers - don't close
        assert!(!processor.should_escape_close(&key_event(KeyCode::Esc, KeyModifiers::ALT), false));

        // Panel captures escape - don't close
        assert!(!processor.should_escape_close(&key_event(KeyCode::Esc, KeyModifiers::NONE), true));
    }

    #[test]
    fn test_custom_binding() {
        let mut processor = DefaultHotkeyProcessor::new();

        // Add custom binding
        processor.bind(
            KeyBinding::alt(KeyCode::Char('z')),
            HotkeyAction::ToggleStacking,
        );

        assert_eq!(
            processor.process_hotkey(&alt_key('z')),
            Some(HotkeyAction::ToggleStacking)
        );

        // Remove binding
        processor.unbind(&KeyBinding::alt(KeyCode::Char('z')));
        assert_eq!(processor.process_hotkey(&alt_key('z')), None);
    }
}
