//! Code-action popup for LSP quick-fixes (e.g. "Import class").
//!
//! Lists the actions returned by `textDocument/codeAction` and lets the user
//! pick one; the chosen action's `WorkspaceEdit` is applied by the app layer.

use lsp_types::{CodeAction, CodeActionOrCommand, CodeActionResponse};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use termide_theme::Theme;
use termide_ui::path_utils::truncate_right;
use unicode_width::UnicodeWidthStr;

const MAX_VISIBLE_ITEMS: usize = 10;
const MAX_POPUP_WIDTH: u16 = 60;
const MIN_POPUP_WIDTH: u16 = 20;

/// Code-action popup state and rendering.
pub struct CodeActionPopup {
    /// Actions offered by the server (only applicable ones are kept).
    actions: Vec<CodeActionOrCommand>,
    selected: usize,
    scroll_offset: usize,
}

impl CodeActionPopup {
    /// Build a popup from a code-action response, keeping `CodeAction` items
    /// (with an inline `edit`, or one resolved lazily on accept). Plain
    /// `Command` items are dropped — they'd need `workspace/executeCommand`,
    /// which isn't supported. Returns `None` when nothing applicable remains.
    pub fn from_response(response: CodeActionResponse) -> Option<Self> {
        let actions: Vec<CodeActionOrCommand> = response
            .into_iter()
            .filter(|item| matches!(item, CodeActionOrCommand::CodeAction(_)))
            .collect();

        if actions.is_empty() {
            None
        } else {
            Some(Self {
                actions,
                selected: 0,
                scroll_offset: 0,
            })
        }
    }

    /// Title of an action, for display.
    fn title(item: &CodeActionOrCommand) -> &str {
        match item {
            CodeActionOrCommand::CodeAction(action) => &action.title,
            CodeActionOrCommand::Command(command) => &command.title,
        }
    }

    /// The selected `CodeAction` (its edit is applied directly when present, or
    /// resolved via `codeAction/resolve` on accept when deferred).
    pub fn selected_code_action(&self) -> Option<CodeAction> {
        match self.actions.get(self.selected)? {
            CodeActionOrCommand::CodeAction(action) => Some(action.clone()),
            CodeActionOrCommand::Command(_) => None,
        }
    }

    /// Select the next action (wraps).
    pub fn select_next(&mut self) {
        if self.actions.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.actions.len();
        self.sync_scroll();
    }

