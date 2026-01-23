//! Modal dialog for adding bookmarks.
//!
//! Provides a modal with path, description, and group fields.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Widget},
};

use crate::base::{button_style, render_modal_block};

use termide_config::constants::{
    MODAL_BUTTON_SPACING, MODAL_MAX_WIDTH_PERCENTAGE_DEFAULT, MODAL_MIN_WIDTH_WIDE,
    MODAL_PADDING_WITH_DOUBLE_BORDER,
};
use termide_i18n as i18n;
use termide_theme::Theme;

use crate::{centered_rect_with_size, Modal, ModalResult, TextInputHandler};

/// Result of bookmark add operation
#[derive(Debug, Clone)]
pub struct BookmarkAddResult {
    /// Path to bookmark
    pub path: String,
    /// Optional description
    pub description: Option<String>,
    /// Optional group
    pub group: Option<String>,
}

/// Focus area in the modal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusArea {
    Path,
    Description,
    Group,
    Buttons,
}

/// Bookmark add modal
#[derive(Debug)]
pub struct BookmarkAddModal {
    path_input: TextInputHandler,
    description_input: TextInputHandler,
    group_input: TextInputHandler,
    existing_groups: Vec<String>,
    show_group_dropdown: bool,
    selected_group_index: usize,
    saved_group_input: String, // For rollback on Escape
    focus: FocusArea,
    selected_button: usize, // 0 = Add, 1 = Cancel
    last_buttons_area: Option<Rect>,
}

impl BookmarkAddModal {
    /// Create a new bookmark add modal
    pub fn new(initial_path: Option<String>, existing_groups: Vec<String>) -> Self {
        let path = initial_path.unwrap_or_default();

        Self {
            path_input: TextInputHandler::with_default(path),
            description_input: TextInputHandler::new(),
            group_input: TextInputHandler::new(),
            existing_groups,
            show_group_dropdown: false,
            selected_group_index: 0,
            saved_group_input: String::new(),
            focus: FocusArea::Path,
            selected_button: 0,
            last_buttons_area: None,
        }
    }

    /// Calculate dynamic modal dimensions
    fn calculate_modal_size(&self, screen_width: u16, screen_height: u16) -> (u16, u16) {
        let t = i18n::t();

        // Calculate width based on content
        let title_width = t.bookmarks_add_title().len() as u16 + 4;
        let path_label_width = t.bookmarks_add_path().len() as u16 + 2;
        let desc_label_width = t.bookmarks_add_description().len() as u16 + 2;
        let group_label_width = t.bookmarks_add_group().len() as u16 + 2;

        let max_label_width = path_label_width
            .max(desc_label_width)
            .max(group_label_width);
        let input_width = 40u16; // Minimum input width

        let content_width = title_width.max(max_label_width + input_width).max(30);
        let total_width = content_width + MODAL_PADDING_WITH_DOUBLE_BORDER;

        let max_width = (screen_width as f32 * MODAL_MAX_WIDTH_PERCENTAGE_DEFAULT) as u16;
        let width = total_width
            .max(MODAL_MIN_WIDTH_WIDE)
            .min(max_width)
            .min(screen_width);

        // Calculate height
        // Border(1) + Path(3) + Description(3) + Group(3) + [Dropdown] + Buttons(1) + Border(1)
        let dropdown_height = if self.show_group_dropdown && !self.existing_groups.is_empty() {
            self.existing_groups.len().min(5) as u16 + 2
        } else {
            0
        };
        let height = (1 + 3 + 3 + 3 + dropdown_height + 1 + 1).min(screen_height);

        (width, height)
    }

    /// Get currently focused input handler
    fn current_input(&mut self) -> Option<&mut TextInputHandler> {
        match self.focus {
            FocusArea::Path => Some(&mut self.path_input),
            FocusArea::Description => Some(&mut self.description_input),
            FocusArea::Group => Some(&mut self.group_input),
            FocusArea::Buttons => None,
        }
    }

    /// Move to next focus area
    fn next_focus(&mut self) {
        self.focus = match self.focus {
            FocusArea::Path => FocusArea::Description,
            FocusArea::Description => FocusArea::Group,
            FocusArea::Group => FocusArea::Buttons,
            FocusArea::Buttons => FocusArea::Path,
        };
        // Close dropdown when leaving group
        if self.focus != FocusArea::Group {
            self.show_group_dropdown = false;
        }
    }

