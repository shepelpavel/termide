//! Unified search modal dialog.
//!
//! Supports three modes:
//! - Text: live search in editor/terminal (Ctrl+F)
//! - FileGlob: file search by glob in file manager (Ctrl+F)
//! - Content: file mask + content regex in file manager (Ctrl+Shift+F)

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
};

use termide_core::SearchMode;
use termide_theme::Theme;

use crate::base::screen_x_to_char_pos;
use crate::input_keys::{handle_input_key, InputKeyResult};
use crate::{base, Modal, ModalResult, TextInputHandler};

/// Search modal result
#[derive(Debug, Clone)]
pub struct SearchModalResult {
    pub mode: SearchMode,
    pub query: String,
    pub content_query: Option<String>,
    /// Replace-with text (Content mode only).
    pub replace_query: Option<String>,
    pub action: SearchAction,
    /// Treat the query as a regular expression.
    pub use_regex: bool,
    /// Case-sensitive matching.
    pub case_sensitive: bool,
}

/// Search action
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchAction {
    /// Search and go to first match
    Search,
    /// Navigate to next match
    Next,
    /// Navigate to previous match
    Previous,
    /// Replace every match (Content mode) — uses `replace_query`.
    ReplaceAll,
    /// Close modal with selection active
    CloseWithSelection,
}

/// Focus area in search modal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusArea {
    /// Main input field (query for Text, mask for FileGlob/Content)
    Input,
    /// Content query field (Content mode only)
    ContentInput,
    /// Replace-with field (Content mode only)
    ReplaceInput,
    /// Navigation buttons
    Buttons,
}

/// A focusable control on the buttons row (navigated with ←/→, activated with
/// Enter/Space, or clicked).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Btn {
    Prev,
    Next,
    ReplaceAll,
    Regex,
    Case,
}

/// Interactive search modal with live preview and navigation
#[derive(Debug)]
pub struct SearchModal {
    mode: SearchMode,
    input_handler: TextInputHandler,
    content_input_handler: TextInputHandler,
    /// Replace-with field (Content mode only).
    replace_input_handler: TextInputHandler,
    focus: FocusArea,
    selected_button: usize, // 0 = Previous, 1 = Next
    /// Match count display (e.g. "3 of 12")
    match_info: Option<(usize, usize)>, // (current, total)
    /// Treat the query as a regular expression (the `[.*]` toggle).
    use_regex: bool,
    /// Case-sensitive matching (the `[Aa]` toggle).
    case_sensitive: bool,
    /// Last rendered areas of the buttons-row controls; the index points into
    /// `buttons()` so clicks dispatch the same control as keyboard focus.
    last_button_areas: Vec<(Rect, usize)>,
    last_close_button_area: Option<Rect>,
    last_input_area: Option<Rect>,
    last_content_input_area: Option<Rect>,
    last_replace_input_area: Option<Rect>,
}

impl SearchModal {
    /// Create new search modal with specified mode
    pub fn new(mode: SearchMode) -> Self {
        Self {
            mode,
            input_handler: TextInputHandler::new(),
            content_input_handler: TextInputHandler::new(),
            replace_input_handler: TextInputHandler::new(),
            focus: FocusArea::Input,
            selected_button: 1, // Next button selected by default
            match_info: None,
            use_regex: false,
            case_sensitive: false,
            last_button_areas: Vec::new(),
            last_close_button_area: None,
            last_input_area: None,
            last_content_input_area: None,
            last_replace_input_area: None,
        }
    }

    /// Create new text search modal (backward compat)
    pub fn new_text() -> Self {
        Self::new(SearchMode::Text)
    }

    /// Create new file glob search modal
    pub fn new_file_glob() -> Self {
        Self::new(SearchMode::FileGlob)
    }

    /// Create new content search modal
    pub fn new_content() -> Self {
        Self::new(SearchMode::Content)
    }

