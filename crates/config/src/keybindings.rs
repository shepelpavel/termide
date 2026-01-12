//! Keybindings configuration for termide.
//!
//! Supports configurable keyboard shortcuts via config.toml sections like:
//! ```toml
//! [general.keybindings]
//! toggle_menu = "Alt+M"
//! new_terminal = "Alt+T"
//!
//! [editor.keybindings]
//! save = "Ctrl+S"
//! copy_files = ["C", "F5"]  # multiple bindings
//! ```

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

/// A keybinding that can be either a single key or multiple alternatives.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum KeyBinding {
    /// Single keybinding: "Ctrl+S"
    Single(String),
    /// Multiple alternatives: ["C", "F5"]
    Multiple(Vec<String>),
}

impl KeyBinding {
    /// Check if a key event matches this binding.
    pub fn matches(&self, event: &KeyEvent) -> bool {
        match self {
            KeyBinding::Single(s) => {
                if let Ok(parsed) = parse_keybinding(s) {
                    parsed.matches(event)
                } else {
                    false
                }
            }
            KeyBinding::Multiple(bindings) => bindings.iter().any(|s| {
                if let Ok(parsed) = parse_keybinding(s) {
                    parsed.matches(event)
                } else {
                    false
                }
            }),
        }
    }

    /// Parse into a list of ParsedKeyBindings.
    pub fn parse(&self) -> Vec<ParsedKeyBinding> {
        match self {
            KeyBinding::Single(s) => parse_keybinding(s).into_iter().collect(),
            KeyBinding::Multiple(bindings) => bindings
                .iter()
                .filter_map(|s| parse_keybinding(s).ok())
                .collect(),
        }
    }
}

/// A parsed keybinding ready for runtime matching.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParsedKeyBinding {
    pub key: KeyCode,
    pub modifiers: KeyModifiers,
}

impl ParsedKeyBinding {
    /// Check if a key event matches this parsed binding.
    pub fn matches(&self, event: &KeyEvent) -> bool {
        // Normalize character case for matching
        let key_matches = match (&self.key, &event.code) {
            (KeyCode::Char(a), KeyCode::Char(b)) => a.eq_ignore_ascii_case(b),
            (a, b) => a == b,
        };

        key_matches && self.modifiers == event.modifiers
    }
}

/// Parse a keybinding string like "Ctrl+Shift+S" into a ParsedKeyBinding.
pub fn parse_keybinding(s: &str) -> Result<ParsedKeyBinding, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("Empty keybinding".to_string());
    }

    let parts: Vec<&str> = s.split('+').collect();
    if parts.is_empty() {
        return Err("Invalid keybinding format".to_string());
    }

    let mut modifiers = KeyModifiers::empty();
    let key_str = parts.last().unwrap().trim();

    // Parse modifiers (all parts except the last one)
    for part in &parts[..parts.len().saturating_sub(1)] {
        let part = part.trim().to_lowercase();
        match part.as_str() {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "alt" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            _ => return Err(format!("Unknown modifier: {}", part)),
        }
    }

    // Parse the key
    let key = parse_key(key_str)?;

    Ok(ParsedKeyBinding { key, modifiers })
}

/// Parse a key name into a KeyCode.
fn parse_key(s: &str) -> Result<KeyCode, String> {
    let lower = s.to_lowercase();
    match lower.as_str() {
        // Special keys
        "enter" | "return" => Ok(KeyCode::Enter),
        "esc" | "escape" => Ok(KeyCode::Esc),
        "tab" => Ok(KeyCode::Tab),
        "space" => Ok(KeyCode::Char(' ')),
        "backspace" | "bs" => Ok(KeyCode::Backspace),
        "delete" | "del" => Ok(KeyCode::Delete),
        "insert" | "ins" => Ok(KeyCode::Insert),
        "home" => Ok(KeyCode::Home),
        "end" => Ok(KeyCode::End),
        "pageup" | "pgup" => Ok(KeyCode::PageUp),
        "pagedown" | "pgdn" | "pgdown" => Ok(KeyCode::PageDown),
        "up" => Ok(KeyCode::Up),
        "down" => Ok(KeyCode::Down),
        "left" => Ok(KeyCode::Left),
        "right" => Ok(KeyCode::Right),

        // Function keys
        "f1" => Ok(KeyCode::F(1)),
        "f2" => Ok(KeyCode::F(2)),
        "f3" => Ok(KeyCode::F(3)),
        "f4" => Ok(KeyCode::F(4)),
        "f5" => Ok(KeyCode::F(5)),
        "f6" => Ok(KeyCode::F(6)),
        "f7" => Ok(KeyCode::F(7)),
        "f8" => Ok(KeyCode::F(8)),
        "f9" => Ok(KeyCode::F(9)),
        "f10" => Ok(KeyCode::F(10)),
        "f11" => Ok(KeyCode::F(11)),
        "f12" => Ok(KeyCode::F(12)),

        // Single character
        _ if s.len() == 1 => Ok(KeyCode::Char(s.chars().next().unwrap())),

        _ => Err(format!("Unknown key: {}", s)),
    }
}

