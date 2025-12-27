//! Information display modal with action buttons.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};
use unicode_width::UnicodeWidthStr;

use termide_config::constants::{
    MODAL_MAX_WIDTH_PERCENTAGE_WIDE, MODAL_MIN_VALUE_WIDTH, MODAL_MIN_WIDTH_WIDE, SPINNER_FRAMES,
    SPINNER_FRAMES_COUNT,
};
use termide_i18n as i18n;
use termide_theme::Theme;

use crate::{centered_rect_with_size, Modal, ModalResult};

/// Action button definition
#[derive(Debug, Clone)]
pub struct ActionButton {
    /// Button label
    pub label: String,
    /// Action identifier returned when button is clicked
    pub action: String,
}

impl ActionButton {
    /// Create a new action button
    pub fn new(label: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            action: action.into(),
        }
    }
}

/// Information modal window with action buttons
#[derive(Debug)]
pub struct InfoActionModal {
    title: String,
    lines: Vec<(String, String)>,
    buttons: Vec<ActionButton>,
    selected_button: usize,
    spinner_frame: usize,
    last_button_areas: Vec<Rect>,
}

impl InfoActionModal {
    /// Create a new information modal window with tabular data and action buttons
    pub fn new(
        title: impl Into<String>,
        lines: Vec<(String, String)>,
        buttons: Vec<ActionButton>,
    ) -> Self {
        Self {
            title: title.into(),
            lines,
            buttons,
            selected_button: 0,
            spinner_frame: 0,
            last_button_areas: Vec::new(),
        }
    }

    /// Set the initially selected button index
    pub fn with_selected_button(mut self, index: usize) -> Self {
        self.selected_button = index.min(self.buttons.len().saturating_sub(1));
        self
    }

    /// Update a specific field value by key
    pub fn update_value(&mut self, key: &str, new_value: String) {
        if let Some(line) = self.lines.iter_mut().find(|(k, _)| k == key) {
            line.1 = new_value;
        }
    }

    /// Advance the spinner frame counter (for animation)
    pub fn advance_spinner(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES_COUNT;
    }

    /// Get the current spinner character
    fn get_spinner_char(&self) -> &str {
        SPINNER_FRAMES[self.spinner_frame]
    }

    /// Wrap text to fit within max_width, breaking on delimiters
    fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
        if max_width == 0 {
            return vec![text.to_string()];
        }

        let text_width = text.width();
        if text_width <= max_width {
            return vec![text.to_string()];
        }

        let mut lines = Vec::new();
        let mut current_line = String::new();
        let mut current_width = 0;

        let parts: Vec<&str> = if text.contains('/') || text.contains('\\') {
            text.split_inclusive(&['/', '\\'][..]).collect()
        } else {
            text.split_inclusive(' ').collect()
        };

        for part in parts {
            let part_width = part.width();

            if part_width > max_width {
                if !current_line.is_empty() {
                    lines.push(current_line.clone());
                    current_line.clear();
                    current_width = 0;
                }

                for ch in part.chars() {
                    let ch_width = ch.to_string().width();
                    if current_width + ch_width > max_width {
                        lines.push(current_line.clone());
                        current_line.clear();
                        current_width = 0;
                    }
                    current_line.push(ch);
                    current_width += ch_width;
                }
            } else if current_width + part_width > max_width {
                if !current_line.is_empty() {
                    lines.push(current_line.clone());
                }
                current_line = part.to_string();
                current_width = part_width;
            } else {
                current_line.push_str(part);
                current_width += part_width;
            }
        }

        if !current_line.is_empty() {
            lines.push(current_line);
        }

        if lines.is_empty() {
            lines.push(String::new());
        }

        lines
    }

    /// Calculate dynamic modal width based on content size
    fn calculate_modal_width(&self, screen_width: u16) -> u16 {
        let max_key_len = self
            .lines
            .iter()
            .map(|(key, _)| key.width())
            .max()
            .unwrap_or(0);

        let t = i18n::t();
        let max_value_len = self
            .lines
            .iter()
            .map(|(_, value)| {
                if value.contains(t.file_info_calculating()) {
                    value.width() + 2
                } else {
                    value.width()
                }
            })
            .max()
            .unwrap_or(0);

        // Calculate buttons width (including spacing)
        let buttons_width: usize = self
            .buttons
            .iter()
            .map(|b| b.label.width() + 4) // "[ label ]" + space
            .sum::<usize>()
            + self.buttons.len().saturating_sub(1) * 2; // spacing between buttons

        let content_width = 6 + max_key_len + 2 + max_value_len;
        let buttons_row_width = buttons_width + 4; // padding

        let required_width = content_width.max(buttons_row_width);

        let max_width = (screen_width as f32 * MODAL_MAX_WIDTH_PERCENTAGE_WIDE) as u16;
        (required_width as u16)
            .max(MODAL_MIN_WIDTH_WIDE)
            .min(max_width)
            .min(screen_width)
    }

    fn select_next_button(&mut self) {
        if !self.buttons.is_empty() {
            self.selected_button = (self.selected_button + 1) % self.buttons.len();
        }
    }

    fn select_prev_button(&mut self) {
        if !self.buttons.is_empty() {
            if self.selected_button == 0 {
                self.selected_button = self.buttons.len() - 1;
            } else {
                self.selected_button -= 1;
            }
        }
    }
}