    /// Get mode
    pub fn mode(&self) -> SearchMode {
        self.mode
    }

    /// Update match information (current index, total count)
    pub fn set_match_info(&mut self, current: usize, total: usize) {
        self.match_info = Some((current, total));
    }

    /// Clear match info
    pub fn clear_match_info(&mut self) {
        self.match_info = None;
    }

    /// Set initial input text (e.g., from previous search)
    pub fn set_input(&mut self, text: String) {
        self.input_handler = TextInputHandler::with_default(text);
    }

    /// Get the title for this mode
    fn title(&self) -> &str {
        match self.mode {
            SearchMode::Text => "Search",
            SearchMode::FileGlob => "Search files",
            SearchMode::Content => "Search content",
        }
    }

    /// Calculate modal size
    fn calculate_modal_size(&self, screen_width: u16, screen_height: u16) -> (u16, u16) {
        let min_width = 60u16;
        let max_width = (screen_width as f32 * 0.6) as u16;
        let width = min_width.min(max_width).min(screen_width);

        // Height depends on mode
        let height = match self.mode {
            SearchMode::Text => 4,     // border + input + buttons + border
            SearchMode::FileGlob => 4, // border + mask + buttons + border
            SearchMode::Content => 6,  // border + mask + find + replace + buttons + border
        };

        (width, height.min(screen_height))
    }

    /// Build result from current state
    fn make_result(&self, action: SearchAction) -> SearchModalResult {
        SearchModalResult {
            mode: self.mode,
            query: self.input_handler.text().to_string(),
            content_query: if self.mode == SearchMode::Content {
                Some(self.content_input_handler.text().to_string())
            } else {
                None
            },
            replace_query: if self.mode == SearchMode::Content {
                Some(self.replace_input_handler.text().to_string())
            } else {
                None
            },
            action,
            use_regex: self.use_regex,
            case_sensitive: self.case_sensitive,
        }
    }

    /// Toggle regex / case and, if there's input, re-run the search so the
    /// results reflect the new mode immediately.
    fn toggle_result(&mut self, regex: bool) -> Result<Option<ModalResult<SearchModalResult>>> {
        if regex {
            self.use_regex = !self.use_regex;
        } else {
            self.case_sensitive = !self.case_sensitive;
        }
        self.clear_match_info();
        if self.has_input() {
            return Ok(Some(ModalResult::Confirmed(
                self.make_result(SearchAction::Search),
            )));
        }
        Ok(None)
    }

    /// The focusable controls on the buttons row, left to right. Order must
    /// match the render order so focus highlight and clicks line up.
    fn buttons(&self) -> Vec<Btn> {
        let mut items = vec![Btn::Prev, Btn::Next];
        if self.mode == SearchMode::Content {
            items.push(Btn::ReplaceAll);
        }
        items.push(Btn::Regex);
        items.push(Btn::Case);
        items
    }

    /// Activate a button (from Enter/Space on the focused control, or a click).
    fn activate_button(&mut self, btn: Btn) -> Result<Option<ModalResult<SearchModalResult>>> {
        match btn {
            Btn::Prev if self.has_input() => Ok(Some(ModalResult::Confirmed(
                self.make_result(SearchAction::Previous),
            ))),
            Btn::Next if self.has_input() => Ok(Some(ModalResult::Confirmed(
                self.make_result(SearchAction::Next),
            ))),
            Btn::ReplaceAll if self.has_input() => Ok(Some(ModalResult::Confirmed(
                self.make_result(SearchAction::ReplaceAll),
            ))),
            Btn::Regex => self.toggle_result(true),
            Btn::Case => self.toggle_result(false),
            _ => Ok(None),
        }
    }

    /// Check if can produce a result (has non-empty input)
    fn has_input(&self) -> bool {
        if self.input_handler.is_empty() {
            return false;
        }
        if self.mode == SearchMode::Content && self.content_input_handler.is_empty() {
            return false;
        }
        true
    }

