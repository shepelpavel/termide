//! Git Diff Panel for termide.
//!
//! Provides a panel for viewing all git diffs with syntax highlighting.

use std::any::Any;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use unicode_width::UnicodeWidthStr;

use termide_config::{is_go_end, is_go_home, is_move_down, is_move_up, Config};
use termide_core::{Panel, PanelEvent, RenderContext, SessionPanel, ThemeColors, WidthPreference};
use termide_git::{self as git};
use termide_theme::Theme;
use termide_ui::ScrollBar;

/// Type of diff line
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Context line (unchanged)
    Context,
    /// Added line
    Added,
    /// Removed line
    Removed,
    /// Hunk header (@@ ... @@)
    HunkHeader,
}

/// A single line in the diff
#[derive(Debug, Clone)]
pub struct DiffLine {
    /// Type of line
    pub kind: LineKind,
    /// Line content (without +/- prefix)
    pub content: String,
    /// Old line number (if applicable)
    pub old_line: Option<usize>,
    /// New line number (if applicable)
    pub new_line: Option<usize>,
}

/// A hunk (block of changes)
#[derive(Debug, Clone)]
pub struct DiffHunk {
    /// Header line (@@ -x,y +a,b @@)
    pub header: String,
    /// Lines in this hunk
    pub lines: Vec<DiffLine>,
}

/// File status in diff
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// Diff information for a single file
#[derive(Debug, Clone)]
pub struct FileDiff {
    /// File path
    pub path: String,
    /// File status
    pub status: FileStatus,
    /// Is this a staged change
    pub staged: bool,
    /// Number of additions
    pub additions: usize,
    /// Number of deletions
    pub deletions: usize,
    /// Hunks
    pub hunks: Vec<DiffHunk>,
}

/// Git Diff Panel
pub struct GitDiffPanel {
    /// Repository path
    repo_path: PathBuf,
    /// Commit hash (None = working directory changes, Some = specific commit)
    commit_hash: Option<String>,
    /// All file diffs
    diffs: Vec<FileDiff>,
    /// Vertical scroll offset (in lines)
    scroll: usize,
    /// Set of collapsed file indices
    collapsed: HashSet<usize>,
    /// Selected file index
    selected_file: usize,
    /// Cached theme colors
    cached_theme: ThemeColors,
    /// Last render area
    last_area: Rect,
    /// Total number of renderable lines (for scrollbar)
    total_lines: usize,
    /// Visible height
    visible_height: usize,
    /// Status message
    status_message: Option<String>,
    /// Cached vim_mode setting for keyboard handling
    vim_mode: bool,
}

impl GitDiffPanel {
    /// Create a new Git Diff panel for working directory changes
    pub fn new(repo_path: PathBuf) -> Self {
        let mut panel = Self {
            repo_path,
            commit_hash: None,
            diffs: Vec::new(),
            scroll: 0,
            collapsed: HashSet::new(),
            selected_file: 0,
            cached_theme: ThemeColors::default(),
            last_area: Rect::default(),
            total_lines: 0,
            visible_height: 0,
            status_message: None,
            vim_mode: false,
        };
        panel.refresh();
        panel
    }

    /// Create a new Git Diff panel for a specific commit
    pub fn new_for_commit(repo_path: PathBuf, commit_hash: String) -> Self {
        let mut panel = Self {
            repo_path,
            commit_hash: Some(commit_hash),
            diffs: Vec::new(),
            scroll: 0,
            collapsed: HashSet::new(),
            selected_file: 0,
            cached_theme: ThemeColors::default(),
            last_area: Rect::default(),
            total_lines: 0,
            visible_height: 0,
            status_message: None,
            vim_mode: false,
        };
        panel.refresh();
        panel
    }

