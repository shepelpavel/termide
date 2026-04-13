//! Modal dialog for creating scripts.
//!
//! Provides a modal with group, name, type selector, and project checkbox fields.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Widget},
};

use crate::base::{button_style, render_input_field, render_modal_block};
use crate::input_keys::{handle_input_key, InputKeyResult};

use termide_config::constants::{
    MODAL_BUTTON_SPACING, MODAL_MAX_WIDTH_PERCENTAGE_DEFAULT, MODAL_MIN_WIDTH_WIDE,
    MODAL_PADDING_WITH_DOUBLE_BORDER,
};
use termide_theme::Theme;
use termide_ui::{SuggestionAction, SuggestionInput};

use crate::{centered_rect_with_size, Modal, ModalResult, TextInputHandler};

/// Type of script to create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptType {
    /// Normal script that runs in a terminal panel.
    Terminal,
    /// Background script (.bg. suffix).
    Background,
    /// Report script that shows modal result (.report. suffix).
    Report,
}

impl ScriptType {
    /// All script types in order.
    const ALL: [ScriptType; 3] = [
        ScriptType::Terminal,
        ScriptType::Background,
        ScriptType::Report,
    ];

    /// Display label with icon.
    fn label(&self) -> &'static str {
        match self {
            ScriptType::Terminal => "\u{1F4BB} Terminal",
            ScriptType::Background => "\u{2699} Background",
            ScriptType::Report => "\u{1F4CB} Report",
        }
    }

    /// Cycle to the next type.
    fn next(self) -> Self {
        match self {
            ScriptType::Terminal => ScriptType::Background,
            ScriptType::Background => ScriptType::Report,
            ScriptType::Report => ScriptType::Terminal,
        }
    }

    /// Cycle to the previous type.
    fn prev(self) -> Self {
        match self {
            ScriptType::Terminal => ScriptType::Report,
            ScriptType::Background => ScriptType::Terminal,
            ScriptType::Report => ScriptType::Background,
        }
    }
}

/// Result of script creation.
#[derive(Debug, Clone)]
pub struct ScriptCreateResult {
    /// Script name (without type suffix).
    pub name: String,
    /// Optional group name.
    pub group: Option<String>,
    /// Type of script.
    pub script_type: ScriptType,
    /// Whether to save as project-local script.
    pub is_project: bool,
}

/// Focus area in the modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusArea {
    Group,
    Name,
    Type,
    ProjectCheckbox,
    Buttons,
}

/// Script create modal.
#[derive(Debug)]
pub struct ScriptCreateModal {
    title: String,
    focus: FocusArea,
    // Group field (dropdown with suggestions)
    group_suggestion: SuggestionInput,
    // Name field
    name_input: TextInputHandler,
    // Type selector
    script_type: ScriptType,
    // Project checkbox
    is_project: bool,
    // Button selection
    selected_button: usize, // 0 = Create, 1 = Cancel
    // Cached areas for mouse handling
    last_buttons_area: Option<Rect>,
    last_group_field_area: Option<Rect>,
    last_group_dropdown_area: Option<Rect>,
    last_checkbox_area: Option<Rect>,
    last_type_area: Option<Rect>,
    last_name_area: Option<Rect>,
}

impl ScriptCreateModal {
    /// Create a new script create modal.
    pub fn new(title: impl Into<String>, existing_groups: Vec<String>) -> Self {
        Self {
            title: title.into(),
            focus: FocusArea::Group,
            group_suggestion: SuggestionInput::new(existing_groups),
            name_input: TextInputHandler::new(),
            script_type: ScriptType::Terminal,
            is_project: false,
            selected_button: 0,
            last_buttons_area: None,
            last_group_field_area: None,
            last_group_dropdown_area: None,
            last_checkbox_area: None,
            last_type_area: None,
            last_name_area: None,
        }
    }

