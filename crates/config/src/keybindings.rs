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

    /// Get the first keybinding as a display string.
    pub fn display(&self) -> &str {
        match self {
            KeyBinding::Single(s) => s.as_str(),
            KeyBinding::Multiple(v) => v.first().map(|s| s.as_str()).unwrap_or(""),
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
    let key_str = parts.last().ok_or("Empty keybinding")?.trim();

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

        // Single character (works for ASCII and multi-byte Unicode)
        _ => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(KeyCode::Char(c)),
                _ => Err(format!("Unknown key: {}", s)),
            }
        }
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
    pub new_journal: Option<KeyBinding>,
    pub open_help: Option<KeyBinding>,
    pub open_preferences: Option<KeyBinding>,
    pub open_sessions: Option<KeyBinding>,
    pub new_session: Option<KeyBinding>,
    pub open_git_status: Option<KeyBinding>,
    pub open_bookmark_add: Option<KeyBinding>,
    pub open_outline: Option<KeyBinding>,
    pub open_diagnostics: Option<KeyBinding>,
    pub open_git_log: Option<KeyBinding>,

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
    pub open_command_palette: Option<KeyBinding>,
}

/// Editor keybindings (editor.keybindings section).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EditorKeybindings {
    // File operations
    pub save: Option<KeyBinding>,
    pub save_as: Option<KeyBinding>,
    pub reload: Option<KeyBinding>,

    // Editing
    pub undo: Option<KeyBinding>,
    pub redo: Option<KeyBinding>,
    pub duplicate_line: Option<KeyBinding>,
    pub toggle_comment: Option<KeyBinding>,

    // Search & Replace
    pub search: Option<KeyBinding>,
    pub search_next: Option<KeyBinding>,
    pub search_prev: Option<KeyBinding>,
    pub replace: Option<KeyBinding>,
    pub replace_current: Option<KeyBinding>,
    pub replace_all: Option<KeyBinding>,

    // Selection
    pub select_all: Option<KeyBinding>,

    // Clipboard
    pub copy: Option<KeyBinding>,
    pub cut: Option<KeyBinding>,
    pub paste: Option<KeyBinding>,

    // LSP
    pub trigger_completion: Option<KeyBinding>,
    pub show_hover: Option<KeyBinding>,
    pub goto_definition: Option<KeyBinding>,
    pub find_references: Option<KeyBinding>,
    pub rename_symbol: Option<KeyBinding>,
}

/// File manager keybindings (file_manager.keybindings section).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileManagerKeybindings {
    // File operations
    pub rename: Option<KeyBinding>,
    pub view: Option<KeyBinding>,
    pub edit: Option<KeyBinding>,
    pub copy: Option<KeyBinding>,
    pub move_item: Option<KeyBinding>,
    pub create_dir: Option<KeyBinding>,
    pub create_file: Option<KeyBinding>,
    pub delete: Option<KeyBinding>,
    pub info: Option<KeyBinding>,

    // Search
    pub search: Option<KeyBinding>,
    pub search_content: Option<KeyBinding>,

    // Navigation
    pub refresh: Option<KeyBinding>,
    pub go_parent: Option<KeyBinding>,
    pub go_home: Option<KeyBinding>,
    pub switch_directory: Option<KeyBinding>,
    pub go_to_path: Option<KeyBinding>,

    // Selection
    pub toggle_selection: Option<KeyBinding>,
    pub select_all: Option<KeyBinding>,

    // Other
    pub open_external: Option<KeyBinding>,
    pub toggle_hidden: Option<KeyBinding>,
    pub clipboard_copy: Option<KeyBinding>,
    pub clipboard_cut: Option<KeyBinding>,
    pub clipboard_paste: Option<KeyBinding>,
}

