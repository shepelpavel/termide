//! Multi-column filter modal for the database viewer.
//!
//! One row per column — column name, a type-aware operator (cycled with
//! ←/→, where the first option means "no condition"), and a value typed in
//! place. Bottom buttons Apply / Clear / Cancel. The panel maps the chosen
//! operator labels back to typed conditions.

use anyhow::Result;
use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr;

use termide_theme::Theme;

use crate::base::render_modal_block;
use crate::{centered_rect_with_size, Modal, ModalResult};

/// A column the modal can filter on.
pub struct DbFilterColumn {
    pub name: String,
    /// Operator labels (the modal prepends a "no condition" sentinel).
    pub operators: Vec<String>,
    /// Pre-selected operator (index into `operators`), if a condition exists.
    pub op: Option<usize>,
    /// Pre-filled value.
    pub value: String,
}

/// One applied condition.
#[derive(Debug, Clone)]
pub struct DbFilterCondition {
    pub column: String,
    pub op: String,
    pub value: String,
}

/// Result of the filter modal: the full set of conditions (empty = clear all).
#[derive(Debug, Clone)]
pub struct DbFilterResult {
    pub conditions: Vec<DbFilterCondition>,
}

#[derive(Debug)]
struct Row {
    name: String,
    operators: Vec<String>,
    /// 0 = no condition; otherwise `operators[op_sel - 1]`.
    op_sel: usize,
    value: String,
}

impl Row {
    fn op_label(&self) -> &str {
        if self.op_sel == 0 {
            "—"
        } else {
            self.operators
                .get(self.op_sel - 1)
                .map(|s| s.as_str())
                .unwrap_or("—")
        }
    }
    /// Operators with a value field (null checks and "no condition" don't).
    fn needs_value(&self) -> bool {
        !matches!(self.op_label(), "—" | "is null" | "is not null")
    }
}

const BTN_APPLY: usize = 0;
const BTN_CLEAR: usize = 1;
const BTN_CANCEL: usize = 2;

/// Modal for editing per-column filter conditions.
#[derive(Debug)]
pub struct DbFilterModal {
    rows: Vec<Row>,
    /// Focused row index, or `rows.len()` for the button bar.
    focus: usize,
    button: usize,
    scroll: usize,
}

impl DbFilterModal {
    pub fn new(columns: Vec<DbFilterColumn>) -> Self {
        let rows = columns
            .into_iter()
            .map(|c| Row {
                name: c.name,
                op_sel: c.op.map_or(0, |i| i + 1),
                operators: c.operators,
                value: c.value,
            })
            .collect();
        Self {
            rows,
            focus: 0,
            button: BTN_APPLY,
            scroll: 0,
        }
    }

    fn buttons_focused(&self) -> bool {
        self.focus >= self.rows.len()
    }

    fn collect(&self) -> DbFilterResult {
        let conditions = self
            .rows
            .iter()
            .filter(|r| r.op_sel > 0)
            .map(|r| DbFilterCondition {
                column: r.name.clone(),
                op: r.op_label().to_string(),
                value: if r.needs_value() {
                    r.value.clone()
                } else {
                    String::new()
                },
            })
            .collect();
        DbFilterResult { conditions }
    }
}

impl Modal for DbFilterModal {
    type Result = DbFilterResult;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let tr = termide_i18n::t();
        let width = 64u16.min(area.width.saturating_sub(2));
        // title + rows region + buttons + padding, capped to the screen.
        let max_rows_h = area.height.saturating_sub(6).max(1);
        let rows_h = (self.rows.len() as u16).min(max_rows_h).max(1);
        let height = rows_h + 4;
        let modal_area = centered_rect_with_size(width, height, area);
        let inner = render_modal_block(modal_area, buf, tr.db_filter_title(), theme);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let base = Style::default().fg(theme.fg);
        let focused = Style::default().fg(theme.selected_fg).bg(theme.selected_bg);

        let visible = inner.height.saturating_sub(1) as usize; // last line = buttons
        let visible = visible.max(1);
        // Keep the focused row visible.
        let focus_row = self.focus.min(self.rows.len());
        if focus_row < self.rows.len() {
            if focus_row < self.scroll {
                self.scroll = focus_row;
            } else if focus_row >= self.scroll + visible {
                self.scroll = focus_row + 1 - visible;
            }
        }

