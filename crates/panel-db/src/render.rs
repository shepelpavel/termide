//! Rendering for [`DbPanel`]: table selector, column headers (with the active
//! sort arrow) and the 2D data grid with a cell cursor.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr;

use termide_db::SortDir;
use termide_ui_render::InlineSelector;

use crate::{ConnState, DbPanel, Section};

const MAX_COL_WIDTH: usize = 40;
const MIN_COL_WIDTH: usize = 3;
/// Display width a column occupies on screen: ` content ` slot + "│" divider.
const COL_OVERHEAD: usize = 3;

impl DbPanel {
    pub(crate) fn render_content(&mut self, area: Rect, buf: &mut Buffer, is_focused: bool) {
        if area.width < 4 || area.height < 2 {
            return;
        }
        let theme = self.cached_theme;
        let base = Style::default().fg(theme.fg).bg(theme.bg);

        // Reset mouse hit-test geometry for this frame.
        self.geom.selector_y = area.y;
        self.geom.header_y = None;
        self.geom.data_y0 = area.y + 2;
        self.geom.columns.clear();
        // Dropdown page = the body height (no scrollbar; we page instead).
        self.dropdown_page_size = (area.height.saturating_sub(1)).max(1) as usize;

        // --- selector row (bracketed chip, like the git-status selectors) ---
        let sel_focused = is_focused && self.section == Section::TableSelector;
        let table_label = self
            .selected_table
            .clone()
            .unwrap_or_else(|| termide_i18n::t().db_no_table().to_string());
        fill_line(buf, area.x, area.y, area.width, base);
        let selector =
            InlineSelector::new(&table_label, self.table_dropdown_open, sel_focused, &theme);
        selector.render(area.x, area.y, area.width, buf);

        // --- body area below selector ---
        let body = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(1),
        };

        let tr = termide_i18n::t();
        match &self.conn {
            ConnState::Connecting(_) => {
                self.center_message(
                    buf,
                    body,
                    tr.db_connecting(),
                    base.fg(self.cached_theme.info),
                );
                return;
            }
            ConnState::Failed(msg) => {
                let style = base.fg(self.cached_theme.error);
                self.center_message(buf, body, &tr.db_connection_failed_fmt(msg), style);
                return;
            }
            ConnState::Connected(_) => {}
        }

        if self.selected_table.is_none() {
            self.center_message(buf, body, tr.db_no_tables(), base);
            return;
        }

        self.render_grid(buf, body, is_focused);

