//! Mouse event handling for the application.

use anyhow::Result;
use crossterm::event::{MouseButton, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use unicode_width::UnicodeWidthStr;

use super::App;
use termide_config::Config;
use termide_i18n as i18n;
use termide_logger as logger;
use termide_theme::Theme;
use termide_ui_render::{
    get_actions_group_items, get_actions_items, get_menu_item_x_position, get_options_items,
    get_sessions_items, get_tools_items, ACTIONS_MENU_INDEX, OPTIONS_MENU_INDEX,
    SESSIONS_MENU_INDEX, TOOLS_MENU_INDEX,
};

impl App {
    /// Apply language by code and save preference (for mouse handler)
    fn apply_language_mouse(&mut self, lang_code: &str, lang_name: &str) -> Result<()> {
        if let Err(e) = i18n::set_language(lang_code) {
            logger::warn(format!("Failed to set language: {}", e));
            self.state
                .set_error(format!("Failed to set language: {}", e));
            return Ok(());
        }

        let t = i18n::t();
        self.state.set_info(t.language_changed(lang_name));

        // Save preference to config file
        if let Err(e) = self.save_language_preference_mouse(lang_code) {
            logger::warn(format!("Failed to save language preference: {}", e));
        }

        Ok(())
    }

    /// Save language preference to config file (for mouse handler)
    fn save_language_preference_mouse(&self, lang_code: &str) -> Result<()> {
        let mut config = Config::load()?;
        config.general.language = lang_code.to_string();
        config.save()?;
        Ok(())
    }
}

impl App {
    /// Handle mouse event
    pub(super) fn handle_mouse_event(&mut self, mouse: crossterm::event::MouseEvent) -> Result<()> {
        // Handle divider drag first (highest priority for smooth resize)
        if self.state.ui.drag.is_dragging() {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    self.handle_divider_drag(mouse.column)?;
                    return Ok(());
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.handle_divider_drag_end()?;
                    return Ok(());
                }
                _ => {}
            }
        }

        // Handle modal mouse events first if a modal is open
        if self.state.active_modal.is_some() {
            let modal_area = Rect {
                x: 0,
                y: 0,
                width: self.state.terminal.width,
                height: self.state.terminal.height,
            };
            self.handle_modal_mouse(mouse, modal_area)?;
            return Ok(());
        }

        // Scroll events should reach panels when no modal is active
        // This allows scrolling terminal history, editor, etc.
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) {
            self.forward_scroll_to_panel_at_cursor(mouse)?;
            return Ok(());
        }

        // Click on menu
        if mouse.row == 0 && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            self.handle_menu_click(mouse.column)?;
            return Ok(());
        }

        // Handle Sessions submenu clicks when it's open
        if self.state.ui.sessions_submenu.open
            && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && self.handle_sessions_submenu_click(mouse.column, mouse.row)?
        {
            return Ok(());
        }

        // Handle Preferences submenu clicks when submenu is open
        if self.state.ui.options_submenu.open
            && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && self.handle_submenu_click(mouse.column, mouse.row)?
        {
            return Ok(());
        }

        // Handle Tools submenu clicks when it's open
        if self.state.ui.tools_submenu.open
            && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && self.handle_tools_submenu_click(mouse.column, mouse.row)?
        {
            return Ok(());
        }

        // Handle Actions submenu clicks when it's open
        if self.state.ui.actions_submenu.open
            && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && self.handle_actions_submenu_click(mouse.column, mouse.row)?
        {
            return Ok(());
        }

        // If menu is open, close it on click outside menu
        if self.state.ui.menu_open && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
        {
            self.state.close_menu();
            return Ok(());
        }

        // Check click on divider for resize (before panel click handling)
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && self.handle_divider_click(mouse.column, mouse.row)?
        {
            return Ok(());
        }

        // Check click on panel [X] button
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            if self.handle_panel_close_click(mouse.column, mouse.row)? {
                return Ok(());
            }

            // Check click on panel to switch focus
            self.handle_panel_focus_click(mouse.column, mouse.row)?;
        }

        // Scroll events handled at the top of this function (before modal check)

        // Other mouse events - to active panel
        self.forward_mouse_to_panel(mouse)?;

        Ok(())
    }

    /// Forward mouse event to active panel
    fn forward_mouse_to_panel(&mut self, mouse: crossterm::event::MouseEvent) -> Result<()> {
        use crate::panel_ext::PanelExt;

        // Determine active panel area
        let panel_area = self.get_active_panel_area();

        // Handle mouse event and collect results
        let (events, modal_request) = if let Some(panel) = self.layout_manager.active_panel_mut() {
            let events = panel.handle_mouse(mouse, panel_area);
            let modal_request = panel.take_modal_request();
            (events, modal_request)
        } else {
            (vec![], None)
        };

        // Process panel events (new event-based architecture)
        self.process_panel_events(events)?;

        // Handle modal window request from panel (legacy, still used)
        if let Some((action, modal)) = modal_request {
            self.handle_modal_request(action, modal)?;
        }

        Ok(())
    }

    /// Forward scroll to panel under mouse cursor
    fn forward_scroll_to_panel_at_cursor(
        &mut self,
        mouse: crossterm::event::MouseEvent,
    ) -> Result<()> {
        let panel_rects = self.calculate_panel_rects();

        for (group_idx, _panel_idx, rect, is_expanded) in panel_rects {
            // Skip collapsed panels
            if !is_expanded {
                continue;
            }

            // Check if mouse is within this panel's area
            if mouse.column >= rect.x
                && mouse.column < rect.x + rect.width
                && mouse.row >= rect.y
                && mouse.row < rect.y + rect.height
            {
                if let Some(group) = self.layout_manager.panel_groups.get_mut(group_idx) {
                    if let Some(panel) = group.expanded_panel_mut() {
                        // handle_mouse returns Vec<PanelEvent>
                        let events = panel.handle_mouse(mouse, rect);
                        self.process_panel_events(events)?;
                    }
                }
                break;
            }
        }

        Ok(())
    }

    /// Get active panel area
    fn get_active_panel_area(&self) -> Rect {
        // Use calculate_panel_rects() to get all panel areas with proper layout calculation
        let panel_rects = self.calculate_panel_rects();

        // Find the active panel based on current focus
        let focused_group_idx = self.layout_manager.focus;

        // Find expanded panel in the focused group
        for (group_idx, _panel_idx, rect, is_expanded) in panel_rects {
            if group_idx == focused_group_idx && is_expanded {
                return rect;
            }
        }

        // Fallback: return full main area if active panel not found
        let width = self.state.terminal.width;
        let height = self.state.terminal.height;
        Rect {
            x: 0,
            y: 1,
            width,
            height: height.saturating_sub(2),
        }
    }

    /// Handle click on panel [X] button or [▶]/[▼] expand/collapse button
    /// Returns true if a button was clicked
    fn handle_panel_close_click(&mut self, click_x: u16, click_y: u16) -> Result<bool> {
        let panel_rects = self.calculate_panel_rects();

        for (group_idx, panel_idx, rect, is_expanded) in panel_rects {
            // Check if click is on this panel's top line
            if click_y != rect.y {
                continue;
            }

            // Check if click is within the panel's horizontal bounds
            if click_x < rect.x || click_x >= rect.x + rect.width {
                continue;
            }

            let relative_x = click_x - rect.x;

            // Button format: ─[X][▶] Title ─── (collapsed)
            //          or:   ┌[X][▼] Title ──┐ (expanded)
            // [X] button: offsets 1-3
            // [▶]/[▼] button: offsets 4-6

            if (1..=3).contains(&relative_x) {
                // Click on [X] button - close panel with confirmation if needed
                termide_logger::debug("Panel close button [X] clicked");
                // First, activate the clicked panel
                if let Some(group) = self.layout_manager.panel_groups.get_mut(group_idx) {
                    group.set_expanded(panel_idx);
                }
                self.layout_manager.focus = group_idx;

                // Now use the same close logic as keyboard shortcut (with confirmation)
                self.handle_close_panel_request(0)?;
                return Ok(true);
            } else if (4..=6).contains(&relative_x) {
                // Click on [▶]/[▼] button - expand/collapse panel
                if let Some(group) = self.layout_manager.panel_groups.get_mut(group_idx) {
                    if is_expanded && group.len() > 1 {
                        // Currently expanded - collapse by expanding next panel
                        let next_idx = (panel_idx + 1) % group.len();
                        group.set_expanded(next_idx);
                    } else {
                        // Currently collapsed - expand this panel
                        group.set_expanded(panel_idx);
                        // Also make this group active
                        self.layout_manager.focus = group_idx;
                    }
                }
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Handle click on panel to switch focus
    fn handle_panel_focus_click(&mut self, click_x: u16, click_y: u16) -> Result<()> {
        let panel_rects = self.calculate_panel_rects();

        for (group_idx, panel_idx, rect, _is_expanded) in panel_rects {
            // Check if click is within this panel's bounds
            if click_x >= rect.x
                && click_x < rect.x + rect.width
                && click_y >= rect.y
                && click_y < rect.y + rect.height
            {
                // Click on a panel group - make it active
                self.layout_manager.focus = group_idx;
                if let Some(group) = self.layout_manager.panel_groups.get_mut(group_idx) {
                    group.set_expanded(panel_idx);
                }
                return Ok(());
            }
        }

        Ok(())
    }

    /// Handle click on menu
    fn handle_menu_click(&mut self, x: u16) -> Result<()> {
        let mut current_x = 1_u16;

        // Get menu items with translations
        let menu_items = termide_ui_render::menu::get_menu_items();

        for (i, item) in menu_items.iter().enumerate() {
            let item_width = item.width() as u16;
            if x >= current_x && x < current_x + item_width {
                // Toggle: if this menu item is already open, close it
                if self.state.ui.menu_open && self.state.ui.selected_menu_item == Some(i) {
                    // Restore theme if nested submenu was open
                    if let Some(original_name) = self.state.ui.theme_preview_original.take() {
                        self.state.theme = Theme::get_by_name(&original_name);
                    }
                    self.state.close_menu();
                } else {
                    // Open menu with the selected item
                    self.state.open_menu(Some(i));
                    self.execute_menu_action()?;
                }
                return Ok(());
            }
            current_x += item_width + 2; // +2 for spaces
        }

        Ok(())
    }

    /// Handle click on Options submenu dropdown
    /// Returns true if click was handled
    fn handle_submenu_click(&mut self, x: u16, y: u16) -> Result<bool> {
        // Get Options dropdown position
        let menu_x = get_menu_item_x_position(OPTIONS_MENU_INDEX);
        let dropdown_y = 1_u16;

        // Calculate Options dropdown dimensions
        let options_items = get_options_items();
        let options_width = options_items
            .iter()
            .map(|i| i.label.width())
            .max()
            .unwrap_or(10) as u16
            + 4;
        let options_height = options_items.len() as u16 + 2; // +2 for borders

        // Check if nested submenu (Themes) is open
        if self.state.ui.nested_submenu.open && self.state.ui.options_submenu.selected == 0 {
            // Theme dropdown is to the right of Options dropdown
            let nested_x = menu_x + options_width;
            let nested_y = dropdown_y + 1;

            let theme_names = Theme::all_theme_names();
            let nested_width = theme_names.iter().map(|n| n.width()).max().unwrap_or(10) as u16 + 6;
            // Must match ThemeDropdown::max_visible
            let max_visible = 25;
            let nested_height = theme_names.len().min(max_visible) as u16 + 2;

            // Check click on theme dropdown
            if x >= nested_x
                && x < nested_x + nested_width
                && y >= nested_y
                && y < nested_y + nested_height
            {
                // Calculate scroll offset same as ThemeDropdown
                let scroll_offset = if self.state.ui.nested_submenu.selected >= max_visible {
                    self.state.ui.nested_submenu.selected - max_visible + 1
                } else {
                    0
                };
                let item_y = y.saturating_sub(nested_y + 1); // -1 for top border
                let item_index = scroll_offset + item_y as usize;
                if item_index < theme_names.len() {
                    // Clear preview state - theme is confirmed
                    self.state.ui.theme_preview_original = None;
                    // Apply selected theme
                    if let Some(name) = theme_names.get(item_index) {
                        self.apply_theme(name)?;
                    }
                    self.state.close_menu();
                    return Ok(true);
                }
            }
        }

        // Check if nested submenu (Language) is open
        if self.state.ui.nested_submenu.open && self.state.ui.options_submenu.selected == 1 {
            // Language dropdown is to the right of Options dropdown
            let nested_x = menu_x + options_width;
            let nested_y = dropdown_y + 2; // Language is at index 1

            let languages = i18n::get_language_list();
            let nested_width = languages
                .iter()
                .map(|(_, name)| name.width())
                .max()
                .unwrap_or(10) as u16
                + 4;
            // Must match LanguageDropdown::max_visible
            let max_visible = 15;
            let nested_height = languages.len().min(max_visible) as u16 + 2;

            // Check click on language dropdown
            if x >= nested_x
                && x < nested_x + nested_width
                && y >= nested_y
                && y < nested_y + nested_height
            {
                // Calculate scroll offset same as LanguageDropdown
                let scroll_offset = if self.state.ui.nested_submenu.selected >= max_visible {
                    self.state.ui.nested_submenu.selected - max_visible + 1
                } else {
                    0
                };
                let item_y = y.saturating_sub(nested_y + 1); // -1 for top border
                let item_index = scroll_offset + item_y as usize;
                if item_index < languages.len() {
                    // Clear preview state - language is confirmed
                    self.state.ui.language_preview_original = None;
                    // Apply selected language
                    if let Some((code, name)) = languages.get(item_index) {
                        self.apply_language_mouse(code, name)?;
                    }
                    self.state.close_menu();
                    return Ok(true);
                }
            }
        }

        // Check click on Options dropdown
        if x >= menu_x
            && x < menu_x + options_width
            && y >= dropdown_y
            && y < dropdown_y + options_height
        {
            let item_y = y.saturating_sub(dropdown_y + 1); // -1 for top border
            let item_index = item_y as usize;
            if item_index < options_items.len() {
                self.state.ui.options_submenu.selected = item_index;
                match item_index {
                    0 => {
                        // Themes - toggle nested submenu
                        if self.state.ui.nested_submenu.open
                            && self.state.ui.options_submenu.selected == 0
                        {
                            // Already open - close it and restore theme
                            if let Some(original_name) = self.state.ui.theme_preview_original.take()
                            {
                                self.state.theme = Theme::get_by_name(&original_name);
                            }
                            self.state.close_nested_submenu();
                        } else {
                            // Open nested submenu with live preview
                            let theme_names = Theme::all_theme_names();
                            let current_idx = theme_names
                                .iter()
                                .position(|n| n == self.state.theme.name)
                                .unwrap_or(0);
                            // Save current theme for restoration on cancel
                            self.state.ui.theme_preview_original =
                                Some(self.state.theme.name.to_string());
                            self.state.open_nested_submenu(current_idx);
                        }
                    }
                    1 => {
                        // Language - toggle nested submenu
                        use termide_i18n as i18n;
                        use termide_ui_render::find_current_language_index;
                        if self.state.ui.nested_submenu.open
                            && self.state.ui.options_submenu.selected == 1
                        {
                            // Already open - close it and restore language
                            if let Some(original_lang) =
                                self.state.ui.language_preview_original.take()
                            {
                                let _ = i18n::set_language(&original_lang);
                            }
                            self.state.close_nested_submenu();
                        } else {
                            // Open nested submenu with live preview
                            let current_idx = find_current_language_index();
                            // Save current language for restoration on cancel
                            self.state.ui.language_preview_original =
                                Some(i18n::current_language());
                            self.state.open_nested_submenu(current_idx);
                        }
                    }
                    2 => {
                        // Manage actions
                        self.state.close_menu();
                        self.handle_manage_actions()?;
                    }
                    3 => {
                        // Edit preferences
                        self.state.close_menu();
                        self.open_config_in_editor()?;
                    }
                    4 => {
                        // Help
                        self.state.close_menu();
                        self.handle_new_help()?;
                    }
                    5 => {
                        // Quit
                        self.state.close_menu();
                        if self.has_panels_requiring_confirmation() {
                            use crate::state::{ActiveModal, PendingAction};
                            use termide_i18n as i18n;
                            let t = i18n::t();
                            let modal = termide_modal::ConfirmModal::new(
                                t.modal_yes(),
                                t.app_quit_confirm(),
                            );
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
                return Ok(true);
            }
        }

        // Click outside dropdowns - close all menus
        self.state.close_menu();
        Ok(true)
    }

    /// Handle click on Sessions submenu dropdown
    /// Returns true if click was handled
    fn handle_sessions_submenu_click(&mut self, x: u16, y: u16) -> Result<bool> {
        // Get Sessions dropdown position
        let menu_x = get_menu_item_x_position(SESSIONS_MENU_INDEX);
        let dropdown_y = 1_u16;

        // Calculate Sessions dropdown dimensions
        let sessions_items = get_sessions_items();
        let sessions_width = sessions_items
            .iter()
            .map(|i| i.label.width())
            .max()
            .unwrap_or(10) as u16
            + 4;
        let sessions_height = sessions_items.len() as u16 + 2; // +2 for borders

        // Check click on Sessions dropdown
        if x >= menu_x
            && x < menu_x + sessions_width
            && y >= dropdown_y
            && y < dropdown_y + sessions_height
        {
            let item_y = y.saturating_sub(dropdown_y + 1); // -1 for top border
            let item_index = item_y as usize;
            if item_index < sessions_items.len() {
                self.state.ui.sessions_submenu.selected = item_index;
                // Execute the action for the selected item
                self.execute_sessions_submenu_action()?;
                return Ok(true);
            }
        }

        // Click outside dropdown - close menu
        self.state.close_menu();
        Ok(true)
    }

    /// Handle click on Tools submenu dropdown
    /// Returns true if click was handled
    fn handle_tools_submenu_click(&mut self, x: u16, y: u16) -> Result<bool> {
        // Get Tools dropdown position
        let menu_x = get_menu_item_x_position(TOOLS_MENU_INDEX);
        let dropdown_y = 1_u16;

        // Calculate Tools dropdown dimensions
        let tools_items = get_tools_items();
        let tools_width = tools_items
            .iter()
            .map(|i| i.label.width())
            .max()
            .unwrap_or(10) as u16
            + 4;
        let tools_height = tools_items.len() as u16 + 2; // +2 for borders

        // Check click on Tools dropdown
        if x >= menu_x
            && x < menu_x + tools_width
            && y >= dropdown_y
            && y < dropdown_y + tools_height
        {
            let item_y = y.saturating_sub(dropdown_y + 1); // -1 for top border
            let item_index = item_y as usize;
            if item_index < tools_items.len() {
                self.state.ui.tools_submenu.selected = item_index;
                // Execute the action for the selected item
                self.execute_tools_submenu_action()?;
                return Ok(true);
            }
        }

        // Click outside dropdown - close menu
        self.state.close_menu();
        Ok(true)
    }

    /// Handle click on Actions submenu dropdown
    /// Returns true if click was handled
    fn handle_actions_submenu_click(&mut self, x: u16, y: u16) -> Result<bool> {
        let registry = match termide_config::actions::ActionsRegistry::load() {
            Some(r) => r,
            None => {
                self.state.close_menu();
                return Ok(true);
            }
        };

        // If nested submenu is open, handle clicks on it first
        if self.state.ui.actions_nested.open {
            if let Some(group_name) = &self.state.ui.current_actions_group.clone() {
                let nested_items = get_actions_group_items(&registry, group_name);
                if !nested_items.is_empty() {
                    // Calculate nested submenu position (to the right of main dropdown)
                    let menu_x = get_menu_item_x_position(ACTIONS_MENU_INDEX);
                    let actions_items = get_actions_items(&registry);
                    let actions_width = actions_items
                        .iter()
                        .map(|i| i.label.width())
                        .max()
                        .unwrap_or(10) as u16
                        + 4;

                    let nested_x = menu_x + actions_width;
                    let dropdown_y = 1_u16;
                    // +1 for dropdown border, align with selected item
                    let nested_y = dropdown_y + 1 + self.state.ui.actions_submenu.selected as u16;
                    let nested_width = nested_items
                        .iter()
                        .map(|i| i.label.width())
                        .max()
                        .unwrap_or(10) as u16
                        + 4;
                    let nested_height = nested_items.len() as u16 + 2;

                    // Check click on nested submenu
                    if x >= nested_x
                        && x < nested_x + nested_width
                        && y >= nested_y
                        && y < nested_y + nested_height
                    {
                        let item_y = y.saturating_sub(nested_y + 1);
                        let item_index = item_y as usize;
                        if item_index < nested_items.len() {
                            self.state.ui.actions_nested.selected = item_index;
                            self.execute_actions_nested_action()?;
                            return Ok(true);
                        }
                    }
                }
            }
        }

        // Get Actions dropdown position
        let menu_x = get_menu_item_x_position(ACTIONS_MENU_INDEX);
        let dropdown_y = 1_u16;

        // Calculate Actions dropdown dimensions
        let actions_items = get_actions_items(&registry);
        let actions_width = actions_items
            .iter()
            .map(|i| i.label.width())
            .max()
            .unwrap_or(10) as u16
            + 4;
        let actions_height = actions_items.len() as u16 + 2;

        // Check click on Actions dropdown
        if x >= menu_x
            && x < menu_x + actions_width
            && y >= dropdown_y
            && y < dropdown_y + actions_height
        {
            let item_y = y.saturating_sub(dropdown_y + 1);
            let item_index = item_y as usize;
            if item_index < actions_items.len() {
                self.state.ui.actions_submenu.selected = item_index;
                // Execute the action (may open nested submenu or run script)
                self.execute_actions_submenu_action()?;
                return Ok(true);
            }
        }

        // Click outside dropdown - close menu
        self.state.close_menu();
        Ok(true)
    }

    /// Handle click on divider to start resize drag
    /// Returns true if click was on a divider
    fn handle_divider_click(&mut self, x: u16, y: u16) -> Result<bool> {
        let terminal_height = self.state.terminal.height;

        if let Some(divider_idx) =
            self.layout_manager
                .find_divider_at_position(x, y, terminal_height)
        {
            // Get current widths of adjacent groups
            let left_width = self
                .layout_manager
                .panel_groups
                .get(divider_idx)
                .and_then(|g| g.width)
                .unwrap_or(0);
            let right_width = self
                .layout_manager
                .panel_groups
                .get(divider_idx + 1)
                .and_then(|g| g.width)
                .unwrap_or(0);

            // Start drag
            self.state
                .ui
                .drag
                .start(divider_idx, x, left_width, right_width);
            self.state.needs_redraw = true;

            return Ok(true);
        }

        Ok(false)
    }

    /// Handle divider drag (update widths)
    fn handle_divider_drag(&mut self, current_x: u16) -> Result<()> {
        let drag = &self.state.ui.drag;
        let Some(divider_idx) = drag.active_divider else {
            return Ok(());
        };

        let delta = current_x as i32 - drag.start_x as i32;
        let (start_left, start_right) = drag.start_widths;

        // Calculate new widths with min_panel_width constraint
        let min_width = self.state.config.general.min_panel_width;
        let total_width = start_left + start_right;

        let new_left = (start_left as i32 + delta)
            .max(min_width as i32)
            .min((total_width - min_width) as i32) as u16;
        let new_right = total_width - new_left;

        // Apply new widths
        self.layout_manager
            .resize_groups(divider_idx, new_left, new_right);
        self.state.needs_redraw = true;

        Ok(())
    }

    /// Handle divider drag end (save session)
    fn handle_divider_drag_end(&mut self) -> Result<()> {
        self.state.ui.drag.end();
        self.state.needs_redraw = true;

        // Save session with new widths (debounce: only on mouse up)
        self.auto_save_session();

        Ok(())
    }

    /// Calculate panel rectangles for mouse hit testing
    /// Returns Vec<(group_idx, panel_idx, rect, is_expanded)>
    fn calculate_panel_rects(&self) -> Vec<(usize, usize, Rect, bool)> {
        let mut result = Vec::new();

        let width = self.state.terminal.width;
        let height = self.state.terminal.height;

        // Main area: from row 1 to height-2 (excluding menu and status bar)
        let main_area = Rect {
            x: 0,
            y: 1,
            width,
            height: height.saturating_sub(2),
        };

        // Calculate group areas
        if !self.layout_manager.panel_groups.is_empty() {
            let groups_area = main_area;

            // Calculate horizontal constraints for groups (using widths)
            // Группы могут иметь фиксированную ширину или auto-width
            let group_constraints: Vec<Constraint> = self
                .layout_manager
                .panel_groups
                .iter()
                .map(|g| {
                    let width = g.width.unwrap_or(groups_area.width);
                    Constraint::Length(width.max(20))
                })
                .collect();

            let group_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(group_constraints)
                .split(groups_area);

            // Process each group
            for (group_idx, group) in self.layout_manager.panel_groups.iter().enumerate() {
                if group.is_empty() || group_chunks[group_idx].height == 0 {
                    continue;
                }

                let group_area = group_chunks[group_idx];
                let expanded_idx = group.expanded_index();

                // Build vertical constraints for panels in group
                let vertical_constraints: Vec<Constraint> = (0..group.len())
                    .map(|i| {
                        if i == expanded_idx {
                            Constraint::Min(0) // Expanded panel
                        } else {
                            Constraint::Length(1) // Collapsed panel (1 line)
                        }
                    })
                    .collect();

                let vertical_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(vertical_constraints)
                    .split(group_area);

                // Add each panel's rect to results
                for panel_idx in 0..group.len() {
                    let is_expanded = panel_idx == expanded_idx;
                    result.push((
                        group_idx,
                        panel_idx,
                        vertical_chunks[panel_idx],
                        is_expanded,
                    ));
                }
            }
        }

        result
    }
}
