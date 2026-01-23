//! Language selection dropdown with live preview.
//!
//! Simple list of languages with native names. Live preview is handled by applying
//! the language on cursor navigation (see menu_actions.rs).

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

/// Language dropdown with live preview on cursor navigation
pub struct LanguageDropdown<'a> {
    /// Language list: (code, native name)
    languages: Vec<(&'static str, &'static str)>,
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

impl<'a> LanguageDropdown<'a> {
    pub fn new(selected: usize, x: u16, y: u16, app_theme: &'a Theme) -> Self {
        let languages = i18n::get_language_list();

        // Calculate scroll offset to keep selected item visible
        let max_visible = 15;
        let scroll_offset = if selected >= max_visible {
            selected - max_visible + 1
        } else {
            0
        };

        Self {
            languages,
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
            .languages
            .iter()
            .map(|(_, name)| name.width())
            .max()
            .unwrap_or(10);
        // " " + name + padding
        (max_name_len + 4).min(30) as u16
    }

    /// Get the height of this dropdown
    pub fn height(&self) -> u16 {
        let items_count = self.languages.len().min(self.max_visible);
        (items_count + 2) as u16 // +2 for borders
    }

    /// Get language code by index
    pub fn get_language_code(&self, index: usize) -> Option<&'static str> {
        self.languages.get(index).map(|(code, _)| *code)
    }

    /// Get total number of languages
    pub fn len(&self) -> usize {
        self.languages.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.languages.is_empty()
    }

    pub fn render(&self, buf: &mut Buffer) {
        if self.languages.is_empty() {
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

        // Build list items - use panel colors (not modal colors)
        let visible_end = (self.scroll_offset + self.max_visible).min(self.languages.len());
        let visible_items = &self.languages[self.scroll_offset..visible_end];

        let items: Vec<ListItem> = visible_items
            .iter()
            .enumerate()
            .map(|(i, (_code, name))| {
                let actual_index = self.scroll_offset + i;
                let is_selected = actual_index == self.selected;

                // Panel-style colors: normal text on panel background, selection highlighted
                let item_style = if is_selected {
                    Style::default()
                        .fg(self.app_theme.selected_fg)
                        .bg(self.app_theme.selected_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.app_theme.fg).bg(self.app_theme.bg)
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

        // Use modal-style colors: bg background, accented_fg for border
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(self.app_theme.accented_fg))
                .style(Style::default().bg(self.app_theme.bg)),
        );

        list.render(area, buf);

        // Render scrollbar on right edge (inside border)
        let visible_count = self.languages.len().min(self.max_visible);
        let theme_colors = ThemeColors::from(self.app_theme);
        ScrollBar::render(
            buf,
            x + width - 1,            // Right border position
            y + 1,                    // Inside top border
            height.saturating_sub(2), // Inside borders
            self.scroll_offset,
            visible_count,
            self.languages.len(),
            &theme_colors,
            true, // Dropdown is always focused when visible
        );
    }
}

/// Find the index of the current language in the supported languages list
pub fn find_current_language_index() -> usize {
    let current = i18n::current_language();
    let languages = i18n::get_language_list();
    languages
        .iter()
        .position(|(code, _)| *code == current)
        .unwrap_or(0)
}