    /// Refresh diff data
    pub fn refresh(&mut self) {
        self.diffs.clear();

        if let Some(hash) = self.commit_hash.clone() {
            // Load commit diff
            self.load_commit_diff(&hash);
        } else {
            // Load working directory changes
            // Get unstaged files
            let unstaged = git::get_unstaged_files(&self.repo_path);
            for file in unstaged {
                if let Some(diff) = self.load_file_diff(&file.path, false, file.status) {
                    self.diffs.push(diff);
                }
            }

            // Get staged files
            let staged = git::get_staged_files(&self.repo_path);
            for file in staged {
                if let Some(diff) = self.load_file_diff(&file.path, true, file.status) {
                    self.diffs.push(diff);
                }
            }
        }

        // Reset selection if needed
        if self.selected_file >= self.diffs.len() && !self.diffs.is_empty() {
            self.selected_file = self.diffs.len() - 1;
        }

        self.calculate_total_lines();
    }

    /// Load diff for a specific commit
    fn load_commit_diff(&mut self, hash: &str) {
        let Some(diff_output) = git::get_commit_diff(&self.repo_path, hash) else {
            return;
        };

        // Parse git show output (contains multiple files)
        // Format:
        // commit <hash>
        // Author: <author>
        // Date:   <date>
        //
        //     <message>
        //
        // diff --git a/<file> b/<file>
        // --- a/<file>
        // +++ b/<file>
        // @@ ... @@
        // ...

        let mut current_file: Option<FileDiff> = None;
        let mut current_hunk: Option<DiffHunk> = None;
        let mut old_line = 0usize;
        let mut new_line = 0usize;

        for line in diff_output.lines() {
            // Start of a new file diff
            if line.starts_with("diff --git ") {
                // Save previous hunk if exists
                if let Some(hunk) = current_hunk.take() {
                    if let Some(ref mut file) = current_file {
                        file.hunks.push(hunk);
                    }
                }
                // Save previous file if exists
                if let Some(file) = current_file.take() {
                    self.diffs.push(file);
                }

                // Parse file path from "diff --git a/<path> b/<path>"
                let path = line
                    .strip_prefix("diff --git ")
                    .and_then(|s| s.split_once(' '))
                    .map(|(a, _)| a.strip_prefix("a/").unwrap_or(a))
                    .unwrap_or("")
                    .to_string();

                current_file = Some(FileDiff {
                    path,
                    status: FileStatus::Modified, // Default, will be updated
                    staged: false,                // Not applicable for commits
                    additions: 0,
                    deletions: 0,
                    hunks: Vec::new(),
                });
            } else if line.starts_with("new file mode") {
                if let Some(ref mut file) = current_file {
                    file.status = FileStatus::Added;
                }
            } else if line.starts_with("deleted file mode") {
                if let Some(ref mut file) = current_file {
                    file.status = FileStatus::Deleted;
                }
            } else if line.starts_with("rename from") || line.starts_with("rename to") {
                if let Some(ref mut file) = current_file {
                    file.status = FileStatus::Renamed;
                }
            } else if line.starts_with("@@") {
                // Save previous hunk if exists
                if let Some(hunk) = current_hunk.take() {
                    if let Some(ref mut file) = current_file {
                        file.hunks.push(hunk);
                    }
                }

                let (old_start, new_start) = Self::parse_hunk_header(line);
                old_line = old_start;
                new_line = new_start;

                current_hunk = Some(DiffHunk {
                    header: line.to_string(),
                    lines: Vec::new(),
                });
            } else if let Some(ref mut hunk) = current_hunk {
                if let Some(rest) = line.strip_prefix('+') {
                    hunk.lines.push(DiffLine {
                        kind: LineKind::Added,
                        content: rest.to_string(),
                        old_line: None,
                        new_line: Some(new_line),
                    });
                    if let Some(ref mut file) = current_file {
                        file.additions += 1;
                    }
                    new_line += 1;
                } else if let Some(rest) = line.strip_prefix('-') {
                    hunk.lines.push(DiffLine {
                        kind: LineKind::Removed,
                        content: rest.to_string(),
                        old_line: Some(old_line),
                        new_line: None,
                    });
                    if let Some(ref mut file) = current_file {
                        file.deletions += 1;
                    }
                    old_line += 1;
                } else if let Some(rest) = line.strip_prefix(' ') {
                    hunk.lines.push(DiffLine {
                        kind: LineKind::Context,
                        content: rest.to_string(),
                        old_line: Some(old_line),
                        new_line: Some(new_line),
                    });
                    old_line += 1;
                    new_line += 1;
                }
            }
        }

        // Don't forget the last hunk and file
        if let Some(hunk) = current_hunk {
            if let Some(ref mut file) = current_file {
                file.hunks.push(hunk);
            }
        }
        if let Some(file) = current_file {
            self.diffs.push(file);
        }
    }

