//! Inline syntax-highlighting language picker.
//!
//! A small dropdown (same widget as the git-log repo/branch selectors) listing
//! the available highlight languages so a mis-detected file can be re-highlighted.
//! Self-contained: the editor owns one, routes keys/mouse to it, and applies the
//! chosen language. Supports keyboard, mouse click, and scroll.

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Widget},
};

use termide_core::ThemeColors;
use termide_ui::ScrollBar;

/// Sentinel first entry that clears the override (re-detect by extension).
pub const AUTO_DETECT: &str = "Auto-detect";

/// Max visible rows before the list scrolls.
const MAX_VISIBLE: u16 = 12;

/// Outcome of routing input to the picker.
pub enum PickerAction {
    /// Still open, just redraw.
    None,
    /// Closed without choosing.
    Cancel,
    /// A language was chosen (`AUTO_DETECT` clears the override).
    Select(String),
}

/// Dropdown state for the language picker.
pub struct SyntaxPicker {
    items: Vec<String>,
    cursor: usize,
    /// Last rendered dropdown rect (incl. border), for click hit-testing.
    area: Option<Rect>,
}

impl SyntaxPicker {
    /// Build a picker over `Auto-detect` + the given language names, with the
    /// cursor on `current` if present.
    pub fn new(languages: &[&str], custom: &[String], current: Option<&str>) -> Self {
        let mut items = Vec::with_capacity(languages.len() + custom.len() + 1);
        items.push(AUTO_DETECT.to_string());
        items.extend(languages.iter().map(|s| s.to_string()));
        items.extend(custom.iter().cloned());
        let cursor = current
            .and_then(|c| items.iter().position(|i| i == c))
            .unwrap_or(0);
        Self {
            items,
            cursor,
            area: None,
        }
    }

    fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn down(&mut self) {
        if self.cursor + 1 < self.items.len() {
            self.cursor += 1;
        }
    }

    /// Move the cursor by `delta` rows (negative = up), clamped. Used for the
    /// mouse wheel, which the host forwards as a coalesced delta.
    pub fn scroll(&mut self, delta: i32) {
        let n = delta.unsigned_abs() as usize;
        if delta < 0 {
            self.cursor = self.cursor.saturating_sub(n);
        } else {
            self.cursor = (self.cursor + n).min(self.items.len().saturating_sub(1));
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PickerAction {
        match key.code {
            KeyCode::Up => {
                self.up();
                PickerAction::None
            }
            KeyCode::Down => {
                self.down();
                PickerAction::None
            }
            KeyCode::PageUp => {
                self.scroll(-(MAX_VISIBLE as i32));
                PickerAction::None
            }
            KeyCode::PageDown => {
                self.scroll(MAX_VISIBLE as i32);
                PickerAction::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                PickerAction::None
            }
            KeyCode::End => {
                self.cursor = self.items.len().saturating_sub(1);
                PickerAction::None
            }
            KeyCode::Enter => PickerAction::Select(self.items[self.cursor].clone()),
            KeyCode::Esc => PickerAction::Cancel,
            _ => PickerAction::None,
        }
    }

    pub fn handle_mouse(&mut self, ev: MouseEvent) -> PickerAction {
        match ev.kind {
            MouseEventKind::ScrollUp => {
                self.up();
                PickerAction::None
            }
            MouseEventKind::ScrollDown => {
                self.down();
                PickerAction::None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(area) = self.area else {
                    return PickerAction::None;
                };
                let inside = ev.column >= area.x
                    && ev.column < area.x + area.width
                    && ev.row > area.y
                    && ev.row < area.y + area.height - 1;
                if !inside {
                    return PickerAction::Cancel; // click outside closes
                }
                let visible = (area.height as usize).saturating_sub(2);
                let scroll = self.cursor.saturating_sub(visible.saturating_sub(1));
                let idx = scroll + (ev.row - area.y - 1) as usize;
                if idx < self.items.len() {
                    PickerAction::Select(self.items[idx].clone())
                } else {
                    PickerAction::None
                }
            }
            _ => PickerAction::None,
        }
    }

    /// Render the dropdown anchored to the bottom-left of `area`.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &ThemeColors) {
        let visible = (self.items.len() as u16).clamp(1, MAX_VISIBLE);
        let box_h = (visible + 2).min(area.height.max(3));
        let visible_rows = box_h.saturating_sub(2) as usize;
        let width = area.width.clamp(4, 28);
        let y = area.y + area.height.saturating_sub(box_h);
        let rect = Rect {
            x: area.x,
            y,
            width,
            height: box_h,
        };
        self.area = Some(rect);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_focused))
            .style(Style::default().bg(theme.bg));
        let inner = block.inner(rect);
        block.render(rect, buf);

        // Scroll so the cursor stays visible.
        let scroll = self.cursor.saturating_sub(visible_rows.saturating_sub(1));
        let iw = inner.width as usize;
        for (row, idx) in (scroll..self.items.len()).take(visible_rows).enumerate() {
            let selected = idx == self.cursor;
            let style = if selected {
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.selection_bg)
            } else {
                Style::default().fg(theme.fg).bg(theme.bg)
            };
            let mut label: String = self.items[idx].chars().take(iw).collect();
            // Pad so the selection highlight spans the full row width.
            while label.chars().count() < iw {
                label.push(' ');
            }
            buf.set_stringn(inner.x, inner.y + row as u16, &label, iw, style);
        }

        // Scrollbar on the right border when the list overflows.
        ScrollBar::render(
            buf,
            rect.x + rect.width - 1,
            inner.y,
            visible_rows as u16,
            scroll,
            visible_rows,
            self.items.len(),
            theme,
            true,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn cursor_starts_on_current_language() {
        // items: [Auto-detect, rust, python, Alatyr]
        let p = SyntaxPicker::new(&["rust", "python"], &["Alatyr".to_string()], Some("python"));
        assert_eq!(p.cursor, 2);
    }

    #[test]
    fn enter_selects_highlighted() {
        let mut p = SyntaxPicker::new(&["rust"], &[], None); // [Auto-detect, rust]
        assert!(matches!(
            p.handle_key(key(KeyCode::Down)),
            PickerAction::None
        ));
        match p.handle_key(key(KeyCode::Enter)) {
            PickerAction::Select(s) => assert_eq!(s, "rust"),
            _ => panic!("expected Select"),
        }
    }

    #[test]
    fn esc_cancels_and_first_item_is_auto_detect() {
        let mut p = SyntaxPicker::new(&["rust"], &[], None);
        assert_eq!(p.items[0], AUTO_DETECT);
        assert!(matches!(
            p.handle_key(key(KeyCode::Esc)),
            PickerAction::Cancel
        ));
    }
}
