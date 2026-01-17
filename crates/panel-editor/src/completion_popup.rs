//! Completion popup for LSP code completion.
//!
//! Displays a dropdown list of completion items at the cursor position.

use lsp_types::{CompletionItem, CompletionItemKind, CompletionResponse};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use termide_theme::Theme;
use unicode_width::UnicodeWidthStr;

/// Maximum number of visible items in the popup.
const MAX_VISIBLE_ITEMS: usize = 10;

/// Maximum width of the popup.
const MAX_POPUP_WIDTH: u16 = 50;

/// Minimum width of the popup.
const MIN_POPUP_WIDTH: u16 = 20;

/// Completion popup state and rendering.
pub struct CompletionPopup {
    /// All completion items.
    items: Vec<CompletionItem>,
    /// Currently selected index.
    selected: usize,
    /// Scroll offset for long lists.
    scroll_offset: usize,
    /// Filter text (what user typed after trigger).
    filter: String,
    /// Filtered item indices.
    filtered_indices: Vec<usize>,
}

impl CompletionPopup {
    /// Create a new completion popup from LSP response.
    pub fn from_response(response: CompletionResponse) -> Self {
        let items = match response {
            CompletionResponse::Array(items) => items,
            CompletionResponse::List(list) => list.items,
        };

        let filtered_indices: Vec<usize> = (0..items.len()).collect();

        Self {
            items,
            selected: 0,
            scroll_offset: 0,
            filter: String::new(),
            filtered_indices,
        }
    }

    /// Check if popup has any items.
    pub fn is_empty(&self) -> bool {
        self.filtered_indices.is_empty()
    }

    /// Get count of visible (filtered) items (used in tests).
    #[cfg(test)]
    pub fn item_count(&self) -> usize {
        self.filtered_indices.len()
    }

