//! Dropdown menu widget.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    prelude::Widget,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem},
};
use unicode_width::UnicodeWidthStr;

use termide_i18n as i18n;
use termide_theme::Theme;

/// Dropdown menu item
#[derive(Debug, Clone)]
pub struct DropdownItem {
    pub label: String,
    pub key: String,
    /// Whether this item opens a submenu
    pub has_submenu: bool,
}

impl DropdownItem {
    pub fn new(label: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            key: key.into(),
            has_submenu: false,
        }
    }

    /// Mark this item as having a submenu
    pub fn with_submenu(mut self) -> Self {
        self.has_submenu = true;
        self
    }
}

/// Dropdown menu
pub struct Dropdown<'a> {
    items: &'a [DropdownItem],
    selected: usize,
    x: u16,
    y: u16,
    theme: &'a Theme,
}

impl<'a> Dropdown<'a> {
    pub fn new(
        items: &'a [DropdownItem],
        selected: usize,
        x: u16,
        y: u16,
        theme: &'a Theme,
    ) -> Self {
        Self {
            items,
            selected,
            x,
            y,
            theme,
        }
    }

    /// Get the width of this dropdown
    pub fn width(&self) -> u16 {
        let max_label_len = self
            .items
            .iter()
            .map(|item| item.label.width())
            .max()
            .unwrap_or(0);
        // " " + label + " ▶" (or "  ")
        (max_label_len + 4).min(40) as u16
    }

    /// Get the height of this dropdown
    pub fn height(&self) -> u16 {
        (self.items.len() + 2) as u16 // +2 for borders
    }

    pub fn render(&self, buf: &mut Buffer) {
        if self.items.is_empty() {
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

        // Create list of items
        let items: Vec<ListItem> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let is_selected = i == self.selected;

                let base_style = if is_selected {
                    Style::default()
                        .bg(self.theme.selected_bg)
                        .fg(self.theme.selected_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.theme.fg)
                };

                let mut spans = vec![Span::styled(" ", base_style)];
                spans.push(Span::styled(&item.label, base_style));

                // Add submenu indicator or padding
                let label_width = item.label.width();
                let padding_len = (width as usize).saturating_sub(label_width + 4); // -4 for " " + " ▶"
                if padding_len > 0 {
                    spans.push(Span::styled(" ".repeat(padding_len), base_style));
                }

                if item.has_submenu {
                    spans.push(Span::styled(" ▶", base_style));
                } else {
                    spans.push(Span::styled("  ", base_style));
                }

                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(self.theme.accented_fg))
                .style(Style::default().bg(self.theme.bg)),
        );

        list.render(area, buf);
    }
}

/// Get preferences submenu items
pub fn get_preferences_items() -> Vec<DropdownItem> {
    let t = i18n::t();
    vec![
        DropdownItem::new(t.preferences_themes(), "themes").with_submenu(),
        DropdownItem::new(t.preferences_edit(), "edit_preferences"),
    ]
}
