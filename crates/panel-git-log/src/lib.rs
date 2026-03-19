//! Git Log Panel for termide.
//!
//! Provides a panel for viewing git commit history with graph visualization.

use std::any::Any;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use unicode_width::UnicodeWidthStr;

use termide_config::{is_go_end, is_go_home, is_move_down, is_move_up, Config};
use termide_core::{
    CommandResult, Panel, PanelCommand, PanelEvent, RenderContext, SessionPanel, ThemeColors,
    WidthPreference,
};
use termide_git::{self as git, truncate_right, truncate_to_width, CommitInfo, RepoManager};
use termide_modal::{ActiveModal, InfoModal, ModalValue, SegmentStyle, StyledSegment};
use termide_state::PendingAction;
use termide_theme::Theme;
use termide_ui::ScrollBar;

/// Section of the Git Log panel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Repository selector (when multiple repos)
    RepoSelector,
    /// Commit list
    Commits,
}

/// Git Log Panel
pub struct GitLogPanel {
    /// Repository manager
    repo_manager: RepoManager,
    /// Current section
    current_section: Section,
    /// Current branch name
    branch: Option<String>,
    /// Commit log entries
    commits: Vec<CommitInfo>,
    /// Currently selected commit index
    selected: usize,
    /// Scroll offset
    scroll: usize,
    /// Number of commits to load
    commit_count: usize,
    /// Cached theme colors
    cached_theme: ThemeColors,
    /// Last render area
    last_area: Rect,
    /// Status message
    status_message: Option<String>,
    /// Cached vim_mode setting for keyboard handling
    vim_mode: bool,
    /// Pending modal request for the app to pick up
    modal_request: Option<(PendingAction, ActiveModal)>,
}

impl GitLogPanel {
    /// Create a new Git Log panel from a list of paths (from panels/session)
    pub fn new(paths: &[PathBuf]) -> Self {
        let mut panel = Self {
            repo_manager: RepoManager::new(paths),
            current_section: Section::Commits,
            branch: None,
            commits: Vec::new(),
            selected: 0,
            scroll: 0,
            commit_count: 100, // Load last 100 commits by default
            cached_theme: ThemeColors::default(),
            last_area: Rect::default(),
            status_message: None,
            vim_mode: false,
            modal_request: None,
        };

        panel.refresh();
        panel
    }

    /// Create panel for a specific repository (used for session restore)
    pub fn new_for_repo(repo_path: PathBuf) -> Self {
        let mut panel = Self {
            repo_manager: RepoManager::for_repo(repo_path),
            current_section: Section::Commits,
            branch: None,
            commits: Vec::new(),
            selected: 0,
            scroll: 0,
            commit_count: 100,
            cached_theme: ThemeColors::default(),
            last_area: Rect::default(),
            status_message: None,
            vim_mode: false,
            modal_request: None,
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

    /// Refresh the commit log
    pub fn refresh(&mut self) {
        let Some(repo) = self.repo_manager.current() else {
            return;
        };
        let repo = repo.to_path_buf();

        self.branch = git::get_current_branch(&repo);
        self.commits = git::get_log_with_graph(&repo, self.commit_count);

        // Adjust selection if needed
        if self.selected >= self.commits.len() && !self.commits.is_empty() {
            self.selected = self.commits.len() - 1;
        }
        // Reset scroll when repo changes
        self.scroll = 0;
    }

    /// Move to next section
    fn next_section(&mut self) {
        self.current_section = match self.current_section {
            Section::RepoSelector => Section::Commits,
            Section::Commits => {
                if self.repo_manager.has_multiple() {
                    Section::RepoSelector
                } else {
                    Section::Commits
                }
            }
        };
    }

    /// Move to previous section
    fn prev_section(&mut self) {
        self.current_section = match self.current_section {
            Section::RepoSelector => Section::Commits,
            Section::Commits => {
                if self.repo_manager.has_multiple() {
                    Section::RepoSelector
                } else {
                    Section::Commits
                }
            }
        };
    }

    /// Move selection up
    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.ensure_visible();
        }
    }

    /// Move selection down
    fn move_down(&mut self) {
        if self.selected + 1 < self.commits.len() {
            self.selected += 1;
            self.ensure_visible();
        }
    }

    /// Page up
    fn page_up(&mut self, page_size: usize) {
        if self.selected > page_size {
            self.selected -= page_size;
        } else {
            self.selected = 0;
        }
        self.ensure_visible();
    }

