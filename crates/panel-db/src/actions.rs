//! Keyboard handling and grid navigation for [`DbPanel`].

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use termide_core::{KeyChord, PanelEvent};
use termide_db::{Condition, DbValue, FilterOp, SortDir, TypeCategory};
use termide_modal::{ActionButton, ActiveModal, DbFilterModal, DbFilterResult, InfoActionModal};
use termide_state::PendingAction;

use crate::{DbPanel, Section, WINDOW};

impl DbPanel {
    pub(crate) fn handle_key_impl(&mut self, chord: KeyChord) -> Vec<PanelEvent> {
        let key = chord.raw;
        let code = key.code;

        // Open table dropdown captures navigation.
        if self.table_dropdown_open {
            match code {
                KeyCode::Up => {
                    self.dropdown_cursor = self.dropdown_cursor.saturating_sub(1);
                    return self.redraw();
                }
                KeyCode::Down => {
                    // Past the page's last row this lands on the next page's
                    // first row (cursor / page_size increments).
                    if self.dropdown_cursor + 1 < self.tables.len() {
                        self.dropdown_cursor += 1;
                    }
                    return self.redraw();
                }
                KeyCode::PageDown => {
                    let ps = self.dropdown_page_size.max(1);
                    let next = (self.dropdown_cursor / ps + 1) * ps;
                    self.dropdown_cursor = if next < self.tables.len() {
                        next // first row of the next page
                    } else {
                        self.tables.len().saturating_sub(1)
                    };
                    return self.redraw();
                }
                KeyCode::PageUp => {
                    let ps = self.dropdown_page_size.max(1);
                    let page = self.dropdown_cursor / ps;
                    self.dropdown_cursor = if page > 0 {
                        page * ps - 1 // last row of the previous page
                    } else {
                        0
                    };
                    return self.redraw();
                }
                KeyCode::Enter => {
                    if let Some(name) = self.tables.get(self.dropdown_cursor).cloned() {
                        self.table_dropdown_open = false;
                        if self.selected_table.as_deref() != Some(name.as_str()) {
                            self.selected_table = Some(name);
                            self.reload_table();
                        }
                    }
                    return self.redraw();
                }
                KeyCode::Esc => {
                    self.table_dropdown_open = false;
                    return self.redraw();
                }
                _ => return vec![],
            }
        }

        // Panel-wide action: refresh the catalog + current view.
        if self.hotkeys.matches("refresh", &key) {
            self.refresh_catalog();
            return self.redraw();
        }

        match code {
            KeyCode::Tab | KeyCode::BackTab => {
                self.section = match self.section {
                    Section::TableSelector => Section::Grid,
                    Section::Grid => Section::TableSelector,
                };
                return self.redraw();
            }
            _ => {}
        }

        match self.section {
            Section::TableSelector => self.handle_selector_key(code),
            Section::Grid => self.handle_grid_key(key),
        }
    }

    fn handle_selector_key(&mut self, code: KeyCode) -> Vec<PanelEvent> {
        match code {
            KeyCode::Enter | KeyCode::Char(' ') => {
                if !self.tables.is_empty() {
                    self.table_dropdown_open = true;
                    self.dropdown_cursor = self
                        .selected_table
                        .as_ref()
                        .and_then(|t| self.tables.iter().position(|n| n == t))
                        .unwrap_or(0);
                }
                self.redraw()
            }
            KeyCode::Down => {
                self.section = Section::Grid;
                self.redraw()
            }
            _ => vec![],
        }
    }

    fn handle_grid_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        if !self.is_connected() {
            return vec![];
        }

        // Configurable action hotkeys (see [database.keybindings]).
        if self.hotkeys.matches("sort", &key) {
            self.cycle_sort();
            return self.redraw();
        }
        if self.hotkeys.matches("filter", &key) {
            self.open_filter();
            return self.redraw();
        }
        if self.hotkeys.matches("clear_filter", &key) {
            return if self.clear_filters() {
                self.redraw()
            } else {
                vec![]
            };
        }
        if self.hotkeys.matches("detail", &key) {
            self.open_row_detail();
            return self.redraw();
        }
        if self.hotkeys.matches("copy_cell", &key) {
            return self.copy(false);
        }
        if self.hotkeys.matches("copy_row", &key) {
            return self.copy(true);
        }