        let name_w = self
            .rows
            .iter()
            .map(|r| UnicodeWidthStr::width(r.name.as_str()))
            .max()
            .unwrap_or(4)
            .clamp(4, 24);

        let start = self.scroll.min(self.rows.len());
        let end = (start + visible).min(self.rows.len());
        for (line, i) in (start..end).enumerate() {
            let r = &self.rows[i];
            let y = inner.y + line as u16;
            let style = if !self.buttons_focused() && i == self.focus {
                focused
            } else {
                base
            };
            let name = pad(&r.name, name_w);
            let value = if r.needs_value() { &r.value } else { "" };
            let text = format!("{name}  ‹ {} ›  {value}", r.op_label());
            // clear the row then draw
            let blanks = " ".repeat(inner.width as usize);
            buf.set_stringn(inner.x, y, &blanks, inner.width as usize, style);
            buf.set_stringn(inner.x, y, &text, inner.width as usize, style);
        }

        // --- buttons ---
        let by = inner.y + inner.height.saturating_sub(1);
        let labels = [
            tr.db_filter_apply(),
            tr.db_filter_clear(),
            tr.db_filter_cancel(),
        ];
        let mut bx = inner.x;
        for (idx, lbl) in labels.iter().enumerate() {
            let chip = format!("[ {lbl} ]");
            let st = if self.buttons_focused() && self.button == idx {
                focused
            } else {
                base.add_modifier(Modifier::BOLD)
            };
            let (nx, _) = buf.set_stringn(
                bx,
                by,
                &chip,
                inner.width.saturating_sub(bx - inner.x) as usize,
                st,
            );
            bx = nx + 2;
        }
    }

    fn handle_key(
        &mut self,
        chord: termide_core::KeyChord,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        let n = self.rows.len();
        match chord.raw.code {
            KeyCode::Esc => return Ok(Some(ModalResult::Cancelled)),
            KeyCode::Up => {
                self.focus = self.focus.saturating_sub(1);
            }
            KeyCode::Down => {
                self.focus = (self.focus + 1).min(n); // n = buttons row
            }
            KeyCode::Tab => {
                self.focus = if self.focus >= n { 0 } else { self.focus + 1 };
            }
            KeyCode::Left => {
                if self.buttons_focused() {
                    self.button = self.button.saturating_sub(1);
                } else if let Some(r) = self.rows.get_mut(self.focus) {
                    r.op_sel = r.op_sel.saturating_sub(1);
                }
            }
            KeyCode::Right => {
                if self.buttons_focused() {
                    self.button = (self.button + 1).min(BTN_CANCEL);
                } else if let Some(r) = self.rows.get_mut(self.focus) {
                    r.op_sel = (r.op_sel + 1).min(r.operators.len());
                }
            }
            KeyCode::Enter => {
                if self.buttons_focused() {
                    return Ok(Some(match self.button {
                        BTN_CANCEL => ModalResult::Cancelled,
                        BTN_CLEAR => ModalResult::Confirmed(DbFilterResult { conditions: vec![] }),
                        _ => ModalResult::Confirmed(self.collect()),
                    }));
                }
                return Ok(Some(ModalResult::Confirmed(self.collect())));
            }
            KeyCode::Char(c) => {
                if !self.buttons_focused() {
                    if let Some(r) = self.rows.get_mut(self.focus) {
                        if r.needs_value() {
                            r.value.push(c);
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                if !self.buttons_focused() {
                    if let Some(r) = self.rows.get_mut(self.focus) {
                        r.value.pop();
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_paste(&mut self, text: &str) -> bool {
        if !self.buttons_focused() {
            if let Some(r) = self.rows.get_mut(self.focus) {
                if r.needs_value() {
                    r.value.push_str(text);
                    return true;
                }
            }
        }
        false
    }
}

/// Pad/truncate to display width `w`.
fn pad(s: &str, w: usize) -> String {
    let sw = UnicodeWidthStr::width(s);
    if sw >= w {
        s.chars().take(w).collect()
    } else {
        format!("{s}{}", " ".repeat(w - sw))
    }
}
