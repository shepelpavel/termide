//! Semantic action system for termide.
//!
//! The normalizer converts raw `KeyEvent`s into semantic `Action` variants.
//! Panels, modals, and menus react to intentions, not keycodes.
//! Unrecognized keys are wrapped in `Action::Other(KeyEvent)`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use termide_config::{matches_binding_or_default, matches_binding_or_defaults, GlobalKeybindings};

/// Semantic action recognized from a KeyEvent.
///
/// Panels interpret these contextually:
/// - `Save` → FM: rename, Editor: save file, Git Status: commit
/// - `View` → FM: view file, Editor: search next, Git Status: view file
/// - `Other(key)` → panel-specific parsing (navigation, text input, etc.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // === F-key universal actions (context-dependent per panel) ===
    /// F1 — Help / about
    Help,
    /// F2, Ctrl+S — Save (FM: rename, Editor: save, Git: commit)
    Save,
    /// F3 — View / search next
    View,
    /// F4 — Edit item
    EditItem,
    /// F5 — Copy item
    CopyItem,
    /// F6 — Move item
    MoveItem,
    /// F7 — Create new item
    CreateItem,
    /// Delete, F8 — Delete item / cancel operation
    DeleteItem,
    /// F9, Alt+M — Toggle menu
    Menu,
    /// F10, Alt+X — Close panel
    ClosePanel,
    /// F11, Alt+Backspace — Toggle panel stacking
    ToggleStack,
    /// F12 — Context menu / properties
    ContextMenu,

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

    // === Unrecognized key — passed through to panel's handle_key ===
    Other(KeyEvent),
}

impl Action {
    /// Get the default key event for this action.
    /// Used by panels like Terminal that need to forward F-key actions to the shell.
    pub fn to_default_key(&self) -> Option<KeyEvent> {
        let key = |code: KeyCode| KeyEvent::new(code, KeyModifiers::NONE);
        match self {
            Action::Help => Some(key(KeyCode::F(1))),
            Action::Save => Some(key(KeyCode::F(2))),
            Action::View => Some(key(KeyCode::F(3))),
            Action::EditItem => Some(key(KeyCode::F(4))),
            Action::CopyItem => Some(key(KeyCode::F(5))),
            Action::MoveItem => Some(key(KeyCode::F(6))),
            Action::CreateItem => Some(key(KeyCode::F(7))),
            Action::DeleteItem => Some(key(KeyCode::F(8))),
            Action::Menu => Some(key(KeyCode::F(9))),
            Action::ClosePanel => Some(key(KeyCode::F(10))),
            Action::ToggleStack => Some(key(KeyCode::F(11))),
            Action::ContextMenu => Some(key(KeyCode::F(12))),
            Action::Other(k) => Some(*k),
            _ => None, // App-level actions don't have a default key
        }
    }
}

