//! Menu actions and panel creation for the application.
//!
//! Handles menu navigation and creating new panels.

// Note: PanelExt is used for editor save operations that require concrete type access.
#![allow(deprecated)]

use anyhow::Result;
use crossterm::event::KeyCode;
use std::path::PathBuf;

use super::App;
use crate::state::{ActiveModal, PendingAction};
use crate::PanelExt;
use termide_i18n as i18n;
use termide_logger as logger;
use termide_panel_editor::Editor;
use termide_panel_file_manager::FileManager;
use termide_panel_misc::{LogViewerPanel as LogViewer, WelcomePanel as Welcome};
use termide_panel_terminal::Terminal;
use termide_ui_render::menu::MENU_ITEM_COUNT;

impl App {
    /// Handle keyboard event in menu
    pub(super) fn handle_menu_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.state.close_menu();
            }
            KeyCode::Left => {
                self.state.prev_menu_item(MENU_ITEM_COUNT);
            }
            KeyCode::Right => {
                self.state.next_menu_item(MENU_ITEM_COUNT);
            }
            KeyCode::Enter => {
                self.execute_menu_action()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Execute action for selected menu item
    pub(super) fn execute_menu_action(&mut self) -> Result<()> {
        if let Some(menu_index) = self.state.ui.selected_menu_item {
            match menu_index {
                0 => {
                    // Sessions - open sessions modal
                    self.state.close_menu();
                    self.handle_open_sessions_modal()?;
                }
                1 => {
                    // Files - open new file manager panel
                    self.handle_new_file_manager()?;
                    self.state.close_menu();
                }
                2 => {
                    // Terminal - open new terminal panel
                    self.handle_new_terminal()?;
                    self.state.close_menu();
                }
                3 => {
                    // Editor - open new editor panel
                    self.handle_new_editor()?;
                    self.state.close_menu();
                }
                4 => {
                    // Debug - open debug panel
                    self.handle_new_debug()?;
                    self.state.close_menu();
                }
                5 => {
                    // Preferences - show submenu modal
                    self.state.close_menu();
                    let t = i18n::t();
                    let items = vec![
                        t.preferences_themes().to_string(), // 0: Themes
                        t.preferences_edit().to_string(),   // 1: Edit preferences
                    ];
                    let modal = termide_modal::SelectModal::single(t.menu_preferences(), "", items);
                    self.state.set_pending_action(
                        PendingAction::PreferencesMenu,
                        ActiveModal::Select(Box::new(modal)),
                    );
                }
                6 => {
                    // Help - show help
                    self.state.close_menu();
                    self.handle_new_help()?;
                }
                7 => {
                    // Quit - exit
                    self.state.close_menu();
                    if self.has_panels_requiring_confirmation() {
                        let t = i18n::t();
                        let modal =
                            termide_modal::ConfirmModal::new(t.modal_yes(), t.app_quit_confirm());
                        self.state.set_pending_action(
                            PendingAction::QuitApplication,
                            ActiveModal::Confirm(Box::new(modal)),
                        );
                    } else {
                        self.state.quit();
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Open sessions modal to switch between projects
    fn handle_open_sessions_modal(&mut self) -> Result<()> {
        use termide_modal::{SessionItem, SessionsModal};
        use termide_session::{format_relative_time, list_all_sessions};

        let t = i18n::t();

        // Get all sessions
        let sessions = list_all_sessions().unwrap_or_default();

        // Get current project path
        let current_project = std::env::current_dir().unwrap_or_default();

        // Convert to SessionItems
        let items: Vec<SessionItem> = sessions
            .into_iter()
            .map(|info| {
                let is_current = info.project_path == current_project;
                let display_path = info.project_path.display().to_string();
                let relative_time = format_relative_time(info.modified);

                SessionItem {
                    project_path: info.project_path,
                    display_path,
                    relative_time,
                    is_current,
                }
            })
            .collect();

        // Only show modal if there are other sessions
        if items.iter().any(|item| !item.is_current) {
            let modal = SessionsModal::new(t.sessions_title(), items);
            self.state.set_pending_action(
                PendingAction::SwitchSession,
                ActiveModal::Sessions(Box::new(modal)),
            );
        }

        Ok(())
    }

    /// Create new terminal
    pub(super) fn handle_new_terminal(&mut self) -> Result<()> {
        logger::debug("Opening new Terminal panel");
        self.close_welcome_panels();
        // Get working directory from current active panel
        let working_dir = self
            .layout_manager
            .active_panel_mut()
            .and_then(|p| p.get_working_directory());

        // Create new terminal
        let width = self.state.terminal.width;
        let height = self.state.terminal.height;
        let term_height = height.saturating_sub(3);
        let term_width = width.saturating_sub(2);

        if let Ok(terminal_panel) = Terminal::new_with_cwd(term_height, term_width, working_dir) {
            self.add_panel(Box::new(terminal_panel));
            self.auto_save_session();
        }
        Ok(())
    }

    /// Create new file manager
    pub(super) fn handle_new_file_manager(&mut self) -> Result<()> {
        logger::debug("Opening new FileManager panel");
        self.close_welcome_panels();
        // Get working directory from current active panel
        let working_dir = self
            .layout_manager
            .active_panel_mut()
            .and_then(|p| p.get_working_directory())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));

        let fm_panel = FileManager::new_with_path(working_dir);
        self.add_panel(Box::new(fm_panel));
        self.auto_save_session();
        Ok(())
    }

    /// Create new editor
    pub(super) fn handle_new_editor(&mut self) -> Result<()> {
        logger::debug("Opening new Editor panel");
        self.close_welcome_panels();
        let editor_panel = Editor::with_config(self.state.editor_config());
        self.add_panel(Box::new(editor_panel));
        self.auto_save_session();
        Ok(())
    }

    /// Create new debug panel (singleton - only one instance allowed)
    pub(super) fn handle_new_debug(&mut self) -> Result<()> {
        // Check if Debug panel already exists and focus it
        if self.focus_existing_debug_panel() {
            logger::debug("Switching focus to existing Log panel");
            return Ok(());
        }

        // No existing Debug panel found, create new one
        logger::debug("Opening new Log panel");
        self.close_welcome_panels();
        let log_panel = LogViewer::new(self.state.theme);
        self.add_panel(Box::new(log_panel));
        self.auto_save_session();
        Ok(())
    }

    /// Find and focus existing Debug panel if it exists
    /// Returns true if Debug panel was found and focused
    fn focus_existing_debug_panel(&mut self) -> bool {
        // Iterate through all panel groups
        for (group_idx, group) in self.layout_manager.panel_groups.iter_mut().enumerate() {
            // Check each panel in the group
            for (panel_idx, panel) in group.panels().iter().enumerate() {
                if panel.is_log_viewer() {
                    // Found Debug panel - set it as expanded and focus the group
                    group.set_expanded(panel_idx);
                    self.layout_manager.focus = group_idx;
                    return true;
                }
            }
        }

        false
    }

    /// Open or switch to help panel (Welcome)
    pub(super) fn handle_new_help(&mut self) -> Result<()> {
        logger::debug("Opening new Help/Welcome panel");
        let welcome = Welcome::new();
        self.add_panel(Box::new(welcome));
        self.auto_save_session();
        Ok(())
    }

    /// Open config file in editor
    pub(super) fn open_config_in_editor(&mut self) -> Result<()> {
        use termide_config::Config;

        let config_path = match Config::config_file_path() {
            Ok(path) => path,
            Err(e) => {
                logger::warn(format!("Failed to get config path: {}", e));
                self.state
                    .set_error(format!("Failed to get config path: {}", e));
                return Ok(());
            }
        };

        self.close_welcome_panels();

        match Editor::open_file_with_config(config_path, self.state.editor_config()) {
            Ok(editor_panel) => {
                self.add_panel(Box::new(editor_panel));
                self.auto_save_session();
            }
            Err(e) => {
                self.state
                    .set_error(format!("Failed to open config: {}", e));
            }
        }

        Ok(())
    }

    /// Check if any panel requires close confirmation
    pub(super) fn has_panels_requiring_confirmation(&self) -> bool {
        // Check if any panel has unsaved changes or running processes
        for panel in self
            .layout_manager
            .panel_groups
            .iter()
            .flat_map(|g| g.panels().iter())
        {
            if panel.needs_close_confirmation().is_some() {
                return true;
            }
        }

        // Check if there's an active batch file operation
        #[allow(clippy::collapsible_match)]
        if let Some(pending) = &self.state.pending_action {
            match pending {
                PendingAction::BatchFileOperation { .. }
                | PendingAction::ContinueBatchOperation { .. } => {
                    return true;
                }
                _ => {}
            }
        }

        false
    }
}