    /// Move to previous focus area
    fn prev_focus(&mut self) {
        self.focus = match self.focus {
            FocusArea::Path => FocusArea::Buttons,
            FocusArea::Description => FocusArea::Path,
            FocusArea::Group => FocusArea::Description,
            FocusArea::Buttons => FocusArea::Group,
        };
        // Close dropdown when leaving group
        if self.focus != FocusArea::Group {
            self.show_group_dropdown = false;
        }
    }

    /// Render a labeled input field
    fn render_input_field(
        &self,
        buf: &mut Buffer,
        area: Rect,
        label: &str,
        input: &TextInputHandler,
        is_focused: bool,
        theme: &Theme,
    ) {
        // Split into label and input
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(15), Constraint::Min(1)])
            .split(area);

        // Render label (same style as InputModal's prompt)
        // Vertically center label in the 3-row height area
        let label_para = Paragraph::new(label.to_string())
            .style(Style::default().fg(theme.fg))
            .alignment(Alignment::Right);
        let label_area = Rect {
            x: chunks[0].x,
            y: chunks[0].y + 1, // Middle row of 3-row height
            width: chunks[0].width,
            height: 1,
        };
        label_para.render(label_area, buf);

        // Render input with border (same style as InputModal)
        let text_before = input.text_before_cursor();
        let text_after = input.text_after_cursor();

        let input_line = if is_focused {
            Line::from(vec![
                Span::styled(text_before, Style::default().fg(theme.fg)),
                Span::styled("█", Style::default().fg(theme.bg).bg(theme.fg)),
                Span::styled(text_after, Style::default().fg(theme.fg)),
            ])
        } else {
            Line::from(vec![Span::styled(
                input.text(),
                Style::default().fg(theme.fg),
            )])
        };

        let border_style = if is_focused {
            Style::default().fg(theme.accented_fg)
        } else {
            Style::default().fg(theme.disabled)
        };

        let input_para = Paragraph::new(input_line)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style),
            )
            .style(Style::default().bg(theme.bg));
        input_para.render(chunks[1], buf);
    }

    /// Render the group input field with dropdown toggle indicator
    fn render_group_field(&self, buf: &mut Buffer, area: Rect, label: &str, theme: &Theme) {
        let is_focused = self.focus == FocusArea::Group;
        let has_groups = !self.existing_groups.is_empty();

        // Split into label and input
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(15), Constraint::Min(1)])
            .split(area);

        // Render label (same style as InputModal's prompt)
        let label_para = Paragraph::new(label.to_string())
            .style(Style::default().fg(theme.fg))
            .alignment(Alignment::Right);
        let label_area = Rect {
            x: chunks[0].x,
            y: chunks[0].y + 1,
            width: chunks[0].width,
            height: 1,
        };
        label_para.render(label_area, buf);

        // Calculate input area and indicator
        let input_area = chunks[1];
        let indicator = if has_groups {
            if self.show_group_dropdown {
                "▲"
            } else {
                "▼"
            }
        } else {
            ""
        };

        // Render input text with cursor and indicator
        let text_before = self.group_input.text_before_cursor();
        let text_after = self.group_input.text_after_cursor();

        // Calculate available width for text (minus borders and indicator)
        let inner_width = input_area.width.saturating_sub(2) as usize; // borders
        let indicator_width = if has_groups { 2 } else { 0 }; // "▲ " or "▼ "
        let text_width = inner_width.saturating_sub(indicator_width);

        // Build the text line with indicator on the right
        let mut spans = Vec::new();
        if is_focused {
            spans.push(Span::styled(text_before, Style::default().fg(theme.fg)));
            spans.push(Span::styled(
                "█",
                Style::default().fg(theme.bg).bg(theme.fg),
            ));
            spans.push(Span::styled(text_after, Style::default().fg(theme.fg)));
        } else {
            spans.push(Span::styled(
                self.group_input.text(),
                Style::default().fg(theme.fg),
            ));
        }

        // Calculate padding needed to push indicator to the right
        let current_text_len = if is_focused {
            text_before.chars().count() + 1 + text_after.chars().count()
        } else {
            self.group_input.text().chars().count()
        };
        let padding_needed = text_width.saturating_sub(current_text_len);
        if padding_needed > 0 {
            spans.push(Span::raw(" ".repeat(padding_needed)));
        }

        // Add indicator
        if has_groups {
            spans.push(Span::styled(
                format!(" {}", indicator),
                Style::default().fg(theme.disabled),
            ));
        }

        let input_line = Line::from(spans);

        let border_style = if is_focused {
            Style::default().fg(theme.accented_fg)
        } else {
            Style::default().fg(theme.disabled)
        };

        // Use different borders based on dropdown state
        let borders = if self.show_group_dropdown && has_groups {
            Borders::LEFT | Borders::TOP | Borders::RIGHT // No bottom border when dropdown shown
        } else {
            Borders::ALL
        };

        let input_para = Paragraph::new(input_line)
            .block(Block::default().borders(borders).border_style(border_style))
            .style(Style::default().bg(theme.bg));
        input_para.render(input_area, buf);
    }
}

