//! Unified modal dialog for creating and editing commands.
//!
//! In Create mode: Group, Display Name, Command, Mode, Hotkey, Project checkbox.
//! In Edit mode: Group/Project are read-only labels; Display Name, Command, Mode, Hotkey are editable.

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

use termide_config::commands::{CommandMetadata, CommandMode};
use termide_config::constants::{
    MODAL_BUTTON_SPACING, MODAL_MAX_WIDTH_PERCENTAGE_DEFAULT, MODAL_MIN_WIDTH_WIDE,
    MODAL_PADDING_WITH_DOUBLE_BORDER,
};
use termide_i18n as i18n;
use termide_theme::Theme;
use termide_ui::{SuggestionAction, SuggestionInput};

use crate::{centered_rect_with_size, Modal, ModalResult, TextInputHandler};

/// Sanitize a string for use as filename/directory name.
pub fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '.' => '-',
            _ => c,
        })
        .collect()
}

/// Modal mode: creating a new command or editing an existing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandConfigMode {
    Create,
    Edit,
}

/// Action the user chose when confirming the modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandConfigAction {
    Save,
}

/// Result returned by the modal on confirmation.
#[derive(Debug, Clone)]
pub struct CommandConfigResult {
    pub name: String,
    pub command: Option<String>,
    pub display_name: Option<String>,
    pub group: Option<String>,
    pub mode: CommandMode,
    pub hotkey: Option<String>,
    pub is_project: bool,
    pub action: CommandConfigAction,
    /// Whether this is an edit (Some) or create (None).
    pub is_edit: bool,
}

/// Focus area in the modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusArea {
    Group,
    Command,
    DisplayName,
    Mode,
    Hotkey,
    ProjectCheckbox,
    Buttons,
}

/// Command configuration modal (unified create/edit).
#[derive(Debug)]
pub struct CommandConfigModal {
    title: String,
    mode: CommandConfigMode,
    focus: FocusArea,
    // Edit-mode: stored command name (TOML key, not editable)
    command_name: String,
    // Fields
    group_suggestion: SuggestionInput,
    command_input: TextInputHandler,
    display_name_input: TextInputHandler,
    command_mode: CommandMode,
    hotkey_input: TextInputHandler,
    is_project: bool,
    // Button selection: 0=Save/Create, 1=Cancel
    selected_button: usize,
    // Validation state
    hotkey_error: bool,
    // Cached areas for mouse handling
    last_buttons_area: Option<Rect>,
    last_group_field_area: Option<Rect>,
    last_group_dropdown_area: Option<Rect>,
    last_command_area: Option<Rect>,
    last_display_name_area: Option<Rect>,
    last_mode_area: Option<Rect>,
    last_hotkey_area: Option<Rect>,
    last_checkbox_area: Option<Rect>,
}

impl CommandConfigModal {
    /// Create a new modal in Create mode.
    pub fn new_create(title: impl Into<String>, existing_groups: Vec<String>) -> Self {
        Self {
            title: title.into(),
            mode: CommandConfigMode::Create,
            focus: FocusArea::Group,
            command_name: String::new(),
            group_suggestion: SuggestionInput::new(existing_groups),
            command_input: TextInputHandler::new(),
            display_name_input: TextInputHandler::new(),
            command_mode: CommandMode::Terminal,
            hotkey_input: TextInputHandler::new(),
            is_project: false,
            selected_button: 0,
            hotkey_error: false,
            last_buttons_area: None,
            last_group_field_area: None,
            last_group_dropdown_area: None,
            last_command_area: None,
            last_display_name_area: None,
            last_mode_area: None,
            last_hotkey_area: None,
            last_checkbox_area: None,
        }
    }

