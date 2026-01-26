//! Text input modal dialog.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use crate::base::{button_style, render_input_field, render_modal_block};
use crate::input_keys::{handle_input_key, InputKeyResult};

use termide_config::constants::MODAL_BUTTON_SPACING;
use termide_i18n as i18n;
use termide_theme::Theme;

use crate::{
    calculate_modal_width, centered_rect_with_size, max_line_width, Modal, ModalResult,
    ModalWidthConfig, TextInputHandler,
};

/// Focus area in the modal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusArea {
    Input,
    Buttons,
}

/// Text input modal window
#[derive(Debug)]
pub struct InputModal {
    title: String,
    prompt: String,
    input_handler: TextInputHandler,
    focus: FocusArea,
    selected_button: usize, // 0 = OK, 1 = Cancel
    last_buttons_area: Option<Rect>,
    last_input_area: Option<Rect>,
}

impl InputModal {
    /// Create a new input modal window
    pub fn new(title: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            prompt: prompt.into(),
            input_handler: TextInputHandler::new(),
            focus: FocusArea::Input,
            selected_button: 0, // OK button selected by default
            last_buttons_area: None,
            last_input_area: None,
        }
    }

    /// Create with default value
    pub fn with_default(
        title: impl Into<String>,
        prompt: impl Into<String>,
        default: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            prompt: prompt.into(),
            input_handler: TextInputHandler::with_default(default),
            focus: FocusArea::Input,
            selected_button: 0, // OK button selected by default
            last_buttons_area: None,
            last_input_area: None,
        }
    }

    /// Calculate dynamic modal width and height
    fn calculate_modal_size(&self, screen_width: u16, screen_height: u16) -> (u16, u16) {
        let title_width = self.title.len() as u16 + 2;
        let prompt_width = max_line_width(&self.prompt);
        let buttons_width = 21u16; // "[ OK ]    [ Cancel ]"
        let input_width = self.input_handler.text().chars().count() as u16 + 20;

        let width = calculate_modal_width(
            [title_width, prompt_width, buttons_width, input_width].into_iter(),
            screen_width,
            ModalWidthConfig {
                wide: false,
                double_border: true,
            },
        );

        // Calculate height: border + prompt + input(3) + buttons + border
        let prompt_lines = if self.prompt.is_empty() {
            0
        } else {
            self.prompt.lines().count().max(1) as u16
        };
        let height = (1 + prompt_lines + 3 + 1 + 1).min(screen_height);

        (width, height)
    }
}

impl Modal for InputModal {
    type Result = String;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        // Calculate dynamic dimensions
        let (modal_width, modal_height) = self.calculate_modal_size(area.width, area.height);

        // Create centered area
        let modal_area = centered_rect_with_size(modal_width, modal_height, area);

        let inner = render_modal_block(modal_area, buf, &self.title, theme);

        // Split into prompt (if not empty), input, and buttons
        let prompt_lines = if self.prompt.is_empty() {
            0
        } else {
            self.prompt.lines().count().max(1) as u16
        };

