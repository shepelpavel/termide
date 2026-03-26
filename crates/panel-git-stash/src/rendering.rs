//! Rendering functions for Git Stash Panel.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use unicode_width::UnicodeWidthStr;

use termide_core::ThemeColors;
use termide_git::{self as git};

use crate::types::Section;
use crate::GitStashPanel;

impl GitStashPanel {
    /// Render the full stash panel content.
    pub(crate) fn render_content(&mut self, area: Rect, buf: &mut Buffer, is_focused: bool) {
        if area.height < 4 {
            return;
        }

        let theme = self.cached_theme;

        let selected_style = Style::default()
            .fg(theme.bg)
            .bg(theme.fg)
            .add_modifier(Modifier::BOLD);
        let normal_style = Style::default().fg(theme.fg);
        let ref_style = Style::default().fg(theme.warning);
        let dim_style = Style::default()
            .fg(theme.disabled)
            .add_modifier(Modifier::DIM);

        let mut y = area.y;

        // === Header line: " [New]" button only (title is in the panel tab) ===
        let btn_label = "[New]";
        let btn_x = area.x + 1; // 1 char left padding
        let new_focused = is_focused && self.current_section == Section::NewButton;
        let btn_style = if new_focused {
            selected_style
        } else {
            Style::default().fg(theme.fg)
        };
        buf.set_string(btn_x, y, btn_label, btn_style);
        self.new_btn_area = Some(Rect::new(btn_x, y, btn_label.width() as u16, 1));
        y += 1;

        // Separator
        render_horizontal_line(area.x, y, area.width, buf, &theme);
        y += 1;

        // List takes all remaining space
        let list_height = (area.y + area.height).saturating_sub(y) as usize;
        self.visible_height = list_height;

        // List area
        if self.stash_entries.is_empty() {
            let msg = "  No stashes";
            buf.set_string(area.x, y, msg, dim_style);
        } else {
            for row in 0..list_height {
                let entry_idx = self.scroll + row;
                let Some(entry) = self.stash_entries.get(entry_idx) else {
                    break;
                };
                let is_selected =
                    entry_idx == self.cursor && is_focused && self.current_section == Section::List;

                let ref_part = format!(" {}  ", entry.ref_str);
                let msg_x = area.x + ref_part.width() as u16;
                let remaining = area.width.saturating_sub(ref_part.width() as u16) as usize;
                if is_selected {
                    for dx in 0..area.width {
                        buf[(area.x + dx, y)]
                            .set_symbol(" ")
                            .set_style(selected_style);
                    }
                    buf.set_string(area.x, y, &ref_part, selected_style);
                    if remaining > 0 {
                        let msg = git::truncate_right(&entry.message, remaining);
                        buf.set_string(msg_x, y, &msg, selected_style);
                    }
                } else {
                    buf.set_string(area.x, y, &ref_part, ref_style);
                    if remaining > 0 {
                        let msg = git::truncate_right(&entry.message, remaining);
                        buf.set_string(msg_x, y, &msg, normal_style);
                    }
                }
                y += 1;
            }
        }
    }
}

/// Draw a horizontal separator line.
fn render_horizontal_line(x: u16, y: u16, width: u16, buf: &mut Buffer, theme: &ThemeColors) {
    let style = Style::default().fg(theme.border);
    for i in 0..width {
        buf[(x + i, y)].set_symbol("─").set_style(style);
    }
}
