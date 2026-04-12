//! Global hotkey handling for the application.
//!
//! Uses HotkeyTable to match key events against configured global actions.

use anyhow::Result;
use crossterm::event::KeyEvent;

use super::App;
use crate::state::{ActiveModal, PendingAction};
use crate::PanelExt;
use termide_config::GlobalKeybindings;
use termide_core::HotkeyTable;
use termide_i18n as i18n;

/// Build a HotkeyTable from GlobalKeybindings config.
pub(super) fn build_global_hotkey_table(kb: &GlobalKeybindings) -> HotkeyTable {
    let mut t = HotkeyTable::new();

    // Menu & UI
    t.insert("menu", &kb.toggle_menu);

    // Panel creation
    t.insert("new_file_manager", &kb.new_file_manager);
    t.insert("new_terminal", &kb.new_terminal);
    t.insert("new_editor", &kb.new_editor);
    t.insert("new_journal", &kb.new_journal);
    t.insert("open_help", &kb.open_help);
    t.insert("open_preferences", &kb.open_preferences);
    t.insert("open_sessions", &kb.open_sessions);
    t.insert("new_session", &kb.new_session);
    t.insert("open_git_status", &kb.open_git_status);
    t.insert("open_outline", &kb.open_outline);
    t.insert("open_diagnostics", &kb.open_diagnostics);
    t.insert("open_git_log", &kb.open_git_log);
    t.insert("open_bookmark_add", &kb.open_bookmark_add);
    t.insert("open_command_palette", &kb.open_command_palette);

    // Navigation
    t.insert("prev_group", &kb.prev_group);
    t.insert("next_group", &kb.next_group);
    t.insert("prev_panel", &kb.prev_panel);
    t.insert("next_panel", &kb.next_panel);
    t.insert("goto_panel_1", &kb.goto_panel_1);
    t.insert("goto_panel_2", &kb.goto_panel_2);
    t.insert("goto_panel_3", &kb.goto_panel_3);
    t.insert("goto_panel_4", &kb.goto_panel_4);
    t.insert("goto_panel_5", &kb.goto_panel_5);
    t.insert("goto_panel_6", &kb.goto_panel_6);
    t.insert("goto_panel_7", &kb.goto_panel_7);
    t.insert("goto_panel_8", &kb.goto_panel_8);
    t.insert("goto_panel_9", &kb.goto_panel_9);

    // Panel management
    t.insert("close_panel", &kb.close_panel);
    t.insert("toggle_stack", &kb.toggle_stack);
    t.insert("swap_left", &kb.swap_left);
    t.insert("swap_right", &kb.swap_right);
    t.insert("move_first", &kb.move_first);
    t.insert("move_last", &kb.move_last);
    t.insert("resize_smaller", &kb.resize_smaller);
    t.insert("resize_larger", &kb.resize_larger);

    // Application
    t.insert("quit", &kb.quit);

    t
}

