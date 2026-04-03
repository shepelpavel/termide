//! Menu actions and panel creation for the application.
//!
//! Handles menu navigation and creating new panels.

// Note: PanelExt is used for editor save operations that require concrete type access.
#![allow(deprecated)]

use anyhow::Result;
use crossterm::event::KeyCode;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(unix)]
use std::sync::Mutex;
#[cfg(unix)]
use std::time::{Duration, Instant};

use super::App;
use crate::state::{ActiveModal, PendingAction};
use crate::PanelExt;
use termide_app_core::Panel;

/// Result of generic submenu keyboard navigation.
enum SubmenuNavAction {
    /// User pressed Esc — close submenu
    Close,
    /// User pressed Enter — execute selected action
    Execute,
    /// User pressed Right — open submenu or go to next root menu
    Right,
    /// User pressed Left — close nested or go to prev root menu
    Left,
    /// User pressed F4 — edit selected item
    Edit,
    /// Navigation handled (Up/Down) or no-op
    None,
}

/// Handle generic submenu keyboard navigation.
/// Updates selection on Up/Down and returns the action for Esc/Enter.
/// `separators` lists indices of separator items that should be skipped.
fn navigate_submenu(
    key: &crossterm::event::KeyEvent,
    submenu: &mut termide_state::SubmenuState,
    item_count: usize,
    separators: &[usize],
) -> SubmenuNavAction {
    match key.code {
        KeyCode::Esc => SubmenuNavAction::Close,
        KeyCode::Left => SubmenuNavAction::Left,
        KeyCode::Up => {
            submenu.select_prev(item_count);
            // Skip separators
            if separators.contains(&submenu.selected) {
                submenu.select_prev(item_count);
            }
            SubmenuNavAction::None
        }
        KeyCode::Down => {
            submenu.select_next(item_count);
            // Skip separators
            if separators.contains(&submenu.selected) {
                submenu.select_next(item_count);
            }
            SubmenuNavAction::None
        }
        KeyCode::Enter => SubmenuNavAction::Execute,
        KeyCode::Right => SubmenuNavAction::Right,
        KeyCode::F(4) => SubmenuNavAction::Edit,
        _ => SubmenuNavAction::None,
    }
}
use termide_config::Config;
use termide_i18n as i18n;

use termide_panel_file_manager::FileManager;
use termide_panel_terminal::Terminal;
use termide_theme::Theme;
use termide_ui_render::menu::{
    BOOKMARKS_MENU_INDEX, INDICATOR_CLOCK_INDEX, INDICATOR_CPU_INDEX, INDICATOR_DISK_INDEX,
    INDICATOR_NET_INDEX, INDICATOR_RAM_INDEX, MENU_TOTAL_COUNT, OPTIONS_MENU_INDEX,
    SCRIPTS_MENU_INDEX, SESSIONS_MENU_INDEX, WINDOWS_MENU_INDEX,
};
use termide_ui_render::{
    OPTIONS_SUBMENU_HELP, OPTIONS_SUBMENU_LANGUAGE, OPTIONS_SUBMENU_PREFERENCES,
    OPTIONS_SUBMENU_QUIT, OPTIONS_SUBMENU_THEMES, SESSIONS_SUBMENU_CHANGE_ROOT,
    SESSIONS_SUBMENU_NEW, SESSIONS_SUBMENU_SWITCH, TOOLS_SUBMENU_DIAGNOSTICS, TOOLS_SUBMENU_EDITOR,
    TOOLS_SUBMENU_FILES, TOOLS_SUBMENU_GIT_LOG, TOOLS_SUBMENU_GIT_STASH, TOOLS_SUBMENU_GIT_STATUS,
    TOOLS_SUBMENU_JOURNAL, TOOLS_SUBMENU_OPERATIONS, TOOLS_SUBMENU_OUTLINE, TOOLS_SUBMENU_TERMINAL,
};

impl App {
    /// Switch to next root menu item and open its submenu
    pub(super) fn switch_to_next_menu(&mut self) -> Result<()> {
        self.state.ui.close_all_submenus();
        self.state.next_menu_item(MENU_TOTAL_COUNT);
        self.execute_menu_action()
    }

    /// Switch to previous root menu item and open its submenu
    pub(super) fn switch_to_prev_menu(&mut self) -> Result<()> {
        self.state.ui.close_all_submenus();
        self.state.prev_menu_item(MENU_TOTAL_COUNT);
        self.execute_menu_action()
    }