/// Git status panel keybindings (git_status.keybindings section).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitStatusKeybindings {
    /// Stage selected file
    pub stage: Option<KeyBinding>,
    /// Unstage selected file
    pub unstage: Option<KeyBinding>,
    /// View diff for selected file
    pub view: Option<KeyBinding>,
    /// Edit selected file in editor
    pub edit: Option<KeyBinding>,
    /// Show file info / context menu
    pub info: Option<KeyBinding>,
    /// Revert (discard changes) for selected file
    pub revert: Option<KeyBinding>,
    /// Refresh git status
    pub refresh: Option<KeyBinding>,
}

impl GitStatusKeybindings {
    /// Fill None values with default keybindings
    pub fn with_defaults(&mut self) {
        macro_rules! set_default {
            ($field:ident, $default:expr) => {
                if self.$field.is_none() {
                    self.$field = Some(KeyBinding::Single($default.into()));
                }
            };
        }

        set_default!(stage, "S");
        set_default!(unstage, "U");
        set_default!(view, "F3");
        set_default!(edit, "F4");
        if self.info.is_none() {
            self.info = Some(KeyBinding::Multiple(vec!["Space".into(), "F12".into()]));
        }
        if self.revert.is_none() {
            self.revert = Some(KeyBinding::Multiple(vec![
                "Backspace".into(),
                "Delete".into(),
            ]));
        }
        set_default!(refresh, "Ctrl+R");
    }
}

/// Git diff panel keybindings (git_diff.keybindings section).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitDiffKeybindings {
    /// Toggle collapse of file diff section
    pub toggle_collapse: Option<KeyBinding>,
    /// Edit file in editor
    pub edit: Option<KeyBinding>,
    /// Refresh diff
    pub refresh: Option<KeyBinding>,
    /// Scroll half page up
    pub scroll_half_up: Option<KeyBinding>,
    /// Scroll half page down
    pub scroll_half_down: Option<KeyBinding>,
    /// Copy selected file path to clipboard
    pub clipboard_copy: Option<KeyBinding>,
}

impl GitDiffKeybindings {
    /// Fill None values with default keybindings
    pub fn with_defaults(&mut self) {
        macro_rules! set_default {
            ($field:ident, $default:expr) => {
                if self.$field.is_none() {
                    self.$field = Some(KeyBinding::Single($default.into()));
                }
            };
        }

        if self.toggle_collapse.is_none() {
            self.toggle_collapse = Some(KeyBinding::Multiple(vec!["Enter".into(), "Space".into()]));
        }
        if self.edit.is_none() {
            self.edit = Some(KeyBinding::Multiple(vec!["F4".into(), "E".into()]));
        }
        if self.refresh.is_none() {
            self.refresh = Some(KeyBinding::Multiple(vec!["F5".into(), "Ctrl+R".into()]));
        }
        set_default!(scroll_half_up, "Ctrl+U");
        set_default!(scroll_half_down, "Ctrl+D");
        set_default!(clipboard_copy, "Ctrl+C");
    }
}

/// Git log panel keybindings (git_log.keybindings section).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitLogKeybindings {
    /// Show commit info
    pub info: Option<KeyBinding>,
    /// View commit diff
    pub view_diff: Option<KeyBinding>,
    /// Checkout commit/branch
    pub checkout: Option<KeyBinding>,
    /// Copy selected commit hash to clipboard
    pub clipboard_copy: Option<KeyBinding>,
}

impl GitLogKeybindings {
    /// Fill None values with default keybindings
    pub fn with_defaults(&mut self) {
        macro_rules! set_default {
            ($field:ident, $default:expr) => {
                if self.$field.is_none() {
                    self.$field = Some(KeyBinding::Single($default.into()));
                }
            };
        }

        set_default!(info, "Space");
        set_default!(view_diff, "D");
        set_default!(checkout, "C");
        set_default!(clipboard_copy, "Ctrl+C");
    }
}