        // Fixed navigation keys.
        let changed = match key.code {
            KeyCode::Up => self.grid_up(),
            KeyCode::Down => self.grid_down(),
            KeyCode::Left => self.grid_left(),
            KeyCode::Right => self.grid_right(),
            KeyCode::PageDown => self.grid_page(true),
            KeyCode::PageUp => self.grid_page(false),
            KeyCode::Home => self.grid_home(),
            KeyCode::End => self.grid_end(),
            _ => false,
        };
        if changed {
            self.redraw()
        } else {
            vec![]
        }
    }

    /// Mouse: click the table selector to open it; click a column header to
    /// cycle its sort; click a data cell to move the cursor there.
    pub(crate) fn handle_mouse_impl(&mut self, event: MouseEvent) -> Vec<PanelEvent> {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return vec![];
        }
        let (row, col) = (event.row, event.column);

        // Open dropdown: pick a table from the open list.
        if self.table_dropdown_open {
            let list_top = self.geom.selector_y + 1;
            if row >= list_top {
                let ps = self.dropdown_page_size.max(1);
                let page_start = (self.dropdown_cursor / ps) * ps;
                let idx = page_start + (row - list_top) as usize;
                if let Some(name) = self.tables.get(idx).cloned() {
                    self.table_dropdown_open = false;
                    if self.selected_table.as_deref() != Some(name.as_str()) {
                        self.selected_table = Some(name);
                        self.reload_table();
                    }
                    return self.redraw();
                }
            }
            self.table_dropdown_open = false;
            return self.redraw();
        }

        // Click on the selector row → open the table dropdown.
        if row == self.geom.selector_y {
            self.section = Section::TableSelector;
            if !self.tables.is_empty() {
                self.table_dropdown_open = true;
                self.dropdown_cursor = self
                    .selected_table
                    .as_ref()
                    .and_then(|t| self.tables.iter().position(|n| n == t))
                    .unwrap_or(0);
            }
            return self.redraw();
        }

        if !self.is_connected() {
            return vec![];
        }

        // Click on a column header → sort by that column.
        if Some(row) == self.geom.header_y {
            if let Some(col_idx) = self.column_at(col) {
                self.section = Section::Grid;
                self.cursor_col = col_idx;
                self.cycle_sort();
                return self.redraw();
            }
            return vec![];
        }

        // Click on a data cell → move the cursor there.
        if row >= self.geom.data_y0 {
            let vis = (row - self.geom.data_y0) as usize;
            let abs = self.row_scroll + vis;
            if abs < self.page.rows.len() {
                self.section = Section::Grid;
                self.cursor_row = abs;
                if let Some(col_idx) = self.column_at(col) {
                    self.cursor_col = col_idx;
                }
                return self.redraw();
            }
        }
        vec![]
    }

    /// Map a screen column to a grid column index using the captured layout.
    fn column_at(&self, x: u16) -> Option<usize> {
        self.geom
            .columns
            .iter()
            .find(|(_, start, end)| x >= *start && x < *end)
            .map(|(idx, _, _)| *idx)
    }

    // --- navigation primitives (scroll is recomputed in render) ---

    fn grid_down(&mut self) -> bool {
        let rows = self.page.rows.len();
        if rows == 0 {
            return false;
        }
        if self.cursor_row + 1 < rows {
            self.cursor_row += 1;
            true
        } else if self.page.has_more {
            self.offset += WINDOW;
            self.cursor_row = 0;
            self.reload_page();
            true
        } else {
            false
        }
    }

    fn grid_up(&mut self) -> bool {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            true
        } else if self.offset > 0 {
            self.offset = self.offset.saturating_sub(WINDOW);
            // Land on the last row of the previous (full) window after it loads.
            self.cursor_row = usize::MAX;
            self.reload_page();
            true
        } else {
            false
        }
    }

    fn grid_left(&mut self) -> bool {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
            true
        } else {
            false
        }
    }

    fn grid_right(&mut self) -> bool {
        let cols = self.col_count();
        if cols > 0 && self.cursor_col + 1 < cols {
            self.cursor_col += 1;
            true
        } else {
            false
        }
    }

    fn grid_page(&mut self, down: bool) -> bool {
        let rows = self.page.rows.len();
        if rows == 0 {
            return false;
        }
        let step = self.visible_rows.max(1);
        if down {
            if self.cursor_row + step < rows {
                self.cursor_row += step;
                true
            } else if self.page.has_more {
                self.offset += WINDOW;
                self.cursor_row = 0;
                self.reload_page();
                true
            } else if self.cursor_row != rows - 1 {
                self.cursor_row = rows - 1;
                true
            } else {
                false
            }
        } else if self.cursor_row >= step {
            self.cursor_row -= step;
            true
        } else if self.offset > 0 {
            self.offset = self.offset.saturating_sub(WINDOW);
            self.cursor_row = usize::MAX;
            self.reload_page();
            true
        } else if self.cursor_row != 0 {
            self.cursor_row = 0;
            true
        } else {
            false
        }
    }

    fn grid_home(&mut self) -> bool {
        self.cursor_col = 0;
        if self.offset > 0 {
            self.offset = 0;
            self.cursor_row = 0;
            self.reload_page();
        } else if self.cursor_row != 0 {
            self.cursor_row = 0;
        } else {
            return false;
        }
        true
    }

    fn grid_end(&mut self) -> bool {
        let Some(total) = self.total_rows else {
            return false;
        };
        if total <= 0 {
            return false;
        }
        let last_offset = ((total as u64 - 1) / WINDOW) * WINDOW;
        if last_offset != self.offset {
            self.offset = last_offset;
            self.cursor_row = usize::MAX;
            self.reload_page();
        } else {
            self.cursor_row = self.page.rows.len().saturating_sub(1);
        }
        true
    }

    fn cycle_sort(&mut self) {
        let names = self.column_names();
        let Some(col) = names.get(self.cursor_col).cloned() else {
            return;
        };
        let current = self
            .order_by
            .first()
            .and_then(|(c, d)| if *c == col { Some(*d) } else { None });
        self.order_by = match current {
            None => vec![(col, SortDir::Asc)],
            Some(SortDir::Asc) => vec![(col, SortDir::Desc)],
            Some(SortDir::Desc) => Vec::new(),
        };
        self.offset = 0;
        self.cursor_row = 0;
        self.reload_page();
    }

    fn copy(&self, whole_row: bool) -> Vec<PanelEvent> {
        let Some(row) = self.page.rows.get(self.cursor_row) else {
            return vec![];
        };
        let text = if whole_row {
            row.iter()
                .map(|v| tsv_escape(&v.display()))
                .collect::<Vec<_>>()
                .join("\t")
        } else {
            match row.get(self.cursor_col) {
                Some(v) => v.display(),
                None => return vec![],
            }
        };
        let t = termide_i18n::t();
        let message = if whole_row {
            t.db_copied_row()
        } else {
            t.db_copied_cell()
        }
        .to_string();
        vec![
            PanelEvent::CopyToClipboard(text),
            PanelEvent::SetStatusMessage {
                message,
                is_error: false,
            },
        ]
    }

    /// Build the row-detail modal for the current row: a key→value list plus
    /// copy-format buttons. The three copy formats are precomputed and carried
    /// in the `PendingAction` so the app can copy without calling back here.
    fn open_row_detail(&mut self) {
        let names = self.column_names();
        let Some(row) = self.page.rows.get(self.cursor_row) else {
            return;
        };
        let table = self.selected_table.clone().unwrap_or_default();

        let lines: Vec<(String, String)> = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let v = row.get(i);
                let text = match v {
                    Some(v) if v.is_null() => "NULL".to_string(),
                    Some(v) => v.display(),
                    None => String::new(),
                };
                (name.clone(), text)
            })
            .collect();

        let tsv = row
            .iter()
            .map(|v| tsv_escape(&v.display()))
            .collect::<Vec<_>>()
            .join("\t");
        let json = row_to_json(&names, row);
        let insert = row_to_insert(&table, &names, row);

        let t = termide_i18n::t();
        let buttons = vec![
            ActionButton::new(t.db_copy_tsv(), "copy_tsv"),
            ActionButton::new(t.db_copy_json(), "copy_json"),
            ActionButton::new(t.db_copy_insert(), "copy_insert"),
            ActionButton::new(t.git_action_close(), "close"),
        ];
        let title = t.db_row_title_fmt(&table);
        let modal = InfoActionModal::new(title, lines, buttons);
        self.modal_request = Some((
            PendingAction::DbRowDetail { tsv, json, insert },
            ActiveModal::InfoAction(Box::new(modal)),
        ));
    }

    /// Open the single-column filter modal for the current column.
    fn open_filter(&mut self) {
        let names = self.column_names();
        let Some(column) = names.get(self.cursor_col).cloned() else {
            return;
        };
        let category = self.category_of(&column);
        let operators: Vec<String> = operators_for(category)
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Prefill from an existing condition on this column.
        let existing = self.filters.iter().find(|c| c.column == column);
        let initial_op = existing.map(|c| label_for(c.op).to_string());
        let initial_value = existing
            .and_then(|c| c.value.as_ref())
            .map(|v| v.display())
            .unwrap_or_default();
        let modal = DbFilterModal::new(column, operators, initial_op, initial_value);
        self.modal_request = Some((
            PendingAction::DbFilter,
            ActiveModal::DbFilter(Box::new(modal)),
        ));
    }

    /// Clear all filters. Returns true if anything changed.
    fn clear_filters(&mut self) -> bool {
        if self.filters.is_empty() {
            return false;
        }
        self.filters.clear();
        self.offset = 0;
        self.cursor_row = 0;
        self.reload_all();
        true
    }

    /// Apply a result from the filter modal (called by the app on the active
    /// panel). Replaces any existing condition on the same column.
    pub fn apply_filter_result(&mut self, r: DbFilterResult) {
        let Some(op) = op_from_label(&r.op) else {
            return;
        };
        let value = if matches!(op, FilterOp::IsNull | FilterOp::IsNotNull) {
            None
        } else {
            Some(parse_value(self.category_of(&r.column), &r.value))
        };
        self.filters.retain(|c| c.column != r.column);
        self.filters.push(Condition {
            column: r.column,
            op,
            value,
        });
        self.offset = 0;
        self.cursor_row = 0;
        self.reload_all();
    }

    /// Type category of a column by name (defaults to Text when unknown).
    fn category_of(&self, name: &str) -> TypeCategory {
        self.columns
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.category)
            .unwrap_or(TypeCategory::Text)
    }

    /// Standard "something changed" response: redraw + refresh the status bar.
    fn redraw(&self) -> Vec<PanelEvent> {
        vec![PanelEvent::NeedsRedraw, self.status_event()]
    }
}

