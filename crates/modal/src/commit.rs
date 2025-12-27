//! Git commit modal dialog with multi-line text area.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use termide_clipboard as clipboard;
use termide_i18n as i18n;
use termide_theme::Theme;
use termide_ui::TextArea;

use crate::{centered_rect_with_size, Modal, ModalResult};

/// Focus area in the modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusArea {
    Textarea,
    Buttons,
}

/// Git commit modal window.
#[derive(Debug)]
pub struct CommitModal {
    /// Number of staged files (for title).
    staged_count: usize,
    /// Repository name (for title).
    repo_name: String,
    /// Branch name (for title).
    branch_name: String,
    /// Multi-line text area for commit message.
    textarea: TextArea,
    /// Current focus area.
    focus: FocusArea,
    /// Selected button index (0 = Commit, 1 = Cancel).
    selected_button: usize,
    /// Last rendered textarea area (for mouse).
    last_textarea_area: Option<Rect>,
    /// Last rendered buttons area (for mouse).
    last_buttons_area: Option<Rect>,
    /// Visible textarea height (for scrolling).
    visible_height: usize,
}

impl CommitModal {
    /// Create a new commit modal.
    pub fn new(staged_count: usize, repo_name: String, branch_name: String) -> Self {
        Self {
            staged_count,
            repo_name,
            branch_name,
            textarea: TextArea::new(),
            focus: FocusArea::Textarea,
            selected_button: 0,
            last_textarea_area: None,
            last_buttons_area: None,
            visible_height: 3,
        }
    }

    /// Calculate modal size based on terminal size.
    fn calculate_modal_size(&self, screen_width: u16, screen_height: u16) -> (u16, u16) {
        // Width: 60% of screen, min 40, max 80
        let width = ((screen_width as f32 * 0.6) as u16)
            .clamp(40, 80)
            .min(screen_width);

        // Height: adaptive based on screen
        // Layout: border(1) + textarea(adaptive) + border(1) + buttons(1) + border(1)
        let min_textarea_height = 3u16;
        let overhead = 4u16; // borders + buttons row
        let available = screen_height.saturating_sub(overhead);
        let textarea_height = available.max(min_textarea_height).min(15);
        let height = textarea_height + overhead;

        (width, height.min(screen_height))
    }

    /// Render the textarea content.
    fn render_textarea(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let cursor = self.textarea.cursor();
        let scroll_offset = self.textarea.scroll_offset();
        let is_focused = self.focus == FocusArea::Textarea;

        // Get selection range for highlighting
        let selection_range = self.textarea.selection_range();

        for (i, line_idx) in (scroll_offset..(scroll_offset + area.height as usize)).enumerate() {
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }

            let line = self
                .textarea
                .lines()
                .get(line_idx)
                .map(|s| s.as_str())
                .unwrap_or("");

            // Determine if this line has selection
            let (sel_start, sel_end) = if let Some((start, end)) = selection_range {
                if line_idx >= start.row && line_idx <= end.row {
                    let line_start = if line_idx == start.row { start.col } else { 0 };
                    let line_end = if line_idx == end.row {
                        end.col
                    } else {
                        line.chars().count()
                    };
                    (Some(line_start), Some(line_end))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

            // Render line with selection highlighting and cursor
            let mut x = area.x;
            let line_len = line.chars().count();
            let is_cursor_line = is_focused && line_idx == cursor.row;

            for (char_idx, ch) in line.chars().enumerate() {
                if x >= area.x + area.width {
                    break;
                }

                let is_cursor_here = is_cursor_line && char_idx == cursor.col;
                let is_selected = sel_start.is_some()
                    && sel_end.is_some()
                    && char_idx >= sel_start.unwrap()
                    && char_idx < sel_end.unwrap();

                let ch_width = ch.width().unwrap_or(1) as u16;

                if is_cursor_here {
                    // Inverted cursor - show character with swapped colors
                    buf.set_string(
                        x,
                        y,
                        ch.to_string(),
                        Style::default().fg(theme.fg).bg(theme.bg),
                    );
                } else if is_selected {
                    buf.set_string(
                        x,
                        y,
                        ch.to_string(),
                        Style::default().fg(theme.selected_fg).bg(theme.selected_bg),
                    );
                } else {
                    buf.set_string(x, y, ch.to_string(), Style::default().fg(theme.bg));
                }
                x += ch_width;
            }

            // Show cursor at end of line or on empty line (inverted space)
            if is_cursor_line && cursor.col >= line_len && x < area.x + area.width {
                buf.set_string(x, y, " ", Style::default().bg(theme.bg));
                x += 1;
            }

            // Fill rest of line with background
            while x < area.x + area.width {
                buf.set_string(x, y, " ", Style::default().bg(theme.fg));
                x += 1;
            }
        }
    }
}

impl Modal for CommitModal {
    type Result = String;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let (modal_width, modal_height) = self.calculate_modal_size(area.width, area.height);
        let modal_area = centered_rect_with_size(modal_width, modal_height, area);

