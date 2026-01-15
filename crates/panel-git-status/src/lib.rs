//! Git Status Panel for termide.
//!
//! Provides a panel for managing git operations: staging, unstaging, commits, push/pull.

#![allow(clippy::too_many_arguments)]

use std::any::Any;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use unicode_width::UnicodeWidthStr;

use termide_config::{
    matches_binding_or_default, matches_binding_or_defaults, Config, GitStatusKeybindings,
};
use termide_core::{
    CommandResult, GitOperationType, Panel, PanelCommand, PanelEvent, RenderContext, SessionPanel,
    ThemeColors,
};
use termide_git::{self as git, truncate_to_width, RepoManager, StagedFile, UnstagedFile};
use termide_modal::{ActionButton, ActiveModal, InfoActionModal};
use termide_state::PendingAction;
use termide_theme::Theme;
use termide_ui::{IndexClickTracker, ScrollBar};
use termide_ui_render::InlineSelector;

/// Section of the Git Status panel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Repository selector
    RepoSelector,
    /// Branch selector
    BranchSelector,
    /// Files list (both unstaged and staged)
    Files,
    /// Action buttons
    Buttons,
}

/// Current selection in the files area
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// Cursor on Unstaged header (selecting [Stage all] button)
    UnstagedHeader,
    /// Cursor on an unstaged file at given index
    UnstagedFile(usize),
    /// Cursor on Staged header (selecting [Unstage all] button)
    StagedHeader,
    /// Cursor on a staged file at given index
    StagedFile(usize),
}

/// Button in the Git Status panel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Commit,
    Pull,
    Push,
    /// Push operation in progress (shows spinner, click cancels)
    Pushing,
    /// Pull operation in progress (shows spinner, click cancels)
    Pulling,
}

/// Spinner animation frames
const SPINNER_FRAMES: [char; 6] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴'];

impl Button {
    fn label(&self, spinner_frame: usize) -> String {
        match self {
            Button::Commit => "Commit".to_string(),
            Button::Pull => "Pull".to_string(),
            Button::Push => "Push".to_string(),
            Button::Pushing => {
                let s = SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()];
                format!("{} Pushing...", s)
            }
            Button::Pulling => {
                let s = SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()];
                format!("{} Pulling...", s)
            }
        }
    }
}

/// Git Status Panel
pub struct GitStatusPanel {
    /// Repository manager
    repo_manager: RepoManager,
    /// Current branch name
    branch: Option<String>,
    /// Available branches for current repo
    branches: Vec<String>,
    /// Ahead/behind counts
    ahead: usize,
    behind: usize,
    /// Unstaged files (modified + untracked)
    unstaged_files: Vec<UnstagedFile>,
    /// Staged files
    staged_files: Vec<StagedFile>,
    /// Current section
    current_section: Section,
    /// Cursor position as virtual line (0..total_virtual_lines, includes headers)
    cursor: usize,
    /// Selected button index
    selected_button: usize,
    /// Unified scroll offset for files area
    scroll_offset: usize,
    /// Cached viewport height for scroll calculations
    viewport_height: usize,
    /// Cached theme colors for rendering
    cached_theme: ThemeColors,
    /// Cached keybindings for keyboard handling
    keybindings: GitStatusKeybindings,
    /// Last render area (for mouse handling)
    last_area: Rect,
    /// Status message
    status_message: Option<String>,
    /// Is repo dropdown expanded
    repo_dropdown_open: bool,
    /// Is branch dropdown expanded
    branch_dropdown_open: bool,
    /// Cursor position in open dropdown
    dropdown_cursor: usize,
    // Layout zones for mouse handling
    /// Selector row Y position
    selector_y: u16,
    /// Branch selector X position (for mouse click detection)
    branch_selector_x: u16,
    /// Files area (combined unstaged + staged)
    files_area: Rect,
    /// Buttons row Y position
    buttons_y: u16,
    /// Repo dropdown area (for mouse click detection)
    repo_dropdown_area: Option<Rect>,
    /// Branch dropdown area (for mouse click detection)
    branch_dropdown_area: Option<Rect>,
    /// Scroll offset in dropdown
    dropdown_scroll: usize,
    /// Stage all button area (for mouse click detection)
    stage_all_btn_area: Option<Rect>,
    /// Unstage all button area (for mouse click detection)
    unstage_all_btn_area: Option<Rect>,
    /// Click tracker for double-click detection in files area
    click_tracker: IndexClickTracker,
    /// Modal request (for file properties)
    modal_request: Option<(termide_state::PendingAction, termide_modal::ActiveModal)>,
    /// Loading indicator flag
    is_loading: bool,
    /// Whether git operation (push/pull) is in progress
    git_operation_in_progress: bool,
    /// Current git operation name ("push" or "pull")
    current_operation: Option<String>,
    /// Spinner animation frame for Pushing/Pulling buttons
    spinner_frame: usize,
}

impl GitStatusPanel {
    /// Create a new Git Status panel from a list of paths (from panels/session)
    pub fn new(paths: &[PathBuf]) -> Self {
        let mut panel = Self {
            repo_manager: RepoManager::new(paths),
            branch: None,
            branches: Vec::new(),
            ahead: 0,
            behind: 0,
            unstaged_files: Vec::new(),
            staged_files: Vec::new(),
            current_section: Section::RepoSelector,
            cursor: 0,
            selected_button: 0,
            scroll_offset: 0,
            viewport_height: 0,
            cached_theme: ThemeColors::default(),
            keybindings: GitStatusKeybindings::default(),
            last_area: Rect::default(),
            status_message: None,
            repo_dropdown_open: false,
            branch_dropdown_open: false,
            dropdown_cursor: 0,
            selector_y: 0,
            branch_selector_x: 0,
            files_area: Rect::default(),
            buttons_y: 0,
            repo_dropdown_area: None,
            branch_dropdown_area: None,
            dropdown_scroll: 0,
            stage_all_btn_area: None,
            unstage_all_btn_area: None,
            click_tracker: IndexClickTracker::new(),
            modal_request: None,
            is_loading: false,
            git_operation_in_progress: false,
            current_operation: None,
            spinner_frame: 0,
        };

        panel.refresh();
        panel
    }

    /// Create panel for a specific repository
    pub fn new_for_repo(repo_path: PathBuf) -> Self {
        let mut panel = Self {
            repo_manager: RepoManager::for_repo(repo_path),
            branch: None,
            branches: Vec::new(),
            ahead: 0,
            behind: 0,
            unstaged_files: Vec::new(),
            staged_files: Vec::new(),
            current_section: Section::RepoSelector,
            cursor: 0,
            selected_button: 0,
            scroll_offset: 0,
            viewport_height: 0,
            cached_theme: ThemeColors::default(),
            keybindings: GitStatusKeybindings::default(),
            last_area: Rect::default(),
            status_message: None,
            repo_dropdown_open: false,
            branch_dropdown_open: false,
            dropdown_cursor: 0,
            selector_y: 0,
            branch_selector_x: 0,
            files_area: Rect::default(),
            buttons_y: 0,
            repo_dropdown_area: None,
            branch_dropdown_area: None,
            dropdown_scroll: 0,
            stage_all_btn_area: None,
            unstage_all_btn_area: None,
            click_tracker: IndexClickTracker::new(),
            modal_request: None,
            is_loading: false,
            git_operation_in_progress: false,
            current_operation: None,
            spinner_frame: 0,
        };

        panel.refresh();
        panel
    }