    /// Create a modal in Edit mode, pre-populated from existing metadata.
    pub fn new_edit(
        title: impl Into<String>,
        command_name: String,
        _group: Option<String>,
        is_project: bool,
        _path: Option<std::path::PathBuf>,
        metadata: Option<CommandMetadata>,
    ) -> Self {
        let display_name_text = metadata
            .as_ref()
            .and_then(|m| m.display_name.clone())
            .unwrap_or_default();
        let mut display_name_input = TextInputHandler::new();
        if !display_name_text.is_empty() {
            display_name_input.set_text(&display_name_text);
        }

        let command_mode = metadata.as_ref().and_then(|m| m.mode).unwrap_or_default();

        let hotkey_text = metadata
            .as_ref()
            .and_then(|m| m.key.clone())
            .unwrap_or_default();
        let mut hotkey_input = TextInputHandler::new();
        if !hotkey_text.is_empty() {
            hotkey_input.set_text(&hotkey_text);
        }

        let command_text = metadata
            .as_ref()
            .and_then(|m| m.command.clone())
            .unwrap_or_default();
        let mut command_input = TextInputHandler::new();
        if !command_text.is_empty() {
            command_input.set_text(&command_text);
        }

        Self {
            title: title.into(),
            mode: CommandConfigMode::Edit,
            focus: FocusArea::DisplayName,
            command_name,
            group_suggestion: SuggestionInput::new(vec![]),
            command_input,
            display_name_input,
            command_mode,
            hotkey_input,
            is_project,
            selected_button: 0,
            hotkey_error: false,
            last_buttons_area: None,
            last_group_field_area: None,
            last_group_dropdown_area: None,
            last_command_area: None,
            last_display_name_area: None,
            last_mode_area: None,
            last_hotkey_area: None,
            last_checkbox_area: None,
        }
    }

    fn is_create(&self) -> bool {
        self.mode == CommandConfigMode::Create
    }

    fn button_count(&self) -> usize {
        2 // Save/Create, Cancel
    }

