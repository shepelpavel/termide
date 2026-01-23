//! Directory picker modal dialog.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Widget},
};

use crate::base::render_modal_block;
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

use termide_theme::Theme;

use crate::{calculate_modal_width, centered_rect_with_size, Modal, ModalResult, ModalWidthConfig};

/// Directory entry for display (only directories)
#[derive(Debug, Clone)]
struct DirEntry {
    /// Entry name
    name: String,
}

/// Directory picker modal window
#[derive(Debug)]
pub struct DirectoryPickerModal {
    title: String,
    current_dir: PathBuf,
    entries: Vec<DirEntry>,
    cursor: usize,
    scroll_offset: usize,
    /// Focus on buttons (true) or list (false)
    button_focused: bool,
    /// Selected button index (0=Create, 1=Cancel)
    selected_button: usize,
    last_list_area: Option<Rect>,
    last_buttons_area: Option<Rect>,
    create_label: String,
    cancel_label: String,
    /// Cached modal width (recalculated only on directory change)
    cached_width: Option<u16>,
}

/// Maximum number of items visible at once
const MAX_VISIBLE_ITEMS: usize = 10;

impl DirectoryPickerModal {
    /// Create a new directory picker modal with custom title and confirm button
    pub fn new(initial_dir: PathBuf, title: String, confirm_label: String) -> Self {
        let t = termide_i18n::t();
        let mut modal = Self {
            title,
            current_dir: initial_dir,
            entries: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            button_focused: false,
            selected_button: 0,
            last_list_area: None,
            last_buttons_area: None,
            create_label: confirm_label,
            cancel_label: t.directory_picker_cancel().to_string(),
            cached_width: None,
        };
        modal.load_directory();
        modal
    }

    /// Get the currently selected path (current_dir + selected entry)
    fn selected_path(&self) -> PathBuf {
        if let Some(entry) = self.entries.get(self.cursor) {
            if entry.name == ".." {
                // For ".." show current_dir (going up means selecting parent)
                self.current_dir.clone()
            } else {
                // For regular entries, show full path
                self.current_dir.join(&entry.name)
            }
        } else {
            // No entries - show current_dir
            self.current_dir.clone()
        }
    }

    /// Load directory contents (only directories)
    fn load_directory(&mut self) {
        self.entries.clear();
        self.cursor = 0;
        self.scroll_offset = 0;
        self.cached_width = None;

        // Add parent directory entry if not at root
        if self.current_dir.parent().is_some() {
            self.entries.push(DirEntry {
                name: "..".to_string(),
            });
        }

        // Read directory contents - only directories
        if let Ok(read_dir) = std::fs::read_dir(&self.current_dir) {
            let mut dirs: Vec<DirEntry> = read_dir
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| {
                    let path = entry.path();
                    // Only include directories
                    if !path.is_dir() {
                        return None;
                    }
                    let name = entry.file_name().to_string_lossy().to_string();
                    // Skip hidden directories
                    if name.starts_with('.') {
                        return None;
                    }
                    Some(DirEntry { name })
                })
                .collect();

            // Sort by name
            dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

            self.entries.extend(dirs);
        }
    }

    /// Calculate dynamic modal width (stable within directory)
    fn calculate_modal_width(&self, screen_width: u16) -> u16 {
        let title_width = self.title.len() as u16 + 4;

        // Use current_dir + max entry name for path width (not selected_path to avoid jitter)
        let max_entry_len = self.entries.iter().map(|e| e.name.len()).max().unwrap_or(0);
        let path_width = self.current_dir.to_string_lossy().len() as u16 + max_entry_len as u16 + 5;

        // Find max entry width: "▶ " + name
        let max_entry_width = max_entry_len as u16 + 4;

        let buttons_width = self.create_label.len() as u16 + self.cancel_label.len() as u16 + 12;

        calculate_modal_width(
            [title_width, path_width, max_entry_width, buttons_width].into_iter(),
            screen_width,
            ModalWidthConfig::wide(),
        )
    }

    /// Get modal width (cached, recalculated on directory change)
    fn get_modal_width(&mut self, screen_width: u16) -> u16 {
        if let Some(width) = self.cached_width {
            return width;
        }
        let width = self.calculate_modal_width(screen_width);
        self.cached_width = Some(width);
        width
    }

    /// Move cursor up
    fn cursor_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.adjust_scroll();
        }
    }

    /// Move cursor down
    fn cursor_down(&mut self) {
        if self.cursor < self.entries.len().saturating_sub(1) {
            self.cursor += 1;
            self.adjust_scroll();
        }
    }

    /// Go to first item
    fn cursor_home(&mut self) {
        self.cursor = 0;
        self.adjust_scroll();
    }

    /// Go to last item
    fn cursor_end(&mut self) {
        self.cursor = self.entries.len().saturating_sub(1);
        self.adjust_scroll();
    }

    /// Adjust scroll to keep cursor visible
    fn adjust_scroll(&mut self) {
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        } else if self.cursor >= self.scroll_offset + MAX_VISIBLE_ITEMS {
            self.scroll_offset = self.cursor - MAX_VISIBLE_ITEMS + 1;
        }
    }

    /// Enter selected directory
    fn enter_directory(&mut self) {
        if let Some(entry) = self.entries.get(self.cursor) {
            if entry.name == ".." {
                // Go to parent
                if let Some(parent) = self.current_dir.parent() {
                    self.current_dir = parent.to_path_buf();
                    self.load_directory();
                }
            } else {
                // Enter subdirectory
                self.current_dir = self.current_dir.join(&entry.name);
                self.load_directory();
            }
        }
    }

    /// Go to parent directory
    fn go_parent(&mut self) {
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            self.load_directory();
        }
    }
}