    /// Update repository list based on new paths from panels
    pub fn update_repos(&mut self, paths: &[PathBuf]) {
        if self.repo_manager.update(paths) {
            self.refresh();
        }
    }

    /// Refresh git status
    pub fn refresh(&mut self) {
        self.is_loading = true;

        let repo = match self.repo_manager.current() {
            Some(r) => r.to_path_buf(),
            None => {
                self.is_loading = false;
                return;
            }
        };

        self.branch = git::get_current_branch(&repo);
        self.branches = git::get_branches(&repo);
        let (ahead, behind) = git::get_ahead_behind(&repo);
        self.ahead = ahead;
        self.behind = behind;
        self.unstaged_files = git::get_unstaged_files(&repo);
        self.staged_files = git::get_staged_files(&repo);

        // Sort by path
        self.unstaged_files.sort_by(|a, b| a.path.cmp(&b.path));
        self.staged_files.sort_by(|a, b| a.path.cmp(&b.path));

        // Adjust cursor to stay within bounds (cursor is virtual line)
        let max_cursor = self.total_virtual_lines().saturating_sub(1);
        if self.cursor > max_cursor {
            self.cursor = max_cursor;
        }
        // Ensure cursor is on a selectable line
        while self.cursor > 0 && !self.is_selectable_line(self.cursor) {
            self.cursor -= 1;
        }

        self.is_loading = false;
    }

    /// Move to next section
    fn next_section(&mut self) {
        self.current_section = match self.current_section {
            Section::RepoSelector => Section::BranchSelector,
            Section::BranchSelector => {
                let total_files = self.unstaged_files.len() + self.staged_files.len();
                if total_files > 0 {
                    Section::Files
                } else {
                    Section::Buttons
                }
            }
            Section::Files => Section::Buttons,
            Section::Buttons => Section::RepoSelector,
        };
    }

    /// Move to previous section
    fn prev_section(&mut self) {
        self.current_section = match self.current_section {
            Section::RepoSelector => Section::Buttons,
            Section::BranchSelector => Section::RepoSelector,
            Section::Files => Section::BranchSelector,
            Section::Buttons => {
                let total_files = self.unstaged_files.len() + self.staged_files.len();
                if total_files > 0 {
                    Section::Files
                } else {
                    Section::BranchSelector
                }
            }
        };
    }