    /// Load diff for a single file
    fn load_file_diff(&self, path: &Path, staged: bool, status_char: char) -> Option<FileDiff> {
        let status = match status_char {
            'A' => FileStatus::Added,
            'D' => FileStatus::Deleted,
            'R' => FileStatus::Renamed,
            _ => FileStatus::Modified,
        };

        let diff_text = git::get_file_diff(&self.repo_path, path, staged)?;
        let stats = git::get_file_diff_stats(&self.repo_path, path, staged);

        let hunks = Self::parse_diff(&diff_text);

        Some(FileDiff {
            path: path.to_string_lossy().to_string(),
            status,
            staged,
            additions: stats.additions,
            deletions: stats.deletions,
            hunks,
        })
    }

    /// Parse unified diff output into hunks
    fn parse_diff(diff_text: &str) -> Vec<DiffHunk> {
        let mut hunks = Vec::new();
        let mut current_hunk: Option<DiffHunk> = None;
        let mut old_line = 0usize;
        let mut new_line = 0usize;

        for line in diff_text.lines() {
            if line.starts_with("@@") {
                // Save previous hunk if exists
                if let Some(hunk) = current_hunk.take() {
                    hunks.push(hunk);
                }

                // Parse hunk header: @@ -old_start,old_count +new_start,new_count @@
                let (old_start, new_start) = Self::parse_hunk_header(line);
                old_line = old_start;
                new_line = new_start;

                current_hunk = Some(DiffHunk {
                    header: line.to_string(),
                    lines: Vec::new(),
                });
            } else if let Some(ref mut hunk) = current_hunk {
                let (kind, content) = if let Some(rest) = line.strip_prefix('+') {
                    let diff_line = DiffLine {
                        kind: LineKind::Added,
                        content: rest.to_string(),
                        old_line: None,
                        new_line: Some(new_line),
                    };
                    new_line += 1;
                    (LineKind::Added, diff_line)
                } else if let Some(rest) = line.strip_prefix('-') {
                    let diff_line = DiffLine {
                        kind: LineKind::Removed,
                        content: rest.to_string(),
                        old_line: Some(old_line),
                        new_line: None,
                    };
                    old_line += 1;
                    (LineKind::Removed, diff_line)
                } else if let Some(rest) = line.strip_prefix(' ') {
                    let diff_line = DiffLine {
                        kind: LineKind::Context,
                        content: rest.to_string(),
                        old_line: Some(old_line),
                        new_line: Some(new_line),
                    };
                    old_line += 1;
                    new_line += 1;
                    (LineKind::Context, diff_line)
                } else {
                    // Line without prefix (shouldn't happen in unified diff)
                    continue;
                };

                let _ = kind; // silence unused warning
                hunk.lines.push(content);
            }
        }

        // Don't forget the last hunk
        if let Some(hunk) = current_hunk {
            hunks.push(hunk);
        }

        hunks
    }

    /// Parse hunk header to get start line numbers
    fn parse_hunk_header(header: &str) -> (usize, usize) {
        // Format: @@ -old_start,old_count +new_start,new_count @@
        let re = regex::Regex::new(r"@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@").ok();
        if let Some(re) = re {
            if let Some(caps) = re.captures(header) {
                let old_start: usize = caps
                    .get(1)
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(1);
                let new_start: usize = caps
                    .get(2)
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(1);
                return (old_start, new_start);
            }
        }
        (1, 1)
    }

    /// Calculate total number of renderable lines
    fn calculate_total_lines(&mut self) {
        let mut total = 0;
        for (i, diff) in self.diffs.iter().enumerate() {
            // File header line
            total += 1;
            // If not collapsed, add hunk lines
            if !self.collapsed.contains(&i) {
                for hunk in &diff.hunks {
                    total += 1; // Hunk header
                    total += hunk.lines.len();
                }
            }
        }
        self.total_lines = total;
    }