    /// Page down
    fn page_down(&mut self, page_size: usize) {
        let max = self.commits.len().saturating_sub(1);
        if self.selected + page_size < max {
            self.selected += page_size;
        } else {
            self.selected = max;
        }
        self.ensure_visible();
    }

    /// Go to first commit
    fn go_to_start(&mut self) {
        self.selected = 0;
        self.scroll = 0;
    }

    /// Go to last commit
    fn go_to_end(&mut self) {
        if !self.commits.is_empty() {
            self.selected = self.commits.len() - 1;
            self.ensure_visible();
        }
    }

    /// Ensure selected item is visible
    fn ensure_visible(&mut self) {
        let visible_height = self.last_area.height.saturating_sub(2) as usize;
        if visible_height == 0 {
            return;
        }

        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible_height {
            self.scroll = self.selected - visible_height + 1;
        }
    }

    /// Get selected commit
    fn selected_commit(&self) -> Option<&CommitInfo> {
        self.commits.get(self.selected)
    }

    /// Take pending modal request (called by the app via PanelExt).
    pub fn take_modal_request(&mut self) -> Option<(PendingAction, ActiveModal)> {
        self.modal_request.take()
    }

    /// Show commit info modal for the selected commit.
    fn show_commit_info(&mut self) {
        let Some(commit) = self.selected_commit() else {
            return;
        };
        if commit.hash.is_empty() {
            return;
        }
        let hash = commit.hash.clone();

        let Some(repo) = self.repo_manager.current() else {
            return;
        };
        let repo = repo.to_path_buf();

        let Some(details) = git::get_commit_details(&repo, &hash) else {
            return;
        };

        let t = termide_i18n::t();
        let short_hash = if details.hash.len() > 8 {
            &details.hash[..8]
        } else {
            &details.hash
        };
        let title = t.git_commit_info_title(short_hash);

        // Build colored file status segments (only non-zero counts)
        let mut file_segments = Vec::new();
        if details.files_modified > 0 {
            file_segments.push(StyledSegment {
                text: details.files_modified.to_string(),
                style: SegmentStyle::Warning,
            });
            file_segments.push(StyledSegment {
                text: format!(" {}", t.git_commit_files_modified()),
                style: SegmentStyle::Default,
            });
        }
        if details.files_added > 0 {
            if !file_segments.is_empty() {
                file_segments.push(StyledSegment {
                    text: "  ".to_string(),
                    style: SegmentStyle::Default,
                });
            }
            file_segments.push(StyledSegment {
                text: details.files_added.to_string(),
                style: SegmentStyle::Success,
            });
            file_segments.push(StyledSegment {
                text: format!(" {}", t.git_commit_files_added()),
                style: SegmentStyle::Default,
            });
        }
        if details.files_deleted > 0 {
            if !file_segments.is_empty() {
                file_segments.push(StyledSegment {
                    text: "  ".to_string(),
                    style: SegmentStyle::Default,
                });
            }
            file_segments.push(StyledSegment {
                text: details.files_deleted.to_string(),
                style: SegmentStyle::Error,
            });
            file_segments.push(StyledSegment {
                text: format!(" {}", t.git_commit_files_deleted()),
                style: SegmentStyle::Default,
            });
        }
        // Fallback if all zero
        if file_segments.is_empty() {
            file_segments.push(StyledSegment {
                text: "0".to_string(),
                style: SegmentStyle::Disabled,
            });
        }

        // Build colored lines segments (+N green, -N red)
        let lines_segments = vec![
            StyledSegment {
                text: format!("+{}", details.insertions),
                style: SegmentStyle::Success,
            },
            StyledSegment {
                text: " / ".to_string(),
                style: SegmentStyle::Default,
            },
            StyledSegment {
                text: format!("-{}", details.deletions),
                style: SegmentStyle::Error,
            },
        ];

        let data = vec![
            (
                t.git_commit_author().to_string(),
                ModalValue::Text(details.author),
            ),
            (
                t.git_commit_date().to_string(),
                ModalValue::Text(details.date),
            ),
            (
                t.git_commit_message().to_string(),
                ModalValue::Text(details.message),
            ),
            (
                t.git_commit_files().to_string(),
                ModalValue::Segments(file_segments),
            ),
            (
                t.git_commit_lines().to_string(),
                ModalValue::Segments(lines_segments),
            ),
        ];

        let modal = InfoModal::new_rich(title, data);
        self.modal_request = Some((
            PendingAction::VfsMessage,
            ActiveModal::Info(Box::new(modal)),
        ));
    }