    /// Get the label prefix for main input
    fn input_label(&self) -> &str {
        match self.mode {
            SearchMode::Text => "",
            SearchMode::FileGlob => "Mask: ",
            SearchMode::Content => "Mask: ",
        }
    }

    /// Handle text input key for the active input field
    fn handle_active_input_key(&mut self, key: KeyEvent) -> bool {
        let handler = match self.focus {
            FocusArea::Input => &mut self.input_handler,
            FocusArea::ContentInput => &mut self.content_input_handler,
            FocusArea::ReplaceInput => &mut self.replace_input_handler,
            _ => return false,
        };

        match handle_input_key(handler, key) {
            InputKeyResult::TextModified => true,
            InputKeyResult::Handled => true,
            InputKeyResult::NotHandled => false,
        }
    }
}

impl Modal for SearchModal {
    type Result = SearchModalResult;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let (modal_width, modal_height) = self.calculate_modal_size(area.width, area.height);
        let modal_area = base::top_center_rect(modal_width, modal_height, area);

        // Render modal frame with [X] close button
        let (inner, close_button_area) =
            base::render_modal_frame(modal_area, buf, theme, self.title());
        self.last_close_button_area = Some(close_button_area);

        // Determine layout based on mode
        let constraints = match self.mode {
            SearchMode::Text | SearchMode::FileGlob => vec![
                Constraint::Length(1), // Input line
                Constraint::Length(1), // Buttons line
            ],
            SearchMode::Content => vec![
                Constraint::Length(1), // Mask input line
                Constraint::Length(1), // Find input line
                Constraint::Length(1), // Replace input line
                Constraint::Length(1), // Buttons line
            ],
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        match self.mode {
            SearchMode::Text => {
                // === Input line ===
                let input_area = chunks[0];
                self.last_input_area = Some(input_area);
                base::render_input_field(
                    buf,
                    input_area.x,
                    input_area.y,
                    input_area.width,
                    self.input_handler.text(),
                    self.input_handler.cursor_pos(),
                    self.input_handler.selection_range(),
                    matches!(self.focus, FocusArea::Input),
                    theme,
                );
                self.render_buttons_and_counter(chunks[1], buf, theme);
            }
            SearchMode::FileGlob => {
                // === Mask input with label ===
                let input_area = chunks[0];
                self.last_input_area = Some(input_area);
                let label = self.input_label();
                let label_width = label.len() as u16;
                buf.set_string(
                    input_area.x,
                    input_area.y,
                    label,
                    Style::default().fg(theme.fg),
                );
                let field_x = input_area.x + label_width;
                let field_w = input_area.width.saturating_sub(label_width);
                base::render_input_field(
                    buf,
                    field_x,
                    input_area.y,
                    field_w,
                    self.input_handler.text(),
                    self.input_handler.cursor_pos(),
                    self.input_handler.selection_range(),
                    matches!(self.focus, FocusArea::Input),
                    theme,
                );
                self.render_buttons_and_counter(chunks[1], buf, theme);
            }
            SearchMode::Content => {
                // === Mask input ===
                let mask_area = chunks[0];
                self.last_input_area = Some(mask_area);
                let label = "Mask: ";
                let label_width = label.len() as u16;
                buf.set_string(
                    mask_area.x,
                    mask_area.y,
                    label,
                    Style::default().fg(theme.fg),
                );
                let field_x = mask_area.x + label_width;
                let field_w = mask_area.width.saturating_sub(label_width);
                base::render_input_field(
                    buf,
                    field_x,
                    mask_area.y,
                    field_w,
                    self.input_handler.text(),
                    self.input_handler.cursor_pos(),
                    self.input_handler.selection_range(),
                    matches!(self.focus, FocusArea::Input),
                    theme,
                );

                // === Content input ===
                let content_area = chunks[1];
                self.last_content_input_area = Some(content_area);
                let clabel = "Find: ";
                let clabel_width = clabel.len() as u16;
                buf.set_string(
                    content_area.x,
                    content_area.y,
                    clabel,
                    Style::default().fg(theme.fg),
                );
                let cfield_x = content_area.x + clabel_width;
                let cfield_w = content_area.width.saturating_sub(clabel_width);
                base::render_input_field(
                    buf,
                    cfield_x,
                    content_area.y,
                    cfield_w,
                    self.content_input_handler.text(),
                    self.content_input_handler.cursor_pos(),
                    self.content_input_handler.selection_range(),
                    matches!(self.focus, FocusArea::ContentInput),
                    theme,
                );

                // === Replace input ===
                let replace_area = chunks[2];
                self.last_replace_input_area = Some(replace_area);
                let rlabel = "Repl: ";
                let rlabel_width = rlabel.len() as u16;
                buf.set_string(
                    replace_area.x,
                    replace_area.y,
                    rlabel,
                    Style::default().fg(theme.fg),
                );
                let rfield_x = replace_area.x + rlabel_width;
                let rfield_w = replace_area.width.saturating_sub(rlabel_width);
                base::render_input_field(
                    buf,
                    rfield_x,
                    replace_area.y,
                    rfield_w,
                    self.replace_input_handler.text(),
                    self.replace_input_handler.cursor_pos(),
                    self.replace_input_handler.selection_range(),
                    matches!(self.focus, FocusArea::ReplaceInput),
                    theme,
                );

                self.render_buttons_and_counter(chunks[3], buf, theme);
            }
        }
    }

