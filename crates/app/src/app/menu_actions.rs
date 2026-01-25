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
                    // Scripts - open submenu dropdown (keep menu open)
                    self.state.open_scripts_submenu();
                }
                3 => {
                    // Bookmarks - open submenu dropdown (keep menu open)
                    self.state.open_bookmarks_submenu();
                }
                4 => {
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
        let panel_paths = self.collect_panel_paths();

        // Get bookmarked directories
        let bookmark_dirs = self.state.bookmarks.directories();

        // Build combined items list
        let mut items: Vec<DirectoryItem> = Vec::new();
        let mut seen_paths = std::collections::HashSet::new();

        // Add panel paths first
        for path in panel_paths {
            let is_current = current_dir.as_ref() == Some(&path);
            let display = path.display().to_string();
            seen_paths.insert(path.clone());
            items.push(DirectoryItem {
                path,
                display,
                is_current,
                is_bookmark: false,
            });
        }

        // Add bookmarked directories (if not already in list)
        for bookmark in bookmark_dirs {
            let path = PathBuf::from(&bookmark.path);
            if !seen_paths.contains(&path) {
                // Show path instead of display name for consistency
                let display = bookmark.path.clone();
                let is_current = current_dir.as_ref() == Some(&path);
                items.push(DirectoryItem {
                    path,
                    display,
                    is_current,
                    is_bookmark: true,
                });
            }
        }

        // Sort items alphabetically by display path
        items.sort_by(|a, b| a.display.cmp(&b.display));

        // If no paths available, show info message
        if items.is_empty() {
            self.state
                .set_info(t.directory_switcher_no_paths().to_string());
            return Ok(());
        }

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

        // Check if active panel is a remote FileManager and clone it
        let remote_info = self
            .layout_manager
            .active_panel_mut()
            .and_then(|p| p.as_file_manager_mut())
            .filter(|fm| fm.is_remote())
            .map(|fm| (fm.display_path(), fm.vfs_manager_arc()));

        let fm_panel = if let Some((vfs_url, vfs_manager)) = remote_info {
            // Clone remote panel with same VFS URL
            FileManager::new_with_vfs_url(&vfs_url, vfs_manager)?
        } else {
            // Fallback to local filesystem
            let working_dir = self
                .layout_manager
                .active_panel_mut()
                .and_then(|p| p.get_working_directory())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));
            FileManager::new_with_path(working_dir)
        };

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

    /// Open scripts folder in file manager
    pub(super) fn handle_manage_scripts(&mut self) -> Result<()> {
        use termide_config::get_data_dir;

        logger::debug("Opening scripts folder in File Manager");
        self.close_welcome_panels();

        // Get the scripts directory path
        let scripts_dir = match get_data_dir() {
            Ok(data_dir) => {
                let scripts_path = data_dir.join("scripts");
                // Create the directory if it doesn't exist
                if !scripts_path.exists() {
                    if let Err(e) = std::fs::create_dir_all(&scripts_path) {
                        logger::warn(format!("Failed to create scripts directory: {}", e));
                    }
                }
                scripts_path
            }
            Err(e) => {
                logger::warn(format!("Failed to get data dir: {}", e));
                self.state
                    .set_error(format!("Failed to get scripts directory: {}", e));
                return Ok(());
            }
        };

        let fm_panel = FileManager::new_with_path(scripts_dir);
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
                self.handle_manage_scripts()?;
            }
            3 => {
                // Manage bookmarks - open bookmarks.toml in editor
                self.state.close_menu();
                self.handle_manage_bookmarks()?;
            }
            4 => {
                // Edit preferences - close menu and open config
                self.state.close_menu();
                self.open_config_in_editor()?;
            }
            5 => {
                // Help - show help
                self.state.close_menu();
                self.handle_new_help()?;
            }
            6 => {
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
    // Scripts submenu handling
    // =========================================================================

    /// Handle keyboard event in Scripts submenu
    pub(super) fn handle_scripts_submenu_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Result<()> {
        // If nested submenu is open, delegate to nested handler
        if self.state.ui.scripts_nested.open {
            return self.handle_scripts_nested_submenu_key(key);
        }

        let registry = termide_config::scripts::ScriptsRegistry::load();
        let item_count = registry
            .as_ref()
            .map(|r| r.root_items.len() + r.groups.len())
            .unwrap_or(0);

        if item_count == 0 {
            // Empty menu - just close on any key
            if matches!(key.code, KeyCode::Esc | KeyCode::Left) {
                self.state.close_scripts_submenu();
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Esc | KeyCode::Left => {
                self.state.close_scripts_submenu();
            }
            KeyCode::Up => {
                if self.state.ui.scripts_submenu.selected > 0 {
                    self.state.ui.scripts_submenu.selected -= 1;
                } else {
                    self.state.ui.scripts_submenu.selected = item_count.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if item_count > 0 {
                    self.state.ui.scripts_submenu.selected =
                        (self.state.ui.scripts_submenu.selected + 1) % item_count;
                }
            }
            KeyCode::Right | KeyCode::Enter => {
                self.execute_scripts_submenu_action()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Execute action for selected Scripts submenu item
    pub(super) fn execute_scripts_submenu_action(&mut self) -> Result<()> {
        let registry = termide_config::scripts::ScriptsRegistry::load();

        // Check if registry is empty - then the only item is "Add script..."
        let is_empty = registry
            .as_ref()
            .map(|r| r.root_items.is_empty() && r.groups.is_empty())
            .unwrap_or(true);

        if is_empty {
            // "Add script..." selected - open scripts folder
            self.state.close_menu();
            self.handle_manage_scripts()?;
            return Ok(());
        }

        let registry = match registry {
            Some(r) => r,
            None => return Ok(()),
        };

        let selected = self.state.ui.scripts_submenu.selected;
        let root_count = registry.root_items.len();

        if selected < root_count {
            // Root item selected - execute the script
            if let Some(script) = registry.root_items.get(selected) {
                self.state.close_menu();
                self.run_script(script)?;
            }
        } else {
            // Group selected - open nested submenu
            let group_idx = selected - root_count;
            if let Some(group) = registry.groups.get(group_idx) {
                self.state.open_scripts_nested_submenu(group.name.clone());
            }
        }

        Ok(())
    }

    /// Handle keyboard event in Scripts nested submenu (group items)
    fn handle_scripts_nested_submenu_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        let registry = termide_config::scripts::ScriptsRegistry::load();
        let group_name = self.state.ui.current_scripts_group.clone();

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
                self.state.close_scripts_nested_submenu();
            }
            KeyCode::Up => {
                if self.state.ui.scripts_nested.selected > 0 {
                    self.state.ui.scripts_nested.selected -= 1;
                } else {
                    self.state.ui.scripts_nested.selected = item_count.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if item_count > 0 {
                    self.state.ui.scripts_nested.selected =
                        (self.state.ui.scripts_nested.selected + 1) % item_count;
                }
            }
            KeyCode::Enter => {
                self.execute_scripts_nested_action()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Execute action for selected item in Scripts nested submenu
    pub(super) fn execute_scripts_nested_action(&mut self) -> Result<()> {
        let registry = match termide_config::scripts::ScriptsRegistry::load() {
            Some(r) => r,
            None => return Ok(()),
        };

        let group_name = match &self.state.ui.current_scripts_group {
            Some(name) => name.clone(),
            None => return Ok(()),
        };

        let group = match registry.groups.iter().find(|g| g.name == group_name) {
            Some(g) => g,
            None => return Ok(()),
        };

        if let Some(script) = group.items.get(self.state.ui.scripts_nested.selected) {
            self.state.close_menu();
            self.run_script(script)?;
        }

        Ok(())
    }

    /// Run a script
    fn run_script(&mut self, script: &termide_config::scripts::ScriptItem) -> Result<()> {
        use termide_panel_terminal::Terminal;

        let cwd = self.get_focused_panel_cwd();

        if script.is_report {
            // Run in background with output capture, show result in modal
            self.run_report_script(script, &cwd)?;
        } else if script.is_background {
            // Fire-and-forget spawn (no terminal panel)
            logger::info(format!(
                "Running background script '{}' in {:?}",
                script.name, cwd
            ));
            match std::process::Command::new(&script.path)
                .current_dir(&cwd)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .stdin(std::process::Stdio::null())
                .spawn()
            {
                Ok(_) => {}
                Err(e) => {
                    logger::error(format!(
                        "Failed to run background script '{}': {}",
                        script.name, e
                    ));
                    self.state.set_error(format!("Failed to run script: {}", e));
                }
            }
        } else {
            // Run in new terminal panel
            logger::info(format!("Running script '{}' in {:?}", script.name, cwd));

            self.close_welcome_panels();

            let width = self.state.terminal.width;
            let height = self.state.terminal.height;
            let term_height = height.saturating_sub(3);
            let term_width = width.saturating_sub(2);

            let command = script.path.to_string_lossy().to_string();

            match Terminal::new_with_cwd(term_height, term_width, Some(cwd)) {
                Ok(mut terminal) => {
                    let _ = terminal.send_command(&command);
                    self.add_panel(Box::new(terminal));
                    self.auto_save_session();
                }
                Err(e) => {
                    logger::error(format!(
                        "Failed to create terminal for script '{}': {}",
                        script.name, e
                    ));
                    self.state.set_error(format!("Failed to run script: {}", e));
                }
            }
        }

        Ok(())
    }

    /// Run a report script in background, capturing output for modal display
    fn run_report_script(
        &mut self,
        script: &termide_config::scripts::ScriptItem,
        cwd: &std::path::Path,
    ) -> Result<()> {
        use crate::state::{ScriptOperationHandle, ScriptOperationResult};

        logger::info(format!(
            "Running report script '{}' in {:?}",
            script.name, cwd
        ));

        let child = std::process::Command::new(&script.path)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        match child {
            Ok(child) => {
                let script_name = script.name.clone();
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
                    script_name: script.name.clone(),
                });
            }
            Err(e) => {
                logger::error(format!(
                    "Failed to run report script '{}': {}",
                    script.name, e
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

    // =========================================================================
    // Bookmarks submenu handling
    // =========================================================================

    /// Handle keyboard event in Bookmarks submenu
    pub(super) fn handle_bookmarks_submenu_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Result<()> {
        // If nested submenu is open, delegate to nested handler
        if self.state.ui.bookmarks_nested.open {
            return self.handle_bookmarks_nested_submenu_key(key);
        }

        use termide_ui_render::get_bookmarks_item_count;
        let item_count = get_bookmarks_item_count(&self.state.bookmarks);

        match key.code {
            KeyCode::Esc | KeyCode::Left => {
                self.state.close_bookmarks_submenu();
            }
            KeyCode::Up => {
                if self.state.ui.bookmarks_submenu.selected > 0 {
                    self.state.ui.bookmarks_submenu.selected -= 1;
                } else {
                    self.state.ui.bookmarks_submenu.selected = item_count.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if item_count > 0 {
                    self.state.ui.bookmarks_submenu.selected =
                        (self.state.ui.bookmarks_submenu.selected + 1) % item_count;
                }
            }
            KeyCode::Right | KeyCode::Enter => {
                self.execute_bookmarks_submenu_action()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Execute action for selected Bookmarks submenu item
    pub(super) fn execute_bookmarks_submenu_action(&mut self) -> Result<()> {
        let selected = self.state.ui.bookmarks_submenu.selected;

        if selected == 0 {
            // Add current - open add bookmark modal
            self.state.close_menu();
            self.handle_add_bookmark()?;
            return Ok(());
        }

        // Get groups and ungrouped counts
        let named_groups: Vec<String> = self
            .state
            .bookmarks
            .named_groups()
            .keys()
            .cloned()
            .collect();
        let ungrouped = self.state.bookmarks.ungrouped();
        let groups_start = 1;
        let ungrouped_start = groups_start + named_groups.len();

        if selected >= groups_start && selected < ungrouped_start {
            // Group selected - open nested submenu
            let group_idx = selected - groups_start;
            if let Some(group_name) = named_groups.get(group_idx) {
                self.state.open_bookmarks_nested_submenu(group_name.clone());
            }
        } else {
            // Ungrouped bookmark selected - open directly
            let ungrouped_idx = selected - ungrouped_start;
            if let Some(bookmark) = ungrouped.get(ungrouped_idx) {
                let path = bookmark.path.clone();
                let bookmark_type = bookmark.bookmark_type();
                self.state.close_menu();
                self.open_bookmark(&path, bookmark_type)?;
            }
        }

        Ok(())
    }

    /// Handle keyboard event in Bookmarks nested submenu (group items)
    fn handle_bookmarks_nested_submenu_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use termide_ui_render::get_bookmarks_group_items;

        let group_name = self.state.ui.current_bookmarks_group.clone();

        let item_count = group_name
            .as_ref()
            .map(|name| get_bookmarks_group_items(&self.state.bookmarks, name).len())
            .unwrap_or(0);

        match key.code {
            KeyCode::Esc | KeyCode::Left => {
                self.state.close_bookmarks_nested_submenu();
            }
            KeyCode::Up => {
                if self.state.ui.bookmarks_nested.selected > 0 {
                    self.state.ui.bookmarks_nested.selected -= 1;
                } else {
                    self.state.ui.bookmarks_nested.selected = item_count.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if item_count > 0 {
                    self.state.ui.bookmarks_nested.selected =
                        (self.state.ui.bookmarks_nested.selected + 1) % item_count;
                }
            }
            KeyCode::Enter => {
                self.execute_bookmarks_nested_action()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Execute action for selected item in Bookmarks nested submenu
    pub(super) fn execute_bookmarks_nested_action(&mut self) -> Result<()> {
        let group_name = match &self.state.ui.current_bookmarks_group {
            Some(name) => name.clone(),
            None => return Ok(()),
        };

        let grouped = self.state.bookmarks.grouped();
        let group_bookmarks = match grouped.get(&group_name) {
            Some(bookmarks) => bookmarks,
            None => return Ok(()),
        };

        if let Some(bookmark) = group_bookmarks.get(self.state.ui.bookmarks_nested.selected) {
            let path = bookmark.path.clone();
            let bookmark_type = bookmark.bookmark_type();
            self.state.close_menu();
            self.open_bookmark(&path, bookmark_type)?;
        }

        Ok(())
    }

    /// Handle adding a bookmark
    pub(super) fn handle_add_bookmark(&mut self) -> Result<()> {
        use termide_modal::BookmarkAddModal;

        // Get current path from active panel
        let current_path = self.get_current_bookmark_path();

        // Get existing group names for autocomplete
        let existing_groups = self.state.bookmarks.group_names();

        let modal = BookmarkAddModal::new(current_path, existing_groups);
        self.state.set_pending_action(
            PendingAction::AddBookmark,
            ActiveModal::BookmarkAdd(Box::new(modal)),
        );

        Ok(())
    }

    /// Get current path from active panel for bookmarking
    fn get_current_bookmark_path(&self) -> Option<String> {
        if let Some(panel) = self.layout_manager.active_panel() {
            // Try to get file path from editor
            if let Some(editor) = panel.as_editor() {
                if let Some(path) = editor.file_path() {
                    return Some(path.display().to_string());
                }
            }
            // Fall back to working directory
            if let Some(cwd) = panel.get_working_directory() {
                return Some(cwd.display().to_string());
            }
        }
        None
    }

    /// Handle managing bookmarks - open bookmarks.toml in editor
    pub(super) fn handle_manage_bookmarks(&mut self) -> Result<()> {
        use termide_config::BookmarksConfig;

        self.close_welcome_panels();

        // Get the bookmarks file path
        let bookmarks_path = match BookmarksConfig::config_file_path() {
            Ok(path) => {
                // Create the file if it doesn't exist
                if !path.exists() {
                    // Ensure parent directory exists
                    if let Some(parent) = path.parent() {
                        if !parent.exists() {
                            if let Err(e) = std::fs::create_dir_all(parent) {
                                logger::warn(format!("Failed to create data directory: {}", e));
                            }
                        }
                    }
                    // Create empty bookmarks file
                    let empty_config = BookmarksConfig::default();
                    if let Err(e) = empty_config.save() {
                        logger::warn(format!("Failed to create bookmarks file: {}", e));
                    }
                }
                path
            }
            Err(e) => {
                logger::warn(format!("Failed to get bookmarks path: {}", e));
                self.state
                    .set_error(format!("Failed to get bookmarks path: {}", e));
                return Ok(());
            }
        };

        let _ = self.open_editor_for_file(bookmarks_path);
        Ok(())
    }

    /// Open a bookmark based on its type
    fn open_bookmark(
        &mut self,
        path: &str,
        bookmark_type: termide_config::BookmarkType,
    ) -> Result<()> {
        use termide_config::BookmarkType;

        match bookmark_type {
            BookmarkType::Directory => {
                // Check if active panel is a file manager - reuse it
                if let Some(panel) = self.layout_manager.active_panel_mut() {
                    if let Some(fm) = panel.as_file_manager_mut() {
                        let _ = fm.navigate_to(PathBuf::from(path));
                        return Ok(());
                    }
                }
                // No active file manager - create new panel
                self.close_welcome_panels();
                let fm_panel = FileManager::new_with_path(PathBuf::from(path));
                self.add_panel(Box::new(fm_panel));
                self.auto_save_session();
            }
            BookmarkType::TextFile => {
                // Open in editor
                let _ = self.open_editor_for_file(PathBuf::from(path));
            }
            BookmarkType::ViewerFile | BookmarkType::HttpLink => {
                // Open with external viewer
                let _ = std::process::Command::new("xdg-open").arg(path).spawn();
            }
            BookmarkType::SshConnection => {
                // Open SSH connection in terminal
                // Parse ssh://[user@]host[:port] format into proper ssh command
                let ssh_cmd = {
                    let url_part = path.strip_prefix("ssh://").unwrap_or(path);
                    let mut cmd_parts = vec!["ssh".to_string()];

                    // Split off any path component (ignore it for SSH)
                    let authority = url_part.split('/').next().unwrap_or(url_part);

                    // Parse user@host:port format
                    let (user_host, port) = if let Some(colon_pos) = authority.rfind(':') {
                        // Check if what's after colon looks like a port number
                        let after_colon = &authority[colon_pos + 1..];
                        if after_colon.chars().all(|c| c.is_ascii_digit())
                            && !after_colon.is_empty()
                        {
                            (&authority[..colon_pos], Some(after_colon))
                        } else {
                            (authority, None)
                        }
                    } else {
                        (authority, None)
                    };

                    // Add port if specified
                    if let Some(port) = port {
                        cmd_parts.push("-p".to_string());
                        cmd_parts.push(port.to_string());
                    }

                    cmd_parts.push(user_host.to_string());
                    cmd_parts.join(" ")
                };

                let width = self.state.terminal.width;
                let height = self.state.terminal.height;
                let term_height = height.saturating_sub(3);
                let term_width = width.saturating_sub(2);

                self.close_welcome_panels();
                if let Ok(terminal) = Terminal::new_with_command(term_height, term_width, &ssh_cmd)
                {
                    self.add_panel(Box::new(terminal));
                    self.auto_save_session();
                }
            }
            BookmarkType::SftpPath
            | BookmarkType::FtpPath
            | BookmarkType::SmbPath
            | BookmarkType::NfsPath => {
                // Navigate to remote path using VFS
                if let Some(panel) = self.layout_manager.active_panel_mut() {
                    if let Some(fm) = panel.as_file_manager_mut() {
                        let _ = fm.navigate_to_url(path);
                        return Ok(());
                    }
                }
                // No active file manager - create new panel and navigate
                self.close_welcome_panels();
                let mut fm_panel = FileManager::new();
                let _ = fm_panel.navigate_to_url(path);
                self.add_panel(Box::new(fm_panel));
                self.auto_save_session();
            }
            BookmarkType::Unknown => {
                // Try to open as text file
                let _ = self.open_editor_for_file(PathBuf::from(path));
            }
        }

        Ok(())
    }
}
