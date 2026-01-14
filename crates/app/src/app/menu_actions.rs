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
use termide_config::Config;
use termide_i18n as i18n;
use termide_logger as logger;
use termide_panel_editor::Editor;
use termide_panel_file_manager::FileManager;
use termide_panel_misc::{LogViewerPanel as LogViewer, WelcomePanel as Welcome};
use termide_panel_terminal::Terminal;
use termide_theme::Theme;
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
                    // Sessions - open submenu dropdown (keep menu open)
                    self.state.open_sessions_submenu();
                }
                1 => {
                    // Tools - open submenu dropdown (keep menu open)
                    self.state.open_tools_submenu();
                }
                2 => {
                    // Options - open submenu dropdown (keep menu open)
                    self.state.open_submenu();
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Open sessions modal to switch between projects
    pub(super) fn handle_open_sessions_modal(&mut self) -> Result<()> {
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
            // Find index of current session to position cursor there
            let current_idx = items.iter().position(|item| item.is_current).unwrap_or(0);
            let modal = SessionsModal::new(t.sessions_title(), items).with_cursor(current_idx);
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

        // Get working directory from current active panel (e.g., FileManager)
        let initial_directory = self
            .layout_manager
            .active_panel_mut()
            .and_then(|p| p.get_working_directory());

        let mut config = self.state.editor_config();
        config.initial_directory = initial_directory;

        let editor_panel = Editor::with_config(config);
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

    // =========================================================================
    // Submenu handling
    // =========================================================================

    /// Handle keyboard event in submenu (Options dropdown)
    pub(super) fn handle_submenu_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        // If nested submenu is open, delegate to nested handler
        if self.state.ui.nested_submenu_open {
            return self.handle_nested_submenu_key(key);
        }

        use termide_ui_render::OPTIONS_SUBMENU_ITEM_COUNT;

        match key.code {
            KeyCode::Esc | KeyCode::Left => {
                // Close submenu, return to menu
                self.state.close_submenu();
            }
            KeyCode::Up => {
                if self.state.ui.selected_submenu_item > 0 {
                    self.state.ui.selected_submenu_item -= 1;
                } else {
                    self.state.ui.selected_submenu_item = OPTIONS_SUBMENU_ITEM_COUNT - 1;
                }
            }
            KeyCode::Down => {
                self.state.ui.selected_submenu_item =
                    (self.state.ui.selected_submenu_item + 1) % OPTIONS_SUBMENU_ITEM_COUNT;
            }
            KeyCode::Right | KeyCode::Enter => {
                self.execute_submenu_action()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Execute action for selected Options submenu item
    fn execute_submenu_action(&mut self) -> Result<()> {
        match self.state.ui.selected_submenu_item {
            0 => {
                // Themes - open nested submenu with live preview
                let theme_names = Theme::all_theme_names();
                let current_idx = theme_names
                    .iter()
                    .position(|n| n == self.state.theme.name)
                    .unwrap_or(0);
                // Save current theme for restoration on cancel
                self.state.ui.theme_preview_original = Some(self.state.theme.name.to_string());
                self.state.open_nested_submenu(current_idx);
            }
            1 => {
                // Edit preferences - close menu and open config
                self.state.close_menu();
                self.open_config_in_editor()?;
            }
            2 => {
                // Help - show help
                self.state.close_menu();
                self.handle_new_help()?;
            }
            3 => {
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
        Ok(())
    }

    /// Handle keyboard event in nested submenu (Themes list)
    fn handle_nested_submenu_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        let theme_names = Theme::all_theme_names();
        let theme_count = theme_names.len();

        match key.code {
            KeyCode::Esc | KeyCode::Left => {
                // Restore original theme on cancel
                if let Some(original_name) = self.state.ui.theme_preview_original.take() {
                    self.state.theme = Theme::get_by_name(&original_name);
                }
                // Close nested submenu, return to parent
                self.state.close_nested_submenu();
            }
            KeyCode::Up => {
                if self.state.ui.selected_nested_item > 0 {
                    self.state.ui.selected_nested_item -= 1;
                } else {
                    self.state.ui.selected_nested_item = theme_count.saturating_sub(1);
                }
                // Live preview: apply theme on cursor move
                if let Some(name) = theme_names.get(self.state.ui.selected_nested_item) {
                    self.state.theme = Theme::get_by_name(name);
                }
            }
            KeyCode::Down => {
                if theme_count > 0 {
                    self.state.ui.selected_nested_item =
                        (self.state.ui.selected_nested_item + 1) % theme_count;
                }
                // Live preview: apply theme on cursor move
                if let Some(name) = theme_names.get(self.state.ui.selected_nested_item) {
                    self.state.theme = Theme::get_by_name(name);
                }
            }
            KeyCode::Enter => {
                // Clear preview state - theme is confirmed
                self.state.ui.theme_preview_original = None;
                // Apply selected theme and save preference
                if let Some(name) = theme_names.get(self.state.ui.selected_nested_item) {
                    self.apply_theme(name)?;
                }
                // Close all menus
                self.state.close_menu();
            }
            _ => {}
        }
        Ok(())
    }

    // =========================================================================
    // Sessions submenu handling
    // =========================================================================

    /// Handle keyboard event in Sessions submenu
    pub(super) fn handle_sessions_submenu_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use termide_ui_render::SESSIONS_SUBMENU_ITEM_COUNT;

        match key.code {
            KeyCode::Esc | KeyCode::Left => {
                // Close submenu, return to menu
                self.state.close_sessions_submenu();
            }
            KeyCode::Up => {
                if self.state.ui.selected_sessions_item > 0 {
                    self.state.ui.selected_sessions_item -= 1;
                } else {
                    self.state.ui.selected_sessions_item = SESSIONS_SUBMENU_ITEM_COUNT - 1;
                }
            }
            KeyCode::Down => {
                self.state.ui.selected_sessions_item =
                    (self.state.ui.selected_sessions_item + 1) % SESSIONS_SUBMENU_ITEM_COUNT;
            }
            KeyCode::Right | KeyCode::Enter => {
                self.execute_sessions_submenu_action()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Execute action for selected Sessions submenu item
    pub(super) fn execute_sessions_submenu_action(&mut self) -> Result<()> {
        match self.state.ui.selected_sessions_item {
            0 => {
                // New session - open directory picker
                self.state.close_menu();
                self.handle_new_session()?;
            }
            1 => {
                // Switch session - open sessions modal
                self.state.close_menu();
                self.handle_open_sessions_modal()?;
            }
            2 => {
                // Change root path - open directory picker
                self.state.close_menu();
                self.handle_change_root_path()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Open directory picker for creating new session
    fn handle_new_session(&mut self) -> Result<()> {
        use termide_modal::DirectoryPickerModal;

        let t = i18n::t();
        // Get current project root as starting directory
        let initial_dir = self.project_root.clone();

        let modal = DirectoryPickerModal::new(
            initial_dir,
            t.sessions_new().to_string(),
            t.directory_picker_create().to_string(),
        );
        self.state.set_pending_action(
            PendingAction::NewSession,
            ActiveModal::DirectoryPicker(Box::new(modal)),
        );

        Ok(())
    }

    /// Open directory picker for changing root path of current session
    fn handle_change_root_path(&mut self) -> Result<()> {
        use termide_modal::DirectoryPickerModal;

        let t = i18n::t();
        // Get current project root as starting directory
        let initial_dir = self.project_root.clone();

        let modal = DirectoryPickerModal::new(
            initial_dir,
            t.sessions_change_root().to_string(),
            t.directory_picker_move().to_string(),
        );
        self.state.set_pending_action(
            PendingAction::ChangeRootPath,
            ActiveModal::DirectoryPicker(Box::new(modal)),
        );

        Ok(())
    }

    /// Apply theme by name and save preference
    pub(super) fn apply_theme(&mut self, theme_name: &str) -> Result<()> {
        let new_theme = Theme::get_by_name(theme_name);
        self.state.theme = new_theme;

        let t = i18n::t();
        self.state.set_info(t.theme_changed(theme_name));

        // Save preference to config file
        if let Err(e) = self.save_theme_preference(theme_name) {
            logger::warn(format!("Failed to save theme preference: {}", e));
        }

        Ok(())
    }

    /// Save theme preference to config file
    fn save_theme_preference(&self, theme_name: &str) -> Result<()> {
        let mut config = Config::load()?;
        config.general.theme = theme_name.to_string();
        config.save()?;
        Ok(())
    }

    // =========================================================================
    // Tools submenu handling
    // =========================================================================

    /// Handle keyboard event in Tools submenu
    pub(super) fn handle_tools_submenu_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use termide_ui_render::TOOLS_SUBMENU_ITEM_COUNT;

        match key.code {
            KeyCode::Esc | KeyCode::Left => {
                // Close submenu, return to menu
                self.state.close_tools_submenu();
            }
            KeyCode::Up => {
                if self.state.ui.selected_tools_item > 0 {
                    self.state.ui.selected_tools_item -= 1;
                } else {
                    self.state.ui.selected_tools_item = TOOLS_SUBMENU_ITEM_COUNT - 1;
                }
            }
            KeyCode::Down => {
                self.state.ui.selected_tools_item =
                    (self.state.ui.selected_tools_item + 1) % TOOLS_SUBMENU_ITEM_COUNT;
            }
            KeyCode::Right | KeyCode::Enter => {
                self.execute_tools_submenu_action()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Execute action for selected Tools submenu item
    pub(super) fn execute_tools_submenu_action(&mut self) -> Result<()> {
        match self.state.ui.selected_tools_item {
            0 => {
                // Files - open new file manager panel
                self.state.close_menu();
                self.handle_new_file_manager()?;
            }
            1 => {
                // Terminal - open new terminal panel
                self.state.close_menu();
                self.handle_new_terminal()?;
            }
            2 => {
                // Editor - open new editor panel
                self.state.close_menu();
                self.handle_new_editor()?;
            }
            3 => {
                // Git Status - open Git Status panel
                self.state.close_menu();
                self.handle_open_git_status()?;
            }
            4 => {
                // Git Log - open Git Log panel
                self.state.close_menu();
                self.handle_open_git_log()?;
            }
            5 => {
                // Journal - open debug panel
                self.state.close_menu();
                self.handle_new_debug()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Open Git Status panel
    pub(super) fn handle_open_git_status(&mut self) -> Result<()> {
        logger::debug("Opening Git Status panel");
        self.close_welcome_panels();

        if !self.find_and_focus_panel_by_name("git_status") {
            let paths = self.collect_panel_paths();
            let git_status_panel = termide_panel_git_status::GitStatusPanel::new(&paths);
            self.add_panel(Box::new(git_status_panel));
        }
        self.auto_save_session();
        Ok(())
    }

    /// Open Git Log panel
    fn handle_open_git_log(&mut self) -> Result<()> {
        logger::debug("Opening Git Log panel");
        self.close_welcome_panels();

        let paths = self.collect_panel_paths();
        let git_log_panel = termide_panel_git_log::GitLogPanel::new(&paths);
        self.add_panel(Box::new(git_log_panel));
        self.auto_save_session();
        Ok(())
    }
}