    /// Handle keyboard event in menu
    pub(super) fn handle_menu_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.state.close_menu();
            }
            KeyCode::Left => {
                self.state.prev_menu_item(MENU_TOTAL_COUNT);
                self.execute_menu_action()?;
            }
            KeyCode::Right => {
                self.state.next_menu_item(MENU_TOTAL_COUNT);
                self.execute_menu_action()?;
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
                SESSIONS_MENU_INDEX => {
                    self.state.open_sessions_submenu();
                }
                WINDOWS_MENU_INDEX => {
                    self.state.open_tools_submenu();
                }
                SCRIPTS_MENU_INDEX => {
                    self.state.open_scripts_submenu();
                }
                BOOKMARKS_MENU_INDEX => {
                    self.state.open_bookmarks_submenu();
                }
                OPTIONS_MENU_INDEX => {
                    self.state.open_submenu();
                }
                INDICATOR_NET_INDEX
                | INDICATOR_CPU_INDEX
                | INDICATOR_RAM_INDEX
                | INDICATOR_CLOCK_INDEX
                | INDICATOR_DISK_INDEX => {
                    self.open_indicator_as_submenu(menu_index);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Open an indicator modal positioned as a dropdown under the indicator.
    pub(super) fn open_indicator_as_submenu(&mut self, menu_index: usize) {
        self.state.close_indicator_modal();

        if menu_index == INDICATOR_DISK_INDEX {
            use crate::state::ResourceModalKind;
            let t = termide_i18n::t();
            let lines = self.build_disk_modal_lines();
            // Use terminal width as anchor — clamping in render will right-align the modal
            let anchor_x = self.state.terminal.width;
            // Bottom edge = status bar row (last row)
            let anchor_y = self.state.terminal.height.saturating_sub(1);
            let modal = termide_modal::InfoModal::new_rich(t.resource_disk_title(), lines)
                .without_button()
                .with_anchor_bottom(anchor_x, anchor_y);
            self.state.active_modal = Some(termide_modal::ActiveModal::Info(Box::new(modal)));
            self.state.resource_modal_kind = Some(ResourceModalKind::Disk);
            self.state.last_resource_modal_refresh = Some(std::time::Instant::now());
            self.state.needs_redraw = true;
            return;
        }

        let (net_range, cpu_range, ram_range, clock_range) = self.get_indicator_ranges();
        let anchor_x = match menu_index {
            INDICATOR_NET_INDEX => net_range.start,
            INDICATOR_CPU_INDEX => cpu_range.start,
            INDICATOR_RAM_INDEX => ram_range.start,
            INDICATOR_CLOCK_INDEX => clock_range.start,
            _ => 0,
        };

        if menu_index == INDICATOR_CLOCK_INDEX {
            let modal = termide_modal::CalendarModal::new().with_anchor(anchor_x, 1);
            self.state.active_modal = Some(termide_modal::ActiveModal::Calendar(Box::new(modal)));
            self.state.needs_redraw = true;
        } else {
            let kind = match menu_index {
                INDICATOR_NET_INDEX => crate::state::ResourceModalKind::Network,
                INDICATOR_CPU_INDEX => crate::state::ResourceModalKind::Cpu,
                INDICATOR_RAM_INDEX => crate::state::ResourceModalKind::Ram,
                _ => return,
            };
            self.open_resource_modal_at(kind, Some((anchor_x, 1)));
        }
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
                let display_path =
                    termide_core::util::shorten_home_path(&info.project_path.display().to_string());
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
            let display = termide_core::util::shorten_home_path(&path.display().to_string());
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
                let display = termide_core::util::shorten_home_path(&bookmark.path);
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

        match navigate_submenu(
            &key,
            &mut self.state.ui.options_submenu,
            OPTIONS_SUBMENU_ITEM_COUNT,
            &[],
        ) {
            SubmenuNavAction::Close => self.state.close_menu(),
            SubmenuNavAction::Execute => self.execute_submenu_action()?,
            SubmenuNavAction::Right => {
                let sel = self.state.ui.options_submenu.selected;
                if sel == OPTIONS_SUBMENU_THEMES || sel == OPTIONS_SUBMENU_LANGUAGE {
                    self.execute_submenu_action()?;
                } else {
                    self.switch_to_next_menu()?;
                }
            }
            SubmenuNavAction::Left => self.switch_to_prev_menu()?,
            SubmenuNavAction::Edit => {}
            SubmenuNavAction::None => {}
        }
        Ok(())
    }

    /// Execute action for selected Options submenu item
    fn execute_submenu_action(&mut self) -> Result<()> {
        match self.state.ui.options_submenu.selected {
            OPTIONS_SUBMENU_THEMES => {
                let theme_names = Theme::all_theme_names();
                let current_idx = theme_names
                    .iter()
                    .position(|n| n == self.state.theme.name)
                    .unwrap_or(0);
                self.state.ui.theme_preview_original = Some(self.state.theme.name.to_string());
                self.state.open_nested_submenu(current_idx);
            }
            OPTIONS_SUBMENU_LANGUAGE => {
                use termide_ui_render::find_current_language_index;
                let current_idx = find_current_language_index();
                self.state.ui.language_preview_original = Some(i18n::current_language());
                self.state.open_nested_submenu(current_idx);
            }
            OPTIONS_SUBMENU_PREFERENCES => {
                self.state.close_menu();
                self.open_config_in_editor()?;
            }
            OPTIONS_SUBMENU_HELP => {
                self.state.close_menu();
                self.handle_new_help()?;
            }
            OPTIONS_SUBMENU_QUIT => {
                self.state.close_menu();
                if self.has_panels_requiring_confirmation() {
                    let t = i18n::t();
                    let modal =
                        termide_modal::ConfirmModal::new(t.app_quit_title(), t.app_quit_confirm());
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
            OPTIONS_SUBMENU_THEMES => self.handle_themes_nested_submenu_key(key),
            OPTIONS_SUBMENU_LANGUAGE => self.handle_language_nested_submenu_key(key),
            _ => Ok(()),
        }
    }

    /// Navigate nested submenu selection up/down with wrapping.
    fn navigate_nested_submenu(&mut self, key_code: KeyCode, count: usize) {
        match key_code {
            KeyCode::Up => {
                if self.state.ui.nested_submenu.selected > 0 {
                    self.state.ui.nested_submenu.selected -= 1;
                } else {
                    self.state.ui.nested_submenu.selected = count.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if count > 0 {
                    self.state.ui.nested_submenu.selected =
                        (self.state.ui.nested_submenu.selected + 1) % count;
                }
            }
            _ => {}
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
            KeyCode::Up | KeyCode::Down => {
                self.navigate_nested_submenu(key.code, theme_count);
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
            KeyCode::Right => {
                // Restore original theme and switch to next root menu
                if let Some(original_name) = self.state.ui.theme_preview_original.take() {
                    self.state.theme = Theme::get_by_name(&original_name);
                }
                self.switch_to_next_menu()?;
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
            KeyCode::Up | KeyCode::Down => {
                self.navigate_nested_submenu(key.code, lang_count);
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
            KeyCode::Right => {
                // Restore original language and switch to next root menu
                if let Some(original_lang) = self.state.ui.language_preview_original.take() {
                    let _ = i18n::set_language(&original_lang);
                }
                self.switch_to_next_menu()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Apply language by code and save preference
    pub(super) fn apply_language(&mut self, lang_code: &str, lang_name: &str) -> Result<()> {
        if let Err(e) = i18n::set_language(lang_code) {
            log::warn!("Failed to set language: {}", e);
            self.state
                .set_error(format!("Failed to set language: {}", e));
            return Ok(());
        }

        let t = i18n::t();
        self.state.set_info(t.language_changed(lang_name));

        // Save preference to config file
        if let Err(e) = self.save_language_preference(lang_code) {
            log::warn!("Failed to save language preference: {}", e);
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

        match navigate_submenu(
            &key,
            &mut self.state.ui.sessions_submenu,
            SESSIONS_SUBMENU_ITEM_COUNT,
            &[],
        ) {
            SubmenuNavAction::Close => self.state.close_menu(),
            SubmenuNavAction::Execute => self.execute_sessions_submenu_action()?,
            SubmenuNavAction::Right => self.switch_to_next_menu()?,
            SubmenuNavAction::Left => self.switch_to_prev_menu()?,
            SubmenuNavAction::Edit => {}
            SubmenuNavAction::None => {}
        }
        Ok(())
    }

    /// Execute action for selected Sessions submenu item
    pub(super) fn execute_sessions_submenu_action(&mut self) -> Result<()> {
        match self.state.ui.sessions_submenu.selected {
            SESSIONS_SUBMENU_NEW => {
                self.state.close_menu();
                self.handle_new_session()?;
            }
            SESSIONS_SUBMENU_SWITCH => {
                self.state.close_menu();
                self.handle_open_sessions_modal()?;
            }
            SESSIONS_SUBMENU_CHANGE_ROOT => {
                self.state.close_menu();
                self.handle_change_root_path()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Open directory picker for creating new session
    pub(super) fn handle_new_session(&mut self) -> Result<()> {
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
            log::warn!("Failed to save theme preference: {}", e);
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
        // If shell picker nested submenu is open, delegate to it
        if self.state.ui.tools_nested.open {
            return self.handle_tools_nested_submenu_key(key);
        }

        use termide_ui_render::TOOLS_SUBMENU_ITEM_COUNT;

        match navigate_submenu(
            &key,
            &mut self.state.ui.tools_submenu,
            TOOLS_SUBMENU_ITEM_COUNT,
            &[],
        ) {
            SubmenuNavAction::Close => self.state.close_menu(),
            SubmenuNavAction::Execute => self.execute_tools_submenu_action()?,
            SubmenuNavAction::Right => {
                // Terminal (index 0) has submenu
                if self.state.ui.tools_submenu.selected == TOOLS_SUBMENU_TERMINAL {
                    self.execute_tools_submenu_action()?;
                } else {
                    self.switch_to_next_menu()?;
                }
            }
            SubmenuNavAction::Left => self.switch_to_prev_menu()?,
            SubmenuNavAction::Edit => {}
            SubmenuNavAction::None => {}
        }
        Ok(())
    }

    /// Handle keyboard event in Tools nested submenu (shell picker)
    fn handle_tools_nested_submenu_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        let item_count = self.state.cached_shells.len();
        if item_count == 0 {
            self.state.close_tools_nested_submenu();
            return Ok(());
        }

        match navigate_submenu(&key, &mut self.state.ui.tools_nested, item_count, &[]) {
            SubmenuNavAction::Close => self.state.close_tools_nested_submenu(),
            SubmenuNavAction::Execute => {
                if let Some(shell) = self
                    .state
                    .cached_shells
                    .get(self.state.ui.tools_nested.selected)
                {
                    let shell_path = shell.path.clone();
                    // Save as default (copy-on-write)
                    {
                        let mut config = (*self.state.config).clone();
                        config.terminal.default_shell = Some(shell_path.clone());
                        self.state.config = Arc::new(config);
                    }
                    if let Err(e) = self.save_shell_preference(&shell_path) {
                        log::warn!("Failed to save shell preference: {}", e);
                    }
                    self.state.close_menu();
                    self.handle_new_terminal_with_shell(Some(&shell_path))?;
                }
            }
            SubmenuNavAction::Right => self.switch_to_next_menu()?,
            SubmenuNavAction::Left => self.state.close_tools_nested_submenu(),
            SubmenuNavAction::Edit => {}
            SubmenuNavAction::None => {}
        }
        Ok(())
    }

    /// Execute action for selected Tools submenu item
    pub(super) fn execute_tools_submenu_action(&mut self) -> Result<()> {
        match self.state.ui.tools_submenu.selected {
            TOOLS_SUBMENU_TERMINAL => {
                self.state.open_tools_nested_submenu(0);
                let default_idx = self
                    .state
                    .config
                    .terminal
                    .default_shell
                    .as_ref()
                    .and_then(|default| {
                        self.state
                            .cached_shells
                            .iter()
                            .position(|s| s.path == *default)
                    })
                    .unwrap_or(0);
                self.state.ui.tools_nested.selected = default_idx;
            }
            TOOLS_SUBMENU_FILES => {
                self.state.close_menu();
                self.handle_new_file_manager()?;
            }
            TOOLS_SUBMENU_EDITOR => {
                self.state.close_menu();
                self.handle_new_editor()?;
            }
            TOOLS_SUBMENU_GIT_STATUS => {
                self.state.close_menu();
                self.handle_open_git_status()?;
            }
            TOOLS_SUBMENU_GIT_LOG => {
                self.state.close_menu();
                self.handle_open_git_log()?;
            }
            TOOLS_SUBMENU_GIT_STASH => {
                self.state.close_menu();
                self.handle_open_git_stash()?;
            }
            TOOLS_SUBMENU_JOURNAL => {
                self.state.close_menu();
                self.handle_new_journal()?;
            }
            TOOLS_SUBMENU_DIAGNOSTICS => {
                self.state.close_menu();
                self.handle_open_diagnostics()?;
            }
            TOOLS_SUBMENU_OPERATIONS => {
                self.state.close_menu();
                self.open_operations_panel()?;
            }
            TOOLS_SUBMENU_OUTLINE => {
                self.state.close_menu();
                self.handle_open_outline()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Notify outline panel that a file was opened/switched.
    pub(crate) fn notify_outline_file_opened(&mut self) {
        let editor_info = self.collect_editor_info_for_outline();
        if let Some((path, content, language, cursor_line)) = editor_info {
            self.push_to_outline(path, &content, language.as_deref(), Some(cursor_line));
        }
    }

    /// Re-sync outline after a panel close: rebind to another editor or clear.
    pub(super) fn resync_outline_after_close(&mut self) {
        // 1. Try the now-active panel (may be the next editor in stack)
        if self.collect_editor_info_for_outline().is_some() {
            self.notify_outline_file_opened();
            return;
        }
        // 2. Try any editor remaining in layout
        let has_editor = self
            .layout_manager
            .iter_all_panels_mut()
            .any(|p| p.as_editor().is_some());
        if has_editor {
            self.populate_outline_from_any_editor();
            return;
        }
        // 3. No editors — clear outline
        for group in &mut self.layout_manager.panel_groups {
            for panel in group.panels_mut() {
                if let Some(outline) = panel
                    .as_any_mut()
                    .downcast_mut::<termide_panel_outline::OutlinePanel>()
                {
                    outline.clear();
                    return;
                }
            }
        }
    }

    /// Collect editor data for outline (extracted for reuse).
    ///
    /// Only returns data when the active panel is an editor.
    /// Switching to non-editor panels keeps the outline bound to the last editor.
    fn collect_editor_info_for_outline(
        &mut self,
    ) -> Option<(Option<std::path::PathBuf>, String, Option<String>, usize)> {
        let panel = self.layout_manager.active_panel_mut()?;
        let editor = panel.as_editor_mut()?;
        let path = editor.file_path().map(|p| p.to_path_buf());
        let content = editor.content_string();
        let cursor_line = editor.cursor_line();
        let language = path
            .as_ref()
            .and_then(|p| termide_highlight::detect_language(p))
            .map(|s| s.to_string());
        Some((path, content, language, cursor_line))
    }

    /// Lightweight check for live editing — only compare edit_version, debounced 1s.
    pub(super) fn check_outline_live_edit(&mut self) {
        let needs_repopulate = self
            .layout_manager
            .panel_groups
            .iter_mut()
            .flat_map(|g| g.panels_mut())
            .find_map(|p| {
                p.as_any_mut()
                    .downcast_mut::<termide_panel_outline::OutlinePanel>()
            })
            .is_some_and(|outline| outline.needs_repopulate());
        if needs_repopulate {
            self.populate_outline_from_any_editor();
            return;
        }

        let Some(panel) = self.layout_manager.active_panel_mut() else {
            return;
        };
        let Some(editor) = panel.as_editor_mut() else {
            return;
        };

        let version = editor.edit_version();
        if version == self.outline_last_version {
            // No edits — also sync cursor cheaply
            let cursor = editor.cursor_line();
            if cursor != self.outline_last_cursor {
                self.outline_last_cursor = cursor;
                self.sync_outline_cursor(cursor);
            }
            return;
        }

        // Version changed — check debounce (1 second since last update)
        let now = std::time::Instant::now();
        if let Some(last) = self.outline_last_edit_time {
            if now.duration_since(last) < std::time::Duration::from_secs(1) {
                return; // Too soon, wait
            }
        }

        self.outline_last_version = version;
        self.outline_last_cursor = editor.cursor_line();
        self.outline_last_edit_time = Some(now);

        // Only now clone content
        let content = editor.content_string();
        let path = editor.file_path().map(|p| p.to_path_buf());
        let language = path
            .as_ref()
            .and_then(|p| termide_highlight::detect_language(p))
            .map(|s| s.to_string());
        self.push_to_outline(
            path,
            &content,
            language.as_deref(),
            Some(self.outline_last_cursor),
        );
    }

    /// Sync only cursor position to outline (no content extraction).
    fn sync_outline_cursor(&mut self, cursor_line: usize) {
        for group in &mut self.layout_manager.panel_groups {
            for panel in group.panels_mut() {
                if let Some(outline) = panel
                    .as_any_mut()
                    .downcast_mut::<termide_panel_outline::OutlinePanel>()
                {
                    outline.sync_cursor_line(cursor_line);
                    return;
                }
            }
        }
    }

    /// Re-extract outline symbols when the tracked file changed on disk.
    pub(super) fn notify_outline_on_fs_change(
        &mut self,
        changed_paths: &std::collections::HashSet<std::path::PathBuf>,
    ) {
        if changed_paths.is_empty() {
            return;
        }
        // Check if outline tracks one of the changed files
        let tracked: Option<std::path::PathBuf> = self.find_outline_tracked_file();
        let Some(tracked_path) = tracked else {
            return;
        };
        if !changed_paths.contains(&tracked_path) {
            return;
        }
        // File changed on disk — re-extract from editor's current content
        self.notify_outline_file_opened();
    }

    /// Find the file path currently tracked by the outline panel.
    fn find_outline_tracked_file(&self) -> Option<std::path::PathBuf> {
        for group in &self.layout_manager.panel_groups {
            for panel in group.panels() {
                if let Some(outline) = panel
                    .as_any()
                    .downcast_ref::<termide_panel_outline::OutlinePanel>()
                {
                    return outline.tracked_file().map(|p| p.to_path_buf());
                }
            }
        }
        None
    }

    /// Populate the outline panel from any editor found in the layout.
    /// Used on first open when the outline itself may already be focused.
    pub(super) fn populate_outline_from_any_editor(&mut self) {
        let editor_info: Option<(Option<std::path::PathBuf>, String, Option<String>)> = {
            let mut info = None;
            for panel in self.layout_manager.iter_all_panels_mut() {
                if let Some(editor) = panel.as_editor_mut() {
                    let path = editor.file_path().map(|p| p.to_path_buf());
                    let content = editor.content_string();
                    let language = path
                        .as_ref()
                        .and_then(|p| termide_highlight::detect_language(p))
                        .map(|s| s.to_string());
                    info = Some((path, content, language));
                    break;
                }
            }
            info
        };

        if let Some((path, content, language)) = editor_info {
            self.push_to_outline(path, &content, language.as_deref(), None);
        }
    }

    /// Apply pending outline navigation to the editor (called from tick).
    pub(super) fn apply_outline_navigation(&mut self) {
        // Collect pending navigation from outline panel
        let nav: Option<termide_panel_outline::OutlineNavigation> = {
            let mut result = None;
            for group in &mut self.layout_manager.panel_groups {
                for panel in group.panels_mut() {
                    if let Some(outline) = panel
                        .as_any_mut()
                        .downcast_mut::<termide_panel_outline::OutlinePanel>()
                    {
                        result = outline.take_pending_navigation();
                        break;
                    }
                }
                if result.is_some() {
                    break;
                }
            }
            result
        };

        // Find the matching editor, expand it if collapsed, and navigate
        if let Some(nav) = nav {
            let mut target: Option<(usize, usize)> = None;
            for (gi, group) in self.layout_manager.panel_groups.iter().enumerate() {
                for (pi, panel) in group.panels().iter().enumerate() {
                    if let Some(editor) = panel.as_editor() {
                        if editor.file_path() == Some(&nav.path) {
                            target = Some((gi, pi));
                            break;
                        }
                    }
                }
                if target.is_some() {
                    break;
                }
            }

            if let Some((gi, pi)) = target {
                // Expand the editor panel if it's collapsed
                if let Some(group) = self.layout_manager.panel_groups.get_mut(gi) {
                    group.set_expanded(pi);
                }
                // Now navigate
                if let Some(group) = self.layout_manager.panel_groups.get_mut(gi) {
                    if let Some(panel) = group.panels_mut().get_mut(pi) {
                        if let Some(editor) = panel.as_editor_mut() {
                            editor.goto_position(nav.line, nav.column);
                        }
                    }
                }
            }
        }
    }

    /// Push collected editor data into the outline panel (if it exists).
    fn push_to_outline(
        &mut self,
        path: Option<std::path::PathBuf>,
        content: &str,
        language: Option<&str>,
        cursor_line: Option<usize>,
    ) {
        let mut symbol_lines_for_editor = Vec::new();
        'outer: for group in &mut self.layout_manager.panel_groups {
            for panel in group.panels_mut() {
                if let Some(outline) = panel
                    .as_any_mut()
                    .downcast_mut::<termide_panel_outline::OutlinePanel>()
                {
                    outline.update_content(path, content, language);
                    if let Some(line) = cursor_line {
                        outline.sync_cursor_line(line);
                    }
                    symbol_lines_for_editor = outline.symbol_lines();
                    break 'outer;
                }
            }
        }
        if let Some(panel) = self.layout_manager.active_panel_mut() {
            if let Some(editor) = panel.as_editor_mut() {
                editor.set_symbol_lines(symbol_lines_for_editor);
            }
        }
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
        // 2 = "Manage scripts" + separator; then scripts or "Add script..."
        let scripts_count = registry
            .as_ref()
            .map(|r| {
                let n = r.root_items.len() + r.groups.len();
                if n == 0 {
                    1
                } else {
                    n
                } // "Add script..." if empty
            })
            .unwrap_or(1);
        let item_count = 2 + scripts_count;

        match navigate_submenu(&key, &mut self.state.ui.scripts_submenu, item_count, &[1]) {
            SubmenuNavAction::Close => self.state.close_menu(),
            SubmenuNavAction::Execute => self.execute_scripts_submenu_action()?,
            SubmenuNavAction::Right => {
                // Groups have submenu — check if selected is a group
                let sel = self.state.ui.scripts_submenu.selected;
                let root_count = registry.as_ref().map(|r| r.root_items.len()).unwrap_or(0);
                if sel >= 2 + root_count {
                    // Group item — open nested
                    self.execute_scripts_submenu_action()?;
                } else {
                    self.switch_to_next_menu()?;
                }
            }
            SubmenuNavAction::Left => self.switch_to_prev_menu()?,
            SubmenuNavAction::Edit => self.edit_selected_script()?,
            SubmenuNavAction::None => {}
        }
        Ok(())
    }

    /// Execute action for selected Scripts submenu item
    pub(super) fn execute_scripts_submenu_action(&mut self) -> Result<()> {
        let selected = self.state.ui.scripts_submenu.selected;

        // Index 0: "Manage scripts"
        if selected == 0 {
            self.state.close_menu();
            self.handle_manage_scripts()?;
            return Ok(());
        }

        // Index 1: separator (should not be reachable)
        // Indices 2+: actual scripts

        let registry = match termide_config::scripts::ScriptsRegistry::load() {
            Some(r) => r,
            None => return Ok(()),
        };

        // Check if "Add script..." is shown (empty registry)
        if registry.root_items.is_empty() && registry.groups.is_empty() {
            // Only "Add script..." at index 2
            self.state.close_menu();
            self.handle_manage_scripts()?;
            return Ok(());
        }

        // Offset by 2 (manage + separator)
        let adjusted = selected.saturating_sub(2);
        let root_count = registry.root_items.len();

        if adjusted < root_count {
            if let Some(script) = registry.root_items.get(adjusted) {
                self.state.close_menu();
                self.run_script(script)?;
            }
        } else {
            let group_idx = adjusted - root_count;
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

        match navigate_submenu(&key, &mut self.state.ui.scripts_nested, item_count, &[]) {
            SubmenuNavAction::Close | SubmenuNavAction::Left => {
                self.state.close_scripts_nested_submenu();
            }
            SubmenuNavAction::Execute => self.execute_scripts_nested_action()?,
            SubmenuNavAction::Right => self.switch_to_next_menu()?,
            SubmenuNavAction::Edit => self.edit_selected_nested_script()?,
            SubmenuNavAction::None => {}
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

    /// Open selected script in editor (F4 from scripts submenu)
    fn edit_selected_script(&mut self) -> Result<()> {
        let selected = self.state.ui.scripts_submenu.selected;

        // Index 0: Manage scripts — open scripts folder
        if selected == 0 {
            self.state.close_menu();
            self.handle_manage_scripts()?;
            return Ok(());
        }

        // Index 1: separator, Index 2+: scripts
        if let Some(registry) = termide_config::scripts::ScriptsRegistry::load() {
            let adjusted = selected.saturating_sub(2);
            let root_count = registry.root_items.len();
            if adjusted < root_count {
                if let Some(script) = registry.root_items.get(adjusted) {
                    self.state.close_menu();
                    let _ = self.open_editor_for_file(script.path.clone());
                }
            }
            // Groups can't be edited directly
        }
        Ok(())
    }

    /// Open selected nested script in editor (F4 from scripts nested submenu)
    fn edit_selected_nested_script(&mut self) -> Result<()> {
        let registry = match termide_config::scripts::ScriptsRegistry::load() {
            Some(r) => r,
            None => return Ok(()),
        };
        let group_name = match &self.state.ui.current_scripts_group {
            Some(name) => name.clone(),
            None => return Ok(()),
        };
        if let Some(group) = registry.groups.iter().find(|g| g.name == group_name) {
            if let Some(script) = group.items.get(self.state.ui.scripts_nested.selected) {
                self.state.close_menu();
                let _ = self.open_editor_for_file(script.path.clone());
            }
        }
        Ok(())
    }

    /// Open bookmarks config in editor (F4 from bookmarks submenu)
    fn edit_bookmarks_config(&mut self) -> Result<()> {
        self.state.close_menu();
        self.handle_manage_bookmarks()?;
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
            log::info!("Running background script '{}' in {:?}", script.name, cwd);
            match shell_command(&script.path, &cwd)
                .current_dir(&cwd)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .stdin(std::process::Stdio::null())
                .spawn()
            {
                Ok(_) => {}
                Err(e) => {
                    log::error!("Failed to run background script '{}': {}", script.name, e);
                    self.state.set_error(format!("Failed to run script: {}", e));
                }
            }
        } else {
            // Run in new terminal panel
            log::info!("Running script '{}' in {:?}", script.name, cwd);

            self.close_help_panels();

            let width = self.state.terminal.width;
            let height = self.state.terminal.height;
            let term_height = height.saturating_sub(3);
            let term_width = width.saturating_sub(2);

            let command = shell_quote(&script.path);

            match Terminal::new_with_cwd(term_height, term_width, Some(cwd)) {
                Ok(mut terminal) => {
                    let _ = terminal.send_command(&command);
                    self.add_panel(Box::new(terminal));
                    self.auto_save_session();
                }
                Err(e) => {
                    log::error!(
                        "Failed to create terminal for script '{}': {}",
                        script.name,
                        e
                    );
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

        log::info!("Running report script '{}' in {:?}", script.name, cwd);

        let child = shell_command(&script.path, cwd)
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
                log::error!("Failed to run report script '{}': {}", script.name, e);
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

        match navigate_submenu(&key, &mut self.state.ui.bookmarks_submenu, item_count, &[2]) {
            SubmenuNavAction::Close => self.state.close_menu(),
            SubmenuNavAction::Execute => self.execute_bookmarks_submenu_action()?,
            SubmenuNavAction::Right => {
                // Groups have submenu — check if selected is a group (indices 3..3+groups_count)
                let sel = self.state.ui.bookmarks_submenu.selected;
                let groups_count = self.state.bookmarks.named_groups().len();
                if sel >= 3 && sel < 3 + groups_count {
                    self.execute_bookmarks_submenu_action()?;
                } else {
                    self.switch_to_next_menu()?;
                }
            }
            SubmenuNavAction::Left => self.switch_to_prev_menu()?,
            SubmenuNavAction::Edit => self.edit_bookmarks_config()?,
            SubmenuNavAction::None => {}
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

        if selected == 1 {
            // Manage bookmarks
            self.state.close_menu();
            self.handle_manage_bookmarks()?;
            return Ok(());
        }

        // Index 2: separator (should not be reachable)
        // Indices 3+: actual bookmarks

        // Get groups and ungrouped counts
        let named_groups: Vec<String> = self
            .state
            .bookmarks
            .named_groups()
            .keys()
            .cloned()
            .collect();
        let ungrouped = self.state.bookmarks.ungrouped();
        let groups_start = 3; // after add + manage + separator
        let ungrouped_start = groups_start + named_groups.len();

        if selected >= groups_start && selected < ungrouped_start {
            // Group selected - open nested submenu
            let group_idx = selected - groups_start;
            if let Some(group_name) = named_groups.get(group_idx) {
                self.state.open_bookmarks_nested_submenu(group_name.clone());
            }
        } else if selected >= ungrouped_start {
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

        match navigate_submenu(&key, &mut self.state.ui.bookmarks_nested, item_count, &[]) {
            SubmenuNavAction::Close | SubmenuNavAction::Left => {
                self.state.close_bookmarks_nested_submenu();
            }
            SubmenuNavAction::Execute => self.execute_bookmarks_nested_action()?,
            SubmenuNavAction::Right => self.switch_to_next_menu()?,
            SubmenuNavAction::Edit => self.edit_bookmarks_config()?,
            SubmenuNavAction::None => {}
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

        self.close_help_panels();

        // Get the bookmarks file path
        let bookmarks_path = match BookmarksConfig::config_file_path() {
            Ok(path) => {
                // Create the file if it doesn't exist
                if !path.exists() {
                    // Ensure parent directory exists
                    if let Some(parent) = path.parent() {
                        if !parent.exists() {
                            if let Err(e) = std::fs::create_dir_all(parent) {
                                log::warn!("Failed to create data directory: {}", e);
                            }
                        }
                    }
                    // Create empty bookmarks file
                    let empty_config = BookmarksConfig::default();
                    if let Err(e) = empty_config.save() {
                        log::warn!("Failed to create bookmarks file: {}", e);
                    }
                }
                path
            }
            Err(e) => {
                log::warn!("Failed to get bookmarks path: {}", e);
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
                        self.state.needs_watcher_registration = true;
                        return Ok(());
                    }
                }
                // No active file manager - create new panel
                self.close_help_panels();
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
                            && after_colon.parse::<u16>().is_ok_and(|p| p > 0)
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

                self.close_help_panels();
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
                        self.state.needs_watcher_registration = true;
                        return Ok(());
                    }
                }
                // No active file manager - create new panel and navigate
                self.close_help_panels();
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

    /// Open the command palette modal.
    pub(super) fn handle_open_command_palette(&mut self) -> Result<()> {
        use termide_app_event::HotkeyAction;
        use termide_modal::{ActiveModal, CommandEntry, CommandPaletteModal};
        use termide_state::PendingAction;

        let kb = &self.state.config.general.keybindings;

        let kb_str = |b: &Option<termide_config::KeyBinding>| {
            b.as_ref()
                .map(|k| k.display().to_string())
                .unwrap_or_default()
        };

        // Build paired lists: actions Vec and display entries Vec.
        // Order: Panels, Git, Navigation, Panel Management, Application.
        let commands: Vec<(HotkeyAction, CommandEntry)> = vec![
            (
                HotkeyAction::NewEditor,
                CommandEntry {
                    label: "New Editor".into(),
                    category: "Panels",
                    keybinding: kb_str(&kb.new_editor),
                },
            ),
            (
                HotkeyAction::NewFileManager,
                CommandEntry {
                    label: "New File Manager".into(),
                    category: "Panels",
                    keybinding: kb_str(&kb.new_file_manager),
                },
            ),
            (
                HotkeyAction::NewTerminal,
                CommandEntry {
                    label: "New Terminal".into(),
                    category: "Panels",
                    keybinding: kb_str(&kb.new_terminal),
                },
            ),
            (
                HotkeyAction::NewJournal,
                CommandEntry {
                    label: "New Journal".into(),
                    category: "Panels",
                    keybinding: kb_str(&kb.new_journal),
                },
            ),
            (
                HotkeyAction::OpenHelp,
                CommandEntry {
                    label: "Open Help".into(),
                    category: "Panels",
                    keybinding: kb_str(&kb.open_help),
                },
            ),
            (
                HotkeyAction::OpenPreferences,
                CommandEntry {
                    label: "Open Preferences".into(),
                    category: "Panels",
                    keybinding: kb_str(&kb.open_preferences),
                },
            ),
            (
                HotkeyAction::OpenGitStatus,
                CommandEntry {
                    label: "Open Git Status".into(),
                    category: "Git",
                    keybinding: kb_str(&kb.open_git_status),
                },
            ),
            (
                HotkeyAction::OpenGitLog,
                CommandEntry {
                    label: "Open Git Log".into(),
                    category: "Git",
                    keybinding: kb_str(&kb.open_git_log),
                },
            ),
            (
                HotkeyAction::OpenSessions,
                CommandEntry {
                    label: "Open Sessions".into(),
                    category: "Navigation",
                    keybinding: kb_str(&kb.open_sessions),
                },
            ),
            (
                HotkeyAction::OpenDirectorySwitcher,
                CommandEntry {
                    label: "Switch Directory".into(),
                    category: "Navigation",
                    keybinding: kb_str(
                        &self.state.config.file_manager.keybindings.switch_directory,
                    ),
                },
            ),
            (
                HotkeyAction::OpenOutline,
                CommandEntry {
                    label: "Open Outline".into(),
                    category: "Navigation",
                    keybinding: kb_str(&kb.open_outline),
                },
            ),
            (
                HotkeyAction::OpenDiagnostics,
                CommandEntry {
                    label: "Open Diagnostics".into(),
                    category: "Navigation",
                    keybinding: kb_str(&kb.open_diagnostics),
                },
            ),
            (
                HotkeyAction::OpenBookmarkAdd,
                CommandEntry {
                    label: "Add Bookmark".into(),
                    category: "Navigation",
                    keybinding: kb_str(&kb.open_bookmark_add),
                },
            ),
            (
                HotkeyAction::ClosePanel,
                CommandEntry {
                    label: "Close Panel".into(),
                    category: "Panel Management",
                    keybinding: kb_str(&kb.close_panel),
                },
            ),
            (
                HotkeyAction::ToggleStacking,
                CommandEntry {
                    label: "Toggle Stacking".into(),
                    category: "Panel Management",
                    keybinding: kb_str(&kb.toggle_stack),
                },
            ),
            (
                HotkeyAction::SwapPanelLeft,
                CommandEntry {
                    label: "Move Panel Left".into(),
                    category: "Panel Management",
                    keybinding: kb_str(&kb.swap_left),
                },
            ),
            (
                HotkeyAction::SwapPanelRight,
                CommandEntry {
                    label: "Move Panel Right".into(),
                    category: "Panel Management",
                    keybinding: kb_str(&kb.swap_right),
                },
            ),
            (
                HotkeyAction::MoveToFirst,
                CommandEntry {
                    label: "Move to First".into(),
                    category: "Panel Management",
                    keybinding: kb_str(&kb.move_first),
                },
            ),
            (
                HotkeyAction::MoveToLast,
                CommandEntry {
                    label: "Move to Last".into(),
                    category: "Panel Management",
                    keybinding: kb_str(&kb.move_last),
                },
            ),
            (
                HotkeyAction::RequestQuit,
                CommandEntry {
                    label: "Quit".into(),
                    category: "Application",
                    keybinding: kb_str(&kb.quit),
                },
            ),
            (
                HotkeyAction::ToggleMenu,
                CommandEntry {
                    label: "Toggle Menu".into(),
                    category: "Application",
                    keybinding: kb_str(&kb.toggle_menu),
                },
            ),
        ];

        let (actions, entries): (Vec<HotkeyAction>, Vec<CommandEntry>) =
            commands.into_iter().unzip();

        self.command_palette_actions = Some(actions);

        let modal = CommandPaletteModal::new(entries);
        self.state.set_pending_action(
            PendingAction::CommandPalette,
            ActiveModal::CommandPalette(Box::new(modal)),
        );

        Ok(())
    }
}

/// Shell-quote a path for safe use in terminal commands.
#[cfg(unix)]
fn shell_quote(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(not(unix))]
fn shell_quote(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// Cached environment variables with timestamp for TTL expiry.
#[cfg(unix)]
type EnvCache = HashMap<PathBuf, (HashMap<String, String>, Instant)>;

/// Cache for project environment variables, keyed by directory path.
#[cfg(unix)]
static DIR_ENV_CACHE: Mutex<Option<EnvCache>> = Mutex::new(None);

#[cfg(unix)]
const ENV_CACHE_TTL: Duration = Duration::from_secs(300);

/// Check if direnv is available in PATH (cached after first call).
#[cfg(unix)]
fn has_direnv() -> bool {
    use std::sync::OnceLock;
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        std::process::Command::new("direnv")
            .arg("version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// Get project environment for a given directory (with caching).
/// Uses `direnv exec <cwd> env` when direnv is available,
/// otherwise `$SHELL -lc env` for login shell environment.
#[cfg(unix)]
fn get_project_env(cwd: &std::path::Path) -> Option<HashMap<String, String>> {
    // Check cache
    if let Ok(guard) = DIR_ENV_CACHE.lock() {
        if let Some(cache) = guard.as_ref() {
            if let Some((env, ts)) = cache.get(cwd) {
                if ts.elapsed() < ENV_CACHE_TTL {
                    return Some(env.clone());
                }
            }
        }
    }

    // Capture environment from the project directory
    let output = if has_direnv() {
        std::process::Command::new("direnv")
            .arg("exec")
            .arg(cwd)
            .arg("env")
            .current_dir(cwd)
            .stderr(std::process::Stdio::null())
            .output()
    } else {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        std::process::Command::new(shell)
            .arg("-lc")
            .arg("env")
            .current_dir(cwd)
            .stderr(std::process::Stdio::null())
            .output()
    };

    let env_map: HashMap<String, String> = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once('=')?;
                Some((key.to_string(), value.to_string()))
            })
            .collect(),
        _ => return None,
    };

    // Update cache
    if let Ok(mut guard) = DIR_ENV_CACHE.lock() {
        let cache = guard.get_or_insert_with(HashMap::new);
        cache.insert(cwd.to_path_buf(), (env_map.clone(), Instant::now()));
    }

    Some(env_map)
}

/// Create a Command that runs a script through the user's shell.
/// On Unix: loads project environment (via direnv or login shell) and
/// runs the script with cached env vars. No direnv noise in stdout/stderr.
/// On Windows: cmd.exe /C "script_path"
fn shell_command(script_path: &std::path::Path, cwd: &std::path::Path) -> std::process::Command {
    #[cfg(unix)]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = std::process::Command::new(&shell);
        cmd.arg("-c").arg(shell_quote(script_path));

        if let Some(env) = get_project_env(cwd) {
            cmd.env_clear();
            cmd.envs(env);
        }

        cmd
    }
    #[cfg(not(unix))]
    {
        let _ = cwd;
        let mut cmd = std::process::Command::new("cmd.exe");
        cmd.arg("/C").arg(shell_quote(script_path));
        cmd
    }
}
