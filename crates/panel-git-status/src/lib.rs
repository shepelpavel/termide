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
    style::{Modifier, Style},
};
use unicode_width::UnicodeWidthStr;

use termide_config::Config;
use termide_core::{
    CommandResult, Panel, PanelCommand, PanelEvent, RenderContext, SessionPanel, ThemeColors,
};
use termide_git::{self as git, StagedFile, UnstagedFile};
use termide_modal::{ActionButton, ActiveModal, InfoActionModal};
use termide_state::PendingAction;
use termide_theme::Theme;

/// Section of the Git Status panel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Repository selector
    RepoSelector,
    /// Branch selector
    BranchSelector,
    /// Unstaged files section
    Unstaged,
    /// Staged files section
    Staged,
    /// Action buttons
    Buttons,
}

/// Button in the Git Status panel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Commit,
    Pull,
    Push,
}

impl Button {
    fn label(&self) -> &'static str {
        match self {
            Button::Commit => "Commit",
            Button::Pull => "Pull",
            Button::Push => "Push",
        }
    }

    fn all() -> &'static [Button] {
        &[Button::Commit, Button::Pull, Button::Push]
    }
}

/// Git Status Panel
pub struct GitStatusPanel {
    /// Available repositories in session
    repos: Vec<PathBuf>,
    /// Currently selected repository index
    selected_repo: usize,
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
    /// Cursor position in unstaged list
    unstaged_cursor: usize,
    /// Cursor position in staged list
    staged_cursor: usize,
    /// Selected button index
    selected_button: usize,
    /// Scroll offset for unstaged list
    unstaged_scroll: usize,
    /// Scroll offset for staged list
    staged_scroll: usize,
    /// Cached theme colors for rendering
    cached_theme: ThemeColors,
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
    /// Unstaged files area
    unstaged_area: Rect,
    /// Staged files area
    staged_area: Rect,
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
    /// Time of last click for double-click detection
    last_click_time: Option<std::time::Instant>,
    /// Section and index of last click
    last_click_section: Option<Section>,
    last_click_index: Option<usize>,
    /// Modal request (for file properties)
    modal_request: Option<(termide_state::PendingAction, termide_modal::ActiveModal)>,
}

impl GitStatusPanel {
    /// Create a new Git Status panel
    pub fn new(project_root: &Path) -> Self {
        // Find all repos in session
        let repos = git::find_all_repos(project_root, 3);
        let repos = if repos.is_empty() {
            // Use project root if it's a git repo
            if git::find_repo_root(project_root).is_some() {
                vec![project_root.to_path_buf()]
            } else {
                vec![]
            }
        } else {
            repos
        };

        let mut panel = Self {
            repos,
            selected_repo: 0,
            branch: None,
            branches: Vec::new(),
            ahead: 0,
            behind: 0,
            unstaged_files: Vec::new(),
            staged_files: Vec::new(),
            current_section: Section::RepoSelector,
            unstaged_cursor: 0,
            staged_cursor: 0,
            selected_button: 0,
            unstaged_scroll: 0,
            staged_scroll: 0,
            cached_theme: ThemeColors::default(),
            last_area: Rect::default(),
            status_message: None,
            repo_dropdown_open: false,
            branch_dropdown_open: false,
            dropdown_cursor: 0,
            selector_y: 0,
            branch_selector_x: 0,
            unstaged_area: Rect::default(),
            staged_area: Rect::default(),
            buttons_y: 0,
            repo_dropdown_area: None,
            branch_dropdown_area: None,
            dropdown_scroll: 0,
            stage_all_btn_area: None,
            unstage_all_btn_area: None,
            last_click_time: None,
            last_click_section: None,
            last_click_index: None,
            modal_request: None,
        };

        panel.refresh();
        panel
    }

    /// Create panel for a specific repository
    pub fn new_for_repo(repo_path: PathBuf) -> Self {
        let mut panel = Self {
            repos: vec![repo_path],
            selected_repo: 0,
            branch: None,
            branches: Vec::new(),
            ahead: 0,
            behind: 0,
            unstaged_files: Vec::new(),
            staged_files: Vec::new(),
            current_section: Section::RepoSelector,
            unstaged_cursor: 0,
            staged_cursor: 0,
            selected_button: 0,
            unstaged_scroll: 0,
            staged_scroll: 0,
            cached_theme: ThemeColors::default(),
            last_area: Rect::default(),
            status_message: None,
            repo_dropdown_open: false,
            branch_dropdown_open: false,
            dropdown_cursor: 0,
            selector_y: 0,
            branch_selector_x: 0,
            unstaged_area: Rect::default(),
            staged_area: Rect::default(),
            buttons_y: 0,
            repo_dropdown_area: None,
            branch_dropdown_area: None,
            dropdown_scroll: 0,
            stage_all_btn_area: None,
            unstage_all_btn_area: None,
            last_click_time: None,
            last_click_section: None,
            last_click_index: None,
            modal_request: None,
        };

        panel.refresh();
        panel
    }

    /// Get current repository path
    fn current_repo(&self) -> Option<&Path> {
        self.repos.get(self.selected_repo).map(|p| p.as_path())
    }