impl Modal for DirectoryPickerModal {
    type Result = PathBuf;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let modal_width = self.get_modal_width(area.width);

        // Height: 1 (border) + 1 (path) + 1 (separator) + list + 1 (separator) + 1 (buttons) + 1 (border)
        let visible_items = self.entries.len().min(MAX_VISIBLE_ITEMS);
        let list_height = visible_items.max(3) as u16;
        let modal_height = 6 + list_height;

        let modal_area = centered_rect_with_size(modal_width, modal_height, area);

        let inner = render_modal_block(modal_area, buf, &self.title, theme);

        // Render selected path (current_dir + selected entry)
        let selected = self.selected_path();
        let path_str = selected.to_string_lossy();
        let path_style = Style::default().fg(theme.fg);
        let path_line = Line::from(Span::styled(path_str.as_ref(), path_style));

        let path_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        buf.set_line(path_area.x, path_area.y, &path_line, path_area.width);

        // Render separator
        let sep_y = inner.y + 1;
        let sep_style = Style::default().fg(theme.accented_bg);
        for x in inner.x..inner.x + inner.width {
            buf[(x, sep_y)].set_char('─').set_style(sep_style);
        }

        // Render file list
        let list_area = Rect {
            x: inner.x,
            y: inner.y + 2,
            width: inner.width,
            height: list_height,
        };

        let mut list_items: Vec<ListItem> = Vec::new();