    /// Select the previous action (wraps).
    pub fn select_prev(&mut self) {
        if self.actions.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.actions.len() - 1
        } else {
            self.selected - 1
        };
        self.sync_scroll();
    }

    fn sync_scroll(&mut self) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + MAX_VISIBLE_ITEMS {
            self.scroll_offset = self.selected + 1 - MAX_VISIBLE_ITEMS;
        }
    }

    /// Render the popup near the cursor. Returns the rect used (for mouse hit
    /// testing / overlay bookkeeping).
    pub fn render(
        &self,
        buf: &mut Buffer,
        area: Rect,
        cursor_x: u16,
        cursor_y: u16,
        theme: &Theme,
    ) -> Option<Rect> {
        if self.actions.is_empty() {
            return None;
        }

        let max_title_width = self
            .actions
            .iter()
            .map(|a| Self::title(a).width() + 2)
            .max()
            .unwrap_or(MIN_POPUP_WIDTH as usize);
        let popup_width = (max_title_width as u16).clamp(MIN_POPUP_WIDTH, MAX_POPUP_WIDTH);

        let visible_count = self.actions.len().min(MAX_VISIBLE_ITEMS);
        let popup_height = visible_count as u16;

        // Prefer below the cursor; flip above when there isn't room.
        let popup_y = if cursor_y + 1 + popup_height <= area.bottom() {
            cursor_y + 1
        } else {
            cursor_y.saturating_sub(popup_height)
        };
        let popup_x = cursor_x.min(area.right().saturating_sub(popup_width));
        let popup_rect = Rect::new(popup_x, popup_y, popup_width, popup_height);

        let bg_style = Style::default().bg(theme.accented_bg).fg(theme.fg);
        for y in popup_rect.top()..popup_rect.bottom() {
            for x in popup_rect.left()..popup_rect.right() {
                if x >= buf.area.left()
                    && x < buf.area.right()
                    && y >= buf.area.top()
                    && y < buf.area.bottom()
                {
                    buf[(x, y)].set_style(bg_style);
                    buf[(x, y)].set_char(' ');
                }
            }
        }

        let selected_style = Style::default()
            .bg(theme.selected_bg)
            .fg(theme.selected_fg)
            .add_modifier(Modifier::BOLD);

        for (display_idx, action) in self
            .actions
            .iter()
            .skip(self.scroll_offset)
            .take(visible_count)
            .enumerate()
        {
            let y = popup_rect.top() + display_idx as u16;
            let is_selected = self.scroll_offset + display_idx == self.selected;
            let style = if is_selected {
                selected_style
            } else {
                bg_style
            };

            let label_start = popup_rect.left() + 1;
            let max_label_len = popup_rect.right().saturating_sub(label_start) as usize;
            let label = truncate_right(Self::title(action), max_label_len);

            let mut x = label_start;
            for ch in label.chars() {
                if x >= buf.area.right() || y >= buf.area.bottom() {
                    break;
                }
                buf[(x, y)].set_char(ch);
                buf[(x, y)].set_style(style);
                x += 1;
            }
            // Paint the selection highlight across the rest of the row.
            while x < popup_rect.right() {
                if x < buf.area.right() && y < buf.area.bottom() {
                    buf[(x, y)].set_style(style);
                }
                x += 1;
            }
        }

        Some(popup_rect)
    }
}

#[cfg(test)]
mod tests {
    use super::CodeActionPopup;
    use lsp_types::{CodeAction, CodeActionOrCommand, Command, WorkspaceEdit};

    fn action_with_edit(title: &str) -> CodeActionOrCommand {
        CodeActionOrCommand::CodeAction(CodeAction {
            title: title.to_string(),
            edit: Some(WorkspaceEdit::default()),
            ..Default::default()
        })
    }

    #[test]
    fn keeps_code_actions_drops_commands() {
        let response = vec![
            action_with_edit("Import App\\Order"),
            // Edit-less actions are kept too (resolved lazily on accept).
            CodeActionOrCommand::CodeAction(CodeAction {
                title: "needs resolve".into(),
                edit: None,
                ..Default::default()
            }),
            // Plain commands are dropped (executeCommand unsupported).
            CodeActionOrCommand::Command(Command {
                title: "Run command".into(),
                command: "phpactor.cmd".into(),
                arguments: None,
            }),
        ];
        let popup = CodeActionPopup::from_response(response).expect("has applicable actions");
        assert!(popup.selected_code_action().is_some());
    }

    #[test]
    fn command_only_or_empty_yields_no_popup() {
        let response = vec![CodeActionOrCommand::Command(Command {
            title: "Run".into(),
            command: "x".into(),
            arguments: None,
        })];
        assert!(CodeActionPopup::from_response(response).is_none());
        assert!(CodeActionPopup::from_response(vec![]).is_none());
    }

    #[test]
    fn selection_wraps() {
        let response = vec![action_with_edit("a"), action_with_edit("b")];
        let mut popup = CodeActionPopup::from_response(response).unwrap();
        assert_eq!(popup.selected_code_action().unwrap().title, "a");
        popup.select_next();
        assert_eq!(popup.selected_code_action().unwrap().title, "b");
        popup.select_next(); // wraps
        assert_eq!(popup.selected_code_action().unwrap().title, "a");
        popup.select_prev(); // wraps to last
        assert_eq!(popup.selected_code_action().unwrap().title, "b");
    }
}
