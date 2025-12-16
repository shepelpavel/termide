//! Sessions selection modal dialog.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Widget},
};
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

use termide_theme::Theme;

use crate::{calculate_modal_width, centered_rect_with_size, Modal, ModalResult, ModalWidthConfig};

/// Item representing a session in the list
#[derive(Debug, Clone)]
pub struct SessionItem {
    /// Original project path
    pub project_path: PathBuf,
    /// Display path (potentially shortened)
    pub display_path: String,
    /// Relative time since last modification (e.g., "2 hours ago")
    pub relative_time: String,
    /// Whether this is the current session
    pub is_current: bool,
}

/// Sessions selection modal window
#[derive(Debug)]
pub struct SessionsModal {
    title: String,
    items: Vec<SessionItem>,
    cursor: usize,
    scroll_offset: usize,
    last_list_area: Option<Rect>,
}

/// Maximum number of items visible at once (each item takes 2 lines)
const MAX_VISIBLE_ITEMS: usize = 6;

impl SessionsModal {
    /// Create a new sessions modal
    pub fn new(title: impl Into<String>, items: Vec<SessionItem>) -> Self {
        // Find the first non-current session to select by default
        let initial_cursor = items.iter().position(|item| !item.is_current).unwrap_or(0);

        Self {
            title: title.into(),
            items,
            cursor: initial_cursor,
            scroll_offset: 0,
            last_list_area: None,
        }
    }

    /// Calculate dynamic modal width
    fn calculate_modal_width(&self, screen_width: u16) -> u16 {
        let title_width = self.title.len() as u16 + 4;

        // Find max path width
        let max_path_width = self
            .items
            .iter()
            .map(|item| item.display_path.len() as u16 + 4) // "▶ " + path + " (current)"
            .max()
            .unwrap_or(40);

        calculate_modal_width(
            [title_width, max_path_width].into_iter(),
            screen_width,
            ModalWidthConfig::wide(),
        )
    }

    /// Move cursor up, skipping current session
    fn cursor_up(&mut self) {
        let mut new_cursor = self.cursor;
        loop {
            if new_cursor == 0 {
                break;
            }
            new_cursor -= 1;
            if !self.items[new_cursor].is_current {
                self.cursor = new_cursor;
                break;
            }
        }
        self.adjust_scroll();
    }

    /// Move cursor down, skipping current session
    fn cursor_down(&mut self) {
        let mut new_cursor = self.cursor;
        loop {
            if new_cursor >= self.items.len().saturating_sub(1) {
                break;
            }
            new_cursor += 1;
            if !self.items[new_cursor].is_current {
                self.cursor = new_cursor;
                break;
            }
        }
        self.adjust_scroll();
    }

    /// Go to first selectable item
    fn cursor_home(&mut self) {
        for (i, item) in self.items.iter().enumerate() {
            if !item.is_current {
                self.cursor = i;
                break;
            }
        }
        self.adjust_scroll();
    }

    /// Go to last selectable item
    fn cursor_end(&mut self) {
        for (i, item) in self.items.iter().enumerate().rev() {
            if !item.is_current {
                self.cursor = i;
                break;
            }
        }
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

    /// Get the selected session's project path
    fn get_selected_path(&self) -> Option<PathBuf> {
        self.items
            .get(self.cursor)
            .filter(|item| !item.is_current)
            .map(|item| item.project_path.clone())
    }
}

impl Modal for SessionsModal {
    type Result = PathBuf;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let modal_width = self.calculate_modal_width(area.width);

        // Each item takes 2 lines (path + time)
        let visible_items = self.items.len().min(MAX_VISIBLE_ITEMS);
        let list_height = (visible_items * 2) as u16;

        // Height: 1 (top border) + list_height + 1 (bottom border)
        let modal_height = 2 + list_height;

        let modal_area = centered_rect_with_size(modal_width, modal_height, area);
        Clear.render(modal_area, buf);