        for (idx, entry) in self
            .entries
            .iter()
            .enumerate()
            .skip(self.scroll_offset)
            .take(MAX_VISIBLE_ITEMS)
        {
            let is_selected = idx == self.cursor && !self.button_focused;

            let prefix = if is_selected { "▶ " } else { "  " };

            let style = if is_selected {
                Style::default()
                    .fg(theme.fg)
                    .bg(theme.bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg)
            };

            let line_width = prefix.width() + entry.name.width();
            let padding = " ".repeat((inner.width as usize).saturating_sub(line_width));

            let line = Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(&entry.name, style),
                Span::styled(padding, style),
            ]);

            list_items.push(ListItem::new(line));
        }

        let list = List::new(list_items).style(Style::default().bg(theme.bg));
        list.render(list_area, buf);
        self.last_list_area = Some(list_area);

        // Render separator before buttons
        let sep2_y = list_area.y + list_area.height;
        for x in inner.x..inner.x + inner.width {
            buf[(x, sep2_y)].set_char('─').set_style(sep_style);
        }

        // Render buttons
        let buttons_y = sep2_y + 1;
        let buttons_area = Rect {
            x: inner.x,
            y: buttons_y,
            width: inner.width,
            height: 1,
        };
        self.last_buttons_area = Some(buttons_area);

        let create_style = if self.button_focused && self.selected_button == 0 {
            Style::default()
                .fg(theme.fg)
                .bg(theme.bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg)
        };

        let cancel_style = if self.button_focused && self.selected_button == 1 {
            Style::default()
                .fg(theme.fg)
                .bg(theme.bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg)
        };

        let create_btn = format!("[ {} ]", self.create_label);
        let cancel_btn = format!("[ {} ]", self.cancel_label);

        // Center buttons
        let total_btn_width = create_btn.width() + 2 + cancel_btn.width();
        let btn_start_x = inner.x + (inner.width.saturating_sub(total_btn_width as u16)) / 2;

        buf.set_string(btn_start_x, buttons_y, &create_btn, create_style);
        buf.set_string(
            btn_start_x + create_btn.width() as u16 + 2,
            buttons_y,
            &cancel_btn,
            cancel_style,
        );
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>> {
        match key.code {
            KeyCode::Esc => Ok(Some(ModalResult::Cancelled)),
            KeyCode::Tab => {
                self.button_focused = !self.button_focused;
                Ok(None)
            }
            KeyCode::BackTab => {
                self.button_focused = !self.button_focused;
                Ok(None)
            }
            KeyCode::Up | KeyCode::Char('k') if !self.button_focused => {
                self.cursor_up();
                Ok(None)
            }
            KeyCode::Down | KeyCode::Char('j') if !self.button_focused => {
                self.cursor_down();
                Ok(None)
            }
            KeyCode::Home if !self.button_focused => {
                self.cursor_home();
                Ok(None)
            }
            KeyCode::End if !self.button_focused => {
                self.cursor_end();
                Ok(None)
            }
            KeyCode::Left if self.button_focused => {
                self.selected_button = 0;
                Ok(None)
            }
            KeyCode::Right if self.button_focused => {
                self.selected_button = 1;
                Ok(None)
            }
            KeyCode::Backspace => {
                self.go_parent();
                Ok(None)
            }
            // Ctrl+Enter to confirm selected path from anywhere
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Ok(Some(ModalResult::Confirmed(self.selected_path())))
            }
            KeyCode::Enter => {
                if self.button_focused {
                    match self.selected_button {
                        0 => Ok(Some(ModalResult::Confirmed(self.selected_path()))),
                        _ => Ok(Some(ModalResult::Cancelled)),
                    }
                } else {
                    // Enter selected directory
                    self.enter_directory();
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        _modal_area: Rect,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        use crate::{check_mouse_click, MouseClickResult};

        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return Ok(None);
        }

        // Check if clicked on buttons area
        if let Some(buttons_area) = self.last_buttons_area {
            if mouse.row == buttons_area.y
                && mouse.column >= buttons_area.x
                && mouse.column < buttons_area.x + buttons_area.width
            {
                // Calculate button positions
                let create_btn_width = self.create_label.len() + 4;
                let total_btn_width = create_btn_width + 2 + self.cancel_label.len() + 4;
                let btn_start_x = buttons_area.x
                    + (buttons_area.width.saturating_sub(total_btn_width as u16)) / 2;

                if mouse.column >= btn_start_x
                    && mouse.column < btn_start_x + create_btn_width as u16
                {
                    // Confirm button clicked
                    return Ok(Some(ModalResult::Confirmed(self.selected_path())));
                } else if mouse.column >= btn_start_x + create_btn_width as u16 + 2 {
                    // Cancel button clicked
                    return Ok(Some(ModalResult::Cancelled));
                }
            }
        }

        // Check if clicked on list
        match check_mouse_click(
            mouse.column,
            mouse.row,
            None,
            self.last_list_area,
            self.scroll_offset,
        ) {
            MouseClickResult::OnListItem(clicked_index) => {
                if clicked_index < self.entries.len() {
                    self.button_focused = false;
                    self.cursor = clicked_index;
                    // Enter directory on click
                    self.enter_directory();
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}
