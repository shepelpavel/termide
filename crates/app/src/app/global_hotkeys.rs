//! Global hotkey handling for the application.
//!
//! Uses the HotkeyProcessor trait to handle Alt+key combinations
//! for navigation, panel management, and quick actions.

use anyhow::Result;

use termide_app_event::HotkeyAction;

use super::App;
use crate::state::{ActiveModal, PendingAction};
use crate::PanelExt;
use termide_i18n as i18n;

impl App {
    /// Handle app-level actions from the normalizer.
    ///
    /// Returns `true` if the action was handled, `false` to pass to panel.
    pub(super) fn handle_app_action(&mut self, kind: &termide_core::HotkeyKind) -> Result<bool> {
        use termide_core::HotkeyKind;

        match kind {
            // Menu
            HotkeyKind::Menu => {
                if self.state.ui.menu_open {
                    self.state.close_menu();
                } else {
                    self.state.open_menu(Some(0));
                    self.execute_menu_action()?;
                }
            }

            // Panel creation
            HotkeyKind::NewFileManager => self.handle_new_file_manager()?,
            HotkeyKind::NewTerminal => self.handle_new_terminal()?,
            HotkeyKind::NewEditor => self.handle_new_editor()?,
            HotkeyKind::NewJournal => self.handle_new_journal()?,
            HotkeyKind::OpenHelp => self.handle_new_help()?,
            HotkeyKind::OpenPreferences => self.open_config_in_editor()?,
            HotkeyKind::OpenSessions => self.handle_open_sessions_modal()?,
            HotkeyKind::NewSession => self.handle_new_session()?,
            HotkeyKind::OpenGitStatus => self.handle_open_git_status()?,
            HotkeyKind::OpenOutline => self.handle_open_outline()?,
            HotkeyKind::OpenDiagnostics => self.handle_open_diagnostics()?,
            HotkeyKind::OpenGitLog => self.handle_open_git_log()?,
            HotkeyKind::OpenBookmarkAdd => self.handle_add_bookmark()?,
            HotkeyKind::OpenCommandPalette => self.handle_open_command_palette()?,

            // Navigation
            HotkeyKind::PrevGroup => self.navigate_to_prev_group(),
            HotkeyKind::NextGroup => self.navigate_to_next_group(),
            HotkeyKind::PrevPanel => self.navigate_to_prev_panel_in_group(),
            HotkeyKind::NextPanel => self.navigate_to_next_panel_in_group(),
            HotkeyKind::GoToPanel(n) => self.navigate_to_group(*n),

            // Panel management
            HotkeyKind::ClosePanel => self.handle_close_panel_request()?,
            HotkeyKind::ToggleStack => self.toggle_panel_stacking(),
            HotkeyKind::SwapLeft => self.handle_swap_panel_left()?,
            HotkeyKind::SwapRight => self.handle_swap_panel_right()?,
            HotkeyKind::MoveFirst => self.move_panel_to_first(),
            HotkeyKind::MoveLast => self.move_panel_to_last(),
            HotkeyKind::ResizeSmaller => self.handle_resize_panel(-1)?,
            HotkeyKind::ResizeLarger => self.handle_resize_panel(1)?,

            // Application
            HotkeyKind::Quit => self.handle_quit_request()?,

            // Not an app-level action
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Execute a hotkey action — legacy adapter for command palette
    pub(in crate::app) fn execute_hotkey_action(&mut self, action: HotkeyAction) -> Result<()> {
        // Convert legacy HotkeyAction to new HotkeyKind and dispatch
        let new_kind = match action {
            HotkeyAction::ToggleMenu => termide_core::HotkeyKind::Menu,
            HotkeyAction::NewFileManager => termide_core::HotkeyKind::NewFileManager,
            HotkeyAction::NewTerminal => termide_core::HotkeyKind::NewTerminal,
            HotkeyAction::NewEditor => termide_core::HotkeyKind::NewEditor,
            HotkeyAction::NewJournal => termide_core::HotkeyKind::NewJournal,
            HotkeyAction::OpenHelp => termide_core::HotkeyKind::OpenHelp,
            HotkeyAction::OpenPreferences => termide_core::HotkeyKind::OpenPreferences,
            HotkeyAction::OpenSessions => termide_core::HotkeyKind::OpenSessions,
            HotkeyAction::NewSession => termide_core::HotkeyKind::NewSession,
            HotkeyAction::OpenGitStatus => termide_core::HotkeyKind::OpenGitStatus,
            HotkeyAction::OpenOutline => termide_core::HotkeyKind::OpenOutline,
            HotkeyAction::OpenDiagnostics => termide_core::HotkeyKind::OpenDiagnostics,
            HotkeyAction::OpenGitLog => termide_core::HotkeyKind::OpenGitLog,
            HotkeyAction::OpenDirectorySwitcher => termide_core::HotkeyKind::OpenSessions,
            HotkeyAction::OpenBookmarkAdd => termide_core::HotkeyKind::OpenBookmarkAdd,
            HotkeyAction::OpenCommandPalette => termide_core::HotkeyKind::OpenCommandPalette,
            HotkeyAction::PrevGroup => termide_core::HotkeyKind::PrevGroup,
            HotkeyAction::NextGroup => termide_core::HotkeyKind::NextGroup,
            HotkeyAction::PrevInGroup => termide_core::HotkeyKind::PrevPanel,
            HotkeyAction::NextInGroup => termide_core::HotkeyKind::NextPanel,
            HotkeyAction::GoToPanel(n) => termide_core::HotkeyKind::GoToPanel(n),
            HotkeyAction::ClosePanel => termide_core::HotkeyKind::ClosePanel,
            HotkeyAction::ToggleStacking => termide_core::HotkeyKind::ToggleStack,
            HotkeyAction::SwapPanelLeft => termide_core::HotkeyKind::SwapLeft,
            HotkeyAction::SwapPanelRight => termide_core::HotkeyKind::SwapRight,
            HotkeyAction::MoveToFirst => termide_core::HotkeyKind::MoveFirst,
            HotkeyAction::MoveToLast => termide_core::HotkeyKind::MoveLast,
            HotkeyAction::ResizePanel(d) => {
                if d > 0 {
                    termide_core::HotkeyKind::ResizeLarger
                } else {
                    termide_core::HotkeyKind::ResizeSmaller
                }
            }
            HotkeyAction::RequestQuit => termide_core::HotkeyKind::Quit,
        };
        self.handle_app_action(&new_kind)?;
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