    /// Open selected commit in browser via remote URL, or show fallback message.
    fn open_commit_external(&mut self) -> Vec<PanelEvent> {
        let Some(commit) = self.selected_commit() else {
            return vec![];
        };
        if commit.hash.is_empty() {
            return vec![];
        }
        let hash = commit.hash.clone();

        let Some(repo) = self.repo_manager.current() else {
            return vec![];
        };

        if let Some(url) = git::get_commit_web_url(repo, &hash) {
            return vec![PanelEvent::OpenExternal(PathBuf::from(url))];
        }

        let t = termide_i18n::t();
        self.status_message = Some(t.git_no_remote_url().to_string());
        vec![]
    }

    /// View diff for selected commit
    fn view_diff(&mut self) -> Vec<PanelEvent> {
        let Some(commit) = self.selected_commit() else {
            return vec![];
        };
        if commit.hash.is_empty() {
            return vec![];
        }

        let Some(repo) = self.repo_manager.current() else {
            return vec![];
        };

        // Open Git Diff panel for this commit
        vec![PanelEvent::OpenGitDiff {
            repo_path: repo.to_path_buf(),
            commit_hash: Some(commit.hash.clone()),
        }]
    }

    /// Render repo selector
    fn render_repo_selector(
        &self,
        x: u16,
        y: u16,
        _width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
    ) {
        let is_focused = self.current_section == Section::RepoSelector;
        let style = if is_focused {
            Style::default()
                .fg(theme.selection_fg)
                .bg(theme.selection_bg)
        } else {
            Style::default().fg(theme.fg)
        };

        if let Some(repo) = self.repo_manager.current() {
            let name = git::get_repo_name(repo);
            let text = format!("[{}]", name);
            buf.set_string(x, y, &text, style);
        }
    }

    /// Render the panel content
    fn render_content(
        &self,
        area: Rect,
        buf: &mut Buffer,
        is_focused: bool,
        border_right_x: Option<u16>,
    ) {
        if area.height < 3 || area.width < 10 {
            return;
        }

        let theme = &self.cached_theme;

        let content_area = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };

        let mut y_offset = 0u16;

        // Render repo selector if multiple repos
        if self.repo_manager.has_multiple() {
            self.render_repo_selector(
                content_area.x,
                content_area.y,
                content_area.width,
                buf,
                theme,
            );
            y_offset = 2;
        }

        let commits_area_height = content_area.height.saturating_sub(y_offset) as usize;
        let commits_start_y = content_area.y + y_offset;

        // Check if scrollbar is needed (will be rendered on border, so no width reservation)
        let needs_scrollbar = ScrollBar::needs_scrollbar(commits_area_height, self.commits.len());
        let commits_width = content_area.width;