    /// Get current selection based on cursor position (virtual line)
    fn get_selection(&self) -> Option<Selection> {
        let unstaged_header = 0;
        let unstaged_start = 1;
        let unstaged_end = unstaged_start + self.unstaged_files.len();
        let staged_header = unstaged_end;
        let staged_start = staged_header + 1;

        if self.cursor == unstaged_header && !self.unstaged_files.is_empty() {
            Some(Selection::UnstagedHeader)
        } else if self.cursor >= unstaged_start && self.cursor < unstaged_end {
            Some(Selection::UnstagedFile(self.cursor - unstaged_start))
        } else if self.cursor == staged_header && !self.staged_files.is_empty() {
            Some(Selection::StagedHeader)
        } else if self.cursor >= staged_start {
            let idx = self.cursor - staged_start;
            if idx < self.staged_files.len() {
                Some(Selection::StagedFile(idx))
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Check if a virtual line is selectable (files and headers with buttons)
    fn is_selectable_line(&self, vline: usize) -> bool {
        let unstaged_header = 0;
        let unstaged_end = 1 + self.unstaged_files.len();
        let staged_header = unstaged_end;
        let staged_end = staged_header + 1 + self.staged_files.len();

        if vline == unstaged_header {
            !self.unstaged_files.is_empty() // Has [Stage all] button
        } else if vline == staged_header {
            !self.staged_files.is_empty() // Has [Unstage all] button
        } else {
            vline > unstaged_header && vline < staged_end && vline != staged_header
        }
    }

    /// Check if there are any files (unstaged or staged)
    fn has_any_files(&self) -> bool {
        !self.unstaged_files.is_empty() || !self.staged_files.is_empty()
    }

    /// Get first selectable virtual line
    fn first_selectable_line(&self) -> usize {
        if !self.unstaged_files.is_empty() {
            0 // Unstaged header
        } else if !self.staged_files.is_empty() {
            1 // Staged header (vline = 1 when no unstaged files)
        } else {
            0
        }
    }

    /// Get last selectable virtual line
    fn last_selectable_line(&self) -> usize {
        let total = self.total_virtual_lines();
        for vline in (0..total).rev() {
            if self.is_selectable_line(vline) {
                return vline;
            }
        }
        0
    }

    /// Cursor position is the virtual line directly
    fn cursor_to_virtual_line(&self) -> usize {
        self.cursor
    }

    /// Ensure cursor is visible in viewport
    fn ensure_cursor_visible(&mut self) {
        if self.viewport_height == 0 {
            return;
        }
        let cursor_line = self.cursor_to_virtual_line();
        if cursor_line < self.scroll_offset {
            self.scroll_offset = cursor_line;
        } else if cursor_line >= self.scroll_offset + self.viewport_height {
            self.scroll_offset = cursor_line - self.viewport_height + 1;
        }
    }

    /// Get total virtual lines count (headers + files)
    fn total_virtual_lines(&self) -> usize {
        2 + self.unstaged_files.len() + self.staged_files.len()
    }

    /// Get file at cursor from unstaged section (for backward compatibility)
    fn get_selected_unstaged(&self) -> Vec<PathBuf> {
        match self.get_selection() {
            Some(Selection::UnstagedFile(idx)) => self
                .unstaged_files
                .get(idx)
                .map(|f| f.path.clone())
                .into_iter()
                .collect(),
            _ => vec![],
        }
    }

    /// Get file at cursor from staged section (for backward compatibility)
    fn get_selected_staged(&self) -> Vec<PathBuf> {
        match self.get_selection() {
            Some(Selection::StagedFile(idx)) => self
                .staged_files
                .get(idx)
                .map(|f| f.path.clone())
                .into_iter()
                .collect(),
            _ => vec![],
        }
    }

    /// Execute a git file operation with common error handling
    fn execute_git_op<F>(&mut self, files: Vec<PathBuf>, op: F, action: &str)
    where
        F: FnOnce(&Path, &[PathBuf]) -> Result<(), String>,
    {
        if files.is_empty() {
            return;
        }
        if let Some(repo) = self.repo_manager.current() {
            match op(repo, &files) {
                Ok(()) => {
                    self.status_message = Some(format!("{} {} file(s)", action, files.len()));
                    self.refresh();
                }
                Err(e) => {
                    self.status_message = Some(format!("{} error: {}", action, e));
                }
            }
        }
    }

    /// Execute stage action
    fn do_stage(&mut self) {
        let files = self.get_selected_unstaged();
        self.execute_git_op(files, git::stage_files, "Staged");
    }

    /// Execute unstage action
    fn do_unstage(&mut self) {
        let files = self.get_selected_staged();
        self.execute_git_op(files, git::unstage_files, "Unstaged");
    }

    /// Stage all unstaged files
    fn do_stage_all(&mut self) {
        let files: Vec<PathBuf> = self.unstaged_files.iter().map(|f| f.path.clone()).collect();
        self.execute_git_op(files, git::stage_files, "Staged");
    }

    /// Unstage all staged files
    fn do_unstage_all(&mut self) {
        let files: Vec<PathBuf> = self.staged_files.iter().map(|f| f.path.clone()).collect();
        self.execute_git_op(files, git::unstage_files, "Unstaged");
    }

    /// Show file properties modal with Diff/Revert actions
    fn show_file_properties(&mut self) -> Vec<PanelEvent> {
        let t = termide_i18n::t();

        let (is_staged, idx) = match self.get_selection() {
            Some(Selection::UnstagedFile(idx)) => (false, idx),
            Some(Selection::StagedFile(idx)) => (true, idx),
            _ => return vec![], // Headers or nothing selected
        };

        let (file_path, status_str) = if is_staged {
            if let Some(file) = self.staged_files.get(idx) {
                let path = PathBuf::from(&file.path);
                let status = match file.status {
                    'A' => t.git_status_added(),
                    'M' => t.git_status_modified(),
                    'D' => t.git_status_deleted(),
                    'R' => t.git_status_renamed(),
                    c => c.to_string(),
                };
                (path, status)
            } else {
                return vec![];
            }
        } else if let Some(file) = self.unstaged_files.get(idx) {
            let path = PathBuf::from(&file.path);
            let status = if file.untracked {
                t.git_status_untracked()
            } else {
                match file.status {
                    'M' => t.git_status_modified(),
                    'D' => t.git_status_deleted(),
                    c => c.to_string(),
                }
            };
            (path, status)
        } else {
            return vec![];
        };

        let Some(repo_path) = self.repo_manager.current().map(|p| p.to_path_buf()) else {
            return vec![];
        };

        // Build data for modal
        let data = vec![
            (
                t.git_props_path().to_string(),
                file_path.display().to_string(),
            ),
            (t.git_props_status().to_string(), status_str),
        ];

        // Build action buttons (Revert shown for all files - staged files will be unstaged first)
        let buttons = vec![
            ActionButton::new(t.git_action_diff(), "diff"),
            ActionButton::new(t.git_action_revert(), "revert"),
            ActionButton::new(t.git_action_close(), "close"),
        ];

        // Select Close button by default
        let selected_button = buttons.len().saturating_sub(1);

        let modal_title = t.git_file_properties_title().to_string();
        let modal =
            InfoActionModal::new(modal_title, data, buttons).with_selected_button(selected_button);

        // Store modal request
        self.modal_request = Some((
            PendingAction::GitFileAction {
                file_path,
                repo_path,
                is_staged,
            },
            ActiveModal::InfoAction(Box::new(modal)),
        ));

        vec![]
    }

    /// Switch to a different branch
    fn switch_to_branch(&mut self, branch_idx: usize) {
        if let Some(branch_name) = self.branches.get(branch_idx) {
            if let Some(repo) = self.repo_manager.current() {
                let branch_name = branch_name.clone();
                match git::checkout_branch(repo, &branch_name) {
                    Ok(()) => {
                        self.status_message = Some(format!("Switched to {}", branch_name));
                        self.refresh();
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Checkout error: {}", e));
                    }
                }
            }
        }
    }

    // =========================================================================
    // Keyboard Navigation Helpers
    // =========================================================================

    /// Handle Up key navigation
    fn handle_up_key(&mut self) {
        match self.current_section {
            Section::RepoSelector => {
                if self.repo_dropdown_open && self.dropdown_cursor > 0 {
                    self.dropdown_cursor -= 1;
                }
            }
            Section::BranchSelector => {
                if self.branch_dropdown_open && self.dropdown_cursor > 0 {
                    self.dropdown_cursor -= 1;
                }
            }
            Section::Files => {
                let first = self.first_selectable_line();
                if self.cursor == first {
                    self.current_section = Section::BranchSelector;
                } else {
                    let mut new_cursor = self.cursor;
                    while new_cursor > 0 {
                        new_cursor -= 1;
                        if self.is_selectable_line(new_cursor) {
                            self.cursor = new_cursor;
                            self.ensure_cursor_visible();
                            break;
                        }
                    }
                }
            }
            Section::Buttons => {
                if self.has_any_files() {
                    self.current_section = Section::Files;
                    self.cursor = self.last_selectable_line();
                    self.ensure_cursor_visible();
                } else {
                    self.current_section = Section::BranchSelector;
                }
            }
        }
    }

    /// Handle Down key navigation
    fn handle_down_key(&mut self) {
        match self.current_section {
            Section::RepoSelector => {
                if self.repo_dropdown_open {
                    if self.dropdown_cursor + 1 < self.repo_manager.len() {
                        self.dropdown_cursor += 1;
                    }
                } else if self.has_any_files() {
                    self.current_section = Section::Files;
                    self.cursor = self.first_selectable_line();
                    self.ensure_cursor_visible();
                } else {
                    self.current_section = Section::Buttons;
                }
            }
            Section::BranchSelector => {
                if self.branch_dropdown_open {
                    if self.dropdown_cursor + 1 < self.branches.len() {
                        self.dropdown_cursor += 1;
                    }
                } else if self.has_any_files() {
                    self.current_section = Section::Files;
                    self.cursor = self.first_selectable_line();
                    self.ensure_cursor_visible();
                } else {
                    self.current_section = Section::Buttons;
                }
            }
            Section::Files => {
                let last = self.last_selectable_line();
                if self.cursor == last {
                    self.current_section = Section::Buttons;
                    let total = self.total_virtual_lines();
                    if total > self.viewport_height {
                        self.scroll_offset = total - self.viewport_height;
                    }
                } else {
                    let max = self.total_virtual_lines();
                    let mut new_cursor = self.cursor;
                    while new_cursor + 1 < max {
                        new_cursor += 1;
                        if self.is_selectable_line(new_cursor) {
                            self.cursor = new_cursor;
                            self.ensure_cursor_visible();
                            break;
                        }
                    }
                }
            }
            Section::Buttons => {
                // At bottom, do nothing
            }
        }
    }

    /// Handle Enter key
    fn handle_enter_key(&mut self) -> Vec<PanelEvent> {
        match self.current_section {
            Section::Files => {
                match self.get_selection() {
                    Some(Selection::UnstagedHeader) => self.do_stage_all(),
                    Some(Selection::StagedHeader) => self.do_unstage_all(),
                    Some(Selection::UnstagedFile(_)) => self.do_stage(),
                    Some(Selection::StagedFile(_)) => self.do_unstage(),
                    None => {}
                }
                vec![]
            }
            Section::RepoSelector => {
                if self.repo_dropdown_open {
                    if self.dropdown_cursor != self.repo_manager.selected_index() {
                        self.repo_manager.select(self.dropdown_cursor);
                        self.refresh();
                    }
                    self.repo_dropdown_open = false;
                } else {
                    self.repo_dropdown_open = true;
                    self.dropdown_cursor = self.repo_manager.selected_index();
                }
                vec![]
            }
            Section::BranchSelector => {
                if self.branch_dropdown_open {
                    self.switch_to_branch(self.dropdown_cursor);
                    self.branch_dropdown_open = false;
                } else {
                    self.branch_dropdown_open = true;
                    self.dropdown_cursor = self
                        .branches
                        .iter()
                        .position(|b| Some(b.as_str()) == self.branch.as_deref())
                        .unwrap_or(0);
                }
                vec![]
            }
            Section::Buttons => self.execute_button(),
        }
    }

    /// Check if current click is a double-click on the same item
    fn check_double_click(&self, now: std::time::Instant, vline: usize) -> bool {
        self.click_tracker.is_double_click_at(now, &vline)
    }

    /// Reset double-click tracking state
    fn reset_click_state(&mut self) {
        self.click_tracker.reset();
    }

    /// Record click for double-click detection
    fn record_click(&mut self, now: std::time::Instant, vline: usize) {
        self.click_tracker.record_at(now, vline);
    }

    /// Get list of buttons that should be visible based on current state
    fn get_visible_buttons(&self) -> Vec<Button> {
        let mut buttons = Vec::new();

        // Commit - only if there are staged files
        if !self.staged_files.is_empty() {
            buttons.push(Button::Commit);
        }

        // Show spinner button if operation in progress
        if self.git_operation_in_progress {
            match self.current_operation.as_deref() {
                Some("push") => buttons.push(Button::Pushing),
                Some("pull") => buttons.push(Button::Pulling),
                _ => {} // Unknown operation, don't show button
            }
        } else {
            // Push - only if ahead > 0
            if self.ahead > 0 {
                buttons.push(Button::Push);
            }

            // Pull - only if behind > 0
            if self.behind > 0 {
                buttons.push(Button::Pull);
            }
        }

        buttons
    }

    /// Execute button action
    fn execute_button(&mut self) -> Vec<PanelEvent> {
        let buttons = self.get_visible_buttons();
        if self.selected_button >= buttons.len() {
            return vec![];
        }
        let button = buttons[self.selected_button];
        match button {
            Button::Commit => {
                if let Some(repo) = self.repo_manager.current() {
                    let staged_count = self.staged_files.len();
                    let repo_name = git::get_repo_name(repo);
                    let branch_name = self
                        .branch
                        .clone()
                        .unwrap_or_else(|| "(detached)".to_string());
                    let modal =
                        termide_modal::CommitModal::new(staged_count, repo_name, branch_name);
                    self.modal_request = Some((
                        termide_state::PendingAction::GitCommit {
                            repo_path: repo.to_path_buf(),
                        },
                        termide_modal::ActiveModal::Commit(Box::new(modal)),
                    ));
                }
                vec![]
            }
            Button::Pull => {
                if let Some(repo) = self.repo_manager.current() {
                    vec![PanelEvent::GitOperation {
                        operation: GitOperationType::Pull,
                        repo_path: repo.to_path_buf(),
                    }]
                } else {
                    vec![]
                }
            }
            Button::Push => {
                if let Some(repo) = self.repo_manager.current() {
                    vec![PanelEvent::GitOperation {
                        operation: GitOperationType::Push,
                        repo_path: repo.to_path_buf(),
                    }]
                } else {
                    vec![]
                }
            }
            Button::Pushing | Button::Pulling => {
                // Click on spinner button cancels the operation
                vec![PanelEvent::CancelGitOperation]
            }
        }
    }

    /// Render the panel content with unified scroll layout:
    /// - Top: Repo selector + Branch selector (sticky)
    /// - Middle: Files area (unified scroll: unstaged header, unstaged files, staged header, staged files)
    /// - Bottom: Action buttons (sticky)
    fn render_content(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        is_focused: bool,
        border_right_x: Option<u16>,
    ) {
        if area.height < 5 {
            return;
        }

        let theme = self.cached_theme.clone();
        let content_area = area;

        // Layout constants
        let selector_height: u16 = 1;
        let separator_height: u16 = 1;
        let buttons_height: u16 = 1;
        let fixed_height = selector_height + separator_height + buttons_height;
        let files_area_height = content_area.height.saturating_sub(fixed_height) as usize;

        // Cache viewport height for scroll calculations
        self.viewport_height = files_area_height;

        // Virtual content layout
        let unstaged_header_line = 0;
        let unstaged_files_start = 1;
        let unstaged_files_end = unstaged_files_start + self.unstaged_files.len();
        let staged_header_line = unstaged_files_end;
        let staged_files_start = staged_header_line + 1;
        let total_virtual_lines = self.total_virtual_lines();

        // Clamp scroll offset
        let max_scroll = total_virtual_lines.saturating_sub(files_area_height);
        if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }

        let mut y = content_area.y;

        // === TOP ZONE: Selectors ===
        self.selector_y = y;

        let repo_name = self
            .repo_manager
            .current()
            .map(git::get_repo_name)
            .unwrap_or_else(|| "No repo".to_string());
        let repo_focused = self.current_section == Section::RepoSelector && is_focused;
        let repo_selector =
            InlineSelector::new(&repo_name, self.repo_dropdown_open, repo_focused, &theme);
        let repo_width = repo_selector.render(content_area.x, y, content_area.width / 2, buf);

        let branch_name = self
            .branch
            .clone()
            .unwrap_or_else(|| "(detached)".to_string());
        let branch_focused = self.current_section == Section::BranchSelector && is_focused;
        let branch_x = content_area.x + repo_width + 2;
        self.branch_selector_x = branch_x;
        let branch_max_width = content_area.width.saturating_sub(repo_width + 2);
        let branch_selector = InlineSelector::new(
            &branch_name,
            self.branch_dropdown_open,
            branch_focused,
            &theme,
        );
        branch_selector.render(branch_x, y, branch_max_width, buf);

        y += selector_height;

        // === MIDDLE ZONE: Files area (unified scroll) ===
        let files_y = y;
        let files_width = content_area.width;

        // Store files area for mouse handling
        self.files_area = Rect {
            x: content_area.x,
            y: files_y,
            width: files_width,
            height: files_area_height as u16,
        };

        let files_active = self.current_section == Section::Files && is_focused;

        // Render visible virtual lines
        for screen_row in 0..files_area_height {
            let vline = self.scroll_offset + screen_row;
            if vline >= total_virtual_lines {
                break;
            }
            let line_y = files_y + screen_row as u16;

            if vline == unstaged_header_line {
                // Unstaged header
                let title = format!("Unstaged ({})", self.unstaged_files.len());
                let btn = if !self.unstaged_files.is_empty() {
                    Some("[Stage all]")
                } else {
                    None
                };
                let is_selected = self.cursor == vline && files_active;
                self.stage_all_btn_area = self.render_section_header_simple(
                    &title,
                    btn,
                    is_selected,
                    content_area.x,
                    line_y,
                    files_width,
                    buf,
                    &theme,
                );
            } else if vline >= unstaged_files_start && vline < unstaged_files_end {
                // Unstaged file
                let file_idx = vline - unstaged_files_start;
                let is_selected = self.cursor == vline && files_active;
                self.render_unstaged_file_line(
                    file_idx,
                    is_selected,
                    content_area.x,
                    line_y,
                    files_width,
                    buf,
                    &theme,
                    files_active,
                );
            } else if vline == staged_header_line {
                // Staged header
                let title = format!("Staged ({})", self.staged_files.len());
                let btn = if !self.staged_files.is_empty() {
                    Some("[Unstage all]")
                } else {
                    None
                };
                let is_selected = self.cursor == vline && files_active;
                self.unstage_all_btn_area = self.render_section_header_simple(
                    &title,
                    btn,
                    is_selected,
                    content_area.x,
                    line_y,
                    files_width,
                    buf,
                    &theme,
                );
            } else if vline >= staged_files_start {
                // Staged file
                let file_idx = vline - staged_files_start;
                let is_selected = self.cursor == vline && files_active;
                self.render_staged_file_line(
                    file_idx,
                    is_selected,
                    content_area.x,
                    line_y,
                    files_width,
                    buf,
                    &theme,
                    files_active,
                );
            }
        }

        // Single scrollbar for entire files area
        if let Some(border_x) = border_right_x {
            ScrollBar::render(
                buf,
                border_x,
                files_y,
                files_area_height as u16,
                self.scroll_offset,
                files_area_height,
                total_virtual_lines,
                &theme,
                files_active,
            );
        }

        // === STICKY HEADERS ===
        // When a section header scrolls out of view, render it at the top of files area
        // so user always knows which section they're viewing

        // Staged header is sticky if we've scrolled past it (into staged files only)
        let staged_sticky =
            self.scroll_offset > staged_header_line && !self.staged_files.is_empty();

        // Unstaged header is sticky if scrolled past line 0, but NOT if staged is sticky
        let unstaged_sticky = self.scroll_offset > unstaged_header_line
            && !self.unstaged_files.is_empty()
            && !staged_sticky;

        if unstaged_sticky {
            let title = format!("Unstaged ({})", self.unstaged_files.len());
            let btn = if !self.unstaged_files.is_empty() {
                Some("[Stage all]")
            } else {
                None
            };
            let is_selected = self.cursor == unstaged_header_line && files_active;
            self.stage_all_btn_area = self.render_section_header_simple(
                &title,
                btn,
                is_selected,
                content_area.x,
                files_y,
                files_width,
                buf,
                &theme,
            );
        }

        if staged_sticky {
            let title = format!("Staged ({})", self.staged_files.len());
            let btn = if !self.staged_files.is_empty() {
                Some("[Unstage all]")
            } else {
                None
            };
            let is_selected = self.cursor == staged_header_line && files_active;
            self.unstage_all_btn_area = self.render_section_header_simple(
                &title,
                btn,
                is_selected,
                content_area.x,
                files_y,
                files_width,
                buf,
                &theme,
            );
        }

        y += files_area_height as u16;

        // Separator before buttons
        self.render_horizontal_line(content_area.x, y, content_area.width, buf, &theme);
        y += separator_height;

        // === BOTTOM ZONE: Buttons ===
        self.buttons_y = y;
        self.render_buttons(
            content_area.x,
            y,
            content_area.width,
            buf,
            &theme,
            is_focused,
        );

        // === DROPDOWNS (rendered last to overlay) ===
        if self.repo_dropdown_open {
            let dropdown_y = content_area.y + 1;
            let max_dropdown_height = content_area.height.saturating_sub(3) as usize;
            let repo_names: Vec<String> = self
                .repo_manager
                .repos()
                .iter()
                .map(|p| git::get_repo_name(p))
                .collect();
            let visible_count = repo_names.len().min(max_dropdown_height);
            let scroll_offset = if self.dropdown_cursor >= visible_count {
                self.dropdown_cursor - visible_count + 1
            } else {
                0
            };
            self.dropdown_scroll = scroll_offset;
            self.repo_dropdown_area = Some(Rect {
                x: content_area.x,
                y: dropdown_y,
                width: content_area.width / 2,
                height: visible_count as u16 + 2,
            });
            self.render_dropdown_list(
                &repo_names,
                self.repo_manager.selected_index(),
                self.dropdown_cursor,
                content_area.x,
                dropdown_y,
                content_area.width / 2,
                max_dropdown_height as u16,
                buf,
                &theme,
            );
        } else {
            self.repo_dropdown_area = None;
        }
        if self.branch_dropdown_open {
            let dropdown_y = content_area.y + 1;
            let max_dropdown_height = content_area.height.saturating_sub(3) as usize;
            let current_branch_idx = self
                .branches
                .iter()
                .position(|b| Some(b.as_str()) == self.branch.as_deref())
                .unwrap_or(0);
            let visible_count = self.branches.len().min(max_dropdown_height);
            let scroll_offset = if self.dropdown_cursor >= visible_count {
                self.dropdown_cursor - visible_count + 1
            } else {
                0
            };
            self.dropdown_scroll = scroll_offset;
            self.branch_dropdown_area = Some(Rect {
                x: branch_x,
                y: dropdown_y,
                width: branch_max_width,
                height: visible_count as u16 + 2,
            });
            self.render_dropdown_list(
                &self.branches,
                current_branch_idx,
                self.dropdown_cursor,
                branch_x,
                dropdown_y,
                branch_max_width,
                max_dropdown_height as u16,
                buf,
                &theme,
            );
        } else {
            self.branch_dropdown_area = None;
        }
    }

    /// Render section header with optional button selection highlighting
    fn render_section_header_simple(
        &self,
        title: &str,
        action_btn: Option<&str>,
        is_selected: bool,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
    ) -> Option<Rect> {
        let header_style = Style::default().fg(theme.disabled);

        // Draw line with embedded title
        let title_with_space = format!(" {} ", title);
        let title_width = title_with_space.width();

        // Left part of line
        buf.set_string(x, y, "─", header_style);

        // Title
        buf.set_string(x + 1, y, &title_with_space, header_style);

        // Rest of line (or action button)
        let after_title = x + 1 + title_width as u16;
        let remaining = width.saturating_sub(1 + title_width as u16);

        if let Some(btn_text) = action_btn {
            let btn_width = btn_text.width() as u16;
            if remaining > btn_width + 2 {
                // Line before button
                let line_width = remaining - btn_width - 1;
                for dx in 0..line_width {
                    buf.set_string(after_title + dx, y, "─", header_style);
                }
                // Button - inverted style when selected
                let btn_x = after_title + line_width;
                let btn_style = if is_selected {
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg)
                };
                buf.set_string(btn_x, y, btn_text, btn_style);
                Some(Rect {
                    x: btn_x,
                    y,
                    width: btn_width,
                    height: 1,
                })
            } else {
                for dx in 0..remaining {
                    buf.set_string(after_title + dx, y, "─", header_style);
                }
                None
            }
        } else {
            for dx in 0..remaining {
                buf.set_string(after_title + dx, y, "─", header_style);
            }
            None
        }
    }

    /// Get color and modifier for file status
    fn get_file_style(status: char, untracked: bool, theme: &ThemeColors) -> (Color, Modifier) {
        if untracked {
            (theme.success, Modifier::empty())
        } else {
            match status {
                'M' => (theme.warning, Modifier::empty()),
                'D' => (theme.error, Modifier::CROSSED_OUT),
                'A' | 'R' => (theme.success, Modifier::empty()),
                _ => (theme.fg, Modifier::empty()),
            }
        }
    }

    /// Render a single file line (works for both staged and unstaged files)
    fn render_file_line(
        path: &Path,
        status: char,
        untracked: bool,
        is_selected: bool,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
        is_focused: bool,
    ) {
        let (fg_color, extra_modifier) = Self::get_file_style(status, untracked, theme);

        let style = if is_selected && is_focused {
            Style::default()
                .fg(theme.bg)
                .bg(fg_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(fg_color).add_modifier(extra_modifier)
        };

        if is_selected && is_focused {
            for dx in 0..width {
                buf[(x + dx, y)].set_symbol(" ").set_style(style);
            }
        }

        let path_str = path.to_string_lossy();
        let line = format!(" {}", path_str);
        let truncated = truncate_to_width(&line, width as usize);
        buf.set_string(x, y, &truncated, style);
    }

    /// Render a single unstaged file line
    fn render_unstaged_file_line(
        &self,
        file_idx: usize,
        is_selected: bool,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
        is_focused: bool,
    ) {
        if let Some(file) = self.unstaged_files.get(file_idx) {
            Self::render_file_line(
                &file.path,
                file.status,
                file.untracked,
                is_selected,
                x,
                y,
                width,
                buf,
                theme,
                is_focused,
            );
        }
    }

    /// Render a single staged file line
    fn render_staged_file_line(
        &self,
        file_idx: usize,
        is_selected: bool,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
        is_focused: bool,
    ) {
        if let Some(file) = self.staged_files.get(file_idx) {
            Self::render_file_line(
                &file.path,
                file.status,
                false, // staged files are never untracked
                is_selected,
                x,
                y,
                width,
                buf,
                theme,
                is_focused,
            );
        }
    }

    fn render_buttons(
        &self,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
        is_focused: bool,
    ) {
        let buttons = self.get_visible_buttons();
        let mut current_x = x;

        for (i, button) in buttons.iter().enumerate() {
            let is_selected = self.current_section == Section::Buttons && i == self.selected_button;
            let label = format!("[{}]", button.label(self.spinner_frame));

            let style = if is_selected && is_focused {
                // Inverted cursor style - only when focused
                Style::default()
                    .fg(theme.bg)
                    .bg(theme.fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg)
            };

            if current_x + label.width() as u16 > x + width {
                // Wrap to next line (not implemented for simplicity)
                break;
            }

            buf.set_string(current_x, y, &label, style);
            current_x += label.width() as u16 + 1;
        }
    }

    // =========================================================================
    // Render helper functions for improved layout
    // =========================================================================

    /// Render a horizontal line separator
    fn render_horizontal_line(
        &self,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
    ) {
        let style = Style::default().fg(theme.border);
        for i in 0..width {
            buf[(x + i, y)].set_symbol("─").set_style(style);
        }
    }

    /// Render dropdown list overlay
    fn render_dropdown_list(
        &self,
        items: &[String],
        selected: usize,
        cursor: usize,
        x: u16,
        y: u16,
        max_width: u16,
        max_height: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
    ) {
        if items.is_empty() {
            return;
        }

        let visible_count = items.len().min(max_height as usize);
        let scroll_offset = if cursor >= visible_count {
            cursor - visible_count + 1
        } else {
            0
        };

        // Calculate dropdown width
        let item_max_width = items.iter().map(|s| s.width()).max().unwrap_or(10);
        let width = (item_max_width + 4).min(max_width as usize) as u16;

        // Border style
        let border_style = Style::default().fg(theme.border_focused);
        let bg_style = Style::default().bg(theme.bg);

        // Clear area and draw border
        let dropdown_height = visible_count as u16 + 2; // +2 for borders
        for dy in 0..dropdown_height {
            for dx in 0..width {
                let cell = &mut buf[(x + dx, y + dy)];
                cell.set_style(bg_style);
                if dy == 0 || dy == dropdown_height - 1 {
                    if dx == 0 {
                        cell.set_symbol(if dy == 0 { "┌" } else { "└" });
                    } else if dx == width - 1 {
                        cell.set_symbol(if dy == 0 { "┐" } else { "┘" });
                    } else {
                        cell.set_symbol("─");
                    }
                    cell.set_style(border_style);
                } else if dx == 0 || dx == width - 1 {
                    cell.set_symbol("│").set_style(border_style);
                } else {
                    cell.set_symbol(" ");
                }
            }
        }

        // Draw items
        for (i, item) in items
            .iter()
            .skip(scroll_offset)
            .take(visible_count)
            .enumerate()
        {
            let item_y = y + 1 + i as u16;
            let is_cursor = scroll_offset + i == cursor;
            let is_selected = scroll_offset + i == selected;

            let style = if is_cursor {
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.selection_bg)
            } else if is_selected {
                Style::default().fg(theme.cursor)
            } else {
                Style::default().fg(theme.fg)
            };

            // Truncate item
            let max_item_width = (width - 2) as usize;
            let display_item = if item.width() > max_item_width {
                &item[..max_item_width]
            } else {
                item
            };

            // Clear line and draw item
            for dx in 1..width - 1 {
                buf[(x + dx, item_y)].set_symbol(" ").set_style(style);
            }
            buf.set_string(x + 1, item_y, display_item, style);
        }
    }

    /// Check if coordinates are within a rect
    fn is_in_rect(&self, col: u16, row: u16, rect: Rect) -> bool {
        col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
    }

    /// Take modal request for app to handle
    pub fn take_modal_request(&mut self) -> Option<(PendingAction, ActiveModal)> {
        self.modal_request.take()
    }
}

impl Panel for GitStatusPanel {
    fn name(&self) -> &'static str {
        "git_status"
    }