    /// Update filter text and re-filter items.
    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_string();
        self.apply_filter();
    }

    /// Append character to filter.
    pub fn append_filter(&mut self, ch: char) {
        self.filter.push(ch);
        self.apply_filter();
    }

    /// Remove last character from filter.
    pub fn backspace_filter(&mut self) {
        self.filter.pop();
        self.apply_filter();
    }

    /// Apply filter to items.
    fn apply_filter(&mut self) {
        if self.filter.is_empty() {
            self.filtered_indices = (0..self.items.len()).collect();
        } else {
            let filter_lower = self.filter.to_lowercase();
            self.filtered_indices = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    let label_lower = item.label.to_lowercase();
                    // Substring match: filter must appear as contiguous substring
                    label_lower.contains(&filter_lower)
                })
                .map(|(idx, _)| idx)
                .collect();
        }

        // Reset selection if it's out of bounds
        if self.selected >= self.filtered_indices.len() {
            self.selected = 0;
        }
        self.scroll_offset = 0;
    }

    /// Select next item.
    pub fn select_next(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.selected = (self.selected + 1) % self.filtered_indices.len();
            self.ensure_visible();
        }
    }

    /// Select previous item.
    pub fn select_prev(&mut self) {
        if !self.filtered_indices.is_empty() {
            if self.selected == 0 {
                self.selected = self.filtered_indices.len() - 1;
            } else {
                self.selected -= 1;
            }
            self.ensure_visible();
        }
    }

    /// Ensure selected item is visible.
    fn ensure_visible(&mut self) {
        let max_visible = MAX_VISIBLE_ITEMS.min(self.filtered_indices.len());
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + max_visible {
            self.scroll_offset = self.selected - max_visible + 1;
        }
    }

    /// Select item at visual row offset from popup top.
    /// Returns true if a valid item was selected.
    pub fn select_at_row(&mut self, row: usize) -> bool {
        let idx = self.scroll_offset + row;
        if idx < self.filtered_indices.len() {
            self.selected = idx;
            true
        } else {
            false
        }
    }

    /// Scroll popup up by given amount.
    pub fn scroll_up(&mut self, amount: usize) {
        if self.scroll_offset > 0 {
            self.scroll_offset = self.scroll_offset.saturating_sub(amount);
            // Adjust selection to stay visible
            let max_visible = MAX_VISIBLE_ITEMS.min(self.filtered_indices.len());
            if self.selected >= self.scroll_offset + max_visible {
                self.selected = self.scroll_offset + max_visible - 1;
            }
        }
    }

    /// Scroll popup down by given amount.
    pub fn scroll_down(&mut self, amount: usize) {
        let max_visible = MAX_VISIBLE_ITEMS.min(self.filtered_indices.len());
        let max_scroll = self.filtered_indices.len().saturating_sub(max_visible);
        if self.scroll_offset < max_scroll {
            self.scroll_offset = (self.scroll_offset + amount).min(max_scroll);
            // Adjust selection to stay visible
            if self.selected < self.scroll_offset {
                self.selected = self.scroll_offset;
            }
        }
    }

    /// Get currently selected completion item.
    pub fn selected_item(&self) -> Option<&CompletionItem> {
        self.filtered_indices
            .get(self.selected)
            .and_then(|&idx| self.items.get(idx))
    }

    /// Get the text to insert for the selected item.
    pub fn selected_insert_text(&self) -> Option<String> {
        self.selected_item().map(|item| {
            // Use insert_text if available, otherwise label
            item.insert_text
                .clone()
                .unwrap_or_else(|| item.label.clone())
        })
    }

    /// Render the completion popup.
    ///
    /// `cursor_x` and `cursor_y` are the screen coordinates of the cursor.
    /// Returns the actual rect used for rendering (for overlay purposes).
    pub fn render(
        &self,
        buf: &mut Buffer,
        area: Rect,
        cursor_x: u16,
        cursor_y: u16,
        theme: &Theme,
    ) -> Option<Rect> {
        if self.filtered_indices.is_empty() {
            return None;
        }

        // Calculate popup dimensions
        let max_label_width = self
            .filtered_indices
            .iter()
            .filter_map(|&idx| self.items.get(idx))
            .map(|item| item.label.width() + 3) // +3 for icon and spacing
            .max()
            .unwrap_or(MIN_POPUP_WIDTH as usize);

        let popup_width = (max_label_width as u16).clamp(MIN_POPUP_WIDTH, MAX_POPUP_WIDTH);

        let visible_count = self.filtered_indices.len().min(MAX_VISIBLE_ITEMS);
        let popup_height = visible_count as u16;

        // Calculate popup position
        // Try to show below cursor, flip above if not enough space
        let (popup_x, popup_y) =
            self.calculate_position(area, cursor_x, cursor_y, popup_width, popup_height);

        let popup_rect = Rect::new(popup_x, popup_y, popup_width, popup_height);

        // Render background (use accented colors for popup)
        // Make sure we stay within buffer bounds
        let bg_style = Style::default().bg(theme.accented_bg).fg(theme.fg);
        for y in popup_rect.top()..popup_rect.bottom() {
            for x in popup_rect.left()..popup_rect.right() {
                if x >= buf.area.left()
                    && x < buf.area.right()
                    && y >= buf.area.top()
                    && y < buf.area.bottom()
                {
                    buf[(x, y)].set_style(bg_style);
                    buf[(x, y)].set_char(' ');
                }
            }
        }

        // Render items
        let selected_style = Style::default()
            .bg(theme.selected_bg)
            .fg(theme.selected_fg)
            .add_modifier(Modifier::BOLD);

        for (display_idx, &item_idx) in self
            .filtered_indices
            .iter()
            .skip(self.scroll_offset)
            .take(visible_count)
            .enumerate()
        {
            let item = &self.items[item_idx];
            let y = popup_rect.top() + display_idx as u16;

            let is_selected = self.scroll_offset + display_idx == self.selected;
            let style = if is_selected {
                selected_style
            } else {
                bg_style
            };

            // Render icon
            let icon = kind_icon(item.kind);
            let icon_x = popup_rect.left();
            if icon_x < buf.area.width && y < buf.area.height {
                buf[(icon_x, y)].set_char(icon);
                buf[(icon_x, y)].set_style(style);
            }

            // Render label
            let label_start = popup_rect.left() + 2;
            let max_label_len = (popup_rect.right().saturating_sub(label_start)) as usize;
            let label = truncate_string(&item.label, max_label_len);

            for (i, ch) in label.chars().enumerate() {
                let x = label_start + i as u16;
                if x < popup_rect.right() && x < buf.area.width && y < buf.area.height {
                    buf[(x, y)].set_char(ch);
                    buf[(x, y)].set_style(style);
                }
            }

            // Fill rest of line with style
            for x in (label_start + label.len() as u16)..popup_rect.right() {
                if x < buf.area.width && y < buf.area.height {
                    buf[(x, y)].set_style(style);
                }
            }
        }

        // Render scroll indicators if needed
        if self.scroll_offset > 0 {
            let x = popup_rect.right().saturating_sub(1);
            let y = popup_rect.top();
            if x < buf.area.width && y < buf.area.height {
                buf[(x, y)].set_char('▲');
            }
        }
        if self.scroll_offset + visible_count < self.filtered_indices.len() {
            let x = popup_rect.right().saturating_sub(1);
            let y = popup_rect.bottom().saturating_sub(1);
            if x < buf.area.width && y < buf.area.height {
                buf[(x, y)].set_char('▼');
            }
        }

        Some(popup_rect)
    }

    /// Calculate popup position.
    fn calculate_position(
        &self,
        area: Rect,
        cursor_x: u16,
        cursor_y: u16,
        width: u16,
        height: u16,
    ) -> (u16, u16) {
        // Try to position below cursor
        let mut x = cursor_x;
        let mut y = cursor_y + 1;

        // Adjust x if popup would go off right edge
        if x + width > area.right() {
            x = area.right().saturating_sub(width);
        }

        // Flip above cursor if not enough space below
        if y + height > area.bottom() {
            if cursor_y >= height {
                y = cursor_y.saturating_sub(height);
            } else {
                // Not enough space above either, just show at bottom
                y = area.bottom().saturating_sub(height);
            }
        }

        (x.max(area.left()), y.max(area.top()))
    }
}

