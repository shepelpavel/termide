//! Git Log Panel for termide.
//!
//! Provides a panel for viewing git commit history with graph visualization.

use std::any::Any;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use unicode_width::UnicodeWidthStr;

use termide_config::Config;
use termide_core::{Panel, PanelEvent, RenderContext, SessionPanel, ThemeColors};
use termide_git::{self as git, CommitInfo};
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
    /// Available repositories
    repos: Vec<PathBuf>,
    /// Currently selected repository index
    selected_repo: usize,
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
}

impl GitLogPanel {
    /// Create a new Git Log panel from a list of paths (from panels/session)
    pub fn new(paths: &[PathBuf]) -> Self {
        // Find all repos based on paths from panels
        let repos = git::find_repos_from_paths(paths, 2);

        let mut panel = Self {
            repos,
            selected_repo: 0,
            current_section: Section::Commits,
            branch: None,
            commits: Vec::new(),
            selected: 0,
            scroll: 0,
            commit_count: 100, // Load last 100 commits by default
            cached_theme: ThemeColors::default(),
            last_area: Rect::default(),
            status_message: None,
        };

        panel.refresh();
        panel
    }

    /// Create panel for a specific repository (used for session restore)
    pub fn new_for_repo(repo_path: PathBuf) -> Self {
        let mut panel = Self {
            repos: vec![repo_path],
            selected_repo: 0,
            current_section: Section::Commits,
            branch: None,
            commits: Vec::new(),
            selected: 0,
            scroll: 0,
            commit_count: 100,
            cached_theme: ThemeColors::default(),
            last_area: Rect::default(),
            status_message: None,
        };

        panel.refresh();
        panel
    }

    /// Get current repository path
    fn current_repo(&self) -> Option<&Path> {
        self.repos.get(self.selected_repo).map(|p| p.as_path())
    }

    /// Update repository list based on new paths from panels
    pub fn update_repos(&mut self, paths: &[PathBuf]) {
        let current_repo = self.current_repo().map(|p| p.to_path_buf());
        let new_repos = git::find_repos_from_paths(paths, 2);

        if new_repos != self.repos {
            self.repos = new_repos;
            // Try to keep the same repo selected
            if let Some(current) = current_repo {
                self.selected_repo = self.repos.iter().position(|r| r == &current).unwrap_or(0);
            } else {
                self.selected_repo = 0;
            }
            self.refresh();
        }
    }