    fn handle_key(
        &mut self,
        chord: termide_core::KeyChord,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        let key = chord.raw;
        match self.focus {
            FocusArea::Input | FocusArea::ContentInput | FocusArea::ReplaceInput => {
                self.handle_input_focus_key(key)
            }
            FocusArea::Buttons => self.handle_buttons_focus_key(key),
        }
    }

    fn handle_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        _modal_area: Rect,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        use crossterm::event::{MouseButton, MouseEventKind};

        let mouse_pos = (mouse.column, mouse.row);

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Check if clicked on close button [X]
                if let Some(close_area) = self.last_close_button_area {
                    if mouse_pos.0 >= close_area.x
                        && mouse_pos.0 < close_area.x + close_area.width
                        && mouse_pos.1 == close_area.y
                    {
                        return Ok(Some(ModalResult::Cancelled));
                    }
                }

                // Check the replace input field — focus it on click
                if let Some(area) = self.last_replace_input_area {
                    if mouse_pos.1 == area.y
                        && mouse_pos.0 >= area.x
                        && mouse_pos.0 < area.x + area.width
                    {
                        self.focus = FocusArea::ReplaceInput;
                        let label_len = "Repl: ".len();
                        let click_x = (mouse_pos.0 - area.x) as usize;
                        let char_pos = screen_x_to_char_pos(
                            self.replace_input_handler.text(),
                            click_x.saturating_sub(label_len),
                        );
                        self.replace_input_handler
                            .set_cursor_with_selection_start(char_pos);
                        return Ok(None);
                    }
                }

                // Check if clicked on any button / toggle
                let clicked = self.last_button_areas.iter().find_map(|(area, idx)| {
                    (mouse_pos.0 >= area.x
                        && mouse_pos.0 < area.x + area.width
                        && mouse_pos.1 == area.y)
                        .then_some(*idx)
                });
                if let Some(idx) = clicked {
                    if let Some(&btn) = self.buttons().get(idx) {
                        return self.activate_button(btn);
                    }
                }