/// Get icon character for completion item kind.
fn kind_icon(kind: Option<CompletionItemKind>) -> char {
    match kind {
        Some(CompletionItemKind::FUNCTION) | Some(CompletionItemKind::METHOD) => 'ƒ',
        Some(CompletionItemKind::VARIABLE) => 'v',
        Some(CompletionItemKind::FIELD) => '→',
        Some(CompletionItemKind::CLASS) | Some(CompletionItemKind::STRUCT) => 'C',
        Some(CompletionItemKind::INTERFACE) => 'I',
        Some(CompletionItemKind::MODULE) => 'M',
        Some(CompletionItemKind::PROPERTY) => 'P',
        Some(CompletionItemKind::UNIT) => 'U',
        Some(CompletionItemKind::VALUE) => '=',
        Some(CompletionItemKind::ENUM) => 'E',
        Some(CompletionItemKind::KEYWORD) => 'K',
        Some(CompletionItemKind::SNIPPET) => 'S',
        Some(CompletionItemKind::TEXT) => 'T',
        Some(CompletionItemKind::CONSTANT) => 'c',
        Some(CompletionItemKind::TYPE_PARAMETER) => '<',
        _ => '·',
    }
}

/// Truncate string to max length with ellipsis.
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.width() <= max_len {
        s.to_string()
    } else if max_len > 1 {
        let mut result = String::new();
        let mut width = 0;
        for ch in s.chars() {
            let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
            if width + ch_width + 1 > max_len {
                result.push('…');
                break;
            }
            result.push(ch);
            width += ch_width;
        }
        result
    } else {
        s.chars().take(max_len).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_string() {
        assert_eq!(truncate_string("hello", 10), "hello");
        assert_eq!(truncate_string("hello world", 8), "hello w…");
        assert_eq!(truncate_string("hi", 2), "hi");
    }

    #[test]
    fn test_empty_popup() {
        let response = CompletionResponse::Array(vec![]);
        let popup = CompletionPopup::from_response(response);
        assert!(popup.is_empty());
    }

    #[test]
    fn test_filter() {
        let items = vec![
            CompletionItem {
                label: "function_one".to_string(),
                ..Default::default()
            },
            CompletionItem {
                label: "function_two".to_string(),
                ..Default::default()
            },
            CompletionItem {
                label: "variable".to_string(),
                ..Default::default()
            },
        ];
        let response = CompletionResponse::Array(items);
        let mut popup = CompletionPopup::from_response(response);

        assert_eq!(popup.item_count(), 3);

        popup.set_filter("func");
        assert_eq!(popup.item_count(), 2);

        popup.set_filter("var");
        assert_eq!(popup.item_count(), 1);

        popup.set_filter("");
        assert_eq!(popup.item_count(), 3);
    }
}