/// Terminal panel keybindings (terminal.keybindings section).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TerminalKeybindings {
    pub copy: Option<KeyBinding>,
    pub paste: Option<KeyBinding>,
    pub scroll_up: Option<KeyBinding>,
    pub scroll_down: Option<KeyBinding>,
    pub scroll_top: Option<KeyBinding>,
    pub scroll_bottom: Option<KeyBinding>,
    pub search: Option<KeyBinding>,
    pub switch_directory: Option<KeyBinding>,
}

// =============================================================================
// Default value implementations for config normalization
// =============================================================================

impl GlobalKeybindings {
    /// Fill None values with default keybindings
    pub fn with_defaults(&mut self) {
        macro_rules! set_default {
            ($field:ident, $default:expr) => {
                if self.$field.is_none() {
                    self.$field = Some(KeyBinding::Single($default.into()));
                }
            };
        }

        // Menu & UI (toggle_menu uses set_default_multiple, defined below)

        // Panel creation
        set_default!(new_file_manager, "Alt+F");
        set_default!(new_terminal, "Alt+T");
        set_default!(new_editor, "Alt+E");
        set_default!(new_journal, "Alt+L");
        // open_help gets F1 alternative below (needs set_default_multiple)
        set_default!(open_preferences, "Alt+P");
        set_default!(open_sessions, "Alt+/");
        set_default!(new_session, "Alt+N");
        set_default!(open_git_status, "Alt+G");
        set_default!(open_bookmark_add, "Alt+B");
        set_default!(open_outline, "Alt+O");
        set_default!(open_diagnostics, "Alt+I");
        set_default!(open_git_log, "Alt+C");

        // Panel management (close_panel and toggle_stack get F-key alternatives below)
        set_default!(swap_left, "Alt+PageUp");
        set_default!(swap_right, "Alt+PageDown");
        set_default!(move_first, "Alt+Home");
        set_default!(move_last, "Alt+End");
        set_default!(resize_smaller, "Alt+-");
        set_default!(resize_larger, "Alt+=");

        // Navigation (with WASD alternatives)
        macro_rules! set_default_multiple {
            ($field:ident, $($default:expr),+) => {
                if self.$field.is_none() {
                    self.$field = Some(KeyBinding::Multiple(vec![$($default.into()),+]));
                }
            };
        }

        set_default_multiple!(toggle_menu, "Alt+M", "F9");
        set_default_multiple!(open_help, "Alt+H", "F1");
        set_default_multiple!(close_panel, "Alt+X", "F10");
        set_default_multiple!(toggle_stack, "Alt+Backspace", "F11");

        set_default_multiple!(prev_group, "Alt+Left", "Alt+A");
        set_default_multiple!(next_group, "Alt+Right", "Alt+D");
        set_default_multiple!(prev_panel, "Alt+Up", "Alt+W");
        set_default_multiple!(next_panel, "Alt+Down", "Alt+S");
        set_default!(goto_panel_1, "Alt+1");
        set_default!(goto_panel_2, "Alt+2");
        set_default!(goto_panel_3, "Alt+3");
        set_default!(goto_panel_4, "Alt+4");
        set_default!(goto_panel_5, "Alt+5");
        set_default!(goto_panel_6, "Alt+6");
        set_default!(goto_panel_7, "Alt+7");
        set_default!(goto_panel_8, "Alt+8");
        set_default!(goto_panel_9, "Alt+9");

        // Application
        set_default!(quit, "Alt+Q");
        set_default!(open_command_palette, "Ctrl+P");
    }
}