                // Check if clicked on main input field
                if let Some(input_area) = self.last_input_area {
                    if mouse_pos.0 >= input_area.x
                        && mouse_pos.0 < input_area.x + input_area.width
                        && mouse_pos.1 == input_area.y
                    {
                        self.focus = FocusArea::Input;
                        let click_x = (mouse_pos.0 - input_area.x) as usize;
                        // Account for label width
                        let label_len = self.input_label().len();
                        let click_x = click_x.saturating_sub(label_len);
                        let char_pos = screen_x_to_char_pos(self.input_handler.text(), click_x);
                        self.input_handler.set_cursor_with_selection_start(char_pos);
                    }
                }

                // Check if clicked on content input field
                if let Some(content_area) = self.last_content_input_area {
                    if mouse_pos.0 >= content_area.x
                        && mouse_pos.0 < content_area.x + content_area.width
                        && mouse_pos.1 == content_area.y
                    {
                        self.focus = FocusArea::ContentInput;
                        let click_x = (mouse_pos.0 - content_area.x) as usize;
                        let label_len = "Find: ".len();
                        let click_x = click_x.saturating_sub(label_len);
                        let char_pos =
                            screen_x_to_char_pos(self.content_input_handler.text(), click_x);
                        self.content_input_handler
                            .set_cursor_with_selection_start(char_pos);
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                // Extend selection during drag on input
                if let Some(input_area) = self.last_input_area {
                    if mouse_pos.1 == input_area.y && self.focus == FocusArea::Input {
                        let drag_x = if mouse_pos.0 < input_area.x {
                            0
                        } else {
                            let label_len = self.input_label().len();
                            (mouse_pos.0 - input_area.x) as usize
                                - label_len.min((mouse_pos.0 - input_area.x) as usize)
                        };
                        let char_pos = screen_x_to_char_pos(self.input_handler.text(), drag_x);
                        self.input_handler.extend_selection_to(char_pos);
                    }
                }
                if let Some(content_area) = self.last_content_input_area {
                    if mouse_pos.1 == content_area.y && self.focus == FocusArea::ContentInput {
                        let drag_x = if mouse_pos.0 < content_area.x {
                            0
                        } else {
                            let label_len = "Find: ".len();
                            (mouse_pos.0 - content_area.x) as usize
                                - label_len.min((mouse_pos.0 - content_area.x) as usize)
                        };
                        let char_pos =
                            screen_x_to_char_pos(self.content_input_handler.text(), drag_x);
                        self.content_input_handler.extend_selection_to(char_pos);
                    }
                }
            }
            _ => {}
        }

        Ok(None)
    }

    fn handle_paste(&mut self, text: &str) -> bool {
        match self.focus {
            FocusArea::Input => {
                self.input_handler.insert_str(text);
                true
            }
            FocusArea::ContentInput => {
                self.content_input_handler.insert_str(text);
                true
            }
            FocusArea::ReplaceInput => {
                self.replace_input_handler.insert_str(text);
                true
            }
            FocusArea::Buttons => false,
        }
    }
}

