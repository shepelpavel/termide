//! Inline selector widget for dropdown-style selectors.

use ratatui::{
    buffer::Buffer,
    style::{Modifier, Style},
};
use unicode_width::UnicodeWidthStr;

use termide_core::ThemeColors;

/// Inline selector widget that renders as `[label ▶]` or `[label ▼]`
pub struct InlineSelector<'a> {
    label: &'a str,
    is_open: bool,
    is_focused: bool,
    theme: &'a ThemeColors,
}

impl<'a> InlineSelector<'a> {
    /// Create a new inline selector
    pub fn new(label: &'a str, is_open: bool, is_focused: bool, theme: &'a ThemeColors) -> Self {
        Self {
            label,
            is_open,
            is_focused,
            theme,
        }
    }

    /// Render the selector at the given position
    /// Returns the rendered width
    pub fn render(&self, x: u16, y: u16, max_width: u16, buf: &mut Buffer) -> u16 {
        // Arrow: ▶/► when collapsed, ▼ when expanded
        const ARROW_CLOSED: &str = if cfg!(windows) { "►" } else { "▶" };
        let arrow = if self.is_open { "▼" } else { ARROW_CLOSED };

        let style = if self.is_focused {
            // Inverted cursor style
            Style::default()
                .fg(self.theme.bg)
                .bg(self.theme.fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.theme.fg)
        };

        // Truncate label if needed: "[" + label + " " + arrow + "]" = 4 extra chars
        let max_label_width = max_width.saturating_sub(4) as usize;
        let truncated_label = if self.label.width() > max_label_width {
            let mut end = 0;
            let mut w = 0;
            for ch in self.label.chars() {
                let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if w + cw > max_label_width {
                    break;
                }
                w += cw;
                end += ch.len_utf8();
            }
            &self.label[..end]
        } else {
            self.label
        };

        let text = format!("[{} {}]", truncated_label, arrow);
        let text_width = text.width() as u16;
        buf.set_string(x, y, &text, style);

        text_width
    }
}