impl EditorKeybindings {
    /// Fill None values with default keybindings
    pub fn with_defaults(&mut self) {
        macro_rules! set_default {
            ($field:ident, $default:expr) => {
                if self.$field.is_none() {
                    self.$field = Some(KeyBinding::Single($default.into()));
                }
            };
        }

        macro_rules! set_default_multiple {
            ($field:ident, $($default:expr),+) => {
                if self.$field.is_none() {
                    self.$field = Some(KeyBinding::Multiple(vec![$($default.into()),+]));
                }
            };
        }

        // File operations
        set_default_multiple!(save, "F2", "Ctrl+S");
        set_default!(save_as, "Ctrl+Shift+S");
        set_default!(reload, "Ctrl+Shift+R");

        // Editing
        set_default!(undo, "Ctrl+Z");
        set_default_multiple!(redo, "Ctrl+Y", "Ctrl+Shift+Z");
        set_default!(duplicate_line, "Ctrl+D");
        set_default!(toggle_comment, "Ctrl+/");

        // Search & Replace
        set_default!(search, "Ctrl+F");
        set_default!(search_next, "F3");
        set_default!(search_prev, "Shift+F3");
        set_default!(replace, "Ctrl+H");
        set_default!(replace_current, "Ctrl+R");
        set_default!(replace_all, "Ctrl+Alt+R");

        // Selection
        set_default!(select_all, "Ctrl+A");

        // Clipboard
        set_default_multiple!(copy, "Ctrl+C", "Ctrl+Insert", "Ctrl+Shift+C");
        set_default_multiple!(cut, "Ctrl+X", "Shift+Delete");
        set_default_multiple!(paste, "Ctrl+V", "Shift+Insert", "Ctrl+Shift+V");

        // LSP
        set_default!(trigger_completion, "Ctrl+.");
        set_default!(show_hover, "Ctrl+K");
        set_default!(goto_definition, "F12");
        set_default!(rename_symbol, "F4");
        set_default_multiple!(find_references, "Shift+F12", "F24");
    }
}

impl FileManagerKeybindings {
    /// Fill None values with default keybindings
    pub fn with_defaults(&mut self) {
        macro_rules! set_default {
            ($field:ident, $default:expr) => {
                if self.$field.is_none() {
                    self.$field = Some(KeyBinding::Single($default.into()));
                }
            };
        }

        // File operations
        if self.rename.is_none() {
            self.rename = Some(KeyBinding::Multiple(vec!["F2".into(), "R".into()]));
        }
        if self.view.is_none() {
            self.view = Some(KeyBinding::Multiple(vec!["F3".into(), "V".into()]));
        }
        if self.edit.is_none() {
            self.edit = Some(KeyBinding::Multiple(vec!["F4".into(), "E".into()]));
        }
        if self.copy.is_none() {
            self.copy = Some(KeyBinding::Multiple(vec!["F5".into(), "C".into()]));
        }
        if self.move_item.is_none() {
            self.move_item = Some(KeyBinding::Multiple(vec!["F6".into(), "M".into()]));
        }
        if self.create_dir.is_none() {
            self.create_dir = Some(KeyBinding::Multiple(vec!["F7".into(), "D".into()]));
        }
        if self.create_file.is_none() {
            self.create_file = Some(KeyBinding::Multiple(vec!["F".into(), "Ctrl+N".into()]));
        }
        if self.delete.is_none() {
            self.delete = Some(KeyBinding::Multiple(vec!["Delete".into(), "F8".into()]));
        }
        if self.info.is_none() {
            self.info = Some(KeyBinding::Multiple(vec!["F12".into(), "Space".into()]));
        }

        // Search
        set_default!(search, "Ctrl+F");
        set_default!(search_content, "Ctrl+Shift+F");

        // Navigation
        set_default!(refresh, "Ctrl+R");
        set_default!(go_parent, "Backspace");
        set_default!(go_home, "~");
        set_default!(switch_directory, "Ctrl+/");
        set_default!(go_to_path, "Ctrl+G");

        // Selection
        set_default!(toggle_selection, "Insert");
        set_default!(select_all, "Ctrl+A");

        // Other
        if self.open_external.is_none() {
            self.open_external = Some(KeyBinding::Multiple(vec!["O".into(), "Alt+Enter".into()]));
        }
        set_default!(toggle_hidden, ".");
        set_default!(clipboard_copy, "Ctrl+C");
        set_default!(clipboard_cut, "Ctrl+X");
        set_default!(clipboard_paste, "Ctrl+V");
    }
}