        let constraints = if prompt_lines > 0 {
            vec![
                Constraint::Length(prompt_lines), // Prompt
                Constraint::Length(3),            // Input
                Constraint::Length(1),            // Buttons
            ]
        } else {
            vec![
                Constraint::Length(3), // Input
                Constraint::Length(1), // Buttons
            ]
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let mut chunk_idx = 0;

        // Render prompt if not empty
        if prompt_lines > 0 {
            let prompt = Paragraph::new(self.prompt.clone())
                .alignment(Alignment::Left)
                .style(Style::default().fg(theme.fg));
            prompt.render(chunks[chunk_idx], buf);
            chunk_idx += 1;
        }

        // Render input field with border
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.accented_fg));
        let input_inner = input_block.inner(chunks[chunk_idx]);
        input_block.render(chunks[chunk_idx], buf);

        // Save input area for mouse handling
        self.last_input_area = Some(input_inner);

        // Render input content with cursor and selection
        render_input_field(
            buf,
            input_inner.x,
            input_inner.y,
            input_inner.width,
            self.input_handler.text(),
            self.input_handler.cursor_pos(),
            self.input_handler.selection_range(),
            self.focus == FocusArea::Input,
            theme,
        );
        chunk_idx += 1;

        // Render buttons
        let t = i18n::t();

        let ok_style = button_style(
            self.focus == FocusArea::Buttons && self.selected_button == 0,
            theme,
        );
        let cancel_style = button_style(
            self.focus == FocusArea::Buttons && self.selected_button == 1,
            theme,
        );

        let buttons = Line::from(vec![
            Span::styled(format!("[ {} ]", t.ui_ok()), ok_style),
            Span::raw("    "),
            Span::styled(format!("[ {} ]", t.ui_cancel()), cancel_style),
        ]);

        let buttons_paragraph = Paragraph::new(buttons).alignment(Alignment::Center);
        buttons_paragraph.render(chunks[chunk_idx], buf);

        // Save buttons area for mouse handling
        self.last_buttons_area = Some(chunks[chunk_idx]);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>> {
        // Escape always cancels
        if key.code == KeyCode::Esc {
            return Ok(Some(ModalResult::Cancelled));
        }

        match self.focus {
            FocusArea::Input => {
                // Try common input handling first
                match handle_input_key(&mut self.input_handler, key) {
                    InputKeyResult::Handled | InputKeyResult::TextModified => {
                        return Ok(None);
                    }
                    InputKeyResult::NotHandled => {}
                }

                // Modal-specific handling
                match key.code {
                    KeyCode::Down | KeyCode::Tab => {
                        // Move focus to buttons
                        self.focus = FocusArea::Buttons;
                        Ok(None)
                    }
                    KeyCode::Enter => {
                        // Confirm input (or cancel if empty)
                        if self.input_handler.is_empty() {
                            Ok(Some(ModalResult::Cancelled))
                        } else {
                            Ok(Some(ModalResult::Confirmed(
                                self.input_handler.text().to_string(),
                            )))
                        }
                    }
                    _ => Ok(None),
                }
            }
            FocusArea::Buttons => {
                // Handle text input keys even when on buttons
                match handle_input_key(&mut self.input_handler, key) {
                    InputKeyResult::Handled | InputKeyResult::TextModified => {
                        // Switch back to input when typing
                        self.focus = FocusArea::Input;
                        return Ok(None);
                    }
                    InputKeyResult::NotHandled => {}
                }

                match key.code {
                    KeyCode::Left => {
                        // Move to previous button (wrap around)
                        self.selected_button = if self.selected_button == 0 { 1 } else { 0 };
                        Ok(None)
                    }
                    KeyCode::Right => {
                        // Move to next button (wrap around)
                        self.selected_button = if self.selected_button == 1 { 0 } else { 1 };
                        Ok(None)
                    }
                    KeyCode::Up | KeyCode::BackTab => {
                        // Move focus back to input
                        self.focus = FocusArea::Input;
                        Ok(None)
                    }
                    KeyCode::Enter => {
                        // Execute selected button action
                        if self.selected_button == 0 {
                            // OK button
                            if self.input_handler.is_empty() {
                                Ok(Some(ModalResult::Cancelled))
                            } else {
                                Ok(Some(ModalResult::Confirmed(
                                    self.input_handler.text().to_string(),
                                )))
                            }
                        } else {
                            // Cancel button
                            Ok(Some(ModalResult::Cancelled))
                        }
                    }
                    _ => Ok(None),
                }
            }
        }
    }

    fn handle_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        _modal_area: Rect,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        use crossterm::event::{MouseButton, MouseEventKind};

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Check if click is on input field
                if let Some(input_area) = self.last_input_area {
                    if mouse.column >= input_area.x
                        && mouse.column < input_area.x + input_area.width
                        && mouse.row == input_area.y
                    {
                        self.focus = FocusArea::Input;
                        let click_x = (mouse.column - input_area.x) as usize;
                        let char_pos = screen_x_to_char_pos(self.input_handler.text(), click_x);
                        self.input_handler.set_cursor_with_selection_start(char_pos);
                        return Ok(None);
                    }
                }

                // Check if click is on buttons
                let Some(buttons_area) = self.last_buttons_area else {
                    return Ok(None);
                };

                if mouse.row < buttons_area.y
                    || mouse.row >= buttons_area.y + buttons_area.height
                    || mouse.column < buttons_area.x
                    || mouse.column >= buttons_area.x + buttons_area.width
                {
                    return Ok(None);
                }

                // Calculate button positions
                let t = i18n::t();
                let ok_text = format!("[ {} ]", t.ui_ok());
                let cancel_text = format!("[ {} ]", t.ui_cancel());
                let total_text_width =
                    ok_text.len() + MODAL_BUTTON_SPACING as usize + cancel_text.len();

                let start_col = buttons_area.x
                    + (buttons_area.width.saturating_sub(total_text_width as u16)) / 2;
                let ok_end = start_col + ok_text.len() as u16;
                let cancel_start = ok_end + MODAL_BUTTON_SPACING;
                let cancel_end = cancel_start + cancel_text.len() as u16;

                if mouse.column >= start_col && mouse.column < ok_end {
                    // OK button clicked
                    self.focus = FocusArea::Buttons;
                    self.selected_button = 0;
                    if self.input_handler.is_empty() {
                        return Ok(Some(ModalResult::Cancelled));
                    } else {
                        return Ok(Some(ModalResult::Confirmed(
                            self.input_handler.text().to_string(),
                        )));
                    }
                } else if mouse.column >= cancel_start && mouse.column < cancel_end {
                    // Cancel button clicked
                    self.focus = FocusArea::Buttons;
                    self.selected_button = 1;
                    return Ok(Some(ModalResult::Cancelled));
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                // Extend selection during drag on input field
                if let Some(input_area) = self.last_input_area {
                    if mouse.row == input_area.y {
                        let drag_x = if mouse.column < input_area.x {
                            0
                        } else {
                            (mouse.column - input_area.x) as usize
                        };
                        let char_pos = screen_x_to_char_pos(self.input_handler.text(), drag_x);
                        self.input_handler.extend_selection_to(char_pos);
                    }
                }
            }
            _ => {}
        }

        Ok(None)
    }

    fn handle_paste(&mut self, text: &str) -> bool {
        self.input_handler.paste(text);
        true
    }
}

/// Convert screen X position to character position in text.
fn screen_x_to_char_pos(text: &str, screen_x: usize) -> usize {
    use unicode_width::UnicodeWidthChar;
    let mut width = 0;
    for (i, c) in text.chars().enumerate() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(1);
        if width + cw > screen_x {
            return i;
        }
        width += cw;
    }
    text.chars().count() // Click past end = cursor at end
}