/// Operator labels offered for a column category (type-aware).
fn operators_for(cat: TypeCategory) -> &'static [&'static str] {
    match cat {
        TypeCategory::Number | TypeCategory::Date => {
            &["=", "≠", ">", "≥", "<", "≤", "is null", "is not null"]
        }
        TypeCategory::Text | TypeCategory::Other => &[
            "contains",
            "starts with",
            "ends with",
            "=",
            "≠",
            "is null",
            "is not null",
        ],
        TypeCategory::Bool => &["=", "≠", "is null", "is not null"],
        TypeCategory::Bytes => &["is null", "is not null"],
    }
}

fn op_from_label(label: &str) -> Option<FilterOp> {
    Some(match label {
        "contains" => FilterOp::Contains,
        "starts with" => FilterOp::StartsWith,
        "ends with" => FilterOp::EndsWith,
        "=" => FilterOp::Eq,
        "≠" => FilterOp::Ne,
        ">" => FilterOp::Gt,
        "≥" => FilterOp::Ge,
        "<" => FilterOp::Lt,
        "≤" => FilterOp::Le,
        "is null" => FilterOp::IsNull,
        "is not null" => FilterOp::IsNotNull,
        _ => return None,
    })
}

fn label_for(op: FilterOp) -> &'static str {
    match op {
        FilterOp::Contains => "contains",
        FilterOp::StartsWith => "starts with",
        FilterOp::EndsWith => "ends with",
        FilterOp::Eq => "=",
        FilterOp::Ne => "≠",
        FilterOp::Gt => ">",
        FilterOp::Ge => "≥",
        FilterOp::Lt => "<",
        FilterOp::Le => "≤",
        FilterOp::IsNull => "is null",
        FilterOp::IsNotNull => "is not null",
    }
}

