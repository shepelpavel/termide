//! Simple dropdown list overlay for selector widgets.

use ratatui::{
    buffer::Buffer,
    style::{Modifier, Style},
};
use unicode_width::UnicodeWidthStr;

use termide_core::ThemeColors;

/// Render a bordered dropdown list overlay directly into the buffer.
///
/// The dropdown draws a box at `(x, y)` and lists `items`, highlighting the
/// `cursor` row with selection colors and the `selected` row with cursor color.
/// Width auto-fits to the longest item (clamped to `max_width`).
/// Height is clamped to `max_height` items; the list scrolls to keep `cursor`
/// visible.
#[allow(clippy::too_many_arguments)]
pub fn render_simple_dropdown(
    items: &[String],
    selected: usize,
    cursor: usize,
    x: u16,
    y: u16,
    max_width: u16,
    max_height: u16,
    buf: &mut Buffer,
    theme: &ThemeColors,
) {
    if items.is_empty() {
        return;
    }

    let visible_count = items.len().min(max_height as usize);
    let scroll_offset = if cursor >= visible_count {
        cursor - visible_count + 1
    } else {
        0
    };

    // Auto-fit width to longest item + padding + borders
    let item_max_width = items.iter().map(|s| s.width()).max().unwrap_or(10);
    let width = (item_max_width + 4).min(max_width as usize) as u16;

    let border_style = Style::default().fg(theme.border_focused);
    let bg_style = Style::default()
        .bg(theme.bg)
        .remove_modifier(Modifier::all());

    // Clear area and draw border
    let dropdown_height = visible_count as u16 + 2; // +2 for top/bottom borders
    for dy in 0..dropdown_height {
        for dx in 0..width {
            let cell = &mut buf[(x + dx, y + dy)];
            cell.set_style(bg_style);
            if dy == 0 || dy == dropdown_height - 1 {
                if dx == 0 {
                    cell.set_symbol(if dy == 0 { "┌" } else { "└" });
                } else if dx == width - 1 {
                    cell.set_symbol(if dy == 0 { "┐" } else { "┘" });
                } else {
                    cell.set_symbol("─");
                }
                cell.set_style(border_style);
            } else if dx == 0 || dx == width - 1 {
                cell.set_symbol("│").set_style(border_style);
            } else {
                cell.set_symbol(" ");
            }
        }
    }

    // Draw items
    for (i, item) in items
        .iter()
        .skip(scroll_offset)
        .take(visible_count)
        .enumerate()
    {
        let item_y = y + 1 + i as u16;
        let is_cursor = scroll_offset + i == cursor;
        let is_selected = scroll_offset + i == selected;

        let style = if is_cursor {
            Style::default()
                .fg(theme.selection_fg)
                .bg(theme.selection_bg)
                .remove_modifier(Modifier::all())
        } else if is_selected {
            Style::default()
                .fg(theme.cursor)
                .remove_modifier(Modifier::all())
        } else {
            Style::default()
                .fg(theme.fg)
                .remove_modifier(Modifier::all())
        };

        // Truncate to fit inside borders
        let max_item_width = (width - 2) as usize;
        let display_item: std::borrow::Cow<str> = if item.width() > max_item_width {
            let mut end = 0;
            let mut w = 0;
            for ch in item.chars() {
                let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if w + cw > max_item_width {
                    break;
                }
                w += cw;
                end += ch.len_utf8();
            }
            std::borrow::Cow::Borrowed(&item[..end])
        } else {
            std::borrow::Cow::Borrowed(item)
        };

        // Clear line and draw item
        for dx in 1..width - 1 {
            buf[(x + dx, item_y)].set_symbol(" ").set_style(style);
        }
        buf.set_string(x + 1, item_y, display_item, style);
    }
}