impl Modal for BookmarkAddModal {
    type Result = BookmarkAddResult;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let t = i18n::t();

        // Calculate dimensions
        let (modal_width, modal_height) = self.calculate_modal_size(area.width, area.height);
        let modal_area = centered_rect_with_size(modal_width, modal_height, area);

        let inner = render_modal_block(modal_area, buf, t.bookmarks_add_title(), theme);

        // Calculate layout
        let dropdown_height = if self.show_group_dropdown && !self.existing_groups.is_empty() {
            self.existing_groups.len().min(5) as u16 + 2
        } else {
            0
        };

        let mut constraints = vec![
            Constraint::Length(3), // Path
            Constraint::Length(3), // Description
            Constraint::Length(3), // Group
        ];
        if dropdown_height > 0 {
            constraints.push(Constraint::Length(dropdown_height));
        }
        constraints.push(Constraint::Length(1)); // Buttons

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let mut chunk_idx = 0;

        // Render Path field
        self.render_input_field(
            buf,
            chunks[chunk_idx],
            t.bookmarks_add_path(),
            &self.path_input,
            self.focus == FocusArea::Path,
            theme,
        );
        chunk_idx += 1;

        // Render Description field
        self.render_input_field(
            buf,
            chunks[chunk_idx],
            t.bookmarks_add_description(),
            &self.description_input,
            self.focus == FocusArea::Description,
            theme,
        );
        chunk_idx += 1;

        // Render Group field with dropdown indicator
        self.render_group_field(buf, chunks[chunk_idx], t.bookmarks_add_group(), theme);
        chunk_idx += 1;

