//! Global hotkey handling for the application.
//!
//! Uses the HotkeyProcessor trait to handle Alt+key combinations
//! for navigation, panel management, and quick actions.

use anyhow::Result;
use crossterm::event::KeyEvent;

use termide_app_event::{HotkeyAction, HotkeyProcessor};

use super::App;
use crate::state::{ActiveModal, PendingAction};
use crate::PanelExt;
use termide_i18n as i18n;

impl App {
    /// Handle app-level actions from the normalizer.
    ///
    /// Returns `true` if the action was handled, `false` to pass to panel.
    pub(super) fn handle_app_action(&mut self, action: &termide_core::Action) -> Result<bool> {
        use termide_core::Action;

        match action {
            // Menu
            Action::Menu => {
                if self.state.ui.menu_open {
                    self.state.close_menu();
                } else {
                    self.state.open_menu(Some(0));
                    self.execute_menu_action()?;
                }
            }

            // Panel creation
            Action::NewFileManager => self.handle_new_file_manager()?,
            Action::NewTerminal => self.handle_new_terminal()?,
            Action::NewEditor => self.handle_new_editor()?,
            Action::NewJournal => self.handle_new_journal()?,
            Action::OpenHelp => self.handle_new_help()?,
            Action::OpenPreferences => self.open_config_in_editor()?,
            Action::OpenSessions => self.handle_open_sessions_modal()?,
            Action::NewSession => self.handle_new_session()?,
            Action::OpenGitStatus => self.handle_open_git_status()?,
            Action::OpenOutline => self.handle_open_outline()?,
            Action::OpenDiagnostics => self.handle_open_diagnostics()?,
            Action::OpenGitLog => self.handle_open_git_log()?,
            Action::OpenBookmarkAdd => self.handle_add_bookmark()?,
            Action::OpenCommandPalette => self.handle_open_command_palette()?,

            // Navigation
            Action::PrevGroup => self.navigate_to_prev_group(),
            Action::NextGroup => self.navigate_to_next_group(),
            Action::PrevPanel => self.navigate_to_prev_panel_in_group(),
            Action::NextPanel => self.navigate_to_next_panel_in_group(),
            Action::GoToPanel(n) => self.navigate_to_group(*n),

            // Panel management
            Action::ClosePanel => self.handle_close_panel_request()?,
            Action::ToggleStack => self.toggle_panel_stacking(),
            Action::SwapLeft => self.handle_swap_panel_left()?,
            Action::SwapRight => self.handle_swap_panel_right()?,
            Action::MoveFirst => self.move_panel_to_first(),
            Action::MoveLast => self.move_panel_to_last(),
            Action::ResizeSmaller => self.handle_resize_panel(-1)?,
            Action::ResizeLarger => self.handle_resize_panel(1)?,

            // Application
            Action::Quit => self.handle_quit_request()?,

            // Not an app-level action
            _ => return Ok(false),
        }
        Ok(true)
    }