impl App {
    /// Handle app-level actions using HotkeyTable matching.
    ///
    /// Returns `true` if the action was handled, `false` to pass to panel.
    pub(super) fn handle_global_hotkey(&mut self, key: &KeyEvent) -> Result<bool> {
        let table = build_global_hotkey_table(&self.state.config.general.keybindings);

        // Menu
        if table.matches("menu", key) {
            if self.state.ui.menu_open {
                self.state.close_menu();
            } else {
                self.state.open_menu(Some(0));
                self.execute_menu_action()?;
            }
            return Ok(true);
        }

        // Panel creation
        if table.matches("new_file_manager", key) {
            self.handle_new_file_manager()?;
            return Ok(true);
        }
        if table.matches("new_terminal", key) {
            self.handle_new_terminal()?;
            return Ok(true);
        }
        if table.matches("new_editor", key) {
            self.handle_new_editor()?;
            return Ok(true);
        }
        if table.matches("new_journal", key) {
            self.handle_new_journal()?;
            return Ok(true);
        }
        if table.matches("open_help", key) {
            self.handle_new_help()?;
            return Ok(true);
        }
        if table.matches("open_preferences", key) {
            self.open_config_in_editor()?;
            return Ok(true);
        }
        if table.matches("open_sessions", key) {
            self.handle_open_sessions_modal()?;
            return Ok(true);
        }
        if table.matches("new_session", key) {
            self.handle_new_session()?;
            return Ok(true);
        }
        if table.matches("open_git_status", key) {
            self.handle_open_git_status()?;
            return Ok(true);
        }
        if table.matches("open_outline", key) {
            self.handle_open_outline()?;
            return Ok(true);
        }
        if table.matches("open_diagnostics", key) {
            self.handle_open_diagnostics()?;
            return Ok(true);
        }
        if table.matches("open_git_log", key) {
            self.handle_open_git_log()?;
            return Ok(true);
        }
        if table.matches("open_bookmark_add", key) {
            self.handle_add_bookmark()?;
            return Ok(true);
        }
        if table.matches("open_command_palette", key) {
            self.handle_open_command_palette()?;
            return Ok(true);
        }

        // Navigation
        if table.matches("prev_group", key) {
            self.navigate_to_prev_group();
            return Ok(true);
        }
        if table.matches("next_group", key) {
            self.navigate_to_next_group();
            return Ok(true);
        }
        if table.matches("prev_panel", key) {
            self.navigate_to_prev_panel_in_group();
            return Ok(true);
        }
        if table.matches("next_panel", key) {
            self.navigate_to_next_panel_in_group();
            return Ok(true);
        }
        for n in 1..=9usize {
            let action = format!("goto_panel_{}", n);
            if table.matches(&action, key) {
                self.navigate_to_group(n);
                return Ok(true);
            }
        }

        // Panel management
        if table.matches("close_panel", key) {
            self.handle_close_panel_request()?;
            return Ok(true);
        }
        if table.matches("toggle_stack", key) {
            self.toggle_panel_stacking();
            return Ok(true);
        }
        if table.matches("swap_left", key) {
            self.handle_swap_panel_left()?;
            return Ok(true);
        }
        if table.matches("swap_right", key) {
            self.handle_swap_panel_right()?;
            return Ok(true);
        }
        if table.matches("move_first", key) {
            self.move_panel_to_first();
            return Ok(true);
        }
        if table.matches("move_last", key) {
            self.move_panel_to_last();
            return Ok(true);
        }
        if table.matches("resize_smaller", key) {
            self.handle_resize_panel(-1)?;
            return Ok(true);
        }
        if table.matches("resize_larger", key) {
            self.handle_resize_panel(1)?;
            return Ok(true);
        }

        // Application
        if table.matches("quit", key) {
            self.handle_quit_request()?;
            return Ok(true);
        }

        Ok(false)
    }

    /// Handle app action by name (used by command palette).
    pub(super) fn handle_app_action_by_name(&mut self, action: &str) -> Result<bool> {
        match action {
            "menu" => {
                if self.state.ui.menu_open {
                    self.state.close_menu();
                } else {
                    self.state.open_menu(Some(0));
                    self.execute_menu_action()?;
                }
            }
            "new_file_manager" => self.handle_new_file_manager()?,
            "new_terminal" => self.handle_new_terminal()?,
            "new_editor" => self.handle_new_editor()?,
            "new_journal" => self.handle_new_journal()?,
            "open_help" => self.handle_new_help()?,
            "open_preferences" => self.open_config_in_editor()?,
            "open_sessions" => self.handle_open_sessions_modal()?,
            "new_session" => self.handle_new_session()?,
            "open_git_status" => self.handle_open_git_status()?,
            "open_outline" => self.handle_open_outline()?,
            "open_diagnostics" => self.handle_open_diagnostics()?,
            "open_git_log" => self.handle_open_git_log()?,
            "open_bookmark_add" => self.handle_add_bookmark()?,
            "open_command_palette" => self.handle_open_command_palette()?,
            "close_panel" => self.handle_close_panel_request()?,
            "toggle_stack" => self.toggle_panel_stacking(),
            "swap_left" => self.handle_swap_panel_left()?,
            "swap_right" => self.handle_swap_panel_right()?,
            "move_first" => self.move_panel_to_first(),
            "move_last" => self.move_panel_to_last(),
            "resize_smaller" => self.handle_resize_panel(-1)?,
            "resize_larger" => self.handle_resize_panel(1)?,
            "quit" => self.handle_quit_request()?,
            _ => return Ok(false),
        }
        Ok(true)
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