    /// Toggle collapse state for selected file
    fn toggle_collapse(&mut self) {
        if self.selected_file < self.diffs.len() {
            if self.collapsed.contains(&self.selected_file) {
                self.collapsed.remove(&self.selected_file);
            } else {
                self.collapsed.insert(self.selected_file);
            }
            self.calculate_total_lines();
        }
    }

    /// Move selection up
    fn move_up(&mut self) {
        if self.selected_file > 0 {
            self.selected_file -= 1;
            self.ensure_file_visible();
        }
    }

    /// Move selection down
    fn move_down(&mut self) {
        if self.selected_file + 1 < self.diffs.len() {
            self.selected_file += 1;
            self.ensure_file_visible();
        }
    }

    /// Scroll up
    fn scroll_up(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_sub(amount);
    }

    /// Scroll down
    fn scroll_down(&mut self, amount: usize) {
        let max_scroll = self.total_lines.saturating_sub(self.visible_height);
        self.scroll = (self.scroll + amount).min(max_scroll);
    }

    /// Page up
    fn page_up(&mut self) {
        self.scroll_up(self.visible_height.saturating_sub(2));
    }

    /// Page down
    fn page_down(&mut self) {
        self.scroll_down(self.visible_height.saturating_sub(2));
    }

    /// Go to start
    fn go_to_start(&mut self) {
        self.scroll = 0;
        self.selected_file = 0;
    }

    /// Go to end
    fn go_to_end(&mut self) {
        if !self.diffs.is_empty() {
            self.selected_file = self.diffs.len() - 1;
            let max_scroll = self.total_lines.saturating_sub(self.visible_height);
            self.scroll = max_scroll;
        }
    }

    /// Ensure selected file is visible
    fn ensure_file_visible(&mut self) {
        // Calculate line number where selected file starts
        let mut line = 0;
        for i in 0..self.selected_file {
            line += 1; // File header
            if !self.collapsed.contains(&i) {
                for hunk in &self.diffs[i].hunks {
                    line += 1 + hunk.lines.len();
                }
            }
        }

        // Adjust scroll to make file visible
        if line < self.scroll {
            self.scroll = line;
        } else if line >= self.scroll + self.visible_height {
            self.scroll = line.saturating_sub(self.visible_height) + 1;
        }
    }

    /// Open selected file in editor
    fn open_file(&self) -> Vec<PanelEvent> {
        if let Some(diff) = self.diffs.get(self.selected_file) {
            let file_path = self.repo_path.join(&diff.path);
            if file_path.exists() {
                return vec![PanelEvent::OpenFile(file_path)];
            }
        }
        vec![]
    }