    // Keep legacy method for now (used by command palette)
    /// Handle global hotkeys (Alt+key combinations) — legacy path
    #[allow(dead_code)]
    pub(super) fn handle_global_hotkeys(&mut self, key: KeyEvent) -> Result<Option<()>> {
        if let Some(action) = self.hotkey_processor.process_hotkey(&key) {
            self.execute_hotkey_action(action)?;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// Execute a hotkey action — legacy path for command palette
    pub(in crate::app) fn execute_hotkey_action(&mut self, action: HotkeyAction) -> Result<()> {
        // Convert legacy HotkeyAction to new Action and dispatch
        let new_action = match action {
            HotkeyAction::ToggleMenu => termide_core::Action::Menu,
            HotkeyAction::NewFileManager => termide_core::Action::NewFileManager,
            HotkeyAction::NewTerminal => termide_core::Action::NewTerminal,
            HotkeyAction::NewEditor => termide_core::Action::NewEditor,
            HotkeyAction::NewJournal => termide_core::Action::NewJournal,
            HotkeyAction::OpenHelp => termide_core::Action::OpenHelp,
            HotkeyAction::OpenPreferences => termide_core::Action::OpenPreferences,
            HotkeyAction::OpenSessions => termide_core::Action::OpenSessions,
            HotkeyAction::NewSession => termide_core::Action::NewSession,
            HotkeyAction::OpenGitStatus => termide_core::Action::OpenGitStatus,
            HotkeyAction::OpenOutline => termide_core::Action::OpenOutline,
            HotkeyAction::OpenDiagnostics => termide_core::Action::OpenDiagnostics,
            HotkeyAction::OpenGitLog => termide_core::Action::OpenGitLog,
            HotkeyAction::OpenDirectorySwitcher => termide_core::Action::OpenSessions,
            HotkeyAction::OpenBookmarkAdd => termide_core::Action::OpenBookmarkAdd,
            HotkeyAction::OpenCommandPalette => termide_core::Action::OpenCommandPalette,
            HotkeyAction::PrevGroup => termide_core::Action::PrevGroup,
            HotkeyAction::NextGroup => termide_core::Action::NextGroup,
            HotkeyAction::PrevInGroup => termide_core::Action::PrevPanel,
            HotkeyAction::NextInGroup => termide_core::Action::NextPanel,
            HotkeyAction::GoToPanel(n) => termide_core::Action::GoToPanel(n),
            HotkeyAction::ClosePanel => termide_core::Action::ClosePanel,
            HotkeyAction::ToggleStacking => termide_core::Action::ToggleStack,
            HotkeyAction::SwapPanelLeft => termide_core::Action::SwapLeft,
            HotkeyAction::SwapPanelRight => termide_core::Action::SwapRight,
            HotkeyAction::MoveToFirst => termide_core::Action::MoveFirst,
            HotkeyAction::MoveToLast => termide_core::Action::MoveLast,
            HotkeyAction::ResizePanel(d) => {
                if d > 0 {
                    termide_core::Action::ResizeLarger
                } else {
                    termide_core::Action::ResizeSmaller
                }
            }
            HotkeyAction::RequestQuit => termide_core::Action::Quit,
        };
        self.handle_app_action(&new_action)?;
        Ok(())
    }

    /// Handle quit request with confirmation if needed
    pub(super) fn handle_quit_request(&mut self) -> Result<()> {
        // Always save session before quit
        self.auto_save_session();

        if self.has_panels_requiring_confirmation() {
            let t = i18n::t();
            let modal = termide_modal::ConfirmModal::new(t.app_quit_title(), t.app_quit_confirm());
            self.state.set_pending_action(
                PendingAction::QuitApplication,
                ActiveModal::Confirm(Box::new(modal)),
            );
        } else {
            self.state.quit();
        }
        Ok(())
    }

    /// Check if session should be saved and save if needed
    fn check_and_save_session(&mut self) {
        if self.state.should_save_session() {
            self.auto_save_session();
            self.state.update_last_session_save();
        }
    }

    /// Close completion popup on active editor (if any) before focus change
    fn close_completion_popup_before_focus_change(&mut self) {
        if let Some(panel) = self.layout_manager.active_panel_mut() {
            if let Some(editor) = panel.as_editor_mut() {
                editor.cancel_completion();
            }
        }
    }

    /// Navigate to previous group with session save
    fn navigate_to_prev_group(&mut self) {
        self.close_completion_popup_before_focus_change();
        self.layout_manager.prev_group();
        self.notify_outline_file_opened();
        self.check_and_save_session();
    }

    /// Navigate to next group with session save
    fn navigate_to_next_group(&mut self) {
        self.close_completion_popup_before_focus_change();
        self.layout_manager.next_group();
        self.notify_outline_file_opened();
        self.check_and_save_session();
    }

    /// Navigate to previous panel in group with session save
    fn navigate_to_prev_panel_in_group(&mut self) {
        self.close_completion_popup_before_focus_change();
        self.layout_manager.prev_panel_in_group();
        self.notify_outline_file_opened();
        self.check_and_save_session();
    }

    /// Navigate to next panel in group with session save
    fn navigate_to_next_panel_in_group(&mut self) {
        self.close_completion_popup_before_focus_change();
        self.layout_manager.next_panel_in_group();
        self.notify_outline_file_opened();
        self.check_and_save_session();
    }

    /// Navigate to specific group by number (1-indexed)
    fn navigate_to_group(&mut self, group_num: usize) {
        self.close_completion_popup_before_focus_change();
        // Convert from 1-indexed (user-facing) to 0-indexed (internal)
        let index = group_num.saturating_sub(1);
        self.layout_manager.set_focus(index);
        self.notify_outline_file_opened();
        self.check_and_save_session();
    }

    /// Toggle panel stacking mode
    fn toggle_panel_stacking(&mut self) {
        let terminal_width = self.state.terminal.width;
        if let Err(e) = self.layout_manager.toggle_panel_stacking(terminal_width) {
            self.state
                .set_error(format!("Cannot toggle stacking: {}", e));
        } else {
            self.auto_save_session();
        }
    }

    /// Move panel to first group
    fn move_panel_to_first(&mut self) {
        let terminal_width = self.state.terminal.width;
        if let Err(e) = self
            .layout_manager
            .move_panel_to_first_group(terminal_width)
        {
            self.state.set_error(format!("Cannot move panel: {}", e));
        } else {
            self.auto_save_session();
        }
    }

    /// Move panel to last group
    fn move_panel_to_last(&mut self) {
        let terminal_width = self.state.terminal.width;
        if let Err(e) = self.layout_manager.move_panel_to_last_group(terminal_width) {
            self.state.set_error(format!("Cannot move panel: {}", e));
        } else {
            self.auto_save_session();
        }
    }
}