impl SearchModal {
    /// Render buttons and match counter line
    fn render_buttons_and_counter(&mut self, buttons_area: Rect, buf: &mut Buffer, theme: &Theme) {
        // Match counter on the right.
        let match_text = if let Some((current, total)) = self.match_info {
            if total == 0 {
                "No matches".to_string()
            } else {
                format!("{} of {}", current + 1, total)
            }
        } else {
            String::new()
        };
        let right_width = match_text.len() as u16;
        if right_width > 0 && buttons_area.width > right_width {
            let right_x = buttons_area.x + buttons_area.width - right_width;
            buf.set_string(
                right_x,
                buttons_area.y,
                &match_text,
                Style::default().fg(theme.fg),
            );
        }
        let counter_left = buttons_area.x + buttons_area.width.saturating_sub(right_width);

        let buttons_focused = matches!(self.focus, FocusArea::Buttons);
        self.last_button_areas.clear();
        let mut x = buttons_area.x;

        for (idx, item) in self.buttons().into_iter().enumerate() {
            let focused = buttons_focused && self.selected_button == idx;
            let (text, style): (String, Style) = match item {
                Btn::Prev | Btn::Next | Btn::ReplaceAll => {
                    let label = match item {
                        Btn::Prev => "\u{25c4} Prev",
                        Btn::Next => "Next \u{25ba}",
                        _ => "Replace all",
                    };
                    let text = if focused {
                        format!("[ {} ]", label)
                    } else {
                        format!("  {}  ", label)
                    };
                    let mut st = if item == Btn::ReplaceAll {
                        Style::default().fg(theme.warning)
                    } else {
                        Style::default().fg(theme.fg)
                    };
                    if focused {
                        st = st.add_modifier(Modifier::BOLD | Modifier::REVERSED);
                    }
                    (text, st)
                }
                Btn::Regex | Btn::Case => {
                    let (label, on) = if item == Btn::Regex {
                        (".*", self.use_regex)
                    } else {
                        ("Aa", self.case_sensitive)
                    };
                    let mut st = if on {
                        Style::default()
                            .fg(theme.accented_fg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.disabled)
                    };
                    if focused {
                        st = st.add_modifier(Modifier::REVERSED);
                    }
                    (format!("[{}]", label), st)
                }
            };

            let w = text.chars().count() as u16;
            if x + w >= counter_left {
                break;
            }
            buf.set_string(x, buttons_area.y, &text, style);
            self.last_button_areas.push((
                Rect {
                    x,
                    y: buttons_area.y,
                    width: w,
                    height: 1,
                },
                idx,
            ));
            x += w + 1;
        }
    }