    /// Render the panel content
    fn render_content(
        &mut self,
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

        self.visible_height = content_area.height as usize;

        // Colors for diff
        let added_style = Style::default()
            .fg(theme.success)
            .bg(ratatui::style::Color::Rgb(0, 40, 0));
        let removed_style = Style::default()
            .fg(theme.error)
            .bg(ratatui::style::Color::Rgb(40, 0, 0));
        let context_style = Style::default().fg(theme.fg);
        let hunk_header_style = Style::default().fg(theme.disabled);
        let file_header_style = Style::default().fg(theme.info);
        let file_header_selected_style = Style::default()
            .fg(theme.selection_fg)
            .bg(theme.selection_bg);
        let line_number_style = Style::default().fg(theme.disabled);

        let line_num_width = 4; // Width for each line number column

        let mut current_line = 0usize;
        let mut y = content_area.y;
        let max_y = content_area.y + content_area.height;

        for (file_idx, diff) in self.diffs.iter().enumerate() {
            if y >= max_y {
                break;
            }

            // File header line
            if current_line >= self.scroll && y < max_y {
                let is_selected = file_idx == self.selected_file;
                let is_collapsed = self.collapsed.contains(&file_idx);

                // Use same style for entire line (text + lines) for uniformity
                // Selected: inverted colors for whole line
                let header_style = if is_selected {
                    file_header_selected_style
                } else {
                    file_header_style
                };

                // Build header components
                let collapse_btn = if is_collapsed { "[▶]" } else { "[▼]" };
                let status_char = match diff.status {
                    FileStatus::Added => "+",
                    FileStatus::Deleted => "-",
                    FileStatus::Modified => "~",
                    FileStatus::Renamed => "R",
                };
                let stats = format!("(+{} -{})", diff.additions, diff.deletions);
                let staged_marker = if diff.staged { " [staged]" } else { "" };

                // Format: ─[▼] ~ path (+N -M) [staged] ─────────
                let title_text = format!(
                    "{} {} {} {}{}",
                    collapse_btn, status_char, &diff.path, stats, staged_marker
                );
                let title_width = title_text.width();

                // Render from edge to edge (panel boundaries, no padding)
                let header_x = area.x;
                let header_end = area.x + area.width;
                let mut x = header_x;

                // Leading line ─
                buf.set_string(x, y, "─", header_style);
                x += 1;

                // Title text
                buf.set_string(x, y, &title_text, header_style);
                x += title_width as u16;

                // Space
                buf.set_string(x, y, " ", header_style);
                x += 1;

                // Trailing line ─────────
                while x < header_end {
                    buf.set_string(x, y, "─", header_style);
                    x += 1;
                }

                y += 1;
            }
            current_line += 1;

            // Skip content if collapsed
            if self.collapsed.contains(&file_idx) {
                continue;
            }

            // Render hunks
            for hunk in &diff.hunks {
                if y >= max_y {
                    break;
                }

                // Hunk header
                if current_line >= self.scroll && y < max_y {
                    // Truncate hunk header to fit
                    let header = if hunk.header.width() > content_area.width as usize {
                        git::truncate_to_width(&hunk.header, content_area.width as usize)
                    } else {
                        hunk.header.clone()
                    };
                    buf.set_string(content_area.x, y, &header, hunk_header_style);
                    y += 1;
                }
                current_line += 1;

                // Hunk lines
                for line in &hunk.lines {
                    if y >= max_y {
                        break;
                    }

                    if current_line >= self.scroll && y < max_y {
                        let (style, prefix) = match line.kind {
                            LineKind::Added => (added_style, "+"),
                            LineKind::Removed => (removed_style, "-"),
                            LineKind::Context => (context_style, " "),
                            LineKind::HunkHeader => (hunk_header_style, " "),
                        };

                        // Clear line with appropriate background
                        for x in content_area.x..content_area.x + content_area.width {
                            if let Some(cell) = buf.cell_mut((x, y)) {
                                cell.set_char(' ');
                                cell.set_style(style);
                            }
                        }

                        let mut x = content_area.x;

                        // Old line number
                        let old_num = line
                            .old_line
                            .map(|n| format!("{:>width$}", n, width = line_num_width))
                            .unwrap_or_else(|| " ".repeat(line_num_width));
                        buf.set_string(x, y, &old_num, line_number_style);
                        x += line_num_width as u16;

                        buf.set_string(x, y, "|", line_number_style);
                        x += 1;

                        // New line number
                        let new_num = line
                            .new_line
                            .map(|n| format!("{:>width$}", n, width = line_num_width))
                            .unwrap_or_else(|| " ".repeat(line_num_width));
                        buf.set_string(x, y, &new_num, line_number_style);
                        x += line_num_width as u16;

                        buf.set_string(x, y, "|", line_number_style);
                        x += 1;

                        // Prefix (+/-)
                        buf.set_string(x, y, prefix, style);
                        x += 1;

                        // Content
                        let remaining_width =
                            (content_area.x + content_area.width).saturating_sub(x) as usize;
                        let content = if line.content.width() > remaining_width {
                            git::truncate_to_width(&line.content, remaining_width)
                        } else {
                            line.content.clone()
                        };
                        buf.set_string(x, y, &content, style);

                        y += 1;
                    }
                    current_line += 1;
                }
            }
        }

        // Render scrollbar
        let needs_scrollbar = ScrollBar::needs_scrollbar(self.visible_height, self.total_lines);
        if needs_scrollbar {
            if let Some(border_x) = border_right_x {
                ScrollBar::render(
                    buf,
                    border_x,
                    content_area.y,
                    content_area.height,
                    self.scroll,
                    self.visible_height,
                    self.total_lines,
                    theme,
                    is_focused,
                );
            }
        }

        // Status message
        if let Some(ref msg) = self.status_message {
            let msg_y = area.y + area.height.saturating_sub(2);
            let style = Style::default().fg(theme.cursor);
            buf.set_string(content_area.x, msg_y, msg, style);
        }
    }
}