        // Dropdown overlay drawn last so it sits above the grid.
        if self.table_dropdown_open {
            self.render_dropdown(buf, area);
        }
    }

    #[allow(clippy::needless_range_loop)]
    fn render_grid(&mut self, buf: &mut Buffer, area: Rect, is_focused: bool) {
        let theme = self.cached_theme;
        let base = Style::default().fg(theme.fg).bg(theme.bg);
        let names = self.column_names();
        if names.is_empty() {
            self.center_message(buf, area, termide_i18n::t().db_loading(), base);
            return;
        }

        // Header occupies row 0; data fills the rest.
        let data_height = area.height.saturating_sub(1) as usize;
        self.visible_rows = data_height;

        // Keep the fetch window equal to the visible height: one fetched
        // window is exactly one screen, so we page instead of scrolling within
        // a 200-row buffer. Re-fetch when the viewport height changes (resize).
        let want = data_height.max(1) as u64;
        if want != self.page_rows {
            self.page_rows = want;
            self.reload_page();
        }

        // Vertical scroll: keep the cursor row visible.
        if self.cursor_row < self.row_scroll {
            self.row_scroll = self.cursor_row;
        } else if data_height > 0 && self.cursor_row >= self.row_scroll + data_height {
            self.row_scroll = self.cursor_row + 1 - data_height;
        }

        // Column widths from the visible window sample.
        let mut widths = self.column_widths(&names);
        // Reserve room for the sort arrow in the sorted column header.
        let sorted = self.order_by.first().cloned();
        if let Some((c, _)) = &sorted {
            if let Some(idx) = names.iter().position(|n| n == c) {
                widths[idx] = (widths[idx] + 2).min(MAX_COL_WIDTH);
            }
        }

        // Horizontal scroll: keep the cursor column visible.
        self.adjust_col_scroll(&widths, area.width as usize);

        let max_x = area.x + area.width;
        let border = base.fg(theme.border);

        // Each column is a slot ` content ` (one space of padding on each side);
        // slots are separated by a "│" divider, so a slot highlighted edge-to-edge
        // reads as one cell bounded by the dividers.

        // --- header row ---
        self.geom.header_y = Some(area.y);
        self.geom.data_y0 = area.y + 1;
        fill_line(buf, area.x, area.y, area.width, base);
        let mut x = area.x;
        for j in self.col_scroll..names.len() {
            if x >= max_x {
                break;
            }
            let mut label = names[j].clone();
            if let Some((c, d)) = &sorted {
                if *c == names[j] {
                    label.push(' ');
                    label.push_str(if *d == SortDir::Asc { "↑" } else { "↓" });
                }
            }
            let slot = format!(" {} ", pad(&label, widths[j]));
            let slot_start = x;
            x = put(
                buf,
                x,
                area.y,
                max_x,
                &slot,
                base.add_modifier(Modifier::BOLD),
            );
            self.geom.columns.push((j, slot_start, x));
            x = put(buf, x, area.y, max_x, "│", border);
        }

        // --- data rows ---
        // Selected row: normal colours but bold. Selected cell: inverse video
        // across the whole slot (border to border).
        let rows = &self.page.rows;
        for vis in 0..data_height {
            let abs = self.row_scroll + vis;
            if abs >= rows.len() {
                break;
            }
            let y = area.y + 1 + vis as u16;
            let is_cur_row = is_focused && self.section == Section::Grid && abs == self.cursor_row;
            let row_style = if is_cur_row {
                base.add_modifier(Modifier::BOLD)
            } else {
                base
            };
            fill_line(buf, area.x, y, area.width, base);

            let row = &rows[abs];
            let mut x = area.x;
            for j in self.col_scroll..names.len() {
                if x >= max_x {
                    break;
                }
                let value = row.get(j);
                let (text, is_null) = match value {
                    Some(v) if v.is_null() => ("NULL".to_string(), true),
                    Some(v) => (v.display(), false),
                    None => (String::new(), false),
                };
                let slot = format!(" {} ", pad(&text, widths[j]));
                let is_cur_cell = is_cur_row && j == self.cursor_col;
                let mut style = row_style;
                if is_null && !is_cur_cell {
                    style = style.fg(theme.disabled);
                }
                if is_cur_cell {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                x = put(buf, x, y, max_x, &slot, style);
                x = put(buf, x, y, max_x, "│", border);
            }
        }

        if self.loading {
            let style = base.fg(theme.info);
            let label = format!(" {} ", termide_i18n::t().db_loading());
            buf.set_stringn(area.x, area.y, &label, area.width as usize, style);
        }
    }

    /// Compute per-column display widths from header + the visible window.
    fn column_widths(&self, names: &[String]) -> Vec<usize> {
        let mut widths: Vec<usize> = names
            .iter()
            .map(|n| UnicodeWidthStr::width(n.as_str()).max(MIN_COL_WIDTH))
            .collect();
        for row in &self.page.rows {
            for (j, w) in widths.iter_mut().enumerate() {
                if let Some(v) = row.get(j) {
                    let text = if v.is_null() {
                        "NULL".to_string()
                    } else {
                        v.display()
                    };
                    let cw = UnicodeWidthStr::width(text.as_str());
                    if cw > *w {
                        *w = cw;
                    }
                }
            }
        }
        for w in widths.iter_mut() {
            *w = (*w).min(MAX_COL_WIDTH);
        }
        widths
    }

    fn adjust_col_scroll(&mut self, widths: &[usize], avail: usize) {
        if self.cursor_col < self.col_scroll {
            self.col_scroll = self.cursor_col;
            return;
        }
        // Grow col_scroll until the cursor column fits within `avail`.
        loop {
            let mut used = 0usize;
            let mut last_visible = self.col_scroll;
            for (j, w) in widths.iter().enumerate().skip(self.col_scroll) {
                let need = w + COL_OVERHEAD;
                if used + need > avail && j > self.col_scroll {
                    break;
                }
                used += need;
                last_visible = j;
            }
            if self.cursor_col <= last_visible || self.col_scroll >= widths.len().saturating_sub(1)
            {
                break;
            }
            self.col_scroll += 1;
        }
    }

    fn render_dropdown(&mut self, buf: &mut Buffer, area: Rect) {
        let theme = self.cached_theme;
        let base = Style::default().fg(theme.fg).bg(theme.bg);
        // Scrollable list: keep the cursor within the visible window.
        let visible = self.dropdown_page_size.max(1);
        if self.dropdown_cursor < self.dropdown_scroll {
            self.dropdown_scroll = self.dropdown_cursor;
        } else if self.dropdown_cursor >= self.dropdown_scroll + visible {
            self.dropdown_scroll = self.dropdown_cursor + 1 - visible;
        }
        let start = self.dropdown_scroll.min(self.tables.len());
        let end = (start + visible).min(self.tables.len());
        let y0 = area.y + 1;
        // Width adapts to the longest table name (+ side padding), clamped to
        // the panel width.
        let longest = self
            .tables
            .iter()
            .map(|n| UnicodeWidthStr::width(n.as_str()))
            .max()
            .unwrap_or(0);
        let width = ((longest + 2) as u16).clamp(10, area.width);
        for (row, i) in (start..end).enumerate() {
            let y = y0 + row as u16;
            if y >= area.y + area.height {
                break;
            }
            let style = if i == self.dropdown_cursor {
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.selection_bg)
            } else {
                base
            };
            fill_line(buf, area.x, y, width, style);
            buf.set_stringn(area.x + 1, y, &self.tables[i], width as usize - 1, style);
        }
    }

    fn center_message(&self, buf: &mut Buffer, area: Rect, msg: &str, style: Style) {
        if area.height == 0 {
            return;
        }
        let y = area.y + area.height / 2;
        let w = UnicodeWidthStr::width(msg).min(area.width as usize);
        let x = area.x + (area.width.saturating_sub(w as u16)) / 2;
        buf.set_stringn(x, y, msg, area.width as usize, style);
    }
}