    fn button_label(&self, index: usize) -> &'static str {
        let t = i18n::t();
        if self.is_create() {
            match index {
                0 => t.command_config_button_create(),
                1 => t.command_config_button_cancel(),
                _ => "",
            }
        } else {
            match index {
                0 => t.command_config_button_save(),
                1 => t.command_config_button_cancel(),
                _ => "",
            }
        }
    }

    fn calculate_modal_size(&self, screen_width: u16, screen_height: u16) -> (u16, u16) {
        let title_width = self.title.len() as u16 + 4;
        let label_width = 15u16;
        let input_width = 48u16;

        let content_width = title_width.max(label_width + input_width).max(30);
        let total_width = content_width + MODAL_PADDING_WITH_DOUBLE_BORDER;

        let max_width = (screen_width as f32 * MODAL_MAX_WIDTH_PERCENTAGE_DEFAULT) as u16;
        let width = total_width
            .max(MODAL_MIN_WIDTH_WIDE)
            .min(max_width)
            .min(screen_width);

        let suggestions = self.group_suggestion.suggestions();
        let dropdown_height =
            if self.is_create() && self.group_suggestion.is_expanded() && !suggestions.is_empty() {
                suggestions.len().min(5) as u16 + 1
            } else {
                0
            };

        // Create: Border(1) + Group(3) + [Dropdown] + Command(3) + DisplayName(3) + Mode(2) + Hotkey(3) + [HotkeyError(1)] + Checkbox(1) + Empty(1) + Buttons(1) + Border(1)
        // Edit:   Border(1) + Group(3) + DisplayName(3) + Command(3) + Mode(2) + Hotkey(3) + [HotkeyError(1)] + Project(3) + Empty(1) + Buttons(1) + Border(1)
        let mut height = if self.is_create() {
            1 + 3 + dropdown_height + 3 + 3 + 2 + 3
        } else {
            1 + 3 + 3 + 3 + 2 + 3
        };
        if self.hotkey_error {
            height += 1;
        }
        height += if self.is_create() {
            1 + 1 + 1 + 1
        } else {
            3 + 1 + 1 + 1
        };
        height = height.min(screen_height);

        (width, height)
    }

    fn next_focus(&mut self) {
        self.group_suggestion.collapse();
        let order: &[FocusArea] = if self.is_create() {
            &[
                FocusArea::Group,
                FocusArea::DisplayName,
                FocusArea::Command,
                FocusArea::Mode,
                FocusArea::Hotkey,
                FocusArea::ProjectCheckbox,
                FocusArea::Buttons,
            ]
        } else {
            &[
                FocusArea::DisplayName,
                FocusArea::Command,
                FocusArea::Mode,
                FocusArea::Hotkey,
                FocusArea::Buttons,
            ]
        };
        if let Some(idx) = order.iter().position(|f| *f == self.focus) {
            self.focus = order[(idx + 1) % order.len()];
        }
    }

    fn prev_focus(&mut self) {
        self.group_suggestion.collapse();
        let order: &[FocusArea] = if self.is_create() {
            &[
                FocusArea::Group,
                FocusArea::DisplayName,
                FocusArea::Command,
                FocusArea::Mode,
                FocusArea::Hotkey,
                FocusArea::ProjectCheckbox,
                FocusArea::Buttons,
            ]
        } else {
            &[
                FocusArea::DisplayName,
                FocusArea::Command,
                FocusArea::Mode,
                FocusArea::Hotkey,
                FocusArea::Buttons,
            ]
        };
        if let Some(idx) = order.iter().position(|f| *f == self.focus) {
            self.focus = order[(idx + order.len() - 1) % order.len()];
        }
    }

    /// Render a labeled input field (3 rows: top padding, label+bordered input, bottom padding).
    fn render_labeled_input_field(
        buf: &mut Buffer,
        area: Rect,
        label: &str,
        input: &TextInputHandler,
        is_focused: bool,
        theme: &Theme,
    ) {
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
            y: chunks[0].y + 1,
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

    /// Render a read-only label-value pair (1 row).
    /// Render a readonly field with border (3 rows), matching input field height.
    fn render_readonly_field(
        buf: &mut Buffer,
        area: Rect,
        label: &str,
        value: &str,
        theme: &Theme,
    ) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(15), Constraint::Min(1)])
            .split(area);

        // Label — vertically centered in 3-row area
        Paragraph::new(label.to_string())
            .style(Style::default().fg(theme.disabled))
            .alignment(Alignment::Right)
            .render(
                Rect {
                    x: chunks[0].x,
                    y: chunks[0].y + 1,
                    width: chunks[0].width,
                    height: 1,
                },
                buf,
            );

        // Bordered readonly value
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.disabled));
        let inner = block.inner(chunks[1]);
        block.render(chunks[1], buf);
        Paragraph::new(Span::styled(value, Style::default().fg(theme.disabled))).render(inner, buf);
    }

    /// Render group input field with dropdown indicator (create mode).
    fn render_group_field(&self, buf: &mut Buffer, area: Rect, label: &str, theme: &Theme) {
        let is_focused = self.focus == FocusArea::Group;
        let has_groups = !self.group_suggestion.suggestions().is_empty();

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(15), Constraint::Min(1)])
            .split(area);

        // Label
        Paragraph::new(label.to_string())
            .style(Style::default().fg(theme.fg))
            .alignment(Alignment::Right)
            .render(
                Rect {
                    x: chunks[0].x,
                    y: chunks[0].y + 1,
                    width: chunks[0].width,
                    height: 1,
                },
                buf,
            );

        let input_area = chunks[1];
        let border_style = if is_focused {
            Style::default().fg(theme.accented_fg)
        } else {
            Style::default().fg(theme.disabled)
        };

        let borders = if self.group_suggestion.is_expanded() && has_groups {
            Borders::LEFT | Borders::TOP | Borders::RIGHT
        } else {
            Borders::ALL
        };

        let input_block = Block::default().borders(borders).border_style(border_style);
        let input_inner = input_block.inner(input_area);
        input_block.render(input_area, buf);

        let indicator_width = if has_groups { 2u16 } else { 0u16 };
        let text_width = input_inner.width.saturating_sub(indicator_width);

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

    fn try_confirm(&self) -> Option<ModalResult<CommandConfigResult>> {
        if self.is_create() {
            let command = {
                let c = self.command_input.text().trim().to_string();
                if c.is_empty() {
                    return None;
                } else {
                    Some(c)
                }
            };
            // Derive name from display_name or command
            let display_name = {
                let d = self.display_name_input.text().trim().to_string();
                if d.is_empty() {
                    None
                } else {
                    Some(d)
                }
            };
            let name = sanitize_filename(
                display_name
                    .as_deref()
                    .unwrap_or_else(|| command.as_deref().unwrap_or("")),
            );
            if name.is_empty() {
                return None;
            }
            let group = {
                let g = sanitize_filename(self.group_suggestion.text().trim());
                if g.is_empty() {
                    None
                } else {
                    Some(g)
                }
            };
            Some(ModalResult::Confirmed(CommandConfigResult {
                name,
                command,
                display_name,
                group,
                mode: self.command_mode,
                hotkey: self.hotkey_value(),
                is_project: self.is_project,
                action: CommandConfigAction::Save,
                is_edit: false,
            }))
        } else {
            let command = {
                let c = self.command_input.text().trim().to_string();
                if c.is_empty() {
                    None
                } else {
                    Some(c)
                }
            };
            let display_name = {
                let d = self.display_name_input.text().trim().to_string();
                if d.is_empty() {
                    None
                } else {
                    Some(d)
                }
            };
            Some(ModalResult::Confirmed(CommandConfigResult {
                name: self.command_name.clone(),
                command,
                display_name,
                group: None,
                mode: self.command_mode,
                hotkey: self.hotkey_value(),
                is_project: self.is_project,
                action: CommandConfigAction::Save,
                is_edit: true,
            }))
        }
    }

    fn hotkey_value(&self) -> Option<String> {
        let h = self.hotkey_input.text().trim().to_string();
        if h.is_empty() {
            None
        } else {
            Some(h)
        }
    }

    fn validate_hotkey(&mut self) {
        let text = self.hotkey_input.text().trim();
        self.hotkey_error = !text.is_empty() && !is_valid_hotkey(text);
    }

    fn mode_label(mode: CommandMode) -> &'static str {
        let t = i18n::t();
        match mode {
            CommandMode::Terminal => t.command_config_mode_terminal(),
            CommandMode::Background => t.command_config_mode_background(),
            CommandMode::Report => t.command_config_mode_report(),
        }
    }
}

