//! Single-condition filter modal for the database viewer.
//!
//! Scoped to one column at a time (the grid's current column): pick a
//! type-aware operator and, unless it's a null check, type a value. The panel
//! accumulates conditions with `AND`; a multi-row single-modal editor is a
//! later enhancement (see `ROADMAP.md.tmp`).

use anyhow::Result;
use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use termide_theme::Theme;

use crate::base::render_modal_block;
use crate::{centered_rect_with_size, Modal, ModalResult};

/// What the filter modal returns on Apply.
#[derive(Debug, Clone)]
pub struct DbFilterResult {
    pub column: String,
    /// Operator label (one of the strings passed in `operators`).
    pub op: String,
    /// Raw value text (ignored by the panel for null operators).
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Operator,
    Value,
}

/// Modal for editing one filter condition on a single column.
#[derive(Debug)]
pub struct DbFilterModal {
    column: String,
    operators: Vec<String>,
    op_index: usize,
    value: String,
    focus: Focus,
}

impl DbFilterModal {
    /// Create a filter modal for `column`, offering `operators` (type-aware
    /// labels). `initial_op`/`initial_value` prefill an existing condition.
    pub fn new(
        column: impl Into<String>,
        operators: Vec<String>,
        initial_op: Option<String>,
        initial_value: String,
    ) -> Self {
        let column = column.into();
        let op_index = initial_op
            .and_then(|op| operators.iter().position(|o| *o == op))
            .unwrap_or(0);
        Self {
            column,
            operators,
            op_index,
            value: initial_value,
            focus: Focus::Operator,
        }
    }

    fn current_op(&self) -> &str {
        self.operators
            .get(self.op_index)
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// Whether the current operator needs a value (null checks don't).
    fn needs_value(&self) -> bool {
        !matches!(self.current_op(), "is null" | "is not null")
    }

    fn result(&self) -> DbFilterResult {
        DbFilterResult {
            column: self.column.clone(),
            op: self.current_op().to_string(),
            value: self.value.clone(),
        }
    }
}

impl Modal for DbFilterModal {
    type Result = DbFilterResult;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let tr = termide_i18n::t();
        let modal_area = centered_rect_with_size(50, 8, area);
        let title = tr.db_filter_title_fmt(&self.column);
        let inner = render_modal_block(modal_area, buf, &title, theme);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let base = Style::default().fg(theme.fg);
        let focused = Style::default().fg(theme.selected_fg).bg(theme.selected_bg);

        // Operator line: ‹ contains ›
        let op_style = if self.focus == Focus::Operator {
            focused
        } else {
            base
        };
        let op_line = format!("{}:  ‹ {} ›", tr.db_filter_operator(), self.current_op());
        buf.set_stringn(inner.x, inner.y, &op_line, inner.width as usize, op_style);

        // Value line (unless the operator is a null check).
        if self.needs_value() {
            let val_style = if self.focus == Focus::Value {
                focused
            } else {
                base
            };
            let val_line = format!("{}:     {}_", tr.db_filter_value(), self.value);
            buf.set_stringn(
                inner.x,
                inner.y + 2,
                &val_line,
                inner.width as usize,
                val_style,
            );
        }

        // Hint line.
        buf.set_stringn(
            inner.x,
            inner.y + inner.height.saturating_sub(1),
            tr.db_filter_hint(),
            inner.width as usize,
            base.add_modifier(Modifier::DIM),
        );
    }

    fn handle_key(
        &mut self,
        chord: termide_core::KeyChord,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        match chord.raw.code {
            KeyCode::Esc => return Ok(Some(ModalResult::Cancelled)),
            KeyCode::Enter => return Ok(Some(ModalResult::Confirmed(self.result()))),
            KeyCode::Tab | KeyCode::Up | KeyCode::Down => {
                self.focus = match self.focus {
                    Focus::Operator => Focus::Value,
                    Focus::Value => Focus::Operator,
                };
            }
            KeyCode::Left if self.focus == Focus::Operator => {
                if self.op_index == 0 {
                    self.op_index = self.operators.len().saturating_sub(1);
                } else {
                    self.op_index -= 1;
                }
            }
            KeyCode::Right if self.focus == Focus::Operator => {
                if !self.operators.is_empty() {
                    self.op_index = (self.op_index + 1) % self.operators.len();
                }
            }
            KeyCode::Char(c) if self.focus == Focus::Value => self.value.push(c),
            KeyCode::Backspace if self.focus == Focus::Value => {
                self.value.pop();
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_paste(&mut self, text: &str) -> bool {
        if self.focus == Focus::Value {
            self.value.push_str(text);
            true
        } else {
            false
        }
    }
}