    /// Refresh the commit log
    pub fn refresh(&mut self) {
        let Some(repo) = self.current_repo() else {
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
                if self.repos.len() > 1 {
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
                if self.repos.len() > 1 {
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

    /// View diff for selected commit
    fn view_diff(&mut self) -> Vec<PanelEvent> {
        let Some(commit) = self.selected_commit() else {
            return vec![];
        };
        if commit.hash.is_empty() {
            return vec![];
        }

        let Some(repo) = self.current_repo() else {
            return vec![];
        };

        // Get diff content
        let Some(diff_content) = git::get_commit_diff(repo, &commit.hash) else {
            self.status_message = Some(format!("Failed to get diff for {}", commit.hash));
            return vec![];
        };

        // Write to temp file
        let temp_path = PathBuf::from(format!("/tmp/termide-diff-{}.diff", commit.hash));
        if let Err(e) = std::fs::write(&temp_path, &diff_content) {
            self.status_message = Some(format!("Failed to write diff: {}", e));
            return vec![];
        }

        // Open in editor
        vec![PanelEvent::OpenFile(temp_path)]
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

        if let Some(repo) = self.current_repo() {
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
        if self.repos.len() > 1 {
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
                    let author = truncate_str(&commit.author, 15);
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
                    let date = truncate_str(&commit.date, 12);
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
            let truncated = if msg.width() > content_area.width as usize {
                &msg[..content_area.width as usize]
            } else {
                msg
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
            let (text, style) = if ref_part.contains("HEAD") {
                // HEAD - red/magenta
                let head_style = if is_selected {
                    Style::default()
                        .fg(theme.selection_fg)
                        .bg(theme.selection_bg)
                } else {
                    Style::default().fg(theme.error)
                };
                (ref_part.to_string(), head_style)
            } else if ref_part.starts_with("tag:") {
                // Tag - yellow
                let tag_style = if is_selected {
                    Style::default()
                        .fg(theme.selection_fg)
                        .bg(theme.selection_bg)
                } else {
                    Style::default().fg(theme.warning)
                };
                (ref_part.to_string(), tag_style)
            } else if ref_part.contains('/') {
                // Remote branch - cyan
                let remote_style = if is_selected {
                    Style::default()
                        .fg(theme.selection_fg)
                        .bg(theme.selection_bg)
                } else {
                    Style::default().fg(theme.info)
                };
                (ref_part.to_string(), remote_style)
            } else {
                // Local branch - green
                let branch_style = if is_selected {
                    Style::default()
                        .fg(theme.selection_fg)
                        .bg(theme.selection_bg)
                } else {
                    Style::default().fg(theme.success)
                };
                (ref_part.to_string(), branch_style)
            };

            let text_width = text.width() as u16;
            if x + text_width <= max_x {
                buf.set_string(x, y, &text, style);
                x += text_width;
            } else {
                // Truncate
                let remaining = (max_x - x) as usize;
                if remaining > 0 {
                    let truncated = truncate_to_width(&text, remaining);
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

/// Truncate string to max characters
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}…", &s[..max.saturating_sub(1)])
    } else {
        s.to_string()
    }
}

/// Truncate string to max display width
fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut width = 0;

    for c in s.chars() {
        let char_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if width + char_width > max_width {
            break;
        }
        result.push(c);
        width += char_width;
    }

    result
}

impl Panel for GitLogPanel {
    fn name(&self) -> &'static str {
        "git_log"
    }

    fn title(&self) -> String {
        let repo_name = self
            .current_repo()
            .map(git::get_repo_name)
            .unwrap_or_else(|| "No repo".to_string());
        let branch = self.branch.as_deref().unwrap_or("(detached)");
        format!("Git Log: {} - {}", repo_name, branch)
    }

    fn prepare_render(&mut self, theme: &Theme, _config: &Config) {
        self.cached_theme = ThemeColors::from(theme);
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
        self.last_area = area;
        self.render_content(area, buf, ctx.is_focused, ctx.border_right_x);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        // Clear status message on any key
        self.status_message = None;

        let page_size = self.last_area.height.saturating_sub(4) as usize;

        match key.code {
            // Tab switches sections
            KeyCode::Tab => {
                self.next_section();
            }
            KeyCode::BackTab => {
                self.prev_section();
            }
            KeyCode::Up | KeyCode::Char('k') => match self.current_section {
                Section::RepoSelector => {
                    if self.selected_repo > 0 {
                        self.selected_repo -= 1;
                        self.selected = 0;
                        self.refresh();
                    }
                }
                Section::Commits => self.move_up(),
            },
            KeyCode::Down | KeyCode::Char('j') => match self.current_section {
                Section::RepoSelector => {
                    if self.selected_repo + 1 < self.repos.len() {
                        self.selected_repo += 1;
                        self.selected = 0;
                        self.refresh();
                    }
                }
                Section::Commits => self.move_down(),
            },
            KeyCode::PageUp => {
                self.page_up(page_size);
            }
            KeyCode::PageDown => {
                self.page_down(page_size);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.go_to_start();
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.go_to_end();
            }
            KeyCode::Enter | KeyCode::Char('d') => {
                // View diff for selected commit
                return self.view_diff();
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.refresh();
                self.status_message = Some("Refreshed".to_string());
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                // Checkout commit (show message for now)
                if let Some(commit) = self.selected_commit() {
                    if !commit.hash.is_empty() {
                        self.status_message =
                            Some(format!("Checkout {} not implemented yet", commit.hash));
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

    fn to_session(&self, _session_dir: &Path) -> Option<SessionPanel> {
        self.current_repo().map(|repo| SessionPanel::GitLog {
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