/// Coerce the user's text into a typed [`DbValue`] for binding, by category.
fn parse_value(cat: TypeCategory, text: &str) -> DbValue {
    match cat {
        TypeCategory::Number => {
            if let Ok(i) = text.parse::<i64>() {
                DbValue::Int(i)
            } else if let Ok(f) = text.parse::<f64>() {
                DbValue::Float(f)
            } else {
                DbValue::Text(text.to_string())
            }
        }
        TypeCategory::Bool => match text.to_ascii_lowercase().as_str() {
            "true" | "1" | "t" | "yes" | "y" => DbValue::Bool(true),
            "false" | "0" | "f" | "no" | "n" => DbValue::Bool(false),
            _ => DbValue::Text(text.to_string()),
        },
        _ => DbValue::Text(text.to_string()),
    }
}

/// JSON-encode a row as `{"col": value, …}`.
fn row_to_json(names: &[String], row: &[DbValue]) -> String {
    let mut out = String::from("{");
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&json_string(name));
        out.push_str(": ");
        out.push_str(&json_value(row.get(i)));
    }
    out.push('}');
    out
}

fn json_value(v: Option<&DbValue>) -> String {
    match v {
        None | Some(DbValue::Null) => "null".to_string(),
        Some(DbValue::Bool(b)) => b.to_string(),
        Some(DbValue::Int(i)) => i.to_string(),
        Some(DbValue::Float(f)) => f.to_string(),
        Some(DbValue::Text(s)) => json_string(s),
        Some(DbValue::Bytes(_)) => json_string(&v.unwrap().display()),
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Build an `INSERT INTO "table" (...) VALUES (...);` statement. Identifiers
/// are double-quoted (portable for SQLite/Postgres); adapt for MySQL backticks.
fn row_to_insert(table: &str, names: &[String], row: &[DbValue]) -> String {
    let cols = names
        .iter()
        .map(|n| format!("\"{}\"", n.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(", ");
    let vals = (0..names.len())
        .map(|i| sql_literal(row.get(i)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO \"{}\" ({}) VALUES ({});",
        table.replace('"', "\"\""),
        cols,
        vals
    )
}

fn sql_literal(v: Option<&DbValue>) -> String {
    match v {
        None | Some(DbValue::Null) => "NULL".to_string(),
        Some(DbValue::Bool(b)) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Some(DbValue::Int(i)) => i.to_string(),
        Some(DbValue::Float(f)) => f.to_string(),
        Some(DbValue::Text(s)) => format!("'{}'", s.replace('\'', "''")),
        Some(DbValue::Bytes(_)) => format!("'{}'", v.unwrap().display().replace('\'', "''")),
    }
}

/// Flatten tabs/newlines so a TSV row stays one line per record.
fn tsv_escape(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}