        // Clear the area
        Clear.render(modal_area, buf);

        // Title with file count, repo and branch
        let t = i18n::t();
        let title = format!(
            " {} ",
            t.git_commit_title(self.staged_count, &self.repo_name, &self.branch_name)
        );

        // Create block with inverted colors (like other modals)
        let block = Block::default()
            .title(Span::styled(
                title,
                Style::default().fg(theme.bg).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.bg))
            .style(Style::default().bg(theme.fg));

        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        // Split inner area: textarea + buttons
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),    // Textarea (takes remaining space)
                Constraint::Length(1), // Buttons
            ])
            .split(inner);

        // Textarea area with border
        let textarea_block = Block::default().borders(Borders::ALL).border_style(
            if self.focus == FocusArea::Textarea {
                Style::default().fg(theme.success)
            } else {
                Style::default().fg(theme.disabled)
            },
        );
        let textarea_inner = textarea_block.inner(chunks[0]);
        textarea_block.render(chunks[0], buf);

        // Update visible height and ensure cursor is visible
        self.visible_height = textarea_inner.height as usize;
        self.textarea.ensure_cursor_visible(self.visible_height);

        // Render textarea content
        self.render_textarea(textarea_inner, buf, theme);
        self.last_textarea_area = Some(textarea_inner);

        // Render buttons
        let commit_label = format!("[ {} ]", t.git_action_commit());
        let cancel_label = format!("[ {} ]", t.ui_cancel());

        let commit_style = if self.focus == FocusArea::Buttons && self.selected_button == 0 {
            Style::default()
                .fg(theme.fg)
                .bg(theme.accented_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.accented_fg)
        };

        let cancel_style = if self.focus == FocusArea::Buttons && self.selected_button == 1 {
            Style::default()
                .fg(theme.fg)
                .bg(theme.accented_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.accented_fg)
        };

        let buttons = Line::from(vec![
            Span::styled(commit_label, commit_style),
            Span::raw("    "),
            Span::styled(cancel_label, cancel_style),
        ]);

        let buttons_paragraph = Paragraph::new(buttons).alignment(Alignment::Center);
        buttons_paragraph.render(chunks[1], buf);
        self.last_buttons_area = Some(chunks[1]);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>> {
        // Escape always cancels
        if key.code == KeyCode::Esc {
            return Ok(Some(ModalResult::Cancelled));
        }

        // Ctrl+Enter commits from anywhere
        if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL) {
            let text = self.textarea.text();
            if text.trim().is_empty() {
                return Ok(Some(ModalResult::Cancelled));
            }
            return Ok(Some(ModalResult::Confirmed(text)));
        }

        match self.focus {
            FocusArea::Textarea => {
                match key.code {
                    KeyCode::Tab => {
                        self.focus = FocusArea::Buttons;
                    }
                    KeyCode::BackTab => {
                        self.focus = FocusArea::Buttons;
                    }
                    KeyCode::Enter => {
                        self.textarea.insert_newline();
                    }
                    KeyCode::Char(c) => {
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            match c {
                                'a' => self.textarea.select_all(),
                                'z' => {
                                    self.textarea.undo();
                                }
                                'y' => {
                                    self.textarea.redo();
                                }
                                'c' => {
                                    // Copy
                                    if let Some(text) = self.textarea.selected_text() {
                                        let _ = clipboard::copy(&text);
                                    }
                                }
                                'x' => {
                                    // Cut
                                    if let Some(text) = self.textarea.selected_text() {
                                        let _ = clipboard::copy(&text);
                                        self.textarea.delete_selection();
                                    }
                                }
                                'v' => {
                                    // Paste
                                    if let Some(text) = clipboard::paste() {
                                        self.textarea.insert_str(&text);
                                    }
                                }
                                _ => {}
                            }
                        } else {
                            self.textarea.insert(c);
                        }
                    }
                    KeyCode::Backspace => {
                        self.textarea.backspace();
                    }
                    KeyCode::Delete => {
                        self.textarea.delete();
                    }
                    KeyCode::Left => {
                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            self.textarea.move_left_with_selection();
                        } else {
                            self.textarea.move_left();
                        }
                    }
                    KeyCode::Right => {
                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            self.textarea.move_right_with_selection();
                        } else {
                            self.textarea.move_right();
                        }
                    }
                    KeyCode::Up => {
                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            self.textarea.move_up_with_selection();
                        } else {
                            self.textarea.move_up();
                        }
                    }
                    KeyCode::Down => {
                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            self.textarea.move_down_with_selection();
                        } else {
                            self.textarea.move_down();
                        }
                    }
                    KeyCode::Home => {
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            self.textarea.move_to_start();
                        } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                            self.textarea.move_home_with_selection();
                        } else {
                            self.textarea.move_home();
                        }
                    }
                    KeyCode::End => {
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            self.textarea.move_to_end();
                        } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                            self.textarea.move_end_with_selection();
                        } else {
                            self.textarea.move_end();
                        }
                    }
                    _ => {}
                }
            }
            FocusArea::Buttons => {
                match key.code {
                    KeyCode::Tab | KeyCode::Up => {
                        self.focus = FocusArea::Textarea;
                    }
                    KeyCode::BackTab => {
                        self.focus = FocusArea::Textarea;
                    }
                    KeyCode::Left => {
                        self.selected_button = if self.selected_button == 0 { 1 } else { 0 };
                    }
                    KeyCode::Right => {
                        self.selected_button = if self.selected_button == 1 { 0 } else { 1 };
                    }
                    KeyCode::Enter => {
                        if self.selected_button == 0 {
                            // Commit button
                            let text = self.textarea.text();
                            if text.trim().is_empty() {
                                return Ok(Some(ModalResult::Cancelled));
                            }
                            return Ok(Some(ModalResult::Confirmed(text)));
                        } else {
                            // Cancel button
                            return Ok(Some(ModalResult::Cancelled));
                        }
                    }
                    KeyCode::Char(c) => {
                        // Switch back to textarea and insert
                        self.focus = FocusArea::Textarea;
                        self.textarea.insert(c);
                    }
                    _ => {}
                }
            }
        }

        Ok(None)
    }

    fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        _modal_area: Rect,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return Ok(None);
        }

        // Check textarea click
        if let Some(textarea_area) = self.last_textarea_area {
            if mouse.row >= textarea_area.y
                && mouse.row < textarea_area.y + textarea_area.height
                && mouse.column >= textarea_area.x
                && mouse.column < textarea_area.x + textarea_area.width
            {
                self.focus = FocusArea::Textarea;
                let row = (mouse.row - textarea_area.y) as usize + self.textarea.scroll_offset();
                let col = (mouse.column - textarea_area.x) as usize;
                self.textarea.set_cursor(row, col);
                return Ok(None);
            }
        }

        // Check buttons click
        if let Some(buttons_area) = self.last_buttons_area {
            if mouse.row >= buttons_area.y
                && mouse.row < buttons_area.y + buttons_area.height
                && mouse.column >= buttons_area.x
                && mouse.column < buttons_area.x + buttons_area.width
            {
                let t = i18n::t();
                let commit_label = format!("[ {} ]", t.git_action_commit());
                let cancel_label = format!("[ {} ]", t.ui_cancel());
                let total_width = commit_label.width() + 4 + cancel_label.width();

                let start_col =
                    buttons_area.x + (buttons_area.width.saturating_sub(total_width as u16)) / 2;
                let commit_end = start_col + commit_label.width() as u16;
                let cancel_start = commit_end + 4;
                let cancel_end = cancel_start + cancel_label.width() as u16;

                if mouse.column >= start_col && mouse.column < commit_end {
                    // Commit clicked
                    let text = self.textarea.text();
                    if text.trim().is_empty() {
                        return Ok(Some(ModalResult::Cancelled));
                    }
                    return Ok(Some(ModalResult::Confirmed(text)));
                } else if mouse.column >= cancel_start && mouse.column < cancel_end {
                    // Cancel clicked
                    return Ok(Some(ModalResult::Cancelled));
                }
            }
        }

        Ok(None)
    }
}