impl TerminalKeybindings {
    /// Fill None values with default keybindings
    pub fn with_defaults(&mut self) {
        macro_rules! set_default {
            ($field:ident, $default:expr) => {
                if self.$field.is_none() {
                    self.$field = Some(KeyBinding::Single($default.into()));
                }
            };
        }

        set_default!(copy, "Ctrl+Shift+C");
        set_default!(paste, "Ctrl+Shift+V");
        set_default!(scroll_up, "Shift+PageUp");
        set_default!(scroll_down, "Shift+PageDown");
        set_default!(scroll_top, "Shift+Home");
        set_default!(scroll_bottom, "Shift+End");
        set_default!(search, "Ctrl+F");
        set_default!(switch_directory, "Ctrl+/");
    }
}

// =============================================================================
// Latin to Cyrillic mapping for keyboard layout support
// =============================================================================

/// Map Cyrillic character to Latin equivalent (ЙЦУКЕН → QWERTY layout).
///
/// This allows Vim commands to work regardless of the current keyboard layout.
/// For example, 'о' (Cyrillic) maps to 'j' for Vim down motion.
pub fn cyrillic_to_latin(c: char) -> Option<char> {
    match c {
        // Lowercase
        'ф' => Some('a'),
        'и' => Some('b'),
        'с' => Some('c'),
        'в' => Some('d'),
        'у' => Some('e'),
        'а' => Some('f'),
        'п' => Some('g'),
        'р' => Some('h'),
        'ш' => Some('i'),
        'о' => Some('j'),
        'л' => Some('k'),
        'д' => Some('l'),
        'ь' => Some('m'),
        'т' => Some('n'),
        'щ' => Some('o'),
        'з' => Some('p'),
        'й' => Some('q'),
        'к' => Some('r'),
        'ы' => Some('s'),
        'е' => Some('t'),
        'г' => Some('u'),
        'м' => Some('v'),
        'ц' => Some('w'),
        'ч' => Some('x'),
        'н' => Some('y'),
        'я' => Some('z'),
        // Uppercase
        'Ф' => Some('A'),
        'И' => Some('B'),
        'С' => Some('C'),
        'В' => Some('D'),
        'У' => Some('E'),
        'А' => Some('F'),
        'П' => Some('G'),
        'Р' => Some('H'),
        'Ш' => Some('I'),
        'О' => Some('J'),
        'Л' => Some('K'),
        'Д' => Some('L'),
        'Ь' => Some('M'),
        'Т' => Some('N'),
        'Щ' => Some('O'),
        'З' => Some('P'),
        'Й' => Some('Q'),
        'К' => Some('R'),
        'Ы' => Some('S'),
        'Е' => Some('T'),
        'Г' => Some('U'),
        'М' => Some('V'),
        'Ц' => Some('W'),
        'Ч' => Some('X'),
        'Н' => Some('Y'),
        'Я' => Some('Z'),
        // Punctuation (for Vim commands like $ ; ^)
        'х' => Some('['),
        'ъ' => Some(']'),
        'ж' => Some(';'),
        'э' => Some('\''),
        'б' => Some(','),
        'ю' => Some('.'),
        _ => None,
    }
}

/// Map Latin character to Cyrillic equivalent (QWERTY → ЙЦУКЕН layout).
///
/// This allows hotkeys to work regardless of the current keyboard layout.
/// For example, Alt+M will also work as Alt+Ь when in Russian layout.
pub fn latin_to_cyrillic(c: char) -> Option<char> {
    match c.to_ascii_lowercase() {
        'a' => Some('ф'),
        'b' => Some('и'),
        'c' => Some('с'),
        'd' => Some('в'),
        'e' => Some('у'),
        'f' => Some('а'),
        'g' => Some('п'),
        'h' => Some('р'),
        'i' => Some('ш'),
        'j' => Some('о'),
        'k' => Some('л'),
        'l' => Some('д'),
        'm' => Some('ь'),
        'n' => Some('т'),
        'o' => Some('щ'),
        'p' => Some('з'),
        'q' => Some('й'),
        'r' => Some('к'),
        's' => Some('ы'),
        't' => Some('е'),
        'u' => Some('г'),
        'v' => Some('м'),
        'w' => Some('ц'),
        'x' => Some('ч'),
        'y' => Some('н'),
        'z' => Some('я'),
        '[' => Some('х'),
        ']' => Some('ъ'),
        ';' => Some('ж'),
        '\'' => Some('э'),
        ',' => Some('б'),
        '.' => Some('ю'),
        '/' => Some('.'),
        _ => None,
    }
}