/// Result from InfoActionModal
#[derive(Debug, Clone)]
pub enum InfoActionResult {
    /// User selected an action button
    Action(String),
    /// User closed the modal
    Closed,
}

impl Modal for InfoActionModal {
    type Result = InfoActionResult;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let modal_width = self.calculate_modal_width(area.width);

        let max_key_len = self
            .lines
            .iter()
            .map(|(key, _)| key.width())
            .max()
            .unwrap_or(0);

        let available_value_width = modal_width
            .saturating_sub(6)
            .saturating_sub(max_key_len as u16)
            .saturating_sub(2)
            .max(MODAL_MIN_VALUE_WIDTH as u16) as usize;

        let t = i18n::t();
        let mut total_data_lines = 0;
        for (_, value) in &self.lines {
            let display_value = if value.contains(t.file_info_calculating()) {
                format!("{} {}", self.get_spinner_char(), value)
            } else {
                value.clone()
            };
            let wrapped = Self::wrap_text(&display_value, available_value_width);
            total_data_lines += wrapped.len();
        }

        // Height: top border + empty + data + empty + buttons + bottom border
        let modal_height = (total_data_lines + 5) as u16;

        let modal_area = centered_rect_with_size(modal_width, modal_height, area);

        Clear.render(modal_area, buf);

        let block = Block::default()
            .title(Span::styled(
                format!(" {} ", self.title),
                Style::default().fg(theme.bg).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.bg))
            .style(Style::default().bg(theme.fg));

        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(total_data_lines as u16),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

        // Render tabular data
        let mut text_lines = Vec::new();
        for (key, value) in &self.lines {
            let padding = " ".repeat(max_key_len - key.width());

            let display_value = if value.contains(t.file_info_calculating()) {
                format!("{} {}", self.get_spinner_char(), value)
            } else {
                value.clone()
            };

            let wrapped_values = Self::wrap_text(&display_value, available_value_width);

            if !wrapped_values.is_empty() {
                let spans = if key.is_empty() {
                    vec![
                        Span::styled(
                            format!("  {}{}", key, padding),
                            Style::default()
                                .fg(theme.accented_fg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw("  "),
                        Span::styled(wrapped_values[0].clone(), Style::default().fg(theme.bg)),
                    ]
                } else {
                    vec![
                        Span::styled(
                            format!("  {}{}", key, padding),
                            Style::default()
                                .fg(theme.accented_fg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(": "),
                        Span::styled(wrapped_values[0].clone(), Style::default().fg(theme.bg)),
                    ]
                };
                text_lines.push(Line::from(spans));

                let indent = " ".repeat(max_key_len + 4);
                for wrapped_line in wrapped_values.iter().skip(1) {
                    text_lines.push(Line::from(vec![Span::styled(
                        format!("{}{}", indent, wrapped_line),
                        Style::default().fg(theme.bg),
                    )]));
                }
            }
        }

        let data = Paragraph::new(text_lines).alignment(Alignment::Left);
        data.render(chunks[1], buf);

        // Render action buttons
        self.last_button_areas.clear();

        let mut button_spans = Vec::new();
        for (i, button) in self.buttons.iter().enumerate() {
            let is_selected = i == self.selected_button;
            let style = if is_selected {
                Style::default()
                    .fg(theme.fg)
                    .bg(theme.accented_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.bg)
            };

            if i > 0 {
                button_spans.push(Span::raw("  "));
            }
            button_spans.push(Span::styled(format!("[ {} ]", button.label), style));
        }

        let buttons_line = Line::from(button_spans);
        let button_paragraph = Paragraph::new(buttons_line).alignment(Alignment::Center);
        button_paragraph.render(chunks[3], buf);

        // Calculate button areas for mouse handling
        let buttons_total_width: usize = self
            .buttons
            .iter()
            .map(|b| b.label.width() + 4)
            .sum::<usize>()
            + self.buttons.len().saturating_sub(1) * 2;

        let start_x =
            chunks[3].x + (chunks[3].width.saturating_sub(buttons_total_width as u16)) / 2;
        let mut current_x = start_x;

        for button in &self.buttons {
            let btn_width = (button.label.width() + 4) as u16;
            self.last_button_areas.push(Rect {
                x: current_x,
                y: chunks[3].y,
                width: btn_width,
                height: 1,
            });
            current_x += btn_width + 2; // button width + spacing
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>> {
        match key.code {
            KeyCode::Esc => Ok(Some(ModalResult::Cancelled)),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(button) = self.buttons.get(self.selected_button) {
                    Ok(Some(ModalResult::Confirmed(InfoActionResult::Action(
                        button.action.clone(),
                    ))))
                } else {
                    Ok(Some(ModalResult::Confirmed(InfoActionResult::Closed)))
                }
            }
            KeyCode::Tab | KeyCode::Right => {
                self.select_next_button();
                Ok(None)
            }
            KeyCode::BackTab | KeyCode::Left => {
                self.select_prev_button();
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        _modal_area: Rect,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return Ok(None);
        }

        // Check if click is on any button
        for (i, button_area) in self.last_button_areas.iter().enumerate() {
            if mouse.row == button_area.y
                && mouse.column >= button_area.x
                && mouse.column < button_area.x + button_area.width
            {
                if let Some(button) = self.buttons.get(i) {
                    return Ok(Some(ModalResult::Confirmed(InfoActionResult::Action(
                        button.action.clone(),
                    ))));
                }
            }
        }

        Ok(None)
    }
}