    fn title(&self) -> String {
        use termide_config::constants::LOADING_INDICATOR;

        let repo_name = self
            .repo_manager
            .current()
            .map(git::get_repo_name)
            .unwrap_or_else(|| "No repo".to_string());
        let branch = self.branch.as_deref().unwrap_or("(detached)");

        let uncommitted = self.unstaged_files.len() + self.staged_files.len();
        let status = format!("*{} ↑{} ↓{}", uncommitted, self.ahead, self.behind);

        if self.is_loading {
            format!(
                "{} Git: {} ({}) {}",
                LOADING_INDICATOR, repo_name, branch, status
            )
        } else {
            format!("Git: {} ({}) {}", repo_name, branch, status)
        }
    }

    fn prepare_render(&mut self, theme: &Theme, config: &Config) {
        self.cached_theme = ThemeColors::from(theme);
        self.keybindings = config.git_status.keybindings.clone();
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
        self.last_area = area;

        // Render content (border is handled by ui-render)
        self.render_content(area, buf, ctx.is_focused, ctx.border_right_x);
    }

    fn captures_escape(&self) -> bool {
        // Capture Escape when dropdown is open (don't close panel)
        self.repo_dropdown_open || self.branch_dropdown_open
    }

    fn handle_command(&mut self, cmd: PanelCommand<'_>) -> CommandResult {
        match cmd {
            PanelCommand::OnGitUpdate { repo_paths } => {
                // Check if current repo is in the updated list
                if let Some(current_repo) = self.repo_manager.current() {
                    let should_refresh = repo_paths
                        .iter()
                        .any(|p| current_repo.starts_with(p) || p.starts_with(current_repo));
                    if should_refresh {
                        self.refresh();
                        return CommandResult::NeedsRedraw(true);
                    }
                }
                CommandResult::NeedsRedraw(false)
            }
            PanelCommand::OnFsUpdate { changed_path } => {
                // Refresh on file changes within current repo
                if let Some(current_repo) = self.repo_manager.current() {
                    if changed_path.starts_with(current_repo) {
                        self.refresh();
                        return CommandResult::NeedsRedraw(true);
                    }
                }
                CommandResult::NeedsRedraw(false)
            }
            PanelCommand::SetGitOperationInProgress {
                in_progress,
                operation,
                spinner_frame,
            } => {
                let changed = self.git_operation_in_progress != in_progress
                    || self.current_operation != operation
                    || self.spinner_frame != spinner_frame;
                if changed {
                    self.git_operation_in_progress = in_progress;
                    self.current_operation = operation;
                    self.spinner_frame = spinner_frame;
                    // Adjust selected button if Push/Pull disappeared
                    let buttons = self.get_visible_buttons();
                    if self.selected_button >= buttons.len() {
                        self.selected_button = buttons.len().saturating_sub(1);
                    }
                    return CommandResult::NeedsRedraw(true);
                }
                CommandResult::NeedsRedraw(false)
            }
            _ => CommandResult::None,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        // Clear status message on any key
        self.status_message = None;

        let kb = &self.keybindings;

        // Configurable keybindings (checked first)

        // Stage file (Insert, Ctrl+S)
        if matches_binding_or_defaults(
            &kb.stage_file,
            &key,
            &[
                (KeyCode::Insert, KeyModifiers::NONE),
                (KeyCode::Char('s'), KeyModifiers::CONTROL),
            ],
        ) {
            if self.current_section == Section::Files
                && matches!(self.get_selection(), Some(Selection::UnstagedFile(_)))
            {
                self.do_stage();
            }
            return vec![];
        }

        // Unstage file (Delete, Ctrl+U)
        if matches_binding_or_defaults(
            &kb.unstage_file,
            &key,
            &[
                (KeyCode::Delete, KeyModifiers::NONE),
                (KeyCode::Char('u'), KeyModifiers::CONTROL),
            ],
        ) {
            if self.current_section == Section::Files
                && matches!(self.get_selection(), Some(Selection::StagedFile(_)))
            {
                self.do_unstage();
            }
            return vec![];
        }

        // Refresh (Ctrl+R)
        if matches_binding_or_default(&kb.refresh, &key, KeyCode::Char('r'), KeyModifiers::CONTROL)
        {
            self.refresh();
            self.status_message = Some("Refreshed".to_string());
            return vec![];
        }

        // Next section (Tab)
        if matches_binding_or_default(&kb.next_section, &key, KeyCode::Tab, KeyModifiers::NONE) {
            self.next_section();
            return vec![];
        }

        // Prev section (Shift+Tab/BackTab)
        if matches_binding_or_default(&kb.prev_section, &key, KeyCode::BackTab, KeyModifiers::NONE)
        {
            self.prev_section();
            return vec![];
        }

        // Non-configurable bindings (navigation)
        match key.code {
            KeyCode::Tab => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.prev_section();
                } else {
                    self.next_section();
                }
            }
            KeyCode::BackTab => {
                self.prev_section();
            }
            KeyCode::Up => self.handle_up_key(),
            KeyCode::Down => self.handle_down_key(),
            KeyCode::PageUp => {
                if self.current_section == Section::Files {
                    let page_size = self.viewport_height.max(1);
                    let mut new_cursor = self.cursor.saturating_sub(page_size);
                    // Find nearest selectable line
                    while new_cursor > 0 && !self.is_selectable_line(new_cursor) {
                        new_cursor -= 1;
                    }
                    if self.is_selectable_line(new_cursor) {
                        self.cursor = new_cursor;
                    }
                    self.ensure_cursor_visible();
                }
            }
            KeyCode::PageDown => {
                if self.current_section == Section::Files {
                    let max = self.total_virtual_lines();
                    let page_size = self.viewport_height.max(1);
                    let target = (self.cursor + page_size).min(max.saturating_sub(1));
                    // Find nearest selectable line, searching backward from target
                    let mut new_cursor = target;
                    while new_cursor > self.cursor && !self.is_selectable_line(new_cursor) {
                        new_cursor -= 1;
                    }
                    if new_cursor > self.cursor && self.is_selectable_line(new_cursor) {
                        self.cursor = new_cursor;
                    }
                    self.ensure_cursor_visible();
                    // If cursor is at last selectable line, scroll to show all content
                    if self.cursor == self.last_selectable_line() && max > self.viewport_height {
                        self.scroll_offset = max.saturating_sub(self.viewport_height);
                    }
                }
            }
            KeyCode::Home => {
                if self.current_section == Section::Files {
                    // Go to first item in current section (unstaged or staged)
                    let unstaged_end = 1 + self.unstaged_files.len();
                    let staged_header = unstaged_end;

                    if self.cursor < staged_header {
                        // In unstaged section - go to first unstaged file or header
                        if !self.unstaged_files.is_empty() {
                            self.cursor = 1; // First unstaged file
                        } else {
                            self.cursor = 0; // Unstaged header
                        }
                    } else {
                        // In staged section - go to first staged file or header
                        if !self.staged_files.is_empty() {
                            self.cursor = staged_header + 1; // First staged file
                        } else {
                            self.cursor = staged_header; // Staged header
                        }
                    }
                    self.ensure_cursor_visible();
                }
            }
            KeyCode::End => {
                if self.current_section == Section::Files {
                    // Go to last item in current section (unstaged or staged)
                    let unstaged_end = 1 + self.unstaged_files.len();
                    let staged_header = unstaged_end;
                    let staged_end = staged_header + 1 + self.staged_files.len();

                    if self.cursor < staged_header {
                        // In unstaged section - go to last unstaged file or header
                        if !self.unstaged_files.is_empty() {
                            self.cursor = unstaged_end - 1; // Last unstaged file
                        } else {
                            self.cursor = 0; // Unstaged header
                        }
                    } else {
                        // In staged section - go to last staged file or header
                        if !self.staged_files.is_empty() {
                            self.cursor = staged_end - 1; // Last staged file
                        } else {
                            self.cursor = staged_header; // Staged header
                        }
                    }
                    self.ensure_cursor_visible();
                }
            }
            KeyCode::Left => match self.current_section {
                Section::BranchSelector => {
                    self.current_section = Section::RepoSelector;
                }
                Section::Buttons => {
                    if self.selected_button > 0 {
                        self.selected_button -= 1;
                    }
                }
                _ => {}
            },
            KeyCode::Right => match self.current_section {
                Section::RepoSelector => {
                    self.current_section = Section::BranchSelector;
                }
                Section::Buttons => {
                    let max = self.get_visible_buttons().len().saturating_sub(1);
                    if self.selected_button < max {
                        self.selected_button += 1;
                    }
                }
                _ => {}
            },
            // Space - open file properties modal (only for files, not headers)
            KeyCode::Char(' ') => {
                if self.current_section == Section::Files
                    && matches!(
                        self.get_selection(),
                        Some(Selection::UnstagedFile(_)) | Some(Selection::StagedFile(_))
                    )
                {
                    return self.show_file_properties();
                }
            }
            KeyCode::Enter => {
                return self.handle_enter_key();
            }
            KeyCode::Esc => {
                // Close any open dropdown
                if self.branch_dropdown_open {
                    self.branch_dropdown_open = false;
                } else if self.repo_dropdown_open {
                    self.repo_dropdown_open = false;
                }
            }
            _ => {}
        }