// =============================================================================
// Keybindings structures for each config section
// =============================================================================

/// Global keybindings (general.keybindings section).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalKeybindings {
    // Menu & UI
    pub toggle_menu: Option<KeyBinding>,

    // Panel creation
    pub new_file_manager: Option<KeyBinding>,
    pub new_terminal: Option<KeyBinding>,
    pub new_editor: Option<KeyBinding>,
    pub new_log_panel: Option<KeyBinding>,
    pub open_help: Option<KeyBinding>,
    pub open_preferences: Option<KeyBinding>,
    pub open_sessions: Option<KeyBinding>,
    pub open_git_status: Option<KeyBinding>,

    // Panel management
    pub close_panel: Option<KeyBinding>,
    pub toggle_stack: Option<KeyBinding>,
    pub swap_left: Option<KeyBinding>,
    pub swap_right: Option<KeyBinding>,
    pub move_first: Option<KeyBinding>,
    pub move_last: Option<KeyBinding>,
    pub resize_smaller: Option<KeyBinding>,
    pub resize_larger: Option<KeyBinding>,

    // Navigation
    pub prev_group: Option<KeyBinding>,
    pub next_group: Option<KeyBinding>,
    pub prev_panel: Option<KeyBinding>,
    pub next_panel: Option<KeyBinding>,
    pub goto_panel_1: Option<KeyBinding>,
    pub goto_panel_2: Option<KeyBinding>,
    pub goto_panel_3: Option<KeyBinding>,
    pub goto_panel_4: Option<KeyBinding>,
    pub goto_panel_5: Option<KeyBinding>,
    pub goto_panel_6: Option<KeyBinding>,
    pub goto_panel_7: Option<KeyBinding>,
    pub goto_panel_8: Option<KeyBinding>,
    pub goto_panel_9: Option<KeyBinding>,

    // Application
    pub quit: Option<KeyBinding>,
}

/// Editor keybindings (editor.keybindings section).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EditorKeybindings {
    // File operations
    pub save: Option<KeyBinding>,
    pub force_save: Option<KeyBinding>,
    pub reload: Option<KeyBinding>,

    // Undo/Redo
    pub undo: Option<KeyBinding>,
    pub redo: Option<KeyBinding>,

    // Clipboard
    pub copy: Option<KeyBinding>,
    pub cut: Option<KeyBinding>,
    pub paste: Option<KeyBinding>,

    // Selection
    pub select_all: Option<KeyBinding>,

    // Editing
    pub duplicate_line: Option<KeyBinding>,

    // Search & Replace
    pub search: Option<KeyBinding>,
    pub search_next: Option<KeyBinding>,
    pub search_prev: Option<KeyBinding>,
    pub replace: Option<KeyBinding>,
    pub replace_current: Option<KeyBinding>,
    pub replace_all: Option<KeyBinding>,
}

/// File manager keybindings (file_manager.keybindings section).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileManagerKeybindings {
    // File operations
    pub copy_files: Option<KeyBinding>,
    pub move_files: Option<KeyBinding>,
    pub delete_files: Option<KeyBinding>,
    pub view_file: Option<KeyBinding>,
    pub edit_file: Option<KeyBinding>,
    pub new_file: Option<KeyBinding>,
    pub new_directory: Option<KeyBinding>,

    // Search
    pub search_files: Option<KeyBinding>,
    pub search_content: Option<KeyBinding>,

    // Navigation
    pub go_home: Option<KeyBinding>,
    pub go_parent: Option<KeyBinding>,
    pub refresh: Option<KeyBinding>,

    // Selection
    pub toggle_selection: Option<KeyBinding>,
    pub select_all: Option<KeyBinding>,

    // Other
    pub open_external: Option<KeyBinding>,
}

/// Git status panel keybindings (git_status.keybindings section).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitStatusKeybindings {
    pub stage_file: Option<KeyBinding>,
    pub unstage_file: Option<KeyBinding>,
    pub refresh: Option<KeyBinding>,
    pub next_section: Option<KeyBinding>,
    pub prev_section: Option<KeyBinding>,
}

/// Terminal panel keybindings (terminal.keybindings section).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TerminalKeybindings {
    pub copy: Option<KeyBinding>,
    pub paste: Option<KeyBinding>,
    pub scroll_up: Option<KeyBinding>,
    pub scroll_down: Option<KeyBinding>,
    pub scroll_top: Option<KeyBinding>,
    pub scroll_bottom: Option<KeyBinding>,
}

// =============================================================================
// Helper macros and functions for matching keybindings
// =============================================================================

/// Helper to check if an event matches a keybinding or a default.
pub fn matches_binding_or_default(
    binding: &Option<KeyBinding>,
    event: &KeyEvent,
    default_key: KeyCode,
    default_modifiers: KeyModifiers,
) -> bool {
    if let Some(kb) = binding {
        kb.matches(event)
    } else {
        let default = ParsedKeyBinding {
            key: default_key,
            modifiers: default_modifiers,
        };
        default.matches(event)
    }
}

