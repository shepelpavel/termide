//! A small scrollable dropdown list, shared by the database and table
//! selectors so both behave and render identically.

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use termide_core::ThemeColors;

/// Outcome of a key press while the dropdown is open.
pub(crate) enum DropdownKey {
    /// Cursor moved / nothing else.
    Nav,
    /// An item was chosen; carries its index.
    Pick(usize),
    /// The dropdown was dismissed.
    Closed,
    /// Key not handled by the dropdown.
    Unhandled,
}

/// Open/scroll state for one selector's list.
#[derive(Default)]
pub(crate) struct Dropdown {
    pub open: bool,
    pub cursor: usize,
    pub scroll: usize,
    /// Visible row count (set during render; used for PageUp/Down + scrolling).
    pub page_size: usize,
}

impl Dropdown {
    pub fn open_at(&mut self, index: usize) {
        self.open = true;
        self.cursor = index;
    }

    /// Handle a key while the list is open.
    pub fn handle_key(&mut self, code: KeyCode, len: usize) -> DropdownKey {
        match code {
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                DropdownKey::Nav
            }
            KeyCode::Down => {
                if self.cursor + 1 < len {
                    self.cursor += 1;
                }
                DropdownKey::Nav
            }
            KeyCode::PageUp => {
                self.cursor = self.cursor.saturating_sub(self.page_size.max(1));
                DropdownKey::Nav
            }
            KeyCode::PageDown => {
                self.cursor = (self.cursor + self.page_size.max(1)).min(len.saturating_sub(1));
                DropdownKey::Nav
            }
            KeyCode::Home => {
                self.cursor = 0;
                DropdownKey::Nav
            }
            KeyCode::End => {
                self.cursor = len.saturating_sub(1);
                DropdownKey::Nav
            }
            KeyCode::Enter => {
                self.open = false;
                DropdownKey::Pick(self.cursor)
            }
            KeyCode::Esc => {
                self.open = false;
                DropdownKey::Closed
            }
            _ => DropdownKey::Unhandled,
        }
    }

    /// Map a clicked screen row to a list index (None if above the list).
    pub fn index_at_row(&self, clicked: u16, list_top: u16) -> Option<usize> {
        if clicked < list_top {
            return None;
        }
        Some(self.scroll + (clicked - list_top) as usize)
    }

    /// Render the list as a scrollable overlay anchored under `area`'s top.
    /// Width adapts to the longest item; the window follows the cursor.
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, items: &[String], theme: &ThemeColors) {
        let base = Style::default().fg(theme.fg).bg(theme.bg);
        let visible = (area.height.saturating_sub(1) as usize).max(1);
        self.page_size = visible;
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + visible {
            self.scroll = self.cursor + 1 - visible;
        }
        let start = self.scroll.min(items.len());
        let end = (start + visible).min(items.len());
        let longest = items
            .iter()
            .map(|n| UnicodeWidthStr::width(n.as_str()))
            .max()
            .unwrap_or(0);
        let width = ((longest + 2) as u16).clamp(10, area.width);
        let y0 = area.y + 1;
        for (row, i) in (start..end).enumerate() {
            let y = y0 + row as u16;
            if y >= area.y + area.height {
                break;
            }
            let style = if i == self.cursor {
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.selection_bg)
            } else {
                base
            };
            let blanks = " ".repeat(width as usize);
            buf.set_stringn(area.x, y, &blanks, width as usize, style);
            buf.set_stringn(area.x + 1, y, &items[i], width as usize - 1, style);
        }
    }
}