        vec![]
    }

    fn handle_mouse(&mut self, event: MouseEvent, _panel_area: Rect) -> Vec<PanelEvent> {
        let col = event.column;
        let row = event.row;

        match event.kind {
            // Scroll handling - unified scroll for files area
            MouseEventKind::ScrollUp => {
                if self.is_in_rect(col, row, self.files_area) {
                    self.scroll_offset = self.scroll_offset.saturating_sub(3);
                }
            }
            MouseEventKind::ScrollDown => {
                if self.is_in_rect(col, row, self.files_area) {
                    let total_lines = self.total_virtual_lines();
                    let max_scroll = total_lines.saturating_sub(self.viewport_height);
                    self.scroll_offset = (self.scroll_offset + 3).min(max_scroll);
                }
            }

            // Click handling
            MouseEventKind::Down(MouseButton::Left) => {
                let now = std::time::Instant::now();

                // Check if click is in open repo dropdown
                if self.repo_dropdown_open {
                    if let Some(dropdown_area) = self.repo_dropdown_area {
                        if self.is_in_rect(col, row, dropdown_area) {
                            // Calculate which item was clicked (accounting for border)
                            let relative_row = row.saturating_sub(dropdown_area.y + 1) as usize;
                            let clicked_idx = self.dropdown_scroll + relative_row;
                            if clicked_idx < self.repo_manager.len()
                                && relative_row < dropdown_area.height.saturating_sub(2) as usize
                            {
                                self.repo_manager.select(clicked_idx);
                                self.refresh();
                            }
                            self.repo_dropdown_open = false;
                            return vec![];
                        }
                    }
                }

                // Check if click is in open branch dropdown
                if self.branch_dropdown_open {
                    if let Some(dropdown_area) = self.branch_dropdown_area {
                        if self.is_in_rect(col, row, dropdown_area) {
                            // Calculate which item was clicked (accounting for border)
                            let relative_row = row.saturating_sub(dropdown_area.y + 1) as usize;
                            let clicked_idx = self.dropdown_scroll + relative_row;
                            if clicked_idx < self.branches.len()
                                && relative_row < dropdown_area.height.saturating_sub(2) as usize
                            {
                                self.switch_to_branch(clicked_idx);
                            }
                            self.branch_dropdown_open = false;
                            return vec![];
                        }
                    }
                }

                // Check if click is on Stage all button
                if let Some(btn_area) = self.stage_all_btn_area {
                    if self.is_in_rect(col, row, btn_area) {
                        self.do_stage_all();
                        return vec![];
                    }
                }

                // Check if click is on Unstage all button
                if let Some(btn_area) = self.unstage_all_btn_area {
                    if self.is_in_rect(col, row, btn_area) {
                        self.do_unstage_all();
                        return vec![];
                    }
                }

                // Check if click is in files area (unified)
                if self.is_in_rect(col, row, self.files_area) {
                    // Close any open dropdown
                    self.repo_dropdown_open = false;
                    self.branch_dropdown_open = false;

                    let relative_row = (row - self.files_area.y) as usize;
                    let vline = self.scroll_offset + relative_row;

                    // Virtual layout constants
                    let unstaged_files_start = 1;
                    let unstaged_files_end = unstaged_files_start + self.unstaged_files.len();
                    let staged_header_line = unstaged_files_end;
                    let staged_files_start = staged_header_line + 1;
                    let staged_files_end = staged_files_start + self.staged_files.len();

                    // Determine what was clicked
                    let unstaged_header_line = 0;

                    if vline == unstaged_header_line && !self.unstaged_files.is_empty() {
                        // Clicked on unstaged header (with Stage all button)
                        self.current_section = Section::Files;
                        self.cursor = vline;
                        self.record_click(now, vline);
                    } else if vline >= unstaged_files_start && vline < unstaged_files_end {
                        // Clicked on unstaged file
                        if self.check_double_click(now, vline) {
                            // Double-click on unstaged = stage file
                            self.cursor = vline;
                            self.current_section = Section::Files;
                            self.do_stage();
                            self.reset_click_state();
                        } else {
                            // Single click - select item
                            self.current_section = Section::Files;
                            self.cursor = vline;
                            self.record_click(now, vline);
                        }
                    } else if vline == staged_header_line && !self.staged_files.is_empty() {
                        // Clicked on staged header (with Unstage all button)
                        self.current_section = Section::Files;
                        self.cursor = vline;
                        self.record_click(now, vline);
                    } else if vline >= staged_files_start && vline < staged_files_end {
                        // Clicked on staged file
                        if self.check_double_click(now, vline) {
                            // Double-click on staged = unstage file
                            self.cursor = vline;
                            self.current_section = Section::Files;
                            self.do_unstage();
                            self.reset_click_state();
                        } else {
                            // Single click - select item
                            self.current_section = Section::Files;
                            self.cursor = vline;
                            self.record_click(now, vline);
                        }
                    }
                    // Clicks on empty header lines are ignored
                }
                // Check if click is on selector row
                else if row == self.selector_y {
                    // Use saved branch_selector_x position for accurate detection
                    if col < self.branch_selector_x {
                        self.current_section = Section::RepoSelector;
                        // Toggle repo dropdown (close branch if open)
                        self.branch_dropdown_open = false;
                        self.repo_dropdown_open = !self.repo_dropdown_open;
                        if self.repo_dropdown_open {
                            self.dropdown_cursor = self.repo_manager.selected_index();
                        }
                    } else {
                        self.current_section = Section::BranchSelector;
                        // Toggle branch dropdown (close repo if open)
                        self.repo_dropdown_open = false;
                        self.branch_dropdown_open = !self.branch_dropdown_open;
                        if self.branch_dropdown_open {
                            self.dropdown_cursor = self
                                .branches
                                .iter()
                                .position(|b| Some(b.as_str()) == self.branch.as_deref())
                                .unwrap_or(0);
                        }
                    }
                    // Reset click state for non-file areas
                    self.reset_click_state();
                }
                // Check if click is on buttons row
                else if row == self.buttons_y {
                    // Close any open dropdown
                    self.repo_dropdown_open = false;
                    self.branch_dropdown_open = false;

                    self.current_section = Section::Buttons;
                    // Calculate which button was clicked based on position
                    let buttons = self.get_visible_buttons();
                    let content_x = self.last_area.x + 1;
                    let mut btn_x = content_x;
                    for (i, button) in buttons.iter().enumerate() {
                        let label = format!("[{}]", button.label(self.spinner_frame));
                        let btn_width = label.width() as u16;
                        if col >= btn_x && col < btn_x + btn_width {
                            self.selected_button = i;
                            // Execute button action on click
                            return self.execute_button();
                        }
                        btn_x += btn_width + 1;
                    }
                    // Reset click state for non-file areas
                    self.reset_click_state();
                }
            }

            _ => {}
        }

        vec![]
    }

    fn to_session(&self, _session_dir: &Path) -> Option<SessionPanel> {
        self.repo_manager
            .current()
            .map(|repo| SessionPanel::GitStatus {
                repo_path: repo.to_path_buf(),
            })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn get_working_directory(&self) -> Option<PathBuf> {
        self.repo_manager.current().map(|p| p.to_path_buf())
    }
}
