//! Git Stash Panel for termide.
//!
//! Provides a dedicated panel for managing git stash entries:
//! listing, creating, popping, applying, dropping, and viewing diffs.

mod keyboard;
mod rendering;
pub mod types;

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect};

use termide_config::Config;
use termide_core::{
    CommandResult, Panel, PanelCommand, PanelEvent, RenderContext, SessionPanel, ThemeColors,
    WidthPreference,
};
use termide_git::{self as git, StashEntry};
use termide_modal::{ActiveModal, SelectModal};
use termide_state::PendingAction;
use termide_theme::Theme;

use types::Section;

/// Git Stash Panel — standalone panel for managing stash entries.
pub struct GitStashPanel {
    /// Repository path
    repo_path: PathBuf,
    /// Cached stash entries
    pub(crate) stash_entries: Vec<StashEntry>,
    /// Cursor position in stash list
    pub(crate) cursor: usize,
    /// Scroll offset in stash list
    pub(crate) scroll: usize,
    /// Currently focused section
    pub(crate) current_section: Section,
    /// Visible list height (set during render)
    pub(crate) visible_height: usize,
    /// Cached theme colors
    pub(crate) cached_theme: ThemeColors,
    /// Last render area
    last_area: Rect,
    /// [New] button area for mouse click detection
    pub(crate) new_btn_area: Option<Rect>,
    /// Status message
    pub(crate) status_message: Option<String>,
    /// Vim mode flag
    pub(crate) vim_mode: bool,
    /// Modal request (for InputModal / SelectModal / ConfirmModal)
    modal_request: Option<(PendingAction, ActiveModal)>,
}

impl GitStashPanel {
    /// Create a new Git Stash panel for a repository.
    pub fn new(repo_path: PathBuf) -> Self {
        let stash_entries = git::stash_list(&repo_path);
        Self {
            repo_path,
            stash_entries,
            cursor: 0,
            scroll: 0,
            current_section: Section::NewButton,
            visible_height: 0,
            cached_theme: ThemeColors::default(),
            last_area: Rect::default(),
            new_btn_area: None,
            status_message: None,
            vim_mode: false,
            modal_request: None,
        }
    }

    /// Refresh stash entries from git.
    pub(crate) fn refresh(&mut self) {
        self.stash_entries = git::stash_list(&self.repo_path);
        // Clamp cursor
        if !self.stash_entries.is_empty() {
            let last = self.stash_entries.len() - 1;
            if self.cursor > last {
                self.cursor = last;
            }
        } else {
            self.cursor = 0;
            self.scroll = 0;
        }
    }

    /// Take the pending modal request (called by PanelExt).
    pub fn take_modal_request(&mut self) -> Option<(PendingAction, ActiveModal)> {
        self.modal_request.take()
    }

    // === Stash operations ===

    /// Create a new stash (opens InputModal).
    pub(crate) fn action_new(&mut self) -> Vec<PanelEvent> {
        let modal = termide_modal::InputModal::new("Stash message", "");
        self.modal_request = Some((
            PendingAction::GitStashPush {
                repo_path: self.repo_path.clone(),
            },
            ActiveModal::Input(Box::new(modal)),
        ));
        vec![]
    }

    /// Show context menu (SelectModal) for the selected stash entry.
    pub(crate) fn action_show_context_menu(&mut self) -> Vec<PanelEvent> {
        let Some(entry) = self.stash_entries.get(self.cursor) else {
            return vec![];
        };

        let modal = SelectModal::single(
            &entry.ref_str,
            &entry.message,
            vec![
                "Pop".to_string(),
                "Apply".to_string(),
                "Drop".to_string(),
                "Diff".to_string(),
            ],
        );
        self.modal_request = Some((
            PendingAction::GitStashAction {
                repo_path: self.repo_path.clone(),
                index: entry.index,
                ref_str: entry.ref_str.clone(),
            },
            ActiveModal::Select(Box::new(modal)),
        ));
        vec![]
    }
}