/// Fill a single row with spaces in `style` (background paint).
fn fill_line(buf: &mut Buffer, x: u16, y: u16, width: u16, style: Style) {
    let blanks = " ".repeat(width as usize);
    buf.set_stringn(x, y, &blanks, width as usize, style);
}

/// Write a string clipped to `max_x`; returns the new x cursor.
fn put(buf: &mut Buffer, x: u16, y: u16, max_x: u16, s: &str, style: Style) -> u16 {
    if x >= max_x {
        return x;
    }
    let budget = (max_x - x) as usize;
    let (nx, _) = buf.set_stringn(x, y, s, budget, style);
    nx
}

/// Pad/truncate `s` to display width `w` (truncation adds an ellipsis).
fn pad(s: &str, w: usize) -> String {
    let sw = UnicodeWidthStr::width(s);
    if sw == w {
        s.to_string()
    } else if sw < w {
        format!("{s}{}", " ".repeat(w - sw))
    } else {
        // Truncate by chars to fit w-1, add ellipsis.
        let mut out = String::new();
        let mut used = 0usize;
        for ch in s.chars() {
            let cw = UnicodeWidthStr::width(ch.to_string().as_str());
            if used + cw > w.saturating_sub(1) {
                break;
            }
            out.push(ch);
            used += cw;
        }
        out.push('…');
        // Pad if the ellipsis left us short.
        let ow = UnicodeWidthStr::width(out.as_str());
        if ow < w {
            out.push_str(&" ".repeat(w - ow));
        }
        out
    }
}
