//! Theme selection dropdown with color previews.

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

/// Theme dropdown with color previews for each theme
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
        // "▶ ● " + name + padding
        (max_name_len + 6).min(30) as u16
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

        // Build list items with theme previews
        let visible_end = (self.scroll_offset + self.max_visible).min(self.theme_names.len());
        let visible_items = &self.theme_names[self.scroll_offset..visible_end];

        let items: Vec<ListItem> = visible_items
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let actual_index = self.scroll_offset + i;
                let is_selected = actual_index == self.selected;

                // Get the theme for color preview
                let preview_theme = Theme::get_by_name(name);

                // Build spans for the line
                let mut spans = Vec::new();

                // Calculate styles
                let item_style = if is_selected {
                    // Inverted colors for selection
                    Style::default()
                        .fg(preview_theme.bg)
                        .bg(preview_theme.fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    // Preview: use theme's own colors
                    Style::default().fg(preview_theme.fg).bg(preview_theme.bg)
                };

                // Circle style: accented_fg color, but follows selection background
                let circle_style = if is_selected {
                    Style::default()
                        .fg(preview_theme.accented_fg)
                        .bg(preview_theme.fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(preview_theme.accented_fg)
                        .bg(preview_theme.bg)
                };

                // Selection marker (▶) - moves with cursor
                let marker = if is_selected { "▶" } else { " " };
                spans.push(Span::styled(marker, item_style));

                // Color preview circle (●) with accented_fg color
                spans.push(Span::styled(" ● ", circle_style));

                // Theme name
                spans.push(Span::styled(name.as_str(), item_style));

                // Padding to fill width with background color
                let content_width = 4 + name.width(); // "▶ ● " + name
                let padding_len = (width as usize).saturating_sub(content_width + 2); // -2 for borders
                if padding_len > 0 {
                    spans.push(Span::styled(" ".repeat(padding_len), item_style));
                }

                ListItem::new(Line::from(spans))
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