        // Render commits
        for (i, commit) in self
            .commits
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(commits_area_height)
        {
            let y = commits_start_y + (i - self.scroll) as u16;
            let is_selected = i == self.selected && self.current_section == Section::Commits;

            // Clear line first
            let clear_style = if is_selected {
                Style::default().bg(theme.selection_bg)
            } else {
                Style::default().bg(theme.bg)
            };
            for x in content_area.x..content_area.x + commits_width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_style(clear_style);
                }
            }

            let mut x_pos = content_area.x;
            let max_x = content_area.x + commits_width;

            // Graph prefix (if available)
            if let Some(ref graph) = commit.graph {
                let graph_style = if is_selected {
                    Style::default()
                        .fg(theme.selection_fg)
                        .bg(theme.selection_bg)
                } else {
                    Style::default().fg(theme.disabled)
                };
                buf.set_string(x_pos, y, graph, graph_style);
                x_pos += graph.width() as u16;
            }

            if !commit.hash.is_empty() && x_pos < max_x {
                // Hash
                let hash_style = if is_selected {
                    Style::default()
                        .fg(theme.selection_fg)
                        .bg(theme.selection_bg)
                } else {
                    Style::default().fg(theme.cursor)
                };
                buf.set_string(x_pos, y, &commit.hash, hash_style);
                x_pos += commit.hash.width() as u16 + 1;

                // Refs (if any) - render with colors
                if let Some(ref refs) = commit.refs {
                    if x_pos < max_x {
                        x_pos = self.render_refs(x_pos, y, max_x, refs, is_selected, buf, theme);
                        x_pos += 1; // space after refs
                    }
                }

                // Author
                if x_pos < max_x {
                    let author = truncate_right(&commit.author, 15);
                    let author_style = if is_selected {
                        Style::default()
                            .fg(theme.selection_fg)
                            .bg(theme.selection_bg)
                    } else {
                        Style::default().fg(theme.info)
                    };
                    buf.set_string(x_pos, y, &author, author_style);
                    x_pos += author.width() as u16 + 1;
                }

                // Date
                if x_pos < max_x {
                    let date = truncate_right(&commit.date, 12);
                    let date_style = if is_selected {
                        Style::default()
                            .fg(theme.selection_fg)
                            .bg(theme.selection_bg)
                    } else {
                        Style::default().fg(theme.disabled)
                    };
                    buf.set_string(x_pos, y, &date, date_style);
                    x_pos += date.width() as u16 + 1;
                }

                // Message
                if x_pos < max_x {
                    let remaining = (max_x - x_pos) as usize;
                    let message = if commit.message.width() > remaining {
                        truncate_to_width(&commit.message, remaining)
                    } else {
                        commit.message.clone()
                    };
                    let msg_style = if is_selected {
                        Style::default()
                            .fg(theme.selection_fg)
                            .bg(theme.selection_bg)
                    } else {
                        Style::default().fg(theme.fg)
                    };
                    buf.set_string(x_pos, y, &message, msg_style);
                }
            }
        }

        // Render scrollbar on border
        if needs_scrollbar {
            if let Some(border_x) = border_right_x {
                ScrollBar::render(
                    buf,
                    border_x,
                    commits_start_y,
                    commits_area_height as u16,
                    self.scroll,
                    commits_area_height,
                    self.commits.len(),
                    theme,
                    is_focused,
                );
            }
        }

        // Status message at bottom
        if let Some(ref msg) = self.status_message {
            let msg_y = area.y + area.height.saturating_sub(2);
            let style = Style::default().fg(theme.cursor);
            let max_w = content_area.width as usize;
            let truncated: std::borrow::Cow<str> = if msg.width() > max_w {
                let mut end = 0;
                let mut w = 0;
                for ch in msg.chars() {
                    let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                    if w + cw > max_w {
                        break;
                    }
                    w += cw;
                    end += ch.len_utf8();
                }
                std::borrow::Cow::Borrowed(&msg[..end])
            } else {
                std::borrow::Cow::Borrowed(msg)
            };
            buf.set_string(content_area.x, msg_y, truncated, style);
        }
    }

    /// Render refs with colors
    /// Returns the x position after rendering
    #[allow(clippy::too_many_arguments)]
    fn render_refs(
        &self,
        start_x: u16,
        y: u16,
        max_x: u16,
        refs: &str,
        is_selected: bool,
        buf: &mut Buffer,
        theme: &ThemeColors,
    ) -> u16 {
        let mut x = start_x;

        // Refs format: (HEAD -> main, origin/main, tag: v1.0)
        // Remove parentheses
        let refs_inner = refs.trim_start_matches('(').trim_end_matches(')');
        if refs_inner.is_empty() {
            return x;
        }

        // Render opening paren
        let paren_style = if is_selected {
            Style::default()
                .fg(theme.selection_fg)
                .bg(theme.selection_bg)
        } else {
            Style::default().fg(theme.disabled)
        };
        buf.set_string(x, y, "(", paren_style);
        x += 1;

        // Parse and render each ref
        for (i, ref_part) in refs_inner.split(", ").enumerate() {
            if x >= max_x {
                break;
            }

            // Add comma separator
            if i > 0 {
                buf.set_string(x, y, ", ", paren_style);
                x += 2;
            }

            if x >= max_x {
                break;
            }

            // Determine ref type and color
            let style = if is_selected {
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.selection_bg)
            } else {
                let fg = if ref_part.contains("HEAD") {
                    theme.error
                } else if ref_part.starts_with("tag:") {
                    theme.warning
                } else if ref_part.contains('/') {
                    theme.info
                } else {
                    theme.success
                };
                Style::default().fg(fg)
            };

            let text_width = ref_part.width() as u16;
            if x + text_width <= max_x {
                buf.set_string(x, y, ref_part, style);
                x += text_width;
            } else {
                // Truncate
                let remaining = (max_x - x) as usize;
                if remaining > 0 {
                    let truncated = truncate_to_width(ref_part, remaining);
                    buf.set_string(x, y, &truncated, style);
                    x = max_x;
                }
                break;
            }
        }

        // Render closing paren
        if x < max_x {
            buf.set_string(x, y, ")", paren_style);
            x += 1;
        }

        x
    }
}