impl Panel for GitStashPanel {
    fn name(&self) -> &'static str {
        "git_stash"
    }

    fn width_preference(&self) -> WidthPreference {
        WidthPreference::PreferNarrow
    }

    fn title(&self) -> String {
        let repo_name = git::get_repo_name(&self.repo_path);
        format!("Stash ({}) — {}", self.stash_entries.len(), repo_name)
    }

    fn prepare_render(&mut self, theme: &Theme, config: Arc<Config>) {
        self.cached_theme = ThemeColors::from(theme);
        self.vim_mode = config.general.vim_mode;
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
        self.last_area = area;
        self.render_content(area, buf, ctx.is_focused);
    }

    fn handle_action(&mut self, hotkey: termide_core::Hotkey) -> Vec<PanelEvent> {
        use termide_core::HotkeyKind;
        match hotkey.kind {
            HotkeyKind::Cancel => {
                self.status_message = None;
                vec![PanelEvent::ClosePanel]
            }
            HotkeyKind::Refresh => {
                self.status_message = None;
                self.refresh();
                vec![]
            }
            HotkeyKind::Enter | HotkeyKind::Space => {
                self.status_message = None;
                match self.current_section {
                    Section::NewButton => self.action_new(),
                    Section::List => self.action_show_context_menu(),
                }
            }
            HotkeyKind::Down => {
                self.status_message = None;
                match self.current_section {
                    Section::NewButton => {
                        if !self.stash_entries.is_empty() {
                            self.current_section = Section::List;
                        }
                    }
                    Section::List => {
                        let last = self.stash_entries.len().saturating_sub(1);
                        if self.cursor < last {
                            self.cursor += 1;
                            self.ensure_cursor_visible();
                        }
                    }
                }
                vec![]
            }
            HotkeyKind::Up => {
                self.status_message = None;
                if let Section::List = self.current_section {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                        self.ensure_cursor_visible();
                    } else {
                        self.current_section = Section::NewButton;
                    }
                }
                vec![]
            }
            HotkeyKind::Home => {
                self.status_message = None;
                if let Section::List = self.current_section {
                    self.cursor = 0;
                    self.scroll = 0;
                }
                vec![]
            }
            HotkeyKind::End => {
                self.status_message = None;
                if let Section::List = self.current_section {
                    self.cursor = self.stash_entries.len().saturating_sub(1);
                    self.ensure_cursor_visible();
                }
                vec![]
            }
            HotkeyKind::Other => self.handle_key_event(hotkey.raw),
            _ => vec![],
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        self.handle_key_event(key)
    }

    fn handle_mouse(&mut self, event: MouseEvent, _panel_area: Rect) -> Vec<PanelEvent> {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let area = self.last_area;
                if area.width == 0 || area.height < 4 {
                    return vec![];
                }

                // Check click on [New] button
                if let Some(btn) = self.new_btn_area {
                    if event.row == btn.y
                        && event.column >= btn.x
                        && event.column < btn.x + btn.width
                    {
                        self.current_section = Section::NewButton;
                        return self.action_new();
                    }
                }

                // List starts at y+2 (header + separator)
                let list_start_y = area.y + 2;

                if event.row >= list_start_y {
                    let clicked_row = (event.row - list_start_y) as usize;
                    let entry_idx = self.scroll + clicked_row;
                    if entry_idx < self.stash_entries.len() {
                        self.current_section = Section::List;
                        self.cursor = entry_idx;
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                if self.scroll > 0 {
                    self.scroll = self.scroll.saturating_sub(3);
                }
            }
            MouseEventKind::ScrollDown => {
                let max_scroll = self.stash_entries.len().saturating_sub(self.visible_height);
                if self.scroll < max_scroll {
                    self.scroll = (self.scroll + 3).min(max_scroll);
                }
            }
            _ => {}
        }
        vec![]
    }

    fn handle_scroll(&mut self, delta: i32, _panel_area: Rect) -> Vec<PanelEvent> {
        let lines = delta.unsigned_abs() as usize * 3;
        if delta < 0 {
            self.scroll = self.scroll.saturating_sub(lines);
        } else {
            let max_scroll = self.stash_entries.len().saturating_sub(self.visible_height);
            self.scroll = (self.scroll + lines).min(max_scroll);
        }
        vec![]
    }

    fn handle_command(&mut self, cmd: PanelCommand<'_>) -> CommandResult {
        match cmd {
            PanelCommand::OnGitUpdate { repo_paths } => {
                let should_refresh = repo_paths
                    .iter()
                    .any(|p| self.repo_path.starts_with(p) || p.starts_with(&self.repo_path));
                if should_refresh {
                    self.refresh();
                    return CommandResult::NeedsRedraw(true);
                }
                CommandResult::NeedsRedraw(false)
            }
            PanelCommand::Reload => {
                self.refresh();
                CommandResult::NeedsRedraw(true)
            }
            _ => CommandResult::None,
        }
    }

    fn to_session(&self, _session_dir: &Path) -> Option<SessionPanel> {
        Some(SessionPanel::GitStash {
            repo_path: self.repo_path.clone(),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn get_working_directory(&self) -> Option<PathBuf> {
        Some(self.repo_path.clone())
    }
}