    /// Calculate dynamic modal dimensions.
    fn calculate_modal_size(&self, screen_width: u16, screen_height: u16) -> (u16, u16) {
        // Calculate width based on content
        let title_width = self.title.len() as u16 + 4;
        let label_width = 15u16; // "Group:" etc. right-aligned in 15 cols
        let input_width = 40u16; // Minimum input width

        let content_width = title_width.max(label_width + input_width).max(30);
        let total_width = content_width + MODAL_PADDING_WITH_DOUBLE_BORDER;

        let max_width = (screen_width as f32 * MODAL_MAX_WIDTH_PERCENTAGE_DEFAULT) as u16;
        let width = total_width
            .max(MODAL_MIN_WIDTH_WIDE)
            .min(max_width)
            .min(screen_width);

        // Calculate height
        // Border(1) + Group(3) + [Dropdown] + Name(3) + Type(2) + Checkbox(1) + Empty(1) + Buttons(1) + Border(1)
        let suggestions = self.group_suggestion.suggestions();
        let dropdown_height = if self.group_suggestion.is_expanded() && !suggestions.is_empty() {
            suggestions.len().min(5) as u16 + 1
        } else {
            0
        };
        let height = (1 + 3 + dropdown_height + 3 + 2 + 1 + 1 + 1 + 1).min(screen_height);

        (width, height)
    }

    /// Move to next focus area.
    fn next_focus(&mut self) {
        self.focus = match self.focus {
            FocusArea::Group => FocusArea::Name,
            FocusArea::Name => FocusArea::Type,
            FocusArea::Type => FocusArea::ProjectCheckbox,
            FocusArea::ProjectCheckbox => FocusArea::Buttons,
            FocusArea::Buttons => FocusArea::Group,
        };
        if self.focus != FocusArea::Group {
            self.group_suggestion.collapse();
        }
    }

    /// Move to previous focus area.
    fn prev_focus(&mut self) {
        self.focus = match self.focus {
            FocusArea::Group => FocusArea::Buttons,
            FocusArea::Name => FocusArea::Group,
            FocusArea::Type => FocusArea::Name,
            FocusArea::ProjectCheckbox => FocusArea::Type,
            FocusArea::Buttons => FocusArea::ProjectCheckbox,
        };
        if self.focus != FocusArea::Group {
            self.group_suggestion.collapse();
        }
    }

    /// Render a labeled input field.
    fn render_labeled_input_field(
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

        // Render label (right-aligned, vertically centered)
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

        // Render input with border
        let border_style = if is_focused {
            Style::default().fg(theme.accented_fg)
        } else {
            Style::default().fg(theme.disabled)
        };

        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style);
        let input_inner = input_block.inner(chunks[1]);
        input_block.render(chunks[1], buf);