// =============================================================================
// Vim-aware navigation helpers for list panels
// =============================================================================

/// Check if key event is a "move up" action.
/// Returns true for Up arrow (without modifiers), or 'k'/'л' when vim_mode is enabled.
pub fn is_move_up(key: &KeyEvent, vim_mode: bool) -> bool {
    if key.code == KeyCode::Up && key.modifiers.is_empty() {
        return true;
    }
    if vim_mode && key.modifiers.is_empty() {
        let normalized = termide_keyboard::normalize_for_matching(key);
        return matches!(normalized.code, KeyCode::Char('k'));
    }
    false
}

/// Check if key event is a "move down" action.
/// Returns true for Down arrow, or 'j' (any layout) when vim_mode is enabled.
pub fn is_move_down(key: &KeyEvent, vim_mode: bool) -> bool {
    if key.code == KeyCode::Down && key.modifiers.is_empty() {
        return true;
    }
    if vim_mode && key.modifiers.is_empty() {
        let normalized = termide_keyboard::normalize_for_matching(key);
        return matches!(normalized.code, KeyCode::Char('j'));
    }
    false
}

/// Check if key event is a "go to start/home" action.
/// Returns true for Home key, or 'g' (any layout) when vim_mode is enabled.
pub fn is_go_home(key: &KeyEvent, vim_mode: bool) -> bool {
    if key.code == KeyCode::Home && key.modifiers.is_empty() {
        return true;
    }
    if vim_mode && key.modifiers.is_empty() {
        let normalized = termide_keyboard::normalize_for_matching(key);
        return matches!(normalized.code, KeyCode::Char('g'));
    }
    false
}

/// Check if key event is a "go to end" action.
/// Returns true for End key, or 'G' (any layout, Shift) when vim_mode is enabled.
pub fn is_go_end(key: &KeyEvent, vim_mode: bool) -> bool {
    if key.code == KeyCode::End && key.modifiers.is_empty() {
        return true;
    }
    if vim_mode && key.modifiers == KeyModifiers::SHIFT {
        let normalized = termide_keyboard::normalize_for_matching(key);
        return matches!(normalized.code, KeyCode::Char('G'));
    }
    false
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

    #[test]
    fn test_latin_to_cyrillic() {
        // Common keys used in hotkeys
        assert_eq!(latin_to_cyrillic('m'), Some('ь'));
        assert_eq!(latin_to_cyrillic('M'), Some('ь')); // uppercase input → lowercase cyrillic
        assert_eq!(latin_to_cyrillic('f'), Some('а'));
        assert_eq!(latin_to_cyrillic('t'), Some('е'));
        assert_eq!(latin_to_cyrillic('g'), Some('п'));
        assert_eq!(latin_to_cyrillic('q'), Some('й'));

        // WASD keys
        assert_eq!(latin_to_cyrillic('w'), Some('ц'));
        assert_eq!(latin_to_cyrillic('a'), Some('ф'));
        assert_eq!(latin_to_cyrillic('s'), Some('ы'));
        assert_eq!(latin_to_cyrillic('d'), Some('в'));

        // Non-letter keys
        assert_eq!(latin_to_cyrillic('1'), None);
        assert_eq!(latin_to_cyrillic('-'), None);
    }
}