/// Normalize a raw KeyEvent into a semantic Action using global keybindings.
///
/// The key should already be translated (Cyrillic → Latin) before calling this.
/// Order: app-level actions first (most specific), then universal F-key actions,
/// then non-F-key actions, then Other.
pub fn normalize(key: KeyEvent, kb: &GlobalKeybindings) -> Action {
    // =========================================================================
    // App-level actions (Alt+key combinations, Ctrl+P)
    // These are checked first because Alt+M should be Menu, not a char input.
    // =========================================================================

    if matches_binding_or_defaults(&kb.quit, &key, &[(KeyCode::Char('q'), KeyModifiers::ALT)]) {
        return Action::Quit;
    }

    if matches_binding_or_defaults(
        &kb.new_file_manager,
        &key,
        &[(KeyCode::Char('f'), KeyModifiers::ALT)],
    ) {
        return Action::NewFileManager;
    }

    if matches_binding_or_defaults(
        &kb.new_terminal,
        &key,
        &[(KeyCode::Char('t'), KeyModifiers::ALT)],
    ) {
        return Action::NewTerminal;
    }

    if matches_binding_or_defaults(
        &kb.new_editor,
        &key,
        &[(KeyCode::Char('e'), KeyModifiers::ALT)],
    ) {
        return Action::NewEditor;
    }

    if matches_binding_or_defaults(
        &kb.new_journal,
        &key,
        &[(KeyCode::Char('l'), KeyModifiers::ALT)],
    ) {
        return Action::NewJournal;
    }

    if matches_binding_or_defaults(
        &kb.open_help,
        &key,
        &[
            (KeyCode::Char('h'), KeyModifiers::ALT),
            (KeyCode::F(1), KeyModifiers::NONE),
        ],
    ) {
        return Action::OpenHelp;
    }

    if matches_binding_or_defaults(
        &kb.open_preferences,
        &key,
        &[(KeyCode::Char('p'), KeyModifiers::ALT)],
    ) {
        return Action::OpenPreferences;
    }

    if matches_binding_or_defaults(
        &kb.open_sessions,
        &key,
        &[(KeyCode::Char('/'), KeyModifiers::ALT)],
    ) {
        return Action::OpenSessions;
    }

    if matches_binding_or_defaults(
        &kb.new_session,
        &key,
        &[(KeyCode::Char('n'), KeyModifiers::ALT)],
    ) {
        return Action::NewSession;
    }

    if matches_binding_or_defaults(
        &kb.open_git_status,
        &key,
        &[(KeyCode::Char('g'), KeyModifiers::ALT)],
    ) {
        return Action::OpenGitStatus;
    }

    if matches_binding_or_defaults(
        &kb.open_git_log,
        &key,
        &[(KeyCode::Char('c'), KeyModifiers::ALT)],
    ) {
        return Action::OpenGitLog;
    }

    if matches_binding_or_defaults(
        &kb.open_outline,
        &key,
        &[(KeyCode::Char('o'), KeyModifiers::ALT)],
    ) {
        return Action::OpenOutline;
    }

    if matches_binding_or_defaults(
        &kb.open_diagnostics,
        &key,
        &[(KeyCode::Char('i'), KeyModifiers::ALT)],
    ) {
        return Action::OpenDiagnostics;
    }

    if matches_binding_or_default(
        &kb.open_bookmark_add,
        &key,
        KeyCode::Char('b'),
        KeyModifiers::ALT,
    ) {
        return Action::OpenBookmarkAdd;
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
        return Action::OpenCommandPalette;
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
        return Action::PrevGroup;
    }

    if matches_binding_or_defaults(
        &kb.next_group,
        &key,
        &[
            (KeyCode::Right, KeyModifiers::ALT),
            (KeyCode::Char('d'), KeyModifiers::ALT),
        ],
    ) {
        return Action::NextGroup;
    }

    if matches_binding_or_defaults(
        &kb.prev_panel,
        &key,
        &[
            (KeyCode::Up, KeyModifiers::ALT),
            (KeyCode::Char('w'), KeyModifiers::ALT),
        ],
    ) {
        return Action::PrevPanel;
    }

    if matches_binding_or_defaults(
        &kb.next_panel,
        &key,
        &[
            (KeyCode::Down, KeyModifiers::ALT),
            (KeyCode::Char('s'), KeyModifiers::ALT),
        ],
    ) {
        return Action::NextPanel;
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
            return Action::GoToPanel(n as usize);
        }
    }

    // Panel management
    if matches_binding_or_defaults(&kb.swap_left, &key, &[(KeyCode::PageUp, KeyModifiers::ALT)]) {
        return Action::SwapLeft;
    }

    if matches_binding_or_defaults(
        &kb.swap_right,
        &key,
        &[(KeyCode::PageDown, KeyModifiers::ALT)],
    ) {
        return Action::SwapRight;
    }

    if matches_binding_or_default(&kb.move_first, &key, KeyCode::Home, KeyModifiers::ALT) {
        return Action::MoveFirst;
    }

    if matches_binding_or_default(&kb.move_last, &key, KeyCode::End, KeyModifiers::ALT) {
        return Action::MoveLast;
    }

    if matches_binding_or_default(
        &kb.resize_smaller,
        &key,
        KeyCode::Char('-'),
        KeyModifiers::ALT,
    ) {
        return Action::ResizeSmaller;
    }

    if matches_binding_or_defaults(
        &kb.resize_larger,
        &key,
        &[
            (KeyCode::Char('='), KeyModifiers::ALT),
            (KeyCode::Char('+'), KeyModifiers::ALT),
        ],
    ) {
        return Action::ResizeLarger;
    }

    // =========================================================================
    // F-key universal actions
    // =========================================================================

    if matches_binding_or_default(&kb.help, &key, KeyCode::F(1), KeyModifiers::NONE) {
        return Action::Help;
    }

    // Save: only F2 by default.
    // Ctrl+S stays as Other so editor/terminal can use it for panel-specific save.
    if matches_binding_or_default(&kb.save, &key, KeyCode::F(2), KeyModifiers::NONE) {
        return Action::Save;
    }

    if matches_binding_or_default(&kb.view, &key, KeyCode::F(3), KeyModifiers::NONE) {
        return Action::View;
    }

    if matches_binding_or_default(&kb.edit_item, &key, KeyCode::F(4), KeyModifiers::NONE) {
        return Action::EditItem;
    }

    if matches_binding_or_default(&kb.copy_item, &key, KeyCode::F(5), KeyModifiers::NONE) {
        return Action::CopyItem;
    }

    if matches_binding_or_default(&kb.move_item, &key, KeyCode::F(6), KeyModifiers::NONE) {
        return Action::MoveItem;
    }

    if matches_binding_or_default(&kb.create_item, &key, KeyCode::F(7), KeyModifiers::NONE) {
        return Action::CreateItem;
    }

    // DeleteItem: only F8 by default.
    // Delete key stays as Other so editor/terminal can use it for char deletion.
    // Panels that want Delete=DeleteItem handle it in their own handle_key.
    if matches_binding_or_default(&kb.delete_item, &key, KeyCode::F(8), KeyModifiers::NONE) {
        return Action::DeleteItem;
    }

    if matches_binding_or_defaults(
        &kb.toggle_menu,
        &key,
        &[
            (KeyCode::Char('m'), KeyModifiers::ALT),
            (KeyCode::F(9), KeyModifiers::NONE),
        ],
    ) {
        return Action::Menu;
    }

    if matches_binding_or_defaults(
        &kb.close_panel,
        &key,
        &[
            (KeyCode::Char('x'), KeyModifiers::ALT),
            (KeyCode::F(10), KeyModifiers::NONE),
        ],
    ) {
        return Action::ClosePanel;
    }

    if matches_binding_or_defaults(
        &kb.toggle_stack,
        &key,
        &[
            (KeyCode::Backspace, KeyModifiers::ALT),
            (KeyCode::F(11), KeyModifiers::NONE),
        ],
    ) {
        return Action::ToggleStack;
    }

    if matches_binding_or_default(&kb.context_menu, &key, KeyCode::F(12), KeyModifiers::NONE) {
        return Action::ContextMenu;
    }

    // =========================================================================
    // Unrecognized — pass through
    // Non-F-key actions (Esc, Ctrl+F, Ctrl+R, Backspace) are NOT normalized
    // because they have panel-specific meanings. Panels handle them in handle_key.
    // =========================================================================
    Action::Other(key)
}
