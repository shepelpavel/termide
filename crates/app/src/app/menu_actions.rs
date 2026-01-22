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
use termide_app_core::Panel;
use termide_config::Config;
use termide_i18n as i18n;
use termide_logger as logger;
use termide_panel_editor::Editor;
use termide_panel_file_manager::FileManager;
use termide_panel_misc::{HelpPanel as Help, JournalPanel as Journal};
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
                    // Actions - open submenu dropdown (keep menu open)
                    self.state.open_actions_submenu();
                }
                3 => {
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

    /// Open directory switcher modal
    pub(super) fn handle_open_directory_switcher(&mut self) -> Result<()> {
        use termide_modal::{DirectoryItem, DirectorySwitcherModal};

        let t = i18n::t();

        // Check if active panel supports directory switching (Terminal or FileManager)
        let panel_supported = self
            .layout_manager
            .active_panel_mut()
            .map(|p| p.as_terminal_mut().is_some() || p.as_file_manager_mut().is_some())
            .unwrap_or(false);

        if !panel_supported {
            self.state
                .set_info(t.directory_switcher_unsupported().to_string());
            return Ok(());
        }

        // For terminal panels, check if there's a running process (cd won't work)
        let has_running_process = self
            .layout_manager
            .active_panel_mut()
            .and_then(|p| p.as_terminal_mut())
            .map(|t| t.has_running_processes())
            .unwrap_or(false);

        if has_running_process {
            self.state
                .set_info(t.directory_switcher_process_running().to_string());
            return Ok(());
        }

        // Get current panel's working directory
        let current_dir = self
            .layout_manager
            .active_panel_mut()
            .and_then(|p| p.get_working_directory());

        // Get all unique paths from all panels
        let paths = self.collect_panel_paths();

        // If no paths available, show info message
        if paths.is_empty() {
            self.state
                .set_info(t.directory_switcher_no_paths().to_string());
            return Ok(());
        }

        // Convert to DirectoryItems with is_current flag
        let items: Vec<DirectoryItem> = paths
            .into_iter()
            .map(|path| {
                let is_current = current_dir.as_ref() == Some(&path);
                let display = path.display().to_string();
                DirectoryItem {
                    path,
                    display,
                    is_current,
                }
            })
            .collect();

        // Find index of current directory to position cursor there
        let current_idx = items.iter().position(|item| item.is_current).unwrap_or(0);
        let modal = DirectorySwitcherModal::new(t.directory_switcher_title(), items)
            .with_cursor(current_idx);
        self.state.set_pending_action(
            PendingAction::SwitchDirectory,
            ActiveModal::DirectorySwitcher(Box::new(modal)),
        );

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

    /// Create new journal panel (singleton - only one instance allowed)
    pub(super) fn handle_new_journal(&mut self) -> Result<()> {
        // Check if Journal panel already exists and focus it
        if self.focus_existing_journal_panel() {
            logger::debug("Switching focus to existing Journal panel");
            return Ok(());
        }

        // No existing Journal panel found, create new one
        logger::debug("Opening new Journal panel");
        self.close_welcome_panels();
        let journal_panel = Journal::new(self.state.theme);
        self.add_panel(Box::new(journal_panel));
        self.auto_save_session();
        Ok(())
    }

    /// Find and focus existing Journal panel if it exists
    /// Returns true if Journal panel was found and focused
    fn focus_existing_journal_panel(&mut self) -> bool {
        // Iterate through all panel groups
        for (group_idx, group) in self.layout_manager.panel_groups.iter_mut().enumerate() {
            // Check each panel in the group
            for (panel_idx, panel) in group.panels().iter().enumerate() {
                if panel.is_journal() {
                    // Found Journal panel - set it as expanded and focus the group
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
        let welcome = Help::new(&self.state.config);
        self.add_panel(Box::new(welcome));
        self.auto_save_session();
        Ok(())
    }

    /// Open actions folder in file manager
    pub(super) fn handle_manage_actions(&mut self) -> Result<()> {
        use termide_config::get_config_dir;

        logger::debug("Opening actions folder in File Manager");
        self.close_welcome_panels();

        // Get the actions directory path
        let actions_dir = match get_config_dir() {
            Ok(config_dir) => {
                let actions_path = config_dir.join("actions");
                // Create the directory if it doesn't exist
                if !actions_path.exists() {
                    if let Err(e) = std::fs::create_dir_all(&actions_path) {
                        logger::warn(format!("Failed to create actions directory: {}", e));
                    }
                }
                actions_path
            }
            Err(e) => {
                logger::warn(format!("Failed to get config dir: {}", e));
                self.state
                    .set_error(format!("Failed to get actions directory: {}", e));
                return Ok(());
            }
        };

        let fm_panel = FileManager::new_with_path(actions_dir);
        self.add_panel(Box::new(fm_panel));
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

        let _ = self.open_editor_for_file(config_path);
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
        if self.state.ui.nested_submenu.open {
            return self.handle_nested_submenu_key(key);
        }

        use termide_ui_render::OPTIONS_SUBMENU_ITEM_COUNT;

        match key.code {
            KeyCode::Esc | KeyCode::Left => {
                // Close submenu, return to menu
                self.state.close_submenu();
            }
            KeyCode::Up => {
                if self.state.ui.options_submenu.selected > 0 {
                    self.state.ui.options_submenu.selected -= 1;
                } else {
                    self.state.ui.options_submenu.selected = OPTIONS_SUBMENU_ITEM_COUNT - 1;
                }
            }
            KeyCode::Down => {
                self.state.ui.options_submenu.selected =
                    (self.state.ui.options_submenu.selected + 1) % OPTIONS_SUBMENU_ITEM_COUNT;
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
        match self.state.ui.options_submenu.selected {
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
                // Language - open nested submenu with live preview
                use termide_ui_render::find_current_language_index;
                let current_idx = find_current_language_index();
                // Save current language for restoration on cancel
                self.state.ui.language_preview_original = Some(i18n::current_language());
                self.state.open_nested_submenu(current_idx);
            }
            2 => {
                // Manage actions - open actions folder in file manager
                self.state.close_menu();
                self.handle_manage_actions()?;
            }
            3 => {
                // Edit preferences - close menu and open config
                self.state.close_menu();
                self.open_config_in_editor()?;
            }
            4 => {
                // Help - show help
                self.state.close_menu();
                self.handle_new_help()?;
            }
            5 => {
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

    /// Handle keyboard event in nested submenu (Themes or Language list)
    fn handle_nested_submenu_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        // Determine which nested submenu is open based on parent submenu item
        match self.state.ui.options_submenu.selected {
            0 => self.handle_themes_nested_submenu_key(key),
            1 => self.handle_language_nested_submenu_key(key),
            _ => Ok(()),
        }
    }

    /// Handle keyboard event in Themes nested submenu
    fn handle_themes_nested_submenu_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
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
                if self.state.ui.nested_submenu.selected > 0 {
                    self.state.ui.nested_submenu.selected -= 1;
                } else {
                    self.state.ui.nested_submenu.selected = theme_count.saturating_sub(1);
                }
                // Live preview: apply theme on cursor move
                if let Some(name) = theme_names.get(self.state.ui.nested_submenu.selected) {
                    self.state.theme = Theme::get_by_name(name);
                }
            }
            KeyCode::Down => {
                if theme_count > 0 {
                    self.state.ui.nested_submenu.selected =
                        (self.state.ui.nested_submenu.selected + 1) % theme_count;
                }
                // Live preview: apply theme on cursor move
                if let Some(name) = theme_names.get(self.state.ui.nested_submenu.selected) {
                    self.state.theme = Theme::get_by_name(name);
                }
            }
            KeyCode::Enter => {
                // Clear preview state - theme is confirmed
                self.state.ui.theme_preview_original = None;
                // Apply selected theme and save preference
                if let Some(name) = theme_names.get(self.state.ui.nested_submenu.selected) {
                    self.apply_theme(name)?;
                }
                // Close all menus
                self.state.close_menu();
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle keyboard event in Language nested submenu
    fn handle_language_nested_submenu_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Result<()> {
        let languages = i18n::get_language_list();
        let lang_count = languages.len();

        match key.code {
            KeyCode::Esc | KeyCode::Left => {
                // Restore original language on cancel
                if let Some(original_lang) = self.state.ui.language_preview_original.take() {
                    let _ = i18n::set_language(&original_lang);
                }
                // Close nested submenu, return to parent
                self.state.close_nested_submenu();
            }
            KeyCode::Up => {
                if self.state.ui.nested_submenu.selected > 0 {
                    self.state.ui.nested_submenu.selected -= 1;
                } else {
                    self.state.ui.nested_submenu.selected = lang_count.saturating_sub(1);
                }
                // Live preview: apply language on cursor move
                if let Some((code, _)) = languages.get(self.state.ui.nested_submenu.selected) {
                    let _ = i18n::set_language(code);
                }
            }
            KeyCode::Down => {
                if lang_count > 0 {
                    self.state.ui.nested_submenu.selected =
                        (self.state.ui.nested_submenu.selected + 1) % lang_count;
                }
                // Live preview: apply language on cursor move
                if let Some((code, _)) = languages.get(self.state.ui.nested_submenu.selected) {
                    let _ = i18n::set_language(code);
                }
            }
            KeyCode::Enter => {
                // Clear preview state - language is confirmed
                self.state.ui.language_preview_original = None;
                // Apply selected language and save preference
                if let Some((code, name)) = languages.get(self.state.ui.nested_submenu.selected) {
                    self.apply_language(code, name)?;
                }
                // Close all menus
                self.state.close_menu();
            }
            _ => {}
        }
        Ok(())
    }

    /// Apply language by code and save preference
    fn apply_language(&mut self, lang_code: &str, lang_name: &str) -> Result<()> {
        if let Err(e) = i18n::set_language(lang_code) {
            logger::warn(format!("Failed to set language: {}", e));
            self.state
                .set_error(format!("Failed to set language: {}", e));
            return Ok(());
        }

        let t = i18n::t();
        self.state.set_info(t.language_changed(lang_name));

        // Save preference to config file
        if let Err(e) = self.save_language_preference(lang_code) {
            logger::warn(format!("Failed to save language preference: {}", e));
        }

        Ok(())
    }

    /// Save language preference to config file
    fn save_language_preference(&self, lang_code: &str) -> Result<()> {
        let mut config = Config::load()?;
        config.general.language = lang_code.to_string();
        config.save()?;
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
                if self.state.ui.sessions_submenu.selected > 0 {
                    self.state.ui.sessions_submenu.selected -= 1;
                } else {
                    self.state.ui.sessions_submenu.selected = SESSIONS_SUBMENU_ITEM_COUNT - 1;
                }
            }
            KeyCode::Down => {
                self.state.ui.sessions_submenu.selected =
                    (self.state.ui.sessions_submenu.selected + 1) % SESSIONS_SUBMENU_ITEM_COUNT;
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
        match self.state.ui.sessions_submenu.selected {
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
                if self.state.ui.tools_submenu.selected > 0 {
                    self.state.ui.tools_submenu.selected -= 1;
                } else {
                    self.state.ui.tools_submenu.selected = TOOLS_SUBMENU_ITEM_COUNT - 1;
                }
            }
            KeyCode::Down => {
                self.state.ui.tools_submenu.selected =
                    (self.state.ui.tools_submenu.selected + 1) % TOOLS_SUBMENU_ITEM_COUNT;
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
        match self.state.ui.tools_submenu.selected {
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
                // Journal - open journal panel
                self.state.close_menu();
                self.handle_new_journal()?;
            }
            6 => {
                // Diagnostics - open diagnostics panel
                self.state.close_menu();
                self.handle_open_diagnostics()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Open Diagnostics panel
    pub(super) fn handle_open_diagnostics(&mut self) -> Result<()> {
        logger::debug("Opening Diagnostics panel");
        self.close_welcome_panels();

        if !self.find_and_focus_panel_by_name("diagnostics") {
            let mut diagnostics_panel =
                termide_panel_diagnostics::DiagnosticsPanel::new(self.state.theme);

            // Initialize with existing diagnostics from all files
            for (path, diags) in &self.state.all_diagnostics {
                diagnostics_panel.update_diagnostics(path.clone(), diags);
            }

            self.add_panel(Box::new(diagnostics_panel));
        }
        self.auto_save_session();
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

    // =========================================================================
    // Actions submenu handling
    // =========================================================================

    /// Handle keyboard event in Actions submenu
    pub(super) fn handle_actions_submenu_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Result<()> {
        // If nested submenu is open, delegate to nested handler
        if self.state.ui.actions_nested.open {
            return self.handle_actions_nested_submenu_key(key);
        }

        let registry = termide_config::actions::ActionsRegistry::load();
        let item_count = registry
            .as_ref()
            .map(|r| r.root_items.len() + r.groups.len())
            .unwrap_or(0);

        if item_count == 0 {
            // Empty menu - just close on any key
            if matches!(key.code, KeyCode::Esc | KeyCode::Left) {
                self.state.close_actions_submenu();
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Esc | KeyCode::Left => {
                self.state.close_actions_submenu();
            }
            KeyCode::Up => {
                if self.state.ui.actions_submenu.selected > 0 {
                    self.state.ui.actions_submenu.selected -= 1;
                } else {
                    self.state.ui.actions_submenu.selected = item_count.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if item_count > 0 {
                    self.state.ui.actions_submenu.selected =
                        (self.state.ui.actions_submenu.selected + 1) % item_count;
                }
            }
            KeyCode::Right | KeyCode::Enter => {
                self.execute_actions_submenu_action()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Execute action for selected Actions submenu item
    pub(super) fn execute_actions_submenu_action(&mut self) -> Result<()> {
        let registry = termide_config::actions::ActionsRegistry::load();

        // Check if registry is empty - then the only item is "Add action..."
        let is_empty = registry
            .as_ref()
            .map(|r| r.root_items.is_empty() && r.groups.is_empty())
            .unwrap_or(true);

        if is_empty {
            // "Add action..." selected - open actions folder
            self.state.close_menu();
            self.handle_manage_actions()?;
            return Ok(());
        }

        let registry = match registry {
            Some(r) => r,
            None => return Ok(()),
        };

        let selected = self.state.ui.actions_submenu.selected;
        let root_count = registry.root_items.len();

        if selected < root_count {
            // Root item selected - execute the action
            if let Some(action) = registry.root_items.get(selected) {
                self.state.close_menu();
                self.run_action_script(action)?;
            }
        } else {
            // Group selected - open nested submenu
            let group_idx = selected - root_count;
            if let Some(group) = registry.groups.get(group_idx) {
                self.state.open_actions_nested_submenu(group.name.clone());
            }
        }

        Ok(())
    }

    /// Handle keyboard event in Actions nested submenu (group items)
    fn handle_actions_nested_submenu_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        let registry = termide_config::actions::ActionsRegistry::load();
        let group_name = self.state.ui.current_actions_group.clone();

        let item_count = registry
            .as_ref()
            .and_then(|r| {
                group_name
                    .as_ref()
                    .and_then(|name| r.groups.iter().find(|g| &g.name == name))
                    .map(|g| g.items.len())
            })
            .unwrap_or(0);

        match key.code {
            KeyCode::Esc | KeyCode::Left => {
                self.state.close_actions_nested_submenu();
            }
            KeyCode::Up => {
                if self.state.ui.actions_nested.selected > 0 {
                    self.state.ui.actions_nested.selected -= 1;
                } else {
                    self.state.ui.actions_nested.selected = item_count.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if item_count > 0 {
                    self.state.ui.actions_nested.selected =
                        (self.state.ui.actions_nested.selected + 1) % item_count;
                }
            }
            KeyCode::Enter => {
                self.execute_actions_nested_action()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Execute action for selected item in Actions nested submenu
    pub(super) fn execute_actions_nested_action(&mut self) -> Result<()> {
        let registry = match termide_config::actions::ActionsRegistry::load() {
            Some(r) => r,
            None => return Ok(()),
        };

        let group_name = match &self.state.ui.current_actions_group {
            Some(name) => name.clone(),
            None => return Ok(()),
        };

        let group = match registry.groups.iter().find(|g| g.name == group_name) {
            Some(g) => g,
            None => return Ok(()),
        };

        if let Some(action) = group.items.get(self.state.ui.actions_nested.selected) {
            self.state.close_menu();
            self.run_action_script(action)?;
        }

        Ok(())
    }

    /// Run an action script
    fn run_action_script(&mut self, action: &termide_config::actions::ActionItem) -> Result<()> {
        use termide_panel_terminal::Terminal;

        let cwd = self.get_focused_panel_cwd();

        if action.is_report {
            // Run in background with output capture, show result in modal
            self.run_report_script(action, &cwd)?;
        } else if action.is_background {
            // Fire-and-forget spawn (no terminal panel)
            logger::info(format!(
                "Running background action '{}' in {:?}",
                action.name, cwd
            ));
            match std::process::Command::new(&action.path)
                .current_dir(&cwd)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .stdin(std::process::Stdio::null())
                .spawn()
            {
                Ok(_) => {}
                Err(e) => {
                    logger::error(format!(
                        "Failed to run background action '{}': {}",
                        action.name, e
                    ));
                    self.state.set_error(format!("Failed to run action: {}", e));
                }
            }
        } else {
            // Run in new terminal panel
            logger::info(format!("Running action '{}' in {:?}", action.name, cwd));

            self.close_welcome_panels();

            let width = self.state.terminal.width;
            let height = self.state.terminal.height;
            let term_height = height.saturating_sub(3);
            let term_width = width.saturating_sub(2);

            let command = action.path.to_string_lossy().to_string();

            match Terminal::new_with_cwd(term_height, term_width, Some(cwd)) {
                Ok(mut terminal) => {
                    let _ = terminal.send_command(&command);
                    self.add_panel(Box::new(terminal));
                    self.auto_save_session();
                }
                Err(e) => {
                    logger::error(format!(
                        "Failed to create terminal for action '{}': {}",
                        action.name, e
                    ));
                    self.state.set_error(format!("Failed to run action: {}", e));
                }
            }
        }

        Ok(())
    }

    /// Run a report script in background, capturing output for modal display
    fn run_report_script(
        &mut self,
        action: &termide_config::actions::ActionItem,
        cwd: &std::path::Path,
    ) -> Result<()> {
        use crate::state::{ScriptOperationHandle, ScriptOperationResult};

        logger::info(format!(
            "Running report script '{}' in {:?}",
            action.name, cwd
        ));

        let child = std::process::Command::new(&action.path)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        match child {
            Ok(child) => {
                let script_name = action.name.clone();
                let (tx, rx) = std::sync::mpsc::channel();

                std::thread::spawn(move || {
                    let output = child.wait_with_output();
                    let result = match output {
                        Ok(out) => ScriptOperationResult {
                            script_name: script_name.clone(),
                            success: out.status.success(),
                            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                        },
                        Err(e) => ScriptOperationResult {
                            script_name: script_name.clone(),
                            success: false,
                            stdout: String::new(),
                            stderr: e.to_string(),
                        },
                    };
                    let _ = tx.send(result);
                });

                self.state.script_operation_handle = Some(ScriptOperationHandle {
                    receiver: rx,
                    script_name: action.name.clone(),
                });
            }
            Err(e) => {
                logger::error(format!(
                    "Failed to run report script '{}': {}",
                    action.name, e
                ));
                self.state.set_error(format!("Failed to run script: {}", e));
            }
        }

        Ok(())
    }

    /// Get the working directory from the focused panel
    fn get_focused_panel_cwd(&self) -> PathBuf {
        // Use the Panel::get_working_directory() method
        if let Some(panel) = self.layout_manager.active_panel() {
            if let Some(cwd) = panel.get_working_directory() {
                return cwd;
            }
        }

        // Fallback to project root
        self.project_root.clone()
    }
}
