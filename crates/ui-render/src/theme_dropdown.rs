//! Theme selection dropdown with live preview.
//!
//! Simple list of theme names. Live preview is handled by applying
//! the theme on cursor navigation (see menu_actions.rs).

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    prelude::Widget,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem},
};
use unicode_width::UnicodeWidthStr;

use termide_theme::Theme;

/// Theme dropdown with live preview on cursor navigation
pub struct ThemeDropdown<'a> {
    /// Theme names to display
    theme_names: &'a [String],
    /// Selected item index
    selected: usize,
    /// X position
    x: u16,
    /// Y position
    y: u16,
    /// App theme for borders
    app_theme: &'a Theme,
    /// Maximum visible items (for scrolling)
    max_visible: usize,
    /// Scroll offset
    scroll_offset: usize,
}

impl<'a> ThemeDropdown<'a> {
    pub fn new(
        theme_names: &'a [String],
        selected: usize,
        x: u16,
        y: u16,
        app_theme: &'a Theme,
    ) -> Self {
        // Calculate scroll offset to keep selected item visible
        let max_visible = 12;
        let scroll_offset = if selected >= max_visible {
            selected - max_visible + 1
        } else {
            0
        };

        Self {
            theme_names,
            selected,
            x,
            y,
            app_theme,
            max_visible,
            scroll_offset,
        }
    }

    /// Get the width of this dropdown
    pub fn width(&self) -> u16 {
        let max_name_len = self
            .theme_names
            .iter()
            .map(|n| n.width())
            .max()
            .unwrap_or(10);
        // "▶ " + name + padding
        (max_name_len + 4).min(30) as u16
    }

    /// Get the height of this dropdown
    pub fn height(&self) -> u16 {
        let items_count = self.theme_names.len().min(self.max_visible);
        (items_count + 2) as u16 // +2 for borders
    }

    pub fn render(&self, buf: &mut Buffer) {
        if self.theme_names.is_empty() {
            return;
        }

        let width = self.width();
        let height = self.height();

        // Check screen boundaries
        let max_x = buf.area.width.saturating_sub(width);
        let max_y = buf.area.height.saturating_sub(height);
        let x = self.x.min(max_x);
        let y = self.y.min(max_y);

        let area = Rect {
            x,
            y,
            width,
            height,
        };

        // Clear area under dropdown
        Clear.render(area, buf);

        // Build list items - simple text, live preview via theme switching
        let visible_end = (self.scroll_offset + self.max_visible).min(self.theme_names.len());
        let visible_items = &self.theme_names[self.scroll_offset..visible_end];

        let items: Vec<ListItem> = visible_items
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let actual_index = self.scroll_offset + i;
                let is_selected = actual_index == self.selected;

                // Use current app theme colors (which changes on cursor move)
                let item_style = if is_selected {
                    Style::default()
                        .fg(self.app_theme.fg)
                        .bg(self.app_theme.accented_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.app_theme.bg).bg(self.app_theme.fg)
                };

                // Build line with padding
                let content = format!(" {}", name);
                let padding_len = (width as usize).saturating_sub(content.width() + 2);
                let padded = if padding_len > 0 {
                    format!("{}{}", content, " ".repeat(padding_len))
                } else {
                    content
                };

                ListItem::new(Line::from(Span::styled(padded, item_style)))
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(self.app_theme.accented_fg))
                .style(Style::default().bg(self.app_theme.bg)),
        );

        list.render(area, buf);
    }
}