        // Create block with inverted colors
        let block = Block::default()
            .title(Span::styled(
                format!(" {} ", self.title),
                Style::default().fg(theme.bg).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.bg))
            .style(Style::default().bg(theme.fg));

        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        // Build list items (2 lines per session)
        let mut list_items: Vec<ListItem> = Vec::new();
        let t = termide_i18n::t();

        for (idx, item) in self
            .items
            .iter()
            .enumerate()
            .skip(self.scroll_offset)
            .take(MAX_VISIBLE_ITEMS)
        {
            let is_selected = idx == self.cursor;
            let is_current = item.is_current;

            // Line 1: Path with selection indicator
            let prefix = if is_selected && !is_current {
                "▶ "
            } else {
                "  "
            };

            let path_suffix = if is_current {
                format!(" {}", t.sessions_current())
            } else {
                String::new()
            };

            let path_style = if is_current {
                // Current session - dimmed, not selectable
                Style::default()
                    .fg(theme.accented_bg)
                    .add_modifier(Modifier::DIM)
            } else if is_selected {
                // Selected item
                Style::default()
                    .fg(theme.fg)
                    .bg(theme.accented_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                // Normal item
                Style::default().fg(theme.bg)
            };

            // Pad line1 to full width (use unicode width for correct calculation)
            let line1_width = prefix.width() + item.display_path.width() + path_suffix.width();
            let padding1 = " ".repeat((inner.width as usize).saturating_sub(line1_width));

            let line1 = Line::from(vec![
                Span::styled(prefix, path_style),
                Span::styled(&item.display_path, path_style),
                Span::styled(path_suffix, path_style),
                Span::styled(padding1, path_style),
            ]);

            // Line 2: Relative time (indented, dimmed) - no highlight even if selected
            let time_style = Style::default()
                .fg(theme.accented_bg)
                .add_modifier(Modifier::DIM);

            // Pad line2 to full width (use unicode width)
            let line2_prefix = "  ";
            let line2_width = line2_prefix.width() + item.relative_time.width();
            let padding2 = " ".repeat((inner.width as usize).saturating_sub(line2_width));

            let line2 = Line::from(vec![
                Span::styled(line2_prefix, time_style),
                Span::styled(&item.relative_time, time_style),
                Span::styled(padding2, time_style),
            ]);

            list_items.push(ListItem::new(vec![line1, line2]));
        }

        let list = List::new(list_items).style(Style::default().bg(theme.fg));
        list.render(inner, buf);

        self.last_list_area = Some(inner);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>> {
        match key.code {
            KeyCode::Esc => Ok(Some(ModalResult::Cancelled)),
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor_up();
                Ok(None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor_down();
                Ok(None)
            }
            KeyCode::Home => {
                self.cursor_home();
                Ok(None)
            }
            KeyCode::End => {
                self.cursor_end();
                Ok(None)
            }
            KeyCode::Enter => {
                if let Some(path) = self.get_selected_path() {
                    Ok(Some(ModalResult::Confirmed(path)))
                } else {
                    Ok(None) // Can't select current session
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
        // Only handle left button press
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return Ok(None);
        }

        let Some(list_area) = self.last_list_area else {
            return Ok(None);
        };

        // Check if click is within list area
        if mouse.row < list_area.y
            || mouse.row >= list_area.y + list_area.height
            || mouse.column < list_area.x
            || mouse.column >= list_area.x + list_area.width
        {
            return Ok(None);
        }

        // Calculate which item was clicked (each item takes 2 lines)
        let relative_row = (mouse.row - list_area.y) as usize;
        let clicked_item_index = self.scroll_offset + (relative_row / 2);

        if clicked_item_index < self.items.len() {
            let item = &self.items[clicked_item_index];

            // Skip if clicking on current session
            if item.is_current {
                return Ok(None);
            }

            // Select and confirm
            self.cursor = clicked_item_index;
            if let Some(path) = self.get_selected_path() {
                return Ok(Some(ModalResult::Confirmed(path)));
            }
        }

        Ok(None)
    }
}
