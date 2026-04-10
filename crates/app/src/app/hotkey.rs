//! Semantic hotkey system for app-level dispatch.
//!
//! The normalizer converts raw `KeyEvent`s into semantic `Hotkey` values.
//! Used by the global hotkey handler and command palette.
//! Unrecognized keys get `HotkeyKind::Other` with the raw event preserved.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use termide_config::{matches_binding_or_default, matches_binding_or_defaults, GlobalKeybindings};

/// Normalized hotkey with semantic kind and original key event.
#[derive(Debug, Clone)]
pub struct Hotkey {
    /// Semantic meaning of the hotkey
    pub kind: HotkeyKind,
    /// Original raw key event (preserved for fallback)
    pub raw: KeyEvent,
}

/// Semantic hotkey kind recognized from a KeyEvent.
///
/// Used for app-level dispatch (global hotkeys, command palette).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyKind {
    // === F-key universal actions (context-dependent per panel) ===
    /// F9, Alt+M — Toggle menu
    Menu,
    /// F10, Alt+X — Close panel
    ClosePanel,
    /// F11, Alt+Backspace — Toggle panel stacking
    ToggleStack,

    // === App-level actions (handled before reaching panels) ===
    /// Alt+Q — Quit application
    Quit,
    /// Alt+F — New file manager panel
    NewFileManager,
    /// Alt+T — New terminal panel
    NewTerminal,
    /// Alt+E — New editor panel
    NewEditor,
    /// Alt+L — New journal panel
    NewJournal,
    /// Alt+H — Open help panel
    OpenHelp,
    /// Alt+P — Open preferences
    OpenPreferences,
    /// Alt+/ — Open sessions
    OpenSessions,
    /// Alt+N — New session
    NewSession,
    /// Alt+G — Open git status
    OpenGitStatus,
    /// Alt+C — Open git log
    OpenGitLog,
    /// Alt+O — Open outline
    OpenOutline,
    /// Alt+I — Open diagnostics
    OpenDiagnostics,
    /// Alt+B — Open bookmark add dialog
    OpenBookmarkAdd,
    /// Ctrl+P — Open command palette
    OpenCommandPalette,
    /// Alt+Left, Alt+A — Navigate to previous group
    PrevGroup,
    /// Alt+Right, Alt+D — Navigate to next group
    NextGroup,
    /// Alt+Up, Alt+W — Navigate to previous panel in group
    PrevPanel,
    /// Alt+Down, Alt+S — Navigate to next panel in group
    NextPanel,
    /// Alt+1..9 — Go to panel by number
    GoToPanel(usize),
    /// Alt+PageUp — Swap panel left
    SwapLeft,
    /// Alt+PageDown — Swap panel right
    SwapRight,
    /// Alt+Home — Move panel to first position
    MoveFirst,
    /// Alt+End — Move panel to last position
    MoveLast,
    /// Alt+- — Resize panel smaller
    ResizeSmaller,
    /// Alt+= — Resize panel larger
    ResizeLarger,

    // === Unrecognized key — raw event is in Hotkey.raw ===
    Other,
}