impl Panel for GitDiffPanel {
    fn name(&self) -> &'static str {
        "git_diff"
    }

    fn width_preference(&self) -> WidthPreference {
        WidthPreference::PreferWide
    }

    fn title(&self) -> String {
        let repo_name = git::get_repo_name(&self.repo_path);
        let file_count = self.diffs.len();
        if let Some(ref hash) = self.commit_hash {
            // Show short hash (first 7 characters)
            let short_hash = if hash.len() > 7 { &hash[..7] } else { hash };
            format!(
                "Git Diff: {} @ {} ({} files)",
                repo_name, short_hash, file_count
            )
        } else {
            format!("Git Diff: {} ({} files)", repo_name, file_count)
        }
    }

    fn prepare_render(&mut self, theme: &Theme, config: &Config) {
        self.cached_theme = ThemeColors::from(theme);
        self.vim_mode = config.general.vim_mode;
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
        self.last_area = area;
        self.render_content(area, buf, ctx.is_focused, ctx.border_right_x);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        self.status_message = None;

        // Vim-aware navigation (j/k/g/G when vim_mode is enabled)
        if is_move_up(&key, self.vim_mode) {
            self.move_up();
            return vec![];
        }
        if is_move_down(&key, self.vim_mode) {
            self.move_down();
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
            // Navigation
            KeyCode::PageUp => self.page_up(),
            KeyCode::PageDown => self.page_down(),

            // Scroll without changing selection
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll_up(self.visible_height / 2);
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll_down(self.visible_height / 2);
            }

            // Collapse/expand
            KeyCode::Enter | KeyCode::Char(' ') => self.toggle_collapse(),

            // Open file
            KeyCode::Char('e') => return self.open_file(),

            // Refresh
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.refresh();
                self.status_message = Some("Refreshed".to_string());
            }

            _ => {}
        }

        vec![]
    }

    fn handle_mouse(&mut self, event: MouseEvent, _panel_area: Rect) -> Vec<PanelEvent> {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Find which file was clicked
                let content_y = self.last_area.y + 1;
                if event.row >= content_y {
                    let clicked_visual_line = (event.row - content_y) as usize + self.scroll;

                    // Find which file this line belongs to
                    let mut current_line = 0;
                    for (file_idx, diff) in self.diffs.iter().enumerate() {
                        let file_header_line = current_line;
                        current_line += 1;

                        if clicked_visual_line == file_header_line {
                            self.selected_file = file_idx;
                            return vec![];
                        }

                        if !self.collapsed.contains(&file_idx) {
                            for hunk in &diff.hunks {
                                current_line += 1 + hunk.lines.len();
                            }
                        }

                        if clicked_visual_line < current_line {
                            self.selected_file = file_idx;
                            return vec![];
                        }
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                self.scroll_up(3);
            }
            MouseEventKind::ScrollDown => {
                self.scroll_down(3);
            }
            _ => {}
        }
        vec![]
    }

    fn handle_scroll(&mut self, delta: i32, _panel_area: Rect) -> Vec<PanelEvent> {
        let lines = delta.unsigned_abs() as usize * 3; // 3 lines per scroll unit
        if delta < 0 {
            self.scroll_up(lines);
        } else {
            self.scroll_down(lines);
        }
        vec![]
    }

    fn to_session(&self, _session_dir: &Path) -> Option<SessionPanel> {
        Some(SessionPanel::GitDiff {
            repo_path: self.repo_path.clone(),
            commit_hash: self.commit_hash.clone(),
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