/// Helper to check if an event matches any of multiple default bindings.
pub fn matches_binding_or_defaults(
    binding: &Option<KeyBinding>,
    event: &KeyEvent,
    defaults: &[(KeyCode, KeyModifiers)],
) -> bool {
    if let Some(kb) = binding {
        kb.matches(event)
    } else {
        defaults.iter().any(|(key, mods)| {
            let default = ParsedKeyBinding {
                key: *key,
                modifiers: *mods,
            };
            default.matches(event)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_key() {
        let kb = parse_keybinding("A").unwrap();
        assert_eq!(kb.key, KeyCode::Char('A'));
        assert_eq!(kb.modifiers, KeyModifiers::empty());
    }

    #[test]
    fn test_parse_ctrl_key() {
        let kb = parse_keybinding("Ctrl+S").unwrap();
        assert_eq!(kb.key, KeyCode::Char('S'));
        assert_eq!(kb.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn test_parse_ctrl_shift_key() {
        let kb = parse_keybinding("Ctrl+Shift+S").unwrap();
        assert_eq!(kb.key, KeyCode::Char('S'));
        assert_eq!(kb.modifiers, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    }

    #[test]
    fn test_parse_alt_key() {
        let kb = parse_keybinding("Alt+F").unwrap();
        assert_eq!(kb.key, KeyCode::Char('F'));
        assert_eq!(kb.modifiers, KeyModifiers::ALT);
    }

    #[test]
    fn test_parse_function_key() {
        let kb = parse_keybinding("F5").unwrap();
        assert_eq!(kb.key, KeyCode::F(5));
        assert_eq!(kb.modifiers, KeyModifiers::empty());
    }

    #[test]
    fn test_parse_shift_function_key() {
        let kb = parse_keybinding("Shift+F3").unwrap();
        assert_eq!(kb.key, KeyCode::F(3));
        assert_eq!(kb.modifiers, KeyModifiers::SHIFT);
    }

    #[test]
    fn test_parse_special_keys() {
        assert_eq!(parse_keybinding("Enter").unwrap().key, KeyCode::Enter);
        assert_eq!(parse_keybinding("Escape").unwrap().key, KeyCode::Esc);
        assert_eq!(parse_keybinding("Tab").unwrap().key, KeyCode::Tab);
        assert_eq!(parse_keybinding("Space").unwrap().key, KeyCode::Char(' '));
        assert_eq!(
            parse_keybinding("Backspace").unwrap().key,
            KeyCode::Backspace
        );
        assert_eq!(parse_keybinding("Delete").unwrap().key, KeyCode::Delete);
        assert_eq!(parse_keybinding("Insert").unwrap().key, KeyCode::Insert);
        assert_eq!(parse_keybinding("Home").unwrap().key, KeyCode::Home);
        assert_eq!(parse_keybinding("End").unwrap().key, KeyCode::End);
        assert_eq!(parse_keybinding("PageUp").unwrap().key, KeyCode::PageUp);
        assert_eq!(parse_keybinding("PageDown").unwrap().key, KeyCode::PageDown);
    }

    #[test]
    fn test_parse_arrow_keys() {
        assert_eq!(parse_keybinding("Up").unwrap().key, KeyCode::Up);
        assert_eq!(parse_keybinding("Down").unwrap().key, KeyCode::Down);
        assert_eq!(parse_keybinding("Left").unwrap().key, KeyCode::Left);
        assert_eq!(parse_keybinding("Right").unwrap().key, KeyCode::Right);
    }

    #[test]
    fn test_parse_case_insensitive() {
        let kb1 = parse_keybinding("ctrl+s").unwrap();
        let kb2 = parse_keybinding("CTRL+S").unwrap();
        let kb3 = parse_keybinding("Ctrl+S").unwrap();
        assert_eq!(kb1.modifiers, kb2.modifiers);
        assert_eq!(kb2.modifiers, kb3.modifiers);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse_keybinding("").is_err());
        assert!(parse_keybinding("InvalidKey").is_err());
        assert!(parse_keybinding("Ctrl+").is_err());
    }

    #[test]
    fn test_keybinding_matches() {
        let kb = KeyBinding::Single("Ctrl+S".to_string());
        let event = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(kb.matches(&event));

        let wrong_event = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty());
        assert!(!kb.matches(&wrong_event));
    }

    #[test]
    fn test_keybinding_multiple_matches() {
        let kb = KeyBinding::Multiple(vec!["C".to_string(), "F5".to_string()]);

        let event_c = KeyEvent::new(KeyCode::Char('C'), KeyModifiers::empty());
        let event_f5 = KeyEvent::new(KeyCode::F(5), KeyModifiers::empty());
        let event_d = KeyEvent::new(KeyCode::Char('D'), KeyModifiers::empty());

        assert!(kb.matches(&event_c));
        assert!(kb.matches(&event_f5));
        assert!(!kb.matches(&event_d));
    }
}