        // Render input content with cursor and selection
        render_input_field(
            buf,
            input_inner.x,
            input_inner.y,
            input_inner.width,
            input.text(),
            input.cursor_pos(),
            input.selection_range(),
            is_focused,
            theme,
        );
    }

    /// Render the group input field with dropdown toggle indicator.
    fn render_group_field(&self, buf: &mut Buffer, area: Rect, label: &str, theme: &Theme) {
        let is_focused = self.focus == FocusArea::Group;
        let has_groups = !self.group_suggestion.suggestions().is_empty();

        // Split into label and input
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(15), Constraint::Min(1)])
            .split(area);

        // Render label
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

        // Calculate input area
        let input_area = chunks[1];
        let indicator = if has_groups {
            if self.group_suggestion.is_expanded() {
                "\u{25B2}" // ▲
            } else {
                "\u{25BC}" // ▼
            }
        } else {
            ""
        };
        let _ = indicator; // used below in rendering

        let border_style = if is_focused {
            Style::default().fg(theme.accented_fg)
        } else {
            Style::default().fg(theme.disabled)
        };

        // Use different borders based on dropdown state
        let borders = if self.group_suggestion.is_expanded() && has_groups {
            Borders::LEFT | Borders::TOP | Borders::RIGHT // No bottom border when dropdown shown
        } else {
            Borders::ALL
        };

        let input_block = Block::default().borders(borders).border_style(border_style);
        let input_inner = input_block.inner(input_area);
        input_block.render(input_area, buf);

        // Calculate area for text input (excluding indicator)
        let indicator_width = if has_groups { 2u16 } else { 0u16 };
        let text_width = input_inner.width.saturating_sub(indicator_width);

        // Render input content with cursor and selection
        let input = self.group_suggestion.input();
        render_input_field(
            buf,
            input_inner.x,
            input_inner.y,
            text_width,
            input.text(),
            input.cursor_pos(),
            input.selection_range(),
            is_focused,
            theme,
        );

        // Render indicator at right edge
        if has_groups {
            let indicator_x = input_inner.x + input_inner.width.saturating_sub(1);
            let indicator_str = if self.group_suggestion.is_expanded() {
                "\u{25B2}"
            } else {
                "\u{25BC}"
            };
            buf.set_string(
                indicator_x,
                input_inner.y,
                indicator_str,
                Style::default().fg(theme.disabled),
            );
        }
    }

    /// Build the confirmed result if validation passes.
    fn try_confirm(&self) -> Option<ModalResult<ScriptCreateResult>> {
        let name = self.name_input.text().trim().to_string();
        if name.is_empty() {
            return None;
        }
        // Name should not contain dots
        if name.contains('.') {
            return None;
        }

        let group = {
            let g = self.group_suggestion.text().trim().to_string();
            if g.is_empty() {
                None
            } else if g.contains('/') || g.contains('\\') {
                // No nested subdirectories — groups are one level only
                return None;
            } else {
                Some(g)
            }
        };

        Some(ModalResult::Confirmed(ScriptCreateResult {
            name,
            group,
            script_type: self.script_type,
            is_project: self.is_project,
        }))
    }
}

impl Modal for ScriptCreateModal {
    type Result = ScriptCreateResult;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        // Calculate dimensions
        let (modal_width, modal_height) = self.calculate_modal_size(area.width, area.height);
        let modal_area = centered_rect_with_size(modal_width, modal_height, area);

        let inner = render_modal_block(modal_area, buf, &self.title, theme);

        // Calculate layout
        let suggestions = self.group_suggestion.suggestions();
        let dropdown_height = if self.group_suggestion.is_expanded() && !suggestions.is_empty() {
            suggestions.len().min(5) as u16 + 1
        } else {
            0
        };

        let mut constraints = vec![
            Constraint::Length(3), // Group
        ];
        if dropdown_height > 0 {
            constraints.push(Constraint::Length(dropdown_height));
        }
        constraints.extend([
            Constraint::Length(3), // Name
            Constraint::Length(2), // Type selector
            Constraint::Length(1), // Checkbox
            Constraint::Length(1), // Empty line
            Constraint::Length(1), // Buttons
        ]);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let mut chunk_idx = 0;