impl Panel for GitLogPanel {
    fn name(&self) -> &'static str {
        "git_log"
    }

    fn width_preference(&self) -> WidthPreference {
        WidthPreference::PreferWide
    }

    fn title(&self) -> String {
        let t = termide_i18n::t();
        let repo_name = self
            .repo_manager
            .current()
            .map(git::get_repo_name)
            .unwrap_or_else(|| t.git_no_repo().to_string());
        let branch = self.branch.as_deref().unwrap_or(t.git_branch_detached());
        t.git_log_title_fmt(&repo_name, branch)
    }

    fn prepare_render(&mut self, theme: &Theme, config: &Config) {
        self.cached_theme = ThemeColors::from(theme);
        self.vim_mode = config.general.vim_mode;
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
        self.last_area = area;
        self.render_content(area, buf, ctx.is_focused, ctx.border_right_x);
    }

    fn handle_command(&mut self, cmd: PanelCommand<'_>) -> CommandResult {
        match cmd {
            PanelCommand::Reload => {
                self.refresh();
                CommandResult::NeedsRedraw(true)
            }
            PanelCommand::UpdateRepoPaths { paths } => {
                self.update_repos(&paths);
                CommandResult::NeedsRedraw(true)
            }
            _ => CommandResult::None,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        // Clear status message on any key
        self.status_message = None;

        let page_size = self.last_area.height.saturating_sub(4) as usize;

        // Vim-aware navigation (j/k/g/G when vim_mode is enabled)
        if is_move_up(&key, self.vim_mode) {
            match self.current_section {
                Section::RepoSelector => {
                    if self.repo_manager.selected_index() > 0 {
                        self.repo_manager.select_prev();
                        self.selected = 0;
                        self.refresh();
                    }
                }
                Section::Commits => self.move_up(),
            }
            return vec![];
        }
        if is_move_down(&key, self.vim_mode) {
            match self.current_section {
                Section::RepoSelector => {
                    if self.repo_manager.selected_index() + 1 < self.repo_manager.len() {
                        self.repo_manager.select_next();
                        self.selected = 0;
                        self.refresh();
                    }
                }
                Section::Commits => self.move_down(),
            }
            return vec![];
        }
        if is_go_home(&key, self.vim_mode) {
            self.go_to_start();
            return vec![];
        }
        if is_go_end(&key, self.vim_mode) {
            self.go_to_end();
            return vec![];
        }

        match key.code {
            // Tab switches sections
            KeyCode::Tab => {
                self.next_section();
            }
            KeyCode::BackTab => {
                self.prev_section();
            }
            KeyCode::PageUp => {
                self.page_up(page_size);
            }
            KeyCode::PageDown => {
                self.page_down(page_size);
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                return self.open_commit_external();
            }
            KeyCode::Enter | KeyCode::Char('d') => {
                // View diff for selected commit
                return self.view_diff();
            }
            KeyCode::Char(' ') => {
                // Show commit info modal
                self.show_commit_info();
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.refresh();
                let t = termide_i18n::t();
                self.status_message = Some(t.git_refreshed().to_string());
            }
            KeyCode::Char('o') => {
                return self.open_commit_external();
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                // Checkout commit (show message for now)
                if let Some(commit) = self.selected_commit() {
                    if !commit.hash.is_empty() {
                        let t = termide_i18n::t();
                        self.status_message = Some(format!(
                            "Checkout {} {}",
                            commit.hash,
                            t.git_checkout_not_impl()
                        ));
                    }
                }
            }
            _ => {}
        }

        vec![]
    }

    fn handle_mouse(&mut self, event: MouseEvent, _panel_area: Rect) -> Vec<PanelEvent> {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Calculate which commit was clicked
                let content_y = self.last_area.y + 1;
                if event.row >= content_y {
                    let clicked_idx = self.scroll + (event.row - content_y) as usize;
                    if clicked_idx < self.commits.len() {
                        self.selected = clicked_idx;
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                self.move_up();
            }
            MouseEventKind::ScrollDown => {
                self.move_down();
            }
            _ => {}
        }
        vec![]
    }

    fn handle_scroll(&mut self, delta: i32, _panel_area: Rect) -> Vec<PanelEvent> {
        let lines = delta.unsigned_abs() as usize;
        if delta < 0 {
            // Scroll up - move selection up by delta
            self.selected = self.selected.saturating_sub(lines);
        } else {
            // Scroll down - move selection down by delta
            self.selected = (self.selected + lines).min(self.commits.len().saturating_sub(1));
        }
        self.ensure_visible();
        vec![]
    }

    fn to_session(&self, _session_dir: &Path) -> Option<SessionPanel> {
        self.repo_manager
            .current()
            .map(|repo| SessionPanel::GitLog {
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