/// Basic hotkey string validation: [Ctrl+][Alt+][Shift+]Key
fn is_valid_hotkey(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let parts: Vec<&str> = s.split('+').collect();
    if parts.is_empty() {
        return false;
    }
    let modifiers = &parts[..parts.len() - 1];
    let key = parts.last().unwrap();

    const VALID_MODS: &[&str] = &["Ctrl", "Alt", "Shift"];
    const VALID_KEYS: &[&str] = &[
        "A",
        "B",
        "C",
        "D",
        "E",
        "F",
        "G",
        "H",
        "I",
        "J",
        "K",
        "L",
        "M",
        "N",
        "O",
        "P",
        "Q",
        "R",
        "S",
        "T",
        "U",
        "V",
        "W",
        "X",
        "Y",
        "Z",
        "0",
        "1",
        "2",
        "3",
        "4",
        "5",
        "6",
        "7",
        "8",
        "9",
        "F1",
        "F2",
        "F3",
        "F4",
        "F5",
        "F6",
        "F7",
        "F8",
        "F9",
        "F10",
        "F11",
        "F12",
        "Enter",
        "Tab",
        "Esc",
        "Space",
        "Backspace",
        "Delete",
        "Insert",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "Left",
        "Right",
        "Up",
        "Down",
    ];

    modifiers.iter().all(|m| VALID_MODS.contains(m)) && VALID_KEYS.contains(key)
}

impl Modal for CommandConfigModal {
    type Result = CommandConfigResult;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let (modal_w, modal_h) = self.calculate_modal_size(area.width, area.height);
        let modal_area = centered_rect_with_size(modal_w, modal_h, area);

        let inner = render_modal_block(modal_area, buf, &self.title, theme);

        // Build layout constraints
        let suggestions: Vec<String> = self.group_suggestion.suggestions().to_vec();
        let dropdown_height =
            if self.is_create() && self.group_suggestion.is_expanded() && !suggestions.is_empty() {
                suggestions.len().min(5) as u16 + 1
            } else {
                0
            };

        let mut constraints: Vec<Constraint> = Vec::new();

        if self.is_create() {
            constraints.push(Constraint::Length(3)); // group
            if dropdown_height > 0 {
                constraints.push(Constraint::Length(dropdown_height));
            }
            constraints.push(Constraint::Length(3)); // command
            constraints.push(Constraint::Length(3)); // display name
            constraints.push(Constraint::Length(2)); // mode selector
            constraints.push(Constraint::Length(3)); // hotkey
            if self.hotkey_error {
                constraints.push(Constraint::Length(1)); // error hint
            }
            constraints.push(Constraint::Length(1)); // checkbox
            constraints.push(Constraint::Length(1)); // spacer
            constraints.push(Constraint::Length(1)); // buttons
        } else {
            constraints.push(Constraint::Length(3)); // group label (readonly, bordered)
            constraints.push(Constraint::Length(3)); // display name
            constraints.push(Constraint::Length(3)); // command
            constraints.push(Constraint::Length(2)); // mode selector
            constraints.push(Constraint::Length(3)); // hotkey
            if self.hotkey_error {
                constraints.push(Constraint::Length(1)); // error hint
            }
            constraints.push(Constraint::Length(3)); // project label (readonly, bordered)
            constraints.push(Constraint::Length(1)); // spacer
            constraints.push(Constraint::Length(1)); // buttons
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let mut chunk_idx = 0;
        let t = i18n::t();

        if self.is_create() {
            // 1. Group field
            self.render_group_field(
                buf,
                chunks[chunk_idx],
                t.command_config_label_group(),
                theme,
            );
            let group_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(15), Constraint::Min(1)])
                .split(chunks[chunk_idx]);
            self.last_group_field_area = Some(group_chunks[1]);
            chunk_idx += 1;