        // Render Group field with dropdown indicator
        self.render_group_field(buf, chunks[chunk_idx], "Group:", theme);
        let group_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(15), Constraint::Min(1)])
            .split(chunks[chunk_idx]);
        self.last_group_field_area = Some(group_chunks[1]);
        chunk_idx += 1;

        // Render group dropdown if visible
        if dropdown_height > 0 {
            let dropdown_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(15), Constraint::Min(1)])
                .split(chunks[chunk_idx]);

            self.last_group_dropdown_area = Some(dropdown_chunks[1]);

            let selected_idx = self.group_suggestion.selected_index();
            let items: Vec<ListItem> = suggestions
                .iter()
                .enumerate()
                .map(|(idx, group)| {
                    let (prefix, style) = if idx == selected_idx {
                        (
                            "\u{25B6} ", // ▶
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
            list.render(dropdown_chunks[1], buf);
            chunk_idx += 1;
        } else {
            self.last_group_dropdown_area = None;
        }

        // Render Name field
        self.render_labeled_input_field(
            buf,
            chunks[chunk_idx],
            "Name:",
            &self.name_input,
            self.focus == FocusArea::Name,
            theme,
        );
        let name_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(15), Constraint::Min(1)])
            .split(chunks[chunk_idx]);
        self.last_name_area = Some(name_chunks[1]);
        chunk_idx += 1;

        // Render Type selector
        {
            let type_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(15), Constraint::Min(1)])
                .split(chunks[chunk_idx]);

            // Render label
            let label_para = Paragraph::new("Type:".to_string())
                .style(Style::default().fg(theme.fg))
                .alignment(Alignment::Right);
            let label_area = Rect {
                x: type_chunks[0].x,
                y: type_chunks[0].y,
                width: type_chunks[0].width,
                height: 1,
            };
            label_para.render(label_area, buf);

            // Render type options inline
            let is_focused = self.focus == FocusArea::Type;
            let type_area = type_chunks[1];
            let mut x_offset = type_area.x + 1;

            for script_type in &ScriptType::ALL {
                let is_selected = *script_type == self.script_type;
                let label = script_type.label();

                let style = if is_selected && is_focused {
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.accented_fg)
                        .add_modifier(Modifier::BOLD)
                } else if is_selected {
                    Style::default()
                        .fg(theme.accented_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.disabled)
                };

                let display = format!(" {} ", label);
                if x_offset + display.len() as u16 <= type_area.x + type_area.width {
                    buf.set_string(x_offset, type_area.y, &display, style);
                    x_offset += display.len() as u16;
                }
            }

            // Show hint on second row if focused
            if is_focused && type_area.height > 1 {
                let hint = "\u{2190}/\u{2192} switch, 1/2/3 select"; // ←/→ switch, 1/2/3 select
                let hint_style = Style::default().fg(theme.disabled);
                buf.set_string(type_area.x + 1, type_area.y + 1, hint, hint_style);
            }

            self.last_type_area = Some(type_area);
            chunk_idx += 1;
        }

        // Render project checkbox (aligned with input fields)
        {
            let cb_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(15), Constraint::Min(1)])
                .split(chunks[chunk_idx]);

            let checkbox_char = if self.is_project { "x" } else { " " };
            let checkbox_style = if self.focus == FocusArea::ProjectCheckbox {
                Style::default().fg(theme.accented_fg)
            } else {
                Style::default().fg(theme.fg)
            };
            let checkbox_text = format!(" [{}] Project script", checkbox_char);
            let checkbox = Paragraph::new(checkbox_text).style(checkbox_style);
            checkbox.render(cb_chunks[1], buf);
            self.last_checkbox_area = Some(cb_chunks[1]);
            chunk_idx += 1;
        }

        // Skip empty line
        chunk_idx += 1;

        // Render buttons
        let create_style = button_style(
            self.focus == FocusArea::Buttons && self.selected_button == 0,
            theme,
        );
        let cancel_style = button_style(
            self.focus == FocusArea::Buttons && self.selected_button == 1,
            theme,
        );

        let buttons = Line::from(vec![
            Span::styled("[ Create ]", create_style),
            Span::raw("    "),
            Span::styled("[ Cancel ]", cancel_style),
        ]);

        let buttons_paragraph = Paragraph::new(buttons).alignment(Alignment::Center);
        buttons_paragraph.render(chunks[chunk_idx], buf);

        // Save buttons area for mouse handling
        self.last_buttons_area = Some(chunks[chunk_idx]);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>> {
        // Escape to cancel
        if key.code == KeyCode::Esc {
            if self.group_suggestion.is_expanded() {
                self.group_suggestion.rollback();
                return Ok(None);
            }
            return Ok(Some(ModalResult::Cancelled));
        }

        // Tab/Shift+Tab for navigation
        if key.code == KeyCode::Tab {
            // When focus is Group and groups exist: Tab toggles dropdown
            if self.focus == FocusArea::Group
                && !self.group_suggestion.suggestions().is_empty()
                && !key.modifiers.contains(KeyModifiers::SHIFT)
            {
                self.group_suggestion.toggle();
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
            FocusArea::Group => {
                // First try suggestion input handling
                match self.group_suggestion.handle_key(key) {
                    SuggestionAction::Handled => return Ok(None),
                    SuggestionAction::Confirmed => return Ok(None),
                    SuggestionAction::Cancelled => return Ok(None),
                    SuggestionAction::TextModified => return Ok(None),
                    SuggestionAction::NotHandled => {}
                }

                // Try common input handling
                match handle_input_key(self.group_suggestion.input_mut(), key) {
                    InputKeyResult::Handled | InputKeyResult::TextModified => {
                        return Ok(None);
                    }
                    InputKeyResult::NotHandled => {}
                }

                // Modal-specific handling
                match key.code {
                    KeyCode::Down => {
                        self.next_focus();
                    }
                    KeyCode::Up => {
                        self.prev_focus();
                    }
                    KeyCode::Enter => {
                        self.next_focus();
                    }
                    _ => {}
                }
                Ok(None)
            }
            FocusArea::Name => {
                // Try common input handling first
                match handle_input_key(&mut self.name_input, key) {
                    InputKeyResult::Handled | InputKeyResult::TextModified => {
                        return Ok(None);
                    }
                    InputKeyResult::NotHandled => {}
                }

                // Modal-specific handling
                match key.code {
                    KeyCode::Down => {
                        self.next_focus();
                    }
                    KeyCode::Up => {
                        self.prev_focus();
                    }
                    KeyCode::Enter => {
                        self.next_focus();
                    }
                    _ => {}
                }
                Ok(None)
            }
            FocusArea::Type => {
                match key.code {
                    KeyCode::Left => {
                        self.script_type = self.script_type.prev();
                    }
                    KeyCode::Right => {
                        self.script_type = self.script_type.next();
                    }
                    KeyCode::Char('1') => {
                        self.script_type = ScriptType::Terminal;
                    }
                    KeyCode::Char('2') => {
                        self.script_type = ScriptType::Background;
                    }
                    KeyCode::Char('3') => {
                        self.script_type = ScriptType::Report;
                    }
                    KeyCode::Down | KeyCode::Enter => {
                        self.next_focus();
                    }
                    KeyCode::Up => {
                        self.prev_focus();
                    }
                    _ => {}
                }
                Ok(None)
            }
            FocusArea::ProjectCheckbox => {
                match key.code {
                    KeyCode::Char(' ') => {
                        self.is_project = !self.is_project;
                    }
                    KeyCode::Down | KeyCode::Enter => {
                        self.next_focus();
                    }
                    KeyCode::Up => {
                        self.prev_focus();
                    }
                    _ => {}
                }
                Ok(None)
            }
            FocusArea::Buttons => {
                // Handle text input keys even when on buttons (redirect to name)
                match handle_input_key(&mut self.name_input, key) {
                    InputKeyResult::Handled | InputKeyResult::TextModified => {
                        self.focus = FocusArea::Name;
                        return Ok(None);
                    }
                    InputKeyResult::NotHandled => {}
                }

                match key.code {
                    KeyCode::Left => {
                        self.selected_button = if self.selected_button == 0 { 1 } else { 0 };
                    }
                    KeyCode::Right => {
                        self.selected_button = if self.selected_button == 1 { 0 } else { 1 };
                    }
                    KeyCode::Up | KeyCode::BackTab => {
                        self.prev_focus();
                    }
                    KeyCode::Enter => {
                        if self.selected_button == 0 {
                            // Create button - validate and return result
                            if let Some(result) = self.try_confirm() {
                                return Ok(Some(result));
                            }
                            // Validation failed, stay on modal
                            return Ok(None);
                        } else {
                            // Cancel button
                            return Ok(Some(ModalResult::Cancelled));
                        }
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

        // Check if click is on group field (toggle dropdown)
        if let Some(group_area) = self.last_group_field_area {
            if mouse.row >= group_area.y
                && mouse.row < group_area.y + group_area.height
                && mouse.column >= group_area.x
                && mouse.column < group_area.x + group_area.width
            {
                self.focus = FocusArea::Group;
                if !self.group_suggestion.suggestions().is_empty() {
                    self.group_suggestion.toggle();
                }
                return Ok(None);
            }
        }

        // Check if click is on group dropdown items
        if let Some(dropdown_area) = self.last_group_dropdown_area {
            if mouse.row >= dropdown_area.y
                && mouse.row < dropdown_area.y + dropdown_area.height
                && mouse.column >= dropdown_area.x
                && mouse.column < dropdown_area.x + dropdown_area.width
            {
                let relative_row = mouse.row.saturating_sub(dropdown_area.y);
                let item_index = relative_row as usize;

                let suggestions_len = self.group_suggestion.suggestions().len();
                if item_index < suggestions_len {
                    self.group_suggestion.select_and_confirm(item_index);
                }
                return Ok(None);
            }
        }

        // Check if click is on name field
        if let Some(name_area) = self.last_name_area {
            if mouse.row >= name_area.y
                && mouse.row < name_area.y + name_area.height
                && mouse.column >= name_area.x
                && mouse.column < name_area.x + name_area.width
            {
                self.focus = FocusArea::Name;
                if self.focus != FocusArea::Group {
                    self.group_suggestion.collapse();
                }
                return Ok(None);
            }
        }

        // Check if click is on type selector
        if let Some(type_area) = self.last_type_area {
            if mouse.row >= type_area.y
                && mouse.row < type_area.y + type_area.height
                && mouse.column >= type_area.x
                && mouse.column < type_area.x + type_area.width
            {
                self.focus = FocusArea::Type;
                self.group_suggestion.collapse();

                // Determine which type was clicked based on column position
                let mut x_offset = type_area.x + 1;
                for script_type in &ScriptType::ALL {
                    let label = script_type.label();
                    let display_len = label.len() + 2; // " label "
                    if mouse.column >= x_offset && mouse.column < x_offset + display_len as u16 {
                        self.script_type = *script_type;
                        break;
                    }
                    x_offset += display_len as u16;
                }
                return Ok(None);
            }
        }

        // Check if click is on project checkbox
        if let Some(checkbox_area) = self.last_checkbox_area {
            if mouse.row >= checkbox_area.y
                && mouse.row < checkbox_area.y + checkbox_area.height
                && mouse.column >= checkbox_area.x
                && mouse.column < checkbox_area.x + checkbox_area.width
            {
                self.focus = FocusArea::ProjectCheckbox;
                self.group_suggestion.collapse();
                self.is_project = !self.is_project;
                return Ok(None);
            }
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

        // Calculate button positions
        let create_text = "[ Create ]";
        let cancel_text = "[ Cancel ]";
        let total_text_width =
            create_text.len() + MODAL_BUTTON_SPACING as usize + cancel_text.len();

        let start_col =
            buttons_area.x + (buttons_area.width.saturating_sub(total_text_width as u16)) / 2;
        let create_end = start_col + create_text.len() as u16;
        let cancel_start = create_end + MODAL_BUTTON_SPACING;
        let cancel_end = cancel_start + cancel_text.len() as u16;

        // Determine which button was clicked
        if mouse.column >= start_col && mouse.column < create_end {
            // Create button clicked
            self.focus = FocusArea::Buttons;
            self.selected_button = 0;
            if let Some(result) = self.try_confirm() {
                Ok(Some(result))
            } else {
                Ok(None)
            }
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
        match self.focus {
            FocusArea::Name => {
                self.name_input.paste(text);
                true
            }
            FocusArea::Group => {
                self.group_suggestion.input_mut().paste(text);
                true
            }
            FocusArea::Type | FocusArea::ProjectCheckbox | FocusArea::Buttons => false,
        }
    }
}
