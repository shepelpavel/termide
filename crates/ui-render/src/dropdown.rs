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

use termide_core::ThemeColors;
use termide_i18n as i18n;
use termide_theme::Theme;
use termide_ui::ScrollBar;

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

/// Maximum visible items in dropdown before scrolling
const MAX_VISIBLE_ITEMS: usize = 20;

/// Dropdown menu
pub struct Dropdown<'a> {
    items: &'a [DropdownItem],
    selected: usize,
    x: u16,
    y: u16,
    theme: &'a Theme,
    max_visible: usize,
    scroll_offset: usize,
}

impl<'a> Dropdown<'a> {
    pub fn new(
        items: &'a [DropdownItem],
        selected: usize,
        x: u16,
        y: u16,
        theme: &'a Theme,
    ) -> Self {
        let max_visible = MAX_VISIBLE_ITEMS.min(items.len());
        // Calculate scroll offset to keep selected item visible
        let scroll_offset = if selected >= max_visible {
            selected - max_visible + 1
        } else {
            0
        };

        Self {
            items,
            selected,
            x,
            y,
            theme,
            max_visible,
            scroll_offset,
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
        let visible_count = self.items.len().min(self.max_visible);
        (visible_count + 2) as u16 // +2 for borders
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

        // Get visible items
        let visible_end = (self.scroll_offset + self.max_visible).min(self.items.len());
        let visible_items = &self.items[self.scroll_offset..visible_end];

        // Create list of items
        let items: Vec<ListItem> = visible_items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let actual_index = self.scroll_offset + i;
                let is_selected = actual_index == self.selected;

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

        // Render scrollbar on right edge (inside border)
        let visible_count = self.items.len().min(self.max_visible);
        let theme_colors = ThemeColors::from(self.theme);
        ScrollBar::render(
            buf,
            x + width - 1,            // Right border position
            y + 1,                    // Inside top border
            height.saturating_sub(2), // Inside borders
            self.scroll_offset,
            visible_count,
            self.items.len(),
            &theme_colors,
            true, // Dropdown is always focused when visible
        );
    }
}

/// Get sessions submenu items
pub fn get_sessions_items() -> Vec<DropdownItem> {
    let t = i18n::t();
    vec![
        DropdownItem::new(t.sessions_new(), "new_session"),
        DropdownItem::new(t.sessions_switch(), "switch_session"),
        DropdownItem::new(t.sessions_change_root(), "change_root"),
    ]
}

/// Number of items in Sessions submenu
pub const SESSIONS_SUBMENU_ITEM_COUNT: usize = 3;

/// Get tools submenu items
pub fn get_tools_items() -> Vec<DropdownItem> {
    let t = i18n::t();
    vec![
        DropdownItem::new(t.tools_files(), "files"),
        DropdownItem::new(t.tools_terminal(), "terminal"),
        DropdownItem::new(t.tools_editor(), "editor"),
        DropdownItem::new(t.tools_git_status(), "git_status"),
        DropdownItem::new(t.tools_git_log(), "git_log"),
        DropdownItem::new(t.tools_journal(), "journal"),
    ]
}

/// Number of items in Tools submenu
pub const TOOLS_SUBMENU_ITEM_COUNT: usize = 6;

/// Get options submenu items
pub fn get_options_items() -> Vec<DropdownItem> {
    let t = i18n::t();
    vec![
        DropdownItem::new(t.preferences_themes(), "themes").with_submenu(),
        DropdownItem::new(t.preferences_edit(), "edit_preferences"),
        DropdownItem::new(t.options_help(), "help"),
        DropdownItem::new(t.menu_quit(), "quit"),
    ]
}

/// Number of items in Options submenu
pub const OPTIONS_SUBMENU_ITEM_COUNT: usize = 4;