            // Group dropdown
            if dropdown_height > 0 {
                let dd_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Length(15), Constraint::Min(1)])
                    .split(chunks[chunk_idx]);

                self.last_group_dropdown_area = Some(dd_chunks[1]);
                let selected_idx = self.group_suggestion.selected_index();
                let items: Vec<ListItem> = suggestions
                    .iter()
                    .enumerate()
                    .map(|(idx, group)| {
                        let (prefix, style) = if idx == selected_idx {
                            (
                                "\u{25B6} ",
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

                List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::LEFT | Borders::BOTTOM | Borders::RIGHT)
                            .border_style(Style::default().fg(theme.accented_fg)),
                    )
                    .style(Style::default().bg(theme.bg))
                    .render(dd_chunks[1], buf);
                chunk_idx += 1;
            } else {
                self.last_group_dropdown_area = None;
            }

            // 2. Display name field (пункт меню)
            Self::render_labeled_input_field(
                buf,
                chunks[chunk_idx],
                t.command_config_label_display_name(),
                &self.display_name_input,
                self.focus == FocusArea::DisplayName,
                theme,
            );
            let dn_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(15), Constraint::Min(1)])
                .split(chunks[chunk_idx]);
            self.last_display_name_area = Some(dn_chunks[1]);
            chunk_idx += 1;

            // 3. Command field
            Self::render_labeled_input_field(
                buf,
                chunks[chunk_idx],
                t.command_config_label_command(),
                &self.command_input,
                self.focus == FocusArea::Command,
                theme,
            );
            let cmd_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(15), Constraint::Min(1)])
                .split(chunks[chunk_idx]);
            self.last_command_area = Some(cmd_chunks[1]);
            chunk_idx += 1;
        } else {
            // 1. Group field (read-only, bordered)
            Self::render_readonly_field(
                buf,
                chunks[chunk_idx],
                t.command_config_label_group(),
                t.command_config_group_root(),
                theme,
            );
            chunk_idx += 1;

            // 2. Display name field
            Self::render_labeled_input_field(
                buf,
                chunks[chunk_idx],
                t.command_config_label_display_name(),
                &self.display_name_input,
                self.focus == FocusArea::DisplayName,
                theme,
            );
            let dn_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(15), Constraint::Min(1)])
                .split(chunks[chunk_idx]);
            self.last_display_name_area = Some(dn_chunks[1]);
            chunk_idx += 1;

            // 3. Command field
            Self::render_labeled_input_field(
                buf,
                chunks[chunk_idx],
                t.command_config_label_command(),
                &self.command_input,
                self.focus == FocusArea::Command,
                theme,
            );
            let cmd_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(15), Constraint::Min(1)])
                .split(chunks[chunk_idx]);
            self.last_command_area = Some(cmd_chunks[1]);
            chunk_idx += 1;
        }

        // Mode selector (2 rows)
        {
            let mode_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(15), Constraint::Min(1)])
                .split(chunks[chunk_idx]);

            let is_focused = self.focus == FocusArea::Mode;

            // Mode label
            Paragraph::new(t.command_config_label_mode().to_string())
                .style(Style::default().fg(theme.fg))
                .alignment(Alignment::Right)
                .render(
                    Rect {
                        x: mode_chunks[0].x,
                        y: mode_chunks[0].y,
                        width: mode_chunks[0].width,
                        height: 1,
                    },
                    buf,
                );

            // Mode buttons
            let mode_area = mode_chunks[1];
            let modes = [
                CommandMode::Terminal,
                CommandMode::Background,
                CommandMode::Report,
            ];
            let mut x_offset = mode_area.x + 1;
            for m in &modes {
                let is_selected = self.command_mode == *m;
                let label = Self::mode_label(*m);
                let style = if is_selected && is_focused {
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.accented_fg)
                        .add_modifier(Modifier::BOLD)
                } else if is_selected {
                    Style::default()
                        .fg(theme.accented_fg)
                        .add_modifier(Modifier::BOLD)
                } else if is_focused {
                    Style::default().fg(theme.fg)
                } else {
                    Style::default().fg(theme.disabled)
                };

                let display = format!(" {} ", label);
                if x_offset + display.len() as u16 <= mode_area.x + mode_area.width {
                    buf.set_string(x_offset, mode_area.y, &display, style);
                    x_offset += display.len() as u16;
                }
            }

            // Hint on second row
            if is_focused && mode_area.height > 1 {
                let hint = "\u{2190}/\u{2192} switch, 1/2/3 select";
                buf.set_string(
                    mode_area.x + 1,
                    mode_area.y + 1,
                    hint,
                    Style::default().fg(theme.disabled),
                );
            }

            self.last_mode_area = Some(mode_area);
            chunk_idx += 1;
        }

        // 4. Hotkey field
        Self::render_labeled_input_field(
            buf,
            chunks[chunk_idx],
            t.command_config_label_hotkey(),
            &self.hotkey_input,
            self.focus == FocusArea::Hotkey,
            theme,
        );
        let hk_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(15), Constraint::Min(1)])
            .split(chunks[chunk_idx]);
        self.last_hotkey_area = Some(hk_chunks[1]);
        chunk_idx += 1;

        // Hotkey error hint
        if self.hotkey_error {
            let err_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(15), Constraint::Min(1)])
                .split(chunks[chunk_idx]);
            Paragraph::new(Span::styled(
                t.command_config_hotkey_invalid(),
                Style::default().fg(theme.error),
            ))
            .render(err_chunks[1], buf);
            chunk_idx += 1;
        }

        // Hotkey hint (when focused, no error)
        if self.focus == FocusArea::Hotkey && !self.hotkey_error {
            // Could render a hint below the input
        }

        if self.is_create() {
            // Project checkbox
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
            let checkbox_text = format!(
                " [{}] {}",
                checkbox_char,
                t.command_config_project_checkbox()
            );
            Paragraph::new(checkbox_text)
                .style(checkbox_style)
                .render(cb_chunks[1], buf);
            self.last_checkbox_area = Some(cb_chunks[1]);
            chunk_idx += 1;
        } else {
            // Project field (read-only, bordered)
            let proj_text = if self.is_project { "yes" } else { "no" };
            Self::render_readonly_field(
                buf,
                chunks[chunk_idx],
                t.command_config_label_project(),
                proj_text,
                theme,
            );
            chunk_idx += 1;
        }

        // Spacer
        chunk_idx += 1;

        // Buttons
        let buttons_area = chunks[chunk_idx];
        self.last_buttons_area = Some(buttons_area);
        let btn_count = self.button_count();

        let spans: Vec<Span> = (0..btn_count)
            .flat_map(|i| {
                let label = self.button_label(i);
                let is_sel = self.focus == FocusArea::Buttons && self.selected_button == i;
                let style = button_style(is_sel, theme);
                let display = format!("[ {} ]", label);
                let mut v = vec![Span::styled(display, style)];
                if i < btn_count - 1 {
                    v.push(Span::raw("    "));
                }
                v
            })
            .collect();

        Paragraph::new(Line::from(spans))
            .alignment(Alignment::Center)
            .render(buttons_area, buf);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>> {
        // Escape
        if key.code == KeyCode::Esc && key.modifiers.is_empty() {
            if self.is_create() && self.group_suggestion.is_expanded() {
                self.group_suggestion.collapse();
                return Ok(None);
            }
            return Ok(Some(ModalResult::Cancelled));
        }

        // Tab / Shift+Tab
        if key.code == KeyCode::Tab {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                self.prev_focus();
            } else if self.is_create()
                && self.focus == FocusArea::Group
                && !self.group_suggestion.suggestions().is_empty()
            {
                if self.group_suggestion.is_expanded() {
                    self.group_suggestion.collapse();
                } else {
                    self.group_suggestion.expand();
                }
            } else {
                self.next_focus();
            }
            return Ok(None);
        }
        if key.modifiers.contains(KeyModifiers::SHIFT) && key.code == KeyCode::BackTab {
            self.prev_focus();
            return Ok(None);
        }

        match self.focus {
            FocusArea::Group if self.is_create() => match self.group_suggestion.handle_key(key) {
                SuggestionAction::Handled => {}
                SuggestionAction::Confirmed => {
                    self.group_suggestion.collapse();
                    self.next_focus();
                }
                SuggestionAction::Cancelled => {
                    self.group_suggestion.collapse();
                }
                SuggestionAction::TextModified => {}
                SuggestionAction::NotHandled => {
                    match handle_input_key(self.group_suggestion.input_mut(), key) {
                        InputKeyResult::Handled | InputKeyResult::TextModified => {}
                        InputKeyResult::NotHandled => match key.code {
                            KeyCode::Down => self.next_focus(),
                            KeyCode::Up => self.prev_focus(),
                            KeyCode::Enter => self.next_focus(),
                            _ => {}
                        },
                    }
                }
            },
            FocusArea::Command => match handle_input_key(&mut self.command_input, key) {
                InputKeyResult::Handled | InputKeyResult::TextModified => {}
                InputKeyResult::NotHandled => match key.code {
                    KeyCode::Down => self.next_focus(),
                    KeyCode::Up => self.prev_focus(),
                    KeyCode::Enter => self.next_focus(),
                    _ => {}
                },
            },
            FocusArea::DisplayName => match handle_input_key(&mut self.display_name_input, key) {
                InputKeyResult::Handled | InputKeyResult::TextModified => {}
                InputKeyResult::NotHandled => match key.code {
                    KeyCode::Down => self.next_focus(),
                    KeyCode::Up => self.prev_focus(),
                    KeyCode::Enter => self.next_focus(),
                    _ => {}
                },
            },
            FocusArea::Mode => match key.code {
                KeyCode::Right => {
                    self.command_mode = match self.command_mode {
                        CommandMode::Terminal => CommandMode::Background,
                        CommandMode::Background => CommandMode::Report,
                        CommandMode::Report => CommandMode::Terminal,
                    };
                }
                KeyCode::Left => {
                    self.command_mode = match self.command_mode {
                        CommandMode::Terminal => CommandMode::Report,
                        CommandMode::Background => CommandMode::Terminal,
                        CommandMode::Report => CommandMode::Background,
                    };
                }
                KeyCode::Char('1') => self.command_mode = CommandMode::Terminal,
                KeyCode::Char('2') => self.command_mode = CommandMode::Background,
                KeyCode::Char('3') => self.command_mode = CommandMode::Report,
                KeyCode::Down | KeyCode::Enter => self.next_focus(),
                KeyCode::Up => self.prev_focus(),
                _ => {}
            },
            FocusArea::Hotkey => match handle_input_key(&mut self.hotkey_input, key) {
                InputKeyResult::Handled | InputKeyResult::TextModified => self.validate_hotkey(),
                InputKeyResult::NotHandled => match key.code {
                    KeyCode::Down => self.next_focus(),
                    KeyCode::Up => self.prev_focus(),
                    KeyCode::Enter => {
                        self.validate_hotkey();
                        if !self.hotkey_error {
                            self.next_focus();
                        }
                    }
                    _ => {}
                },
            },
            FocusArea::ProjectCheckbox if self.is_create() => match key.code {
                KeyCode::Char(' ') => self.is_project = !self.is_project,
                KeyCode::Down | KeyCode::Enter => self.next_focus(),
                KeyCode::Up => self.prev_focus(),
                _ => {}
            },
            FocusArea::Buttons => match key.code {
                KeyCode::Left => {
                    if self.selected_button > 0 {
                        self.selected_button -= 1;
                    } else {
                        self.selected_button = self.button_count() - 1;
                    }
                }
                KeyCode::Right => {
                    self.selected_button += 1;
                    if self.selected_button >= self.button_count() {
                        self.selected_button = 0;
                    }
                }
                KeyCode::Up | KeyCode::BackTab => {
                    self.selected_button = 0;
                    self.prev_focus();
                }
                KeyCode::Enter => {
                    if self.selected_button == self.button_count() - 1 {
                        return Ok(Some(ModalResult::Cancelled));
                    }
                    if let Some(result) = self.try_confirm() {
                        return Ok(Some(result));
                    }
                }
                _ => {}
            },
            // Read-only fields in edit mode — navigate away
            _ => match key.code {
                KeyCode::Down | KeyCode::Enter => self.next_focus(),
                KeyCode::Up => self.prev_focus(),
                _ => {}
            },
        }

        Ok(None)
    }

    fn handle_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        _modal_area: Rect,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        let col = mouse.column;
        let row = mouse.row;

        use crossterm::event::MouseButton;
        if mouse.kind != crossterm::event::MouseEventKind::Down(MouseButton::Left) {
            return Ok(None);
        }

        // Group field (create mode)
        if self.is_create() {
            if let Some(area) = self.last_group_field_area {
                if col >= area.x
                    && col < area.x + area.width
                    && row >= area.y
                    && row < area.y + area.height
                {
                    self.focus = FocusArea::Group;
                    if !self.group_suggestion.suggestions().is_empty() {
                        if self.group_suggestion.is_expanded() {
                            self.group_suggestion.collapse();
                        } else {
                            self.group_suggestion.expand();
                        }
                    }
                    return Ok(None);
                }
            }
            if let Some(area) = self.last_group_dropdown_area {
                if col >= area.x
                    && col < area.x + area.width
                    && row >= area.y
                    && row < area.y + area.height
                {
                    let idx = (row - area.y) as usize;
                    if idx < self.group_suggestion.suggestions().len() {
                        self.group_suggestion.select_and_confirm(idx);
                    }
                    return Ok(None);
                }
            }
            if let Some(area) = self.last_command_area {
                if col >= area.x
                    && col < area.x + area.width
                    && row >= area.y
                    && row < area.y + area.height
                {
                    self.focus = FocusArea::Command;
                    self.group_suggestion.collapse();
                    return Ok(None);
                }
            }
            if let Some(area) = self.last_checkbox_area {
                if col >= area.x
                    && col < area.x + area.width
                    && row >= area.y
                    && row < area.y + area.height
                {
                    self.focus = FocusArea::ProjectCheckbox;
                    self.is_project = !self.is_project;
                    return Ok(None);
                }
            }
        }

        // Display name
        if let Some(area) = self.last_display_name_area {
            if col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                self.focus = FocusArea::DisplayName;
                self.group_suggestion.collapse();
                return Ok(None);
            }
        }

        // Mode selector
        if let Some(area) = self.last_mode_area {
            if col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                self.focus = FocusArea::Mode;
                self.group_suggestion.collapse();
                let modes = [
                    CommandMode::Terminal,
                    CommandMode::Background,
                    CommandMode::Report,
                ];
                let mut x_offset = area.x + 1;
                for m in &modes {
                    let label = Self::mode_label(*m);
                    let w = label.len() as u16 + 2;
                    if col >= x_offset && col < x_offset + w {
                        self.command_mode = *m;
                        break;
                    }
                    x_offset += w + 1;
                }
                return Ok(None);
            }
        }

        // Hotkey
        if let Some(area) = self.last_hotkey_area {
            if col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                self.focus = FocusArea::Hotkey;
                return Ok(None);
            }
        }

        // Buttons
        if let Some(area) = self.last_buttons_area {
            if col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                self.focus = FocusArea::Buttons;
                let btn_count = self.button_count();

                // Calculate button positions
                let mut button_starts: Vec<(u16, u16)> = Vec::new();
                let mut bx = area.x;
                let total_width: u16 = (0..btn_count)
                    .map(|i| {
                        let w = self.button_label(i).len() as u16 + 4; // "[ label ]"
                        button_starts.push((bx, w));
                        bx += w + MODAL_BUTTON_SPACING;
                        w + if i < btn_count - 1 {
                            MODAL_BUTTON_SPACING
                        } else {
                            0
                        }
                    })
                    .sum();

                // Recalculate with centering
                let start_x = area.x + (area.width.saturating_sub(total_width)) / 2;
                button_starts.clear();
                let mut cx = start_x;
                for i in 0..btn_count {
                    let label = self.button_label(i);
                    let w = label.len() as u16 + 4;
                    button_starts.push((cx, w));
                    cx += w + MODAL_BUTTON_SPACING;
                }

                for (i, (sx, w)) in button_starts.iter().enumerate() {
                    if col >= *sx && col < sx + w {
                        self.selected_button = i;
                        if i == self.button_count() - 1 {
                            return Ok(Some(ModalResult::Cancelled));
                        }
                        if let Some(result) = self.try_confirm() {
                            return Ok(Some(result));
                        }
                        break;
                    }
                }
                return Ok(None);
            }
        }

        Ok(None)
    }

    fn handle_paste(&mut self, text: &str) -> bool {
        match self.focus {
            FocusArea::Group if self.is_create() => {
                self.group_suggestion.input_mut().paste(text);
                true
            }
            FocusArea::Command => {
                self.command_input.paste(text);
                true
            }
            FocusArea::DisplayName => {
                self.display_name_input.paste(text);
                true
            }
            FocusArea::Hotkey => {
                self.hotkey_input.paste(text);
                self.validate_hotkey();
                true
            }
            _ => false,
        }
    }
}
