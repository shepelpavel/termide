//! Keyboard handling for Git Stash Panel.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use termide_config::{is_go_end, is_go_home, is_move_down, is_move_up};
use termide_core::PanelEvent;

use crate::types::Section;
use crate::GitStashPanel;

impl GitStashPanel {
    /// Handle keyboard input, returning any panel events.
    pub(crate) fn handle_key_event(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        self.status_message = None;

        match self.current_section {
            Section::NewButton => self.handle_new_button_key(key),
            Section::List => self.handle_list_key(key),
        }
    }

    /// Handle keys when [New] button is focused.
    fn handle_new_button_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        if is_move_down(&key, self.vim_mode) {
            if !self.stash_entries.is_empty() {
                self.current_section = Section::List;
            }
            return vec![];
        }

        match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => {
                return self.action_new();
            }
            KeyCode::Esc => {
                return vec![PanelEvent::ClosePanel];
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.refresh();
            }
            _ => {}
        }
        vec![]
    }

    /// Handle keys when List section is focused.
    fn handle_list_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        let last = self.stash_entries.len().saturating_sub(1);

        // Vim-aware navigation
        if is_move_up(&key, self.vim_mode) {
            if self.cursor > 0 {
                self.cursor -= 1;
                self.ensure_cursor_visible();
            } else {
                // At top of list — go to [New] button
                self.current_section = Section::NewButton;
            }
            return vec![];
        }
        if is_move_down(&key, self.vim_mode) {
            if self.cursor < last {
                self.cursor += 1;
                self.ensure_cursor_visible();
            }
            return vec![];
        }
        if is_go_home(&key, self.vim_mode) {
            self.cursor = 0;
            self.scroll = 0;
            return vec![];
        }
        if is_go_end(&key, self.vim_mode) {
            self.cursor = last;
            self.ensure_cursor_visible();
            return vec![];
        }

        match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => {
                return self.action_show_context_menu();
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                return self.action_new();
            }
            KeyCode::Esc => {
                return vec![PanelEvent::ClosePanel];
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.refresh();
            }
            _ => {}
        }
        vec![]
    }

    /// Ensure cursor is within visible scroll window.
    pub(crate) fn ensure_cursor_visible(&mut self) {
        if self.visible_height == 0 {
            return;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        }
        if self.cursor >= self.scroll + self.visible_height {
            self.scroll = self.cursor - self.visible_height + 1;
        }
    }
}