    /// Handle key when focus is on input field(s)
    fn handle_input_focus_key(
        &mut self,
        key: KeyEvent,
    ) -> Result<Option<ModalResult<SearchModalResult>>> {
        match (key.code, key.modifiers) {
            // Tab - next match (for Text mode: live, for others: trigger search)
            (KeyCode::Tab, KeyModifiers::NONE) => {
                if self.has_input() {
                    let action = if self.mode != SearchMode::Text && self.match_info.is_none() {
                        SearchAction::Search
                    } else {
                        SearchAction::Next
                    };
                    return Ok(Some(ModalResult::Confirmed(self.make_result(action))));
                }
            }
            // Shift+Tab - previous match
            (KeyCode::BackTab, _) => {
                if self.has_input() {
                    let action = if self.mode != SearchMode::Text && self.match_info.is_none() {
                        SearchAction::Search
                    } else {
                        SearchAction::Previous
                    };
                    return Ok(Some(ModalResult::Confirmed(self.make_result(action))));
                }
            }
            // Down - move focus
            (KeyCode::Down, KeyModifiers::NONE) => match (self.mode, self.focus) {
                (SearchMode::Content, FocusArea::Input) => {
                    self.focus = FocusArea::ContentInput;
                }
                (SearchMode::Content, FocusArea::ContentInput) => {
                    self.focus = FocusArea::ReplaceInput;
                }
                (SearchMode::Content, FocusArea::ReplaceInput) => {
                    if self.has_input() {
                        self.focus = FocusArea::Buttons;
                    }
                }
                (_, FocusArea::Input) => {
                    if self.has_input() {
                        self.focus = FocusArea::Buttons;
                    }
                }
                _ => {}
            },
            // Up - move focus
            (KeyCode::Up, KeyModifiers::NONE) => match (self.mode, self.focus) {
                (SearchMode::Content, FocusArea::ContentInput) => {
                    self.focus = FocusArea::Input;
                }
                (SearchMode::Content, FocusArea::ReplaceInput) => {
                    self.focus = FocusArea::ContentInput;
                }
                _ => {}
            },
            // Enter
            (KeyCode::Enter, KeyModifiers::NONE) => {
                // In the replace field, Enter applies the replacement to all
                // matches (the app gates it behind a confirmation).
                if self.focus == FocusArea::ReplaceInput && self.has_input() {
                    return Ok(Some(ModalResult::Confirmed(
                        self.make_result(SearchAction::ReplaceAll),
                    )));
                }
                if self.mode == SearchMode::Text {
                    // Text mode: close with selection
                    if self.has_input() {
                        return Ok(Some(ModalResult::Confirmed(
                            self.make_result(SearchAction::CloseWithSelection),
                        )));
                    }
                } else {
                    // FileGlob/Content: search if no results yet, otherwise select
                    if self.has_input() {
                        let action = if self.match_info.is_some() {
                            SearchAction::CloseWithSelection
                        } else {
                            SearchAction::Search
                        };
                        return Ok(Some(ModalResult::Confirmed(self.make_result(action))));
                    }
                }
            }
            // Esc - close with selection if results exist, otherwise cancel
            (KeyCode::Esc, KeyModifiers::NONE) => {
                if self.match_info.is_some() {
                    return Ok(Some(ModalResult::Confirmed(
                        self.make_result(SearchAction::CloseWithSelection),
                    )));
                }
                return Ok(Some(ModalResult::Cancelled));
            }
            // F3 - next match (or search if not yet performed)
            (KeyCode::F(3), KeyModifiers::NONE) => {
                if self.has_input() {
                    let action = if self.mode != SearchMode::Text && self.match_info.is_none() {
                        SearchAction::Search
                    } else {
                        SearchAction::Next
                    };
                    return Ok(Some(ModalResult::Confirmed(self.make_result(action))));
                }
            }
            // Shift+F3 - previous match (or search if not yet performed)
            (KeyCode::F(3), KeyModifiers::SHIFT) => {
                if self.has_input() {
                    let action = if self.mode != SearchMode::Text && self.match_info.is_none() {
                        SearchAction::Search
                    } else {
                        SearchAction::Previous
                    };
                    return Ok(Some(ModalResult::Confirmed(self.make_result(action))));
                }
            }
            // Character input
            (KeyCode::Char(_), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                self.handle_active_input_key(key);
                self.clear_match_info();
                // Text mode: live search on every keystroke
                if self.mode == SearchMode::Text && self.has_input() {
                    return Ok(Some(ModalResult::Confirmed(
                        self.make_result(SearchAction::Search),
                    )));
                }
            }
            // Backspace
            (KeyCode::Backspace, KeyModifiers::NONE) => {
                self.handle_active_input_key(key);
                self.clear_match_info();
                // Text mode: live search
                if self.mode == SearchMode::Text && self.has_input() {
                    return Ok(Some(ModalResult::Confirmed(
                        self.make_result(SearchAction::Search),
                    )));
                }
            }
            // Delete
            (KeyCode::Delete, KeyModifiers::NONE) => {
                self.handle_active_input_key(key);
                self.clear_match_info();
                if self.mode == SearchMode::Text && self.has_input() {
                    return Ok(Some(ModalResult::Confirmed(
                        self.make_result(SearchAction::Search),
                    )));
                }
            }
            // Ctrl+V - paste
            (KeyCode::Char('v'), KeyModifiers::CONTROL) => {
                if let Some(text) = termide_clipboard::paste() {
                    match self.focus {
                        FocusArea::Input => self.input_handler.insert_str(&text),
                        FocusArea::ContentInput => self.content_input_handler.insert_str(&text),
                        _ => {}
                    }
                    self.clear_match_info();
                    if self.mode == SearchMode::Text && self.has_input() {
                        return Ok(Some(ModalResult::Confirmed(
                            self.make_result(SearchAction::Search),
                        )));
                    }
                }
            }
            // Ctrl+A - select all
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                self.handle_active_input_key(key);
            }
            // Ctrl+C - copy
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                let handler = match self.focus {
                    FocusArea::Input => &self.input_handler,
                    FocusArea::ContentInput => &self.content_input_handler,
                    _ => return Ok(None),
                };
                if let Some(text) = handler.selected_text() {
                    let _ = termide_clipboard::copy(text);
                }
            }
            // Ctrl+X - cut
            (KeyCode::Char('x'), KeyModifiers::CONTROL) => {
                let handler = match self.focus {
                    FocusArea::Input => &mut self.input_handler,
                    FocusArea::ContentInput => &mut self.content_input_handler,
                    _ => return Ok(None),
                };
                if let Some(text) = handler.selected_text() {
                    let _ = termide_clipboard::copy(text);
                    handler.delete_selection();
                    self.clear_match_info();
                    if self.mode == SearchMode::Text && self.has_input() {
                        return Ok(Some(ModalResult::Confirmed(
                            self.make_result(SearchAction::Search),
                        )));
                    }
                }
            }
            // Ctrl+Z - undo
            (KeyCode::Char('z'), KeyModifiers::CONTROL) => {
                self.handle_active_input_key(key);
                self.clear_match_info();
                if self.mode == SearchMode::Text && self.has_input() {
                    return Ok(Some(ModalResult::Confirmed(
                        self.make_result(SearchAction::Search),
                    )));
                }
            }
            // Ctrl+Y - redo
            (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
                self.handle_active_input_key(key);
                self.clear_match_info();
                if self.mode == SearchMode::Text && self.has_input() {
                    return Ok(Some(ModalResult::Confirmed(
                        self.make_result(SearchAction::Search),
                    )));
                }
            }
            // Other keys: delegate to input handler
            _ => {
                self.handle_active_input_key(key);
            }
        }

        Ok(None)
    }

    /// Handle key when focus is on buttons
    fn handle_buttons_focus_key(
        &mut self,
        key: KeyEvent,
    ) -> Result<Option<ModalResult<SearchModalResult>>> {
        match (key.code, key.modifiers) {
            (KeyCode::Left, KeyModifiers::NONE) => {
                let len = self.buttons().len();
                self.selected_button = (self.selected_button + len - 1) % len;
            }
            (KeyCode::Right, KeyModifiers::NONE) => {
                let len = self.buttons().len();
                self.selected_button = (self.selected_button + 1) % len;
            }
            // Activate the focused control (button or toggle).
            (KeyCode::Enter, KeyModifiers::NONE) | (KeyCode::Char(' '), KeyModifiers::NONE) => {
                if let Some(&btn) = self.buttons().get(self.selected_button) {
                    return self.activate_button(btn);
                }
            }
            (KeyCode::Tab, KeyModifiers::NONE) => {
                if self.has_input() {
                    let action = if self.mode != SearchMode::Text && self.match_info.is_none() {
                        SearchAction::Search
                    } else {
                        SearchAction::Next
                    };
                    return Ok(Some(ModalResult::Confirmed(self.make_result(action))));
                }
            }
            (KeyCode::BackTab, _) => {
                if self.has_input() {
                    let action = if self.mode != SearchMode::Text && self.match_info.is_none() {
                        SearchAction::Search
                    } else {
                        SearchAction::Previous
                    };
                    return Ok(Some(ModalResult::Confirmed(self.make_result(action))));
                }
            }
            (KeyCode::Up, KeyModifiers::NONE) => match self.mode {
                SearchMode::Content => self.focus = FocusArea::ContentInput,
                _ => self.focus = FocusArea::Input,
            },
            (KeyCode::Esc, KeyModifiers::NONE) => {
                if self.match_info.is_some() {
                    return Ok(Some(ModalResult::Confirmed(
                        self.make_result(SearchAction::CloseWithSelection),
                    )));
                }
                return Ok(Some(ModalResult::Cancelled));
            }
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                self.focus = FocusArea::Input;
                self.input_handler.insert_char(ch);
                if self.mode == SearchMode::Text {
                    return Ok(Some(ModalResult::Confirmed(
                        self.make_result(SearchAction::Search),
                    )));
                }
            }
            _ => {}
        }

        Ok(None)
    }
}