/// Normalize a raw KeyEvent into a semantic Hotkey using global keybindings.
///
/// The key should already be translated (Cyrillic → Latin) before calling this.
/// Order: app-level actions first (most specific), then universal F-key actions,
/// then Other.
pub fn normalize(key: KeyEvent, kb: &GlobalKeybindings) -> Hotkey {
    macro_rules! hotkey {
        ($kind:expr) => {
            return Hotkey {
                kind: $kind,
                raw: key,
            }
        };
    }

    // =========================================================================
    // App-level actions (Alt+key combinations, Ctrl+P)
    // =========================================================================

    if matches_binding_or_defaults(&kb.quit, &key, &[(KeyCode::Char('q'), KeyModifiers::ALT)]) {
        hotkey!(HotkeyKind::Quit);
    }

    if matches_binding_or_defaults(
        &kb.new_file_manager,
        &key,
        &[(KeyCode::Char('f'), KeyModifiers::ALT)],
    ) {
        hotkey!(HotkeyKind::NewFileManager);
    }

    if matches_binding_or_defaults(
        &kb.new_terminal,
        &key,
        &[(KeyCode::Char('t'), KeyModifiers::ALT)],
    ) {
        hotkey!(HotkeyKind::NewTerminal);
    }

    if matches_binding_or_defaults(
        &kb.new_editor,
        &key,
        &[(KeyCode::Char('e'), KeyModifiers::ALT)],
    ) {
        hotkey!(HotkeyKind::NewEditor);
    }

    if matches_binding_or_defaults(
        &kb.new_journal,
        &key,
        &[(KeyCode::Char('l'), KeyModifiers::ALT)],
    ) {
        hotkey!(HotkeyKind::NewJournal);
    }

    if matches_binding_or_defaults(
        &kb.open_help,
        &key,
        &[
            (KeyCode::Char('h'), KeyModifiers::ALT),
            (KeyCode::F(1), KeyModifiers::NONE),
        ],
    ) {
        hotkey!(HotkeyKind::OpenHelp);
    }

    if matches_binding_or_defaults(
        &kb.open_preferences,
        &key,
        &[(KeyCode::Char('p'), KeyModifiers::ALT)],
    ) {
        hotkey!(HotkeyKind::OpenPreferences);
    }

    if matches_binding_or_defaults(
        &kb.open_sessions,
        &key,
        &[(KeyCode::Char('/'), KeyModifiers::ALT)],
    ) {
        hotkey!(HotkeyKind::OpenSessions);
    }

    if matches_binding_or_defaults(
        &kb.new_session,
        &key,
        &[(KeyCode::Char('n'), KeyModifiers::ALT)],
    ) {
        hotkey!(HotkeyKind::NewSession);
    }

    if matches_binding_or_defaults(
        &kb.open_git_status,
        &key,
        &[(KeyCode::Char('g'), KeyModifiers::ALT)],
    ) {
        hotkey!(HotkeyKind::OpenGitStatus);
    }

    if matches_binding_or_defaults(
        &kb.open_git_log,
        &key,
        &[(KeyCode::Char('c'), KeyModifiers::ALT)],
    ) {
        hotkey!(HotkeyKind::OpenGitLog);
    }

    if matches_binding_or_defaults(
        &kb.open_outline,
        &key,
        &[(KeyCode::Char('o'), KeyModifiers::ALT)],
    ) {
        hotkey!(HotkeyKind::OpenOutline);
    }

    if matches_binding_or_defaults(
        &kb.open_diagnostics,
        &key,
        &[(KeyCode::Char('i'), KeyModifiers::ALT)],
    ) {
        hotkey!(HotkeyKind::OpenDiagnostics);
    }

    if matches_binding_or_default(
        &kb.open_bookmark_add,
        &key,
        KeyCode::Char('b'),
        KeyModifiers::ALT,
    ) {
        hotkey!(HotkeyKind::OpenBookmarkAdd);
    }

    if matches_binding_or_defaults(
        &kb.open_command_palette,
        &key,
        &[
            (KeyCode::Char('p'), KeyModifiers::CONTROL),
            (
                KeyCode::Char('P'),
                KeyModifiers::CONTROL.union(KeyModifiers::SHIFT),
            ),
        ],
    ) {
        hotkey!(HotkeyKind::OpenCommandPalette);
    }

    // Navigation
    if matches_binding_or_defaults(
        &kb.prev_group,
        &key,
        &[
            (KeyCode::Left, KeyModifiers::ALT),
            (KeyCode::Char('a'), KeyModifiers::ALT),
        ],
    ) {
        hotkey!(HotkeyKind::PrevGroup);
    }

    if matches_binding_or_defaults(
        &kb.next_group,
        &key,
        &[
            (KeyCode::Right, KeyModifiers::ALT),
            (KeyCode::Char('d'), KeyModifiers::ALT),
        ],
    ) {
        hotkey!(HotkeyKind::NextGroup);
    }

    if matches_binding_or_defaults(
        &kb.prev_panel,
        &key,
        &[
            (KeyCode::Up, KeyModifiers::ALT),
            (KeyCode::Char('w'), KeyModifiers::ALT),
        ],
    ) {
        hotkey!(HotkeyKind::PrevPanel);
    }

    if matches_binding_or_defaults(
        &kb.next_panel,
        &key,
        &[
            (KeyCode::Down, KeyModifiers::ALT),
            (KeyCode::Char('s'), KeyModifiers::ALT),
        ],
    ) {
        hotkey!(HotkeyKind::NextPanel);
    }

    // GoToPanel 1-9
    for n in 1..=9u8 {
        let field = match n {
            1 => &kb.goto_panel_1,
            2 => &kb.goto_panel_2,
            3 => &kb.goto_panel_3,
            4 => &kb.goto_panel_4,
            5 => &kb.goto_panel_5,
            6 => &kb.goto_panel_6,
            7 => &kb.goto_panel_7,
            8 => &kb.goto_panel_8,
            9 => &kb.goto_panel_9,
            _ => unreachable!(),
        };
        let digit = char::from(b'0' + n);
        if matches_binding_or_default(field, &key, KeyCode::Char(digit), KeyModifiers::ALT) {
            hotkey!(HotkeyKind::GoToPanel(n as usize));
        }
    }

    // Panel management
    if matches_binding_or_defaults(&kb.swap_left, &key, &[(KeyCode::PageUp, KeyModifiers::ALT)]) {
        hotkey!(HotkeyKind::SwapLeft);
    }

    if matches_binding_or_defaults(
        &kb.swap_right,
        &key,
        &[(KeyCode::PageDown, KeyModifiers::ALT)],
    ) {
        hotkey!(HotkeyKind::SwapRight);
    }

    if matches_binding_or_default(&kb.move_first, &key, KeyCode::Home, KeyModifiers::ALT) {
        hotkey!(HotkeyKind::MoveFirst);
    }

    if matches_binding_or_default(&kb.move_last, &key, KeyCode::End, KeyModifiers::ALT) {
        hotkey!(HotkeyKind::MoveLast);
    }

    if matches_binding_or_default(
        &kb.resize_smaller,
        &key,
        KeyCode::Char('-'),
        KeyModifiers::ALT,
    ) {
        hotkey!(HotkeyKind::ResizeSmaller);
    }

    if matches_binding_or_defaults(
        &kb.resize_larger,
        &key,
        &[
            (KeyCode::Char('='), KeyModifiers::ALT),
            (KeyCode::Char('+'), KeyModifiers::ALT),
        ],
    ) {
        hotkey!(HotkeyKind::ResizeLarger);
    }

    // =========================================================================
    // F-key actions used at app level
    // =========================================================================

    if matches_binding_or_defaults(
        &kb.toggle_menu,
        &key,
        &[
            (KeyCode::Char('m'), KeyModifiers::ALT),
            (KeyCode::F(9), KeyModifiers::NONE),
        ],
    ) {
        hotkey!(HotkeyKind::Menu);
    }

    if matches_binding_or_defaults(
        &kb.close_panel,
        &key,
        &[
            (KeyCode::Char('x'), KeyModifiers::ALT),
            (KeyCode::F(10), KeyModifiers::NONE),
        ],
    ) {
        hotkey!(HotkeyKind::ClosePanel);
    }

    if matches_binding_or_defaults(
        &kb.toggle_stack,
        &key,
        &[
            (KeyCode::Backspace, KeyModifiers::ALT),
            (KeyCode::F(11), KeyModifiers::NONE),
        ],
    ) {
        hotkey!(HotkeyKind::ToggleStack);
    }

    // =========================================================================
    // Unrecognized — pass through
    // =========================================================================
    Hotkey {
        kind: HotkeyKind::Other,
        raw: key,
    }
}
