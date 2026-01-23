//! Sessions selection modal dialog.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
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
        Self {
            title: title.into(),
            items,
            cursor: 0,
            scroll_offset: 0,
            last_list_area: None,
        }
    }

    /// Set initial cursor position (for selecting current session)
    pub fn with_cursor(mut self, index: usize) -> Self {
        self.cursor = index.min(self.items.len().saturating_sub(1));
        self.adjust_scroll();
        self
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

    /// Move cursor up
    fn cursor_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.adjust_scroll();
        }
    }

    /// Move cursor down
    fn cursor_down(&mut self) {
        if self.cursor < self.items.len().saturating_sub(1) {
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
        self.cursor = self.items.len().saturating_sub(1);
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

    /// Get the selected session
    fn get_selected(&self) -> Option<&SessionItem> {
        self.items.get(self.cursor)
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

        let inner = render_modal_block(modal_area, buf, &self.title, theme);

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
            let prefix = if is_selected { "▶ " } else { "  " };

            let path_suffix = if is_current {
                format!(" {}", t.sessions_current())
            } else {
                String::new()
            };

            let path_style = if is_selected {
                Style::default()
                    .fg(theme.fg)
                    .bg(theme.bg)
                    .add_modifier(Modifier::BOLD)
            } else if is_current {
                // Current session - same color as panel border and selected files
                Style::default().fg(theme.accented_fg)
            } else {
                Style::default().fg(theme.fg)
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

        let list = List::new(list_items).style(Style::default().bg(theme.bg));
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
                if let Some(item) = self.get_selected() {
                    if item.is_current {
                        // Current session selected - just close modal
                        Ok(Some(ModalResult::Cancelled))
                    } else {
                        Ok(Some(ModalResult::Confirmed(item.project_path.clone())))
                    }
                } else {
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
        use crate::{check_mouse_click_with_item_height, MouseClickResult};

        // Only handle left button press
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return Ok(None);
        }

        // Sessions items are 2 lines each
        const LINES_PER_ITEM: usize = 2;

        match check_mouse_click_with_item_height(
            mouse.column,
            mouse.row,
            None, // No modal area check for sessions modal
            self.last_list_area,
            self.scroll_offset,
            LINES_PER_ITEM,
        ) {
            MouseClickResult::OutsideModal | MouseClickResult::OutsideList => Ok(None),
            MouseClickResult::OnListItem(clicked_index) => {
                if clicked_index < self.items.len() {
                    let item = &self.items[clicked_index];
                    self.cursor = clicked_index;

                    if item.is_current {
                        // Current session clicked - just close modal
                        return Ok(Some(ModalResult::Cancelled));
                    } else {
                        return Ok(Some(ModalResult::Confirmed(item.project_path.clone())));
                    }
                }
                Ok(None)
            }
        }
    }
}