    /// Refresh git status
    pub fn refresh(&mut self) {
        let repo = match self.repos.get(self.selected_repo) {
            Some(r) => r.clone(),
            None => return,
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

        // Adjust cursors
        if self.unstaged_cursor >= self.unstaged_files.len() && !self.unstaged_files.is_empty() {
            self.unstaged_cursor = self.unstaged_files.len() - 1;
        }
        if self.staged_cursor >= self.staged_files.len() && !self.staged_files.is_empty() {
            self.staged_cursor = self.staged_files.len() - 1;
        }
    }

    /// Move to next section
    fn next_section(&mut self) {
        self.current_section = match self.current_section {
            Section::RepoSelector => Section::BranchSelector,
            Section::BranchSelector => {
                if !self.unstaged_files.is_empty() {
                    Section::Unstaged
                } else if !self.staged_files.is_empty() {
                    Section::Staged
                } else {
                    Section::Buttons
                }
            }
            Section::Unstaged => {
                if !self.staged_files.is_empty() {
                    Section::Staged
                } else {
                    Section::Buttons
                }
            }
            Section::Staged => Section::Buttons,
            Section::Buttons => Section::RepoSelector,
        };
    }

    /// Move to previous section
    fn prev_section(&mut self) {
        self.current_section = match self.current_section {
            Section::RepoSelector => Section::Buttons,
            Section::BranchSelector => Section::RepoSelector,
            Section::Unstaged => Section::BranchSelector,
            Section::Staged => {
                if !self.unstaged_files.is_empty() {
                    Section::Unstaged
                } else {
                    Section::BranchSelector
                }
            }
            Section::Buttons => {
                if !self.staged_files.is_empty() {
                    Section::Staged
                } else if !self.unstaged_files.is_empty() {
                    Section::Unstaged
                } else {
                    Section::BranchSelector
                }
            }
        };
    }

    /// Get file at cursor from unstaged section
    fn get_selected_unstaged(&self) -> Vec<PathBuf> {
        self.unstaged_files
            .get(self.unstaged_cursor)
            .map(|f| f.path.clone())
            .into_iter()
            .collect()
    }

    /// Get file at cursor from staged section
    fn get_selected_staged(&self) -> Vec<PathBuf> {
        self.staged_files
            .get(self.staged_cursor)
            .map(|f| f.path.clone())
            .into_iter()
            .collect()
    }

    /// Execute stage action
    fn do_stage(&mut self) {
        if let Some(repo) = self.current_repo() {
            let files = self.get_selected_unstaged();
            if !files.is_empty() {
                match git::stage_files(repo, &files) {
                    Ok(()) => {
                        self.status_message = Some(format!("Staged {} file(s)", files.len()));
                        self.refresh();
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Stage error: {}", e));
                    }
                }
            }
        }
    }

    /// Execute unstage action
    fn do_unstage(&mut self) {
        if let Some(repo) = self.current_repo() {
            let files = self.get_selected_staged();
            if !files.is_empty() {
                match git::unstage_files(repo, &files) {
                    Ok(()) => {
                        self.status_message = Some(format!("Unstaged {} file(s)", files.len()));
                        self.refresh();
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Unstage error: {}", e));
                    }
                }
            }
        }
    }

    /// Execute revert action
    #[allow(dead_code)]
    fn do_revert(&mut self) {
        if let Some(repo) = self.current_repo() {
            let files = self.get_selected_unstaged();
            if !files.is_empty() {
                match git::revert_files(repo, &files) {
                    Ok(()) => {
                        self.status_message = Some(format!("Reverted {} file(s)", files.len()));
                        self.refresh();
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Revert error: {}", e));
                    }
                }
            }
        }
    }

    /// Stage all unstaged files
    fn do_stage_all(&mut self) {
        if let Some(repo) = self.current_repo() {
            let files: Vec<PathBuf> = self.unstaged_files.iter().map(|f| f.path.clone()).collect();
            if !files.is_empty() {
                match git::stage_files(repo, &files) {
                    Ok(()) => {
                        self.status_message = Some(format!("Staged {} file(s)", files.len()));
                        self.refresh();
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Stage error: {}", e));
                    }
                }
            }
        }
    }

    /// Unstage all staged files
    fn do_unstage_all(&mut self) {
        if let Some(repo) = self.current_repo() {
            let files: Vec<PathBuf> = self.staged_files.iter().map(|f| f.path.clone()).collect();
            if !files.is_empty() {
                match git::unstage_files(repo, &files) {
                    Ok(()) => {
                        self.status_message = Some(format!("Unstaged {} file(s)", files.len()));
                        self.refresh();
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Unstage error: {}", e));
                    }
                }
            }
        }
    }

    /// Show file properties modal with Diff/Revert actions
    fn show_file_properties(&mut self, is_staged: bool) -> Vec<PanelEvent> {
        let t = termide_i18n::t();

        let (file_path, status_str) = if is_staged {
            if let Some(file) = self.staged_files.get(self.staged_cursor) {
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
        } else if let Some(file) = self.unstaged_files.get(self.unstaged_cursor) {
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

        let Some(repo_path) = self.current_repo().map(|p| p.to_path_buf()) else {
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

        // Build action buttons
        let mut buttons = Vec::new();
        buttons.push(ActionButton::new(t.git_action_diff(), "diff"));
        if !is_staged {
            // Only show Revert for unstaged files
            buttons.push(ActionButton::new(t.git_action_revert(), "revert"));
        }
        buttons.push(ActionButton::new(t.git_action_close(), "close"));

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
            },
            ActiveModal::InfoAction(Box::new(modal)),
        ));

        vec![]
    }

    /// Switch to a different branch
    fn switch_to_branch(&mut self, branch_idx: usize) {
        if let Some(branch_name) = self.branches.get(branch_idx) {
            if let Some(repo) = self.current_repo() {
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

    /// Ensure unstaged cursor is visible (auto-scroll)
    fn ensure_unstaged_visible(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
        if self.unstaged_cursor < self.unstaged_scroll {
            self.unstaged_scroll = self.unstaged_cursor;
        } else if self.unstaged_cursor >= self.unstaged_scroll + visible_height {
            self.unstaged_scroll = self.unstaged_cursor - visible_height + 1;
        }
    }

    /// Ensure staged cursor is visible (auto-scroll)
    fn ensure_staged_visible(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
        if self.staged_cursor < self.staged_scroll {
            self.staged_scroll = self.staged_cursor;
        } else if self.staged_cursor >= self.staged_scroll + visible_height {
            self.staged_scroll = self.staged_cursor - visible_height + 1;
        }
    }

    /// Calculate visible height for unstaged section based on last_area
    fn calc_unstaged_visible_height(&self) -> usize {
        // Layout constants (same as in render_content)
        let selector_height: u16 = 1;
        let separator_height: u16 = 1;
        let buttons_height: u16 = 2;
        let fixed_height = selector_height + separator_height * 3 + buttons_height;

        let content_height = self.last_area.height.saturating_sub(fixed_height + 2);
        let unstaged_height = content_height / 2;
        let files_area_height = unstaged_height.saturating_sub(1);

        files_area_height as usize
    }

    /// Calculate visible height for staged section based on last_area
    fn calc_staged_visible_height(&self) -> usize {
        // Layout constants (same as in render_content)
        let selector_height: u16 = 1;
        let separator_height: u16 = 1;
        let buttons_height: u16 = 2;
        let fixed_height = selector_height + separator_height * 3 + buttons_height;

        let content_height = self.last_area.height.saturating_sub(fixed_height + 2);
        let unstaged_height = content_height / 2;
        let staged_height = content_height.saturating_sub(unstaged_height);
        let files_area_height = staged_height.saturating_sub(1);

        files_area_height as usize
    }

    /// Execute button action
    fn execute_button(&mut self) -> Vec<PanelEvent> {
        let button = Button::all()[self.selected_button];
        match button {
            Button::Commit => {
                // TODO: Open commit modal
                self.status_message = Some("Commit modal not implemented yet".to_string());
                vec![]
            }
            Button::Pull => {
                if let Some(repo) = self.current_repo() {
                    vec![PanelEvent::RunCommand {
                        command: "git pull".to_string(),
                        cwd: Some(repo.to_path_buf()),
                    }]
                } else {
                    vec![]
                }
            }
            Button::Push => {
                if let Some(repo) = self.current_repo() {
                    vec![PanelEvent::RunCommand {
                        command: "git push".to_string(),
                        cwd: Some(repo.to_path_buf()),
                    }]
                } else {
                    vec![]
                }
            }
        }
    }

    /// Render the panel content with new layout:
    /// - Top: Repo selector + Branch selector (sticky)
    /// - Middle: Unstaged section (scrollable with scrollbar)
    /// - Middle: Staged section (scrollable with scrollbar)
    /// - Bottom: Action buttons (sticky)
    fn render_content(&mut self, area: Rect, buf: &mut Buffer, is_focused: bool) {
        if area.height < 5 {
            return;
        }

        let theme = self.cached_theme.clone();

        // area is already the inner content area (border handled by ui-render)
        let content_area = area;

        // Layout constants
        let selector_height: u16 = 1; // Top: repo + branch selectors
        let separator_height: u16 = 1; // Horizontal line before buttons only
        let buttons_height: u16 = 1; // Bottom: buttons row

        // Fixed zones (selector + separator before buttons + buttons)
        let fixed_height = selector_height + separator_height + buttons_height;
        let content_height = content_area.height.saturating_sub(fixed_height);

        // Calculate needed height for each zone (header + files)
        let unstaged_needed = 1 + self.unstaged_files.len() as u16;
        let staged_needed = 1 + self.staged_files.len() as u16;
        let total_needed = unstaged_needed + staged_needed;

        // Dynamic height allocation
        let (unstaged_total, staged_total) = if total_needed <= content_height {
            // Both fit completely
            (unstaged_needed, staged_needed)
        } else if unstaged_needed <= content_height / 2 {
            // Unstaged fits in half, give staged the rest
            (
                unstaged_needed,
                content_height.saturating_sub(unstaged_needed),
            )
        } else if staged_needed <= content_height / 2 {
            // Staged fits in half, give unstaged the rest
            (content_height.saturating_sub(staged_needed), staged_needed)
        } else {
            // Both need more than half - split evenly
            let half = content_height / 2;
            (half, content_height.saturating_sub(half))
        };

        let mut y = content_area.y;

        // === TOP ZONE: Selectors ===
        self.selector_y = y;

        // Repo selector
        let repo_name = self
            .current_repo()
            .map(git::get_repo_name)
            .unwrap_or_else(|| "No repo".to_string());
        let repo_focused = self.current_section == Section::RepoSelector && is_focused;
        let repo_width = self.render_selector(
            &repo_name,
            self.repo_dropdown_open,
            repo_focused,
            content_area.x,
            y,
            content_area.width / 2,
            buf,
            &theme,
        );

        // Branch selector (next to repo)
        let branch_name = self
            .branch
            .clone()
            .unwrap_or_else(|| "(detached)".to_string());
        let branch_focused = self.current_section == Section::BranchSelector && is_focused;
        let branch_x = content_area.x + repo_width + 2;
        self.branch_selector_x = branch_x; // Save for mouse click detection
        let branch_max_width = content_area.width.saturating_sub(repo_width + 2);
        self.render_selector(
            &branch_name,
            self.branch_dropdown_open,
            branch_focused,
            branch_x,
            y,
            branch_max_width,
            buf,
            &theme,
        );

        // Status indicators (right-aligned) - always show all three
        let uncommitted = self.unstaged_files.len() + self.staged_files.len();
        let status_text = format!("*{} ↑{} ↓{}", uncommitted, self.ahead, self.behind);
        let status_width = status_text.width() as u16;
        let status_x = content_area.x + content_area.width - status_width;
        buf.set_string(status_x, y, &status_text, Style::default().fg(theme.fg));

        y += selector_height;

        // === MIDDLE ZONE: Unstaged section ===
        let unstaged_active = self.current_section == Section::Unstaged;
        let unstaged_title = format!("Unstaged ({})", self.unstaged_files.len());
        let stage_all_btn = if !self.unstaged_files.is_empty() {
            Some("[Stage all]")
        } else {
            None
        };
        self.stage_all_btn_area = self.render_section_header(
            &unstaged_title,
            stage_all_btn,
            content_area.x,
            y,
            content_area.width,
            buf,
            &theme,
            unstaged_active,
        );
        y += 1;

        // Unstaged files area (with scrollbar on right)
        let files_width = content_area.width.saturating_sub(1); // Leave space for scrollbar
        let unstaged_files_height = unstaged_total.saturating_sub(1); // Minus header

        // Store unstaged area for mouse handling
        self.unstaged_area = Rect {
            x: content_area.x,
            y,
            width: files_width,
            height: unstaged_files_height,
        };

        self.render_unstaged_files(
            content_area.x,
            y,
            files_width,
            unstaged_files_height,
            buf,
            &theme,
            is_focused,
        );

        // Scrollbar for unstaged
        self.render_scrollbar(
            content_area.x + content_area.width - 1,
            y,
            unstaged_files_height,
            self.unstaged_scroll,
            self.unstaged_files.len(),
            buf,
            &theme,
            unstaged_active,
        );
        y += unstaged_files_height;

        // === MIDDLE ZONE: Staged section ===
        let staged_active = self.current_section == Section::Staged;
        let staged_title = format!("Staged ({})", self.staged_files.len());
        let unstage_all_btn = if !self.staged_files.is_empty() {
            Some("[Unstage all]")
        } else {
            None
        };
        self.unstage_all_btn_area = self.render_section_header(
            &staged_title,
            unstage_all_btn,
            content_area.x,
            y,
            content_area.width,
            buf,
            &theme,
            staged_active,
        );
        y += 1;

        // Staged files area (with scrollbar on right)
        let staged_files_height = staged_total.saturating_sub(1); // Minus header

        // Store staged area for mouse handling
        self.staged_area = Rect {
            x: content_area.x,
            y,
            width: files_width,
            height: staged_files_height,
        };

        self.render_staged_files(
            content_area.x,
            y,
            files_width,
            staged_files_height,
            buf,
            &theme,
            is_focused,
        );

        // Scrollbar for staged
        self.render_scrollbar(
            content_area.x + content_area.width - 1,
            y,
            staged_files_height,
            self.staged_scroll,
            self.staged_files.len(),
            buf,
            &theme,
            staged_active,
        );
        y += staged_files_height;

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
            let dropdown_y = content_area.y + 1; // Below selector
            let max_dropdown_height = content_area.height.saturating_sub(3) as usize;
            let repo_names: Vec<String> =
                self.repos.iter().map(|p| git::get_repo_name(p)).collect();
            let visible_count = repo_names.len().min(max_dropdown_height);
            let scroll_offset = if self.dropdown_cursor >= visible_count {
                self.dropdown_cursor - visible_count + 1
            } else {
                0
            };
            self.dropdown_scroll = scroll_offset;
            // Save dropdown area for mouse click detection
            self.repo_dropdown_area = Some(Rect {
                x: content_area.x,
                y: dropdown_y,
                width: content_area.width / 2,
                height: visible_count as u16 + 2,
            });
            self.render_dropdown_list(
                &repo_names,
                self.selected_repo,
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
            let dropdown_y = content_area.y + 1; // Below selector
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
            // Save dropdown area for mouse click detection
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

    /// Render unstaged files list (without header)
    fn render_unstaged_files(
        &self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
        is_focused: bool,
    ) {
        if height == 0 {
            return;
        }

        for (i, file) in self
            .unstaged_files
            .iter()
            .enumerate()
            .skip(self.unstaged_scroll)
            .take(height as usize)
        {
            let line_y = y + (i - self.unstaged_scroll) as u16;
            let is_cursor = self.current_section == Section::Unstaged && i == self.unstaged_cursor;

            // Determine color based on status (like file manager)
            let (fg_color, extra_modifier) = if file.untracked {
                (theme.success, Modifier::empty()) // Untracked = green (like Added)
            } else {
                match file.status {
                    'M' => (theme.warning, Modifier::empty()), // Modified = yellow
                    'D' => (theme.error, Modifier::CROSSED_OUT), // Deleted = red + strikethrough
                    'A' => (theme.success, Modifier::empty()), // Added = green
                    _ => (theme.fg, Modifier::empty()),        // Default
                }
            };

            let style = if is_cursor && is_focused {
                // Inverted cursor: fg becomes bg, theme.bg becomes fg
                Style::default()
                    .fg(theme.bg)
                    .bg(fg_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(fg_color).add_modifier(extra_modifier)
            };

            // Fill entire line width for cursor (only when focused)
            if is_cursor && is_focused {
                for dx in 0..width {
                    buf[(x + dx, line_y)].set_symbol(" ").set_style(style);
                }
            }

            let path_str = file.path.to_string_lossy();
            let line = format!(" {}", path_str); // No status char, just path
            let truncated = if line.width() > width as usize {
                &line[..width as usize]
            } else {
                &line
            };
            buf.set_string(x, line_y, truncated, style);
        }
    }

    /// Render staged files list (without header)
    fn render_staged_files(
        &self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
        is_focused: bool,
    ) {
        if height == 0 {
            return;
        }

        for (i, file) in self
            .staged_files
            .iter()
            .enumerate()
            .skip(self.staged_scroll)
            .take(height as usize)
        {
            let line_y = y + (i - self.staged_scroll) as u16;
            let is_cursor = self.current_section == Section::Staged && i == self.staged_cursor;

            // Determine color based on status (like file manager)
            let (fg_color, extra_modifier) = match file.status {
                'M' => (theme.warning, Modifier::empty()), // Modified = yellow
                'D' => (theme.error, Modifier::CROSSED_OUT), // Deleted = red + strikethrough
                'A' => (theme.success, Modifier::empty()), // Added = green
                'R' => (theme.success, Modifier::empty()), // Renamed = green
                _ => (theme.fg, Modifier::empty()),        // Default
            };

            let style = if is_cursor && is_focused {
                // Inverted cursor: fg becomes bg, theme.bg becomes fg
                Style::default()
                    .fg(theme.bg)
                    .bg(fg_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(fg_color).add_modifier(extra_modifier)
            };

            // Fill entire line width for cursor (only when focused)
            if is_cursor && is_focused {
                for dx in 0..width {
                    buf[(x + dx, line_y)].set_symbol(" ").set_style(style);
                }
            }

            let path_str = file.path.to_string_lossy();
            let line = format!(" {}", path_str); // No status char, just path
            let truncated = if line.width() > width as usize {
                &line[..width as usize]
            } else {
                &line
            };
            buf.set_string(x, line_y, truncated, style);
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
        let buttons = Button::all();
        let mut current_x = x;

        for (i, button) in buttons.iter().enumerate() {
            let is_selected = self.current_section == Section::Buttons && i == self.selected_button;
            let label = format!("[{}]", button.label());

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

    /// Render section header with horizontal line: "─ Title (count) ───── [Button]"
    /// Active zone uses focused border color, inactive uses normal border.
    /// Title text uses the same color as the line.
    /// Returns the button area if button_label was provided.
    fn render_section_header(
        &self,
        title: &str,
        button_label: Option<&str>,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
        is_active: bool,
    ) -> Option<Rect> {
        // Active zone: focused border color for line and title (bold)
        // Inactive zone: normal border color for line and title
        let line_color = if is_active {
            theme.border_focused
        } else {
            theme.border
        };
        let line_style = Style::default().fg(line_color);
        let title_style = if is_active {
            Style::default().fg(line_color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(line_color)
        };

        // Calculate button position if present
        let (button_area, line_end) = if let Some(btn) = button_label {
            let btn_width = btn.width() as u16;
            let btn_x = x + width - btn_width;
            let area = Rect {
                x: btn_x,
                y,
                width: btn_width,
                height: 1,
            };
            (Some(area), btn_x)
        } else {
            (None, x + width)
        };

        // "─ "
        buf[(x, y)].set_symbol("─").set_style(line_style);
        buf[(x + 1, y)].set_symbol(" ").set_style(line_style);

        // Title
        let title_x = x + 2;
        buf.set_string(title_x, y, title, title_style);

        // " ─────────" (up to button or end)
        let after_title = title_x + title.width() as u16 + 1;
        for i in after_title..line_end {
            buf[(i, y)].set_symbol("─").set_style(line_style);
        }

        // Render button if present (same style as title)
        if let Some(btn) = button_label {
            buf.set_string(line_end, y, btn, title_style);
        }

        button_area
    }

    /// Render vertical scrollbar
    /// Active zone uses focused border color for thumb, inactive uses disabled.
    fn render_scrollbar(
        &self,
        x: u16,
        y_start: u16,
        height: u16,
        scroll_offset: usize,
        total_items: usize,
        buf: &mut Buffer,
        theme: &ThemeColors,
        is_active: bool,
    ) {
        if total_items == 0 || height == 0 || total_items <= height as usize {
            return; // No scrollbar needed
        }

        let track_style = Style::default().fg(theme.disabled);
        let thumb_color = if is_active {
            theme.border_focused
        } else {
            theme.disabled
        };
        let thumb_style = Style::default().fg(thumb_color);

        let visible_ratio = height as f32 / total_items as f32;
        let thumb_height = (height as f32 * visible_ratio).max(1.0) as u16;
        let max_scroll = total_items.saturating_sub(height as usize);
        let scroll_ratio = if max_scroll > 0 {
            scroll_offset as f32 / max_scroll as f32
        } else {
            0.0
        };
        let thumb_pos = ((height.saturating_sub(thumb_height)) as f32 * scroll_ratio) as u16;

        for i in 0..height {
            let symbol = if i >= thumb_pos && i < thumb_pos + thumb_height {
                "█" // Thumb
            } else {
                "░" // Track
            };
            let style = if i >= thumb_pos && i < thumb_pos + thumb_height {
                thumb_style
            } else {
                track_style
            };
            buf[(x, y_start + i)].set_symbol(symbol).set_style(style);
        }
    }

    /// Render inline dropdown selector: "[label ▼]"
    fn render_selector(
        &self,
        label: &str,
        is_open: bool,
        is_focused: bool,
        x: u16,
        y: u16,
        max_width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
    ) -> u16 {
        let arrow = if is_open { "▲" } else { "▼" };
        let style = if is_focused {
            // Inverted cursor style
            Style::default()
                .fg(theme.bg)
                .bg(theme.fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg)
        };

        // Truncate label if needed
        let max_label_len = max_width.saturating_sub(4) as usize; // "[" + " " + arrow + "]"
        let truncated_label = if label.width() > max_label_len {
            &label[..max_label_len]
        } else {
            label
        };

        let text = format!("[{} {}]", truncated_label, arrow);
        let text_width = text.width() as u16;
        buf.set_string(x, y, &text, style);

        text_width
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
        let repo_name = self
            .current_repo()
            .map(git::get_repo_name)
            .unwrap_or_else(|| "No repo".to_string());

        let branch = self.branch.as_deref().unwrap_or("(detached)");

        let mut title = format!("Git Status: {} ({})", repo_name, branch);

        if self.ahead > 0 || self.behind > 0 {
            title.push(' ');
            if self.ahead > 0 {
                title.push_str(&format!("↑{}", self.ahead));
            }
            if self.behind > 0 {
                title.push_str(&format!("↓{}", self.behind));
            }
        }

        title
    }

    fn prepare_render(&mut self, theme: &Theme, _config: &Config) {
        self.cached_theme = ThemeColors::from(theme);
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
        self.last_area = area;

        // Render content (border is handled by ui-render)
        self.render_content(area, buf, ctx.is_focused);
    }

    fn captures_escape(&self) -> bool {
        // Capture Escape when dropdown is open (don't close panel)
        self.repo_dropdown_open || self.branch_dropdown_open
    }

    fn handle_command(&mut self, cmd: PanelCommand<'_>) -> CommandResult {
        match cmd {
            PanelCommand::OnGitUpdate { repo_paths } => {
                // Check if current repo is in the updated list
                if let Some(current_repo) = self.current_repo() {
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
            _ => CommandResult::None,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        // Clear status message on any key
        self.status_message = None;

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
            KeyCode::Up => {
                match self.current_section {
                    Section::RepoSelector => {
                        // Navigate in dropdown if open
                        if self.repo_dropdown_open && self.dropdown_cursor > 0 {
                            self.dropdown_cursor -= 1;
                        }
                    }
                    Section::BranchSelector => {
                        // Navigate in dropdown if open
                        if self.branch_dropdown_open && self.dropdown_cursor > 0 {
                            self.dropdown_cursor -= 1;
                        }
                    }
                    Section::Unstaged => {
                        if self.unstaged_cursor > 0 {
                            self.unstaged_cursor -= 1;
                            let visible = self.calc_unstaged_visible_height();
                            self.ensure_unstaged_visible(visible);
                        }
                    }
                    Section::Staged => {
                        if self.staged_cursor > 0 {
                            self.staged_cursor -= 1;
                            let visible = self.calc_staged_visible_height();
                            self.ensure_staged_visible(visible);
                        }
                    }
                    Section::Buttons => {
                        // Navigate up from buttons goes to previous section
                        self.prev_section();
                    }
                }
            }
            KeyCode::Down => {
                match self.current_section {
                    Section::RepoSelector => {
                        // Navigate in dropdown if open
                        if self.repo_dropdown_open && self.dropdown_cursor + 1 < self.repos.len() {
                            self.dropdown_cursor += 1;
                        }
                    }
                    Section::BranchSelector => {
                        // Navigate in dropdown if open
                        if self.branch_dropdown_open
                            && self.dropdown_cursor + 1 < self.branches.len()
                        {
                            self.dropdown_cursor += 1;
                        }
                    }
                    Section::Unstaged => {
                        if self.unstaged_cursor + 1 < self.unstaged_files.len() {
                            self.unstaged_cursor += 1;
                            let visible = self.calc_unstaged_visible_height();
                            self.ensure_unstaged_visible(visible);
                        }
                    }
                    Section::Staged => {
                        if self.staged_cursor + 1 < self.staged_files.len() {
                            self.staged_cursor += 1;
                            let visible = self.calc_staged_visible_height();
                            self.ensure_staged_visible(visible);
                        }
                    }
                    Section::Buttons => {
                        // Do nothing, at bottom
                    }
                }
            }
            KeyCode::Left => {
                if self.current_section == Section::Buttons && self.selected_button > 0 {
                    self.selected_button -= 1;
                }
            }
            KeyCode::Right => {
                if self.current_section == Section::Buttons {
                    let max = Button::all().len().saturating_sub(1);
                    if self.selected_button < max {
                        self.selected_button += 1;
                    }
                }
            }
            // Space - open file properties modal
            KeyCode::Char(' ') => match self.current_section {
                Section::Unstaged if !self.unstaged_files.is_empty() => {
                    return self.show_file_properties(false);
                }
                Section::Staged if !self.staged_files.is_empty() => {
                    return self.show_file_properties(true);
                }
                _ => {}
            },
            // Insert - stage file instantly in Unstaged zone
            KeyCode::Insert => {
                if self.current_section == Section::Unstaged && !self.unstaged_files.is_empty() {
                    self.do_stage();
                }
            }
            // Delete - unstage file instantly in Staged zone
            KeyCode::Delete => {
                if self.current_section == Section::Staged && !self.staged_files.is_empty() {
                    self.do_unstage();
                }
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.refresh();
                self.status_message = Some("Refreshed".to_string());
            }
            KeyCode::Enter => {
                match self.current_section {
                    // Enter in Unstaged zone - stage file
                    Section::Unstaged if !self.unstaged_files.is_empty() => {
                        self.do_stage();
                    }
                    // Enter in Staged zone - unstage file
                    Section::Staged if !self.staged_files.is_empty() => {
                        self.do_unstage();
                    }
                    Section::RepoSelector => {
                        if self.repo_dropdown_open {
                            // Select repo and close dropdown
                            if self.dropdown_cursor != self.selected_repo {
                                self.selected_repo = self.dropdown_cursor;
                                self.refresh();
                            }
                            self.repo_dropdown_open = false;
                        } else {
                            // Open dropdown
                            self.repo_dropdown_open = true;
                            self.dropdown_cursor = self.selected_repo;
                        }
                    }
                    Section::BranchSelector => {
                        if self.branch_dropdown_open {
                            // Select branch and close dropdown
                            self.switch_to_branch(self.dropdown_cursor);
                            self.branch_dropdown_open = false;
                        } else {
                            // Open dropdown
                            self.branch_dropdown_open = true;
                            // Set cursor to current branch
                            self.dropdown_cursor = self
                                .branches
                                .iter()
                                .position(|b| Some(b.as_str()) == self.branch.as_deref())
                                .unwrap_or(0);
                        }
                    }
                    Section::Buttons => {
                        return self.execute_button();
                    }
                    _ => {}
                }
            }
            KeyCode::Esc => {
                // Close any open dropdown
                if self.branch_dropdown_open {
                    self.branch_dropdown_open = false;
                } else if self.repo_dropdown_open {
                    self.repo_dropdown_open = false;
                }
            }
            // Shortcut keys for actions
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.do_stage();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.do_unstage();
            }
            _ => {}
        }

        vec![]
    }

    fn handle_mouse(&mut self, event: MouseEvent, _panel_area: Rect) -> Vec<PanelEvent> {
        let col = event.column;
        let row = event.row;

        match event.kind {
            // Scroll handling - scroll zone under mouse cursor
            MouseEventKind::ScrollUp => {
                if self.is_in_rect(col, row, self.unstaged_area) {
                    self.unstaged_scroll = self.unstaged_scroll.saturating_sub(3);
                } else if self.is_in_rect(col, row, self.staged_area) {
                    self.staged_scroll = self.staged_scroll.saturating_sub(3);
                }
            }
            MouseEventKind::ScrollDown => {
                if self.is_in_rect(col, row, self.unstaged_area) {
                    let visible = self.unstaged_area.height as usize;
                    let max_scroll = self.unstaged_files.len().saturating_sub(visible);
                    self.unstaged_scroll = (self.unstaged_scroll + 3).min(max_scroll);
                } else if self.is_in_rect(col, row, self.staged_area) {
                    let visible = self.staged_area.height as usize;
                    let max_scroll = self.staged_files.len().saturating_sub(visible);
                    self.staged_scroll = (self.staged_scroll + 3).min(max_scroll);
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
                            if clicked_idx < self.repos.len()
                                && relative_row < dropdown_area.height.saturating_sub(2) as usize
                            {
                                self.selected_repo = clicked_idx;
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

                // Check if click is in unstaged area
                if self.is_in_rect(col, row, self.unstaged_area) {
                    // Close any open dropdown
                    self.repo_dropdown_open = false;
                    self.branch_dropdown_open = false;

                    let relative_row = (row - self.unstaged_area.y) as usize;
                    let idx = self.unstaged_scroll + relative_row;

                    if idx < self.unstaged_files.len() {
                        // Check for double-click
                        let is_double_click =
                            if let (Some(last_time), Some(last_section), Some(last_idx)) = (
                                self.last_click_time,
                                self.last_click_section,
                                self.last_click_index,
                            ) {
                                now.duration_since(last_time).as_millis()
                                    < termide_config::constants::DOUBLE_CLICK_INTERVAL_MS
                                    && last_section == Section::Unstaged
                                    && last_idx == idx
                            } else {
                                false
                            };

                        if is_double_click {
                            // Double-click in unstaged = stage file
                            self.unstaged_cursor = idx;
                            self.current_section = Section::Unstaged;
                            self.do_stage();
                            // Reset click state
                            self.last_click_time = None;
                            self.last_click_section = None;
                            self.last_click_index = None;
                        } else {
                            // Single click - select item
                            self.current_section = Section::Unstaged;
                            self.unstaged_cursor = idx;
                            // Save for double-click detection
                            self.last_click_time = Some(now);
                            self.last_click_section = Some(Section::Unstaged);
                            self.last_click_index = Some(idx);
                        }
                    }
                }
                // Check if click is in staged area
                else if self.is_in_rect(col, row, self.staged_area) {
                    // Close any open dropdown
                    self.repo_dropdown_open = false;
                    self.branch_dropdown_open = false;

                    let relative_row = (row - self.staged_area.y) as usize;
                    let idx = self.staged_scroll + relative_row;

                    if idx < self.staged_files.len() {
                        // Check for double-click
                        let is_double_click =
                            if let (Some(last_time), Some(last_section), Some(last_idx)) = (
                                self.last_click_time,
                                self.last_click_section,
                                self.last_click_index,
                            ) {
                                now.duration_since(last_time).as_millis()
                                    < termide_config::constants::DOUBLE_CLICK_INTERVAL_MS
                                    && last_section == Section::Staged
                                    && last_idx == idx
                            } else {
                                false
                            };

                        if is_double_click {
                            // Double-click in staged = unstage file
                            self.staged_cursor = idx;
                            self.current_section = Section::Staged;
                            self.do_unstage();
                            // Reset click state
                            self.last_click_time = None;
                            self.last_click_section = None;
                            self.last_click_index = None;
                        } else {
                            // Single click - select item
                            self.current_section = Section::Staged;
                            self.staged_cursor = idx;
                            // Save for double-click detection
                            self.last_click_time = Some(now);
                            self.last_click_section = Some(Section::Staged);
                            self.last_click_index = Some(idx);
                        }
                    }
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
                            self.dropdown_cursor = self.selected_repo;
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
                    self.last_click_time = None;
                    self.last_click_section = None;
                    self.last_click_index = None;
                }
                // Check if click is on buttons row
                else if row == self.buttons_y {
                    // Close any open dropdown
                    self.repo_dropdown_open = false;
                    self.branch_dropdown_open = false;

                    self.current_section = Section::Buttons;
                    // Calculate which button was clicked based on position
                    let buttons = Button::all();
                    let content_x = self.last_area.x + 1;
                    let mut btn_x = content_x;
                    for (i, button) in buttons.iter().enumerate() {
                        let label = format!("[{}]", button.label());
                        let btn_width = label.width() as u16;
                        if col >= btn_x && col < btn_x + btn_width {
                            self.selected_button = i;
                            // Execute button action on click
                            return self.execute_button();
                        }
                        btn_x += btn_width + 1;
                    }
                    // Reset click state for non-file areas
                    self.last_click_time = None;
                    self.last_click_section = None;
                    self.last_click_index = None;
                }
            }

            _ => {}
        }

        vec![]
    }

    fn to_session(&self, _session_dir: &Path) -> Option<SessionPanel> {
        self.current_repo().map(|repo| SessionPanel::GitStatus {
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
        self.current_repo().map(|p| p.to_path_buf())
    }
}