        // Render group dropdown if visible (visually connected to input field)
        if dropdown_height > 0 {
            // Split dropdown area same as input field to align borders
            let dropdown_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(15), Constraint::Min(1)])
                .split(chunks[chunk_idx]);

            let items: Vec<ListItem> = self
                .existing_groups
                .iter()
                .enumerate()
                .map(|(idx, group)| {
                    let (prefix, style) = if idx == self.selected_group_index {
                        (
                            "▶ ",
                            Style::default()
                                .fg(theme.selected_fg)
                                .bg(theme.selected_bg)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        ("  ", Style::default().fg(theme.fg))
                    };
                    ListItem::new(Line::from(Span::styled(
                        format!("{}{}", prefix, group),
                        style,
                    )))
                })
                .collect();

            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::LEFT | Borders::BOTTOM | Borders::RIGHT)
                        .border_style(Style::default().fg(theme.accented_fg)),
                )
                .style(Style::default().bg(theme.bg));
            list.render(dropdown_chunks[1], buf); // Render in right chunk (after label)
            chunk_idx += 1;
        }

        // Render buttons
        let add_style = button_style(
            self.focus == FocusArea::Buttons && self.selected_button == 0,
            theme,
        );
        let cancel_style = button_style(
            self.focus == FocusArea::Buttons && self.selected_button == 1,
            theme,
        );

        let buttons = Line::from(vec![
            Span::styled(format!("[ {} ]", t.ui_ok()), add_style),
            Span::raw("    "),
            Span::styled(format!("[ {} ]", t.ui_cancel()), cancel_style),
        ]);

        let buttons_paragraph = Paragraph::new(buttons).alignment(Alignment::Center);
        buttons_paragraph.render(chunks[chunk_idx], buf);

        // Save buttons area for mouse handling
        self.last_buttons_area = Some(chunks[chunk_idx]);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>> {
        // Escape to cancel
        if key.code == KeyCode::Esc {
            if self.show_group_dropdown {
                // Rollback: restore saved input
                self.group_input = TextInputHandler::with_default(self.saved_group_input.clone());
                self.show_group_dropdown = false;
                return Ok(None);
            }
            return Ok(Some(ModalResult::Cancelled));
        }

        // Tab/Shift+Tab for navigation (or dropdown toggle on Group field)
        if key.code == KeyCode::Tab {
            // When focus is Group and groups exist: Tab toggles dropdown
            if self.focus == FocusArea::Group
                && !self.existing_groups.is_empty()
                && !key.modifiers.contains(KeyModifiers::SHIFT)
            {
                if self.show_group_dropdown {
                    self.show_group_dropdown = false;
                } else {
                    self.saved_group_input = self.group_input.text().to_string();
                    self.show_group_dropdown = true;
                    self.selected_group_index = 0;
                }
                return Ok(None);
            }
            // Otherwise: standard Tab navigation
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                self.prev_focus();
            } else {
                self.next_focus();
            }
            return Ok(None);
        }

        // Handle based on focus
        match self.focus {
            FocusArea::Path | FocusArea::Description => {
                // Standard input handling
                match key.code {
                    KeyCode::Down => {
                        self.next_focus();
                    }
                    KeyCode::Up => {
                        self.prev_focus();
                    }
                    KeyCode::Enter => {
                        // Move to next field or confirm if on buttons
                        self.next_focus();
                    }
                    KeyCode::Char(c) => {
                        if !key.modifiers.contains(KeyModifiers::CONTROL) {
                            if let Some(input) = self.current_input() {
                                input.insert_char(c);
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        if let Some(input) = self.current_input() {
                            input.backspace();
                        }
                    }
                    KeyCode::Delete => {
                        if let Some(input) = self.current_input() {
                            input.delete();
                        }
                    }
                    KeyCode::Left => {
                        if let Some(input) = self.current_input() {
                            input.move_left();
                        }
                    }
                    KeyCode::Right => {
                        if let Some(input) = self.current_input() {
                            input.move_right();
                        }
                    }
                    KeyCode::Home => {
                        if let Some(input) = self.current_input() {
                            input.move_home();
                        }
                    }
                    KeyCode::End => {
                        if let Some(input) = self.current_input() {
                            input.move_end();
                        }
                    }
                    _ => {}
                }
                Ok(None)
            }
            FocusArea::Group => {
                // Group field with dropdown support
                if self.show_group_dropdown && !self.existing_groups.is_empty() {
                    match key.code {
                        KeyCode::Up => {
                            if self.selected_group_index > 0 {
                                self.selected_group_index -= 1;
                            }
                        }
                        KeyCode::Down => {
                            if self.selected_group_index < self.existing_groups.len() - 1 {
                                self.selected_group_index += 1;
                            }
                        }
                        KeyCode::Enter => {
                            // Select group from dropdown
                            if let Some(group) = self.existing_groups.get(self.selected_group_index)
                            {
                                self.group_input = TextInputHandler::with_default(group.clone());
                            }
                            self.show_group_dropdown = false;
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Down => {
                            // Down arrow moves to next focus (Buttons)
                            // Tab handles dropdown toggle
                            self.next_focus();
                        }
                        KeyCode::Up => {
                            self.prev_focus();
                        }
                        KeyCode::Enter => {
                            self.next_focus();
                        }
                        KeyCode::Char(c) => {
                            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                                self.group_input.insert_char(c);
                            }
                        }
                        KeyCode::Backspace => {
                            self.group_input.backspace();
                        }
                        KeyCode::Delete => {
                            self.group_input.delete();
                        }
                        KeyCode::Left => {
                            self.group_input.move_left();
                        }
                        KeyCode::Right => {
                            self.group_input.move_right();
                        }
                        KeyCode::Home => {
                            self.group_input.move_home();
                        }
                        KeyCode::End => {
                            self.group_input.move_end();
                        }
                        _ => {}
                    }
                }
                Ok(None)
            }
            FocusArea::Buttons => {
                match key.code {
                    KeyCode::Left => {
                        self.selected_button = if self.selected_button == 0 { 1 } else { 0 };
                    }
                    KeyCode::Right => {
                        self.selected_button = if self.selected_button == 1 { 0 } else { 1 };
                    }
                    KeyCode::Up => {
                        self.prev_focus();
                    }
                    KeyCode::Enter => {
                        if self.selected_button == 0 {
                            // Add button - validate and return result
                            let path = self.path_input.text().to_string();
                            if path.is_empty() {
                                // Can't add empty path
                                return Ok(None);
                            }

                            let description = {
                                let d = self.description_input.text();
                                if d.is_empty() {
                                    None
                                } else {
                                    Some(d.to_string())
                                }
                            };

                            let group = {
                                let g = self.group_input.text();
                                if g.is_empty() {
                                    None
                                } else {
                                    Some(g.to_string())
                                }
                            };

                            return Ok(Some(ModalResult::Confirmed(BookmarkAddResult {
                                path,
                                description,
                                group,
                            })));
                        } else {
                            // Cancel button
                            return Ok(Some(ModalResult::Cancelled));
                        }
                    }
                    KeyCode::Char(c) => {
                        if !key.modifiers.contains(KeyModifiers::CONTROL) {
                            // Switch back to path input and insert character
                            self.focus = FocusArea::Path;
                            self.path_input.insert_char(c);
                        }
                    }
                    KeyCode::Backspace => {
                        // Switch back to path input and delete character
                        self.focus = FocusArea::Path;
                        self.path_input.backspace();
                    }
                    _ => {}
                }
                Ok(None)
            }
        }
    }

    fn handle_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        _modal_area: Rect,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        use crossterm::event::MouseEventKind;

        // Only handle left button press
        if mouse.kind != MouseEventKind::Down(crossterm::event::MouseButton::Left) {
            return Ok(None);
        }

        // Check if we have stored buttons area
        let Some(buttons_area) = self.last_buttons_area else {
            return Ok(None);
        };

        // Check if click is within buttons area
        if mouse.row < buttons_area.y
            || mouse.row >= buttons_area.y + buttons_area.height
            || mouse.column < buttons_area.x
            || mouse.column >= buttons_area.x + buttons_area.width
        {
            return Ok(None);
        }

        // Calculate button positions (same logic as InputModal)
        let t = i18n::t();
        let ok_text = format!("[ {} ]", t.ui_ok());
        let cancel_text = format!("[ {} ]", t.ui_cancel());
        let total_text_width = ok_text.len() + MODAL_BUTTON_SPACING as usize + cancel_text.len();

        let start_col =
            buttons_area.x + (buttons_area.width.saturating_sub(total_text_width as u16)) / 2;
        let ok_end = start_col + ok_text.len() as u16;
        let cancel_start = ok_end + MODAL_BUTTON_SPACING;
        let cancel_end = cancel_start + cancel_text.len() as u16;

        // Determine which button was clicked
        if mouse.column >= start_col && mouse.column < ok_end {
            // OK button clicked
            self.focus = FocusArea::Buttons;
            self.selected_button = 0;
            // Execute OK action immediately
            let path = self.path_input.text().to_string();
            if path.is_empty() {
                return Ok(None);
            }

            let description = {
                let d = self.description_input.text();
                if d.is_empty() {
                    None
                } else {
                    Some(d.to_string())
                }
            };

            let group = {
                let g = self.group_input.text();
                if g.is_empty() {
                    None
                } else {
                    Some(g.to_string())
                }
            };

            Ok(Some(ModalResult::Confirmed(BookmarkAddResult {
                path,
                description,
                group,
            })))
        } else if mouse.column >= cancel_start && mouse.column < cancel_end {
            // Cancel button clicked
            self.focus = FocusArea::Buttons;
            self.selected_button = 1;
            Ok(Some(ModalResult::Cancelled))
        } else {
            Ok(None)
        }
    }

    fn handle_paste(&mut self, text: &str) -> bool {
        if let Some(input) = self.current_input() {
            input.paste(text);
            true
        } else {
            false
        }
    }
}
