//! Unified tree search modal for finding files and content.
//!
//! Replaces FileSearchModal and ContentSearchModal with a single modal
//! that supports glob-based file search (Ctrl+F) and content search (Ctrl+Shift+F).

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Widget},
};

use crate::base::{render_input_field, render_modal_block};
use crate::input_keys::{handle_input_key, InputKeyResult};
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

use termide_core::util::is_binary_file;
use termide_git::{GitStatus, GitStatusCache};
use termide_theme::Theme;
use termide_ui::grapheme_utils::{prepare_matched_line, truncate_from_start};

use crate::{
    calculate_modal_width, centered_rect_with_size, Modal, ModalResult, ModalWidthConfig,
    TextInputHandler,
};

/// Maximum total results to collect
const MAX_RESULTS: usize = 500;

/// Modal mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TreeSearchMode {
    /// Ctrl+F — file search by glob mask only
    FileSearch,
    /// Ctrl+Shift+F — file mask + content regex
    ContentSearch,
}

/// Which field is currently focused
#[derive(Debug, Clone, Copy, PartialEq)]
enum FocusState {
    MaskInput,
    ContentInput,
    Results,
}

/// Content match info for ContentSearch mode
#[derive(Debug, Clone)]
struct ContentMatch {
    line_number: usize,
    line_before: Option<String>,
    matched_line: String,
    match_start: usize,
    match_end: usize,
    line_after: Option<String>,
}

/// A node in the result tree
#[derive(Debug, Clone)]
struct ResultTreeNode {
    name: String,
    full_path: PathBuf,
    depth: usize,
    is_dir: bool,
    git_status: GitStatus,
    content_match: Option<ContentMatch>,
}

/// Result type returned when a tree search item is selected
#[derive(Debug, Clone)]
pub enum TreeSearchResult {
    /// Navigate to file (file search mode)
    NavigateToFile(PathBuf),
    /// Open file at specific line (content search mode)
    OpenAtLine { path: PathBuf, line: usize },
}

/// Search results from background thread
enum SearchResults {
    FileResults(Vec<FileResult>),
    ContentResults(Vec<ContentResult>),
}

#[derive(Debug, Clone)]
struct FileResult {
    full_path: PathBuf,
    relative_path: String,
    git_status: GitStatus,
    is_dir: bool,
}

#[derive(Debug, Clone)]
struct ContentResult {
    full_path: PathBuf,
    relative_path: String,
    line_number: usize,
    line_before: Option<String>,
    matched_line: String,
    match_start: usize,
    match_end: usize,
    line_after: Option<String>,
    git_status: GitStatus,
}

/// Unified tree search modal
pub struct TreeSearchModal {
    mode: TreeSearchMode,
    title: String,
    mask_input: TextInputHandler,
    content_input: TextInputHandler,
    base_path: PathBuf,
    max_file_size: u64,
    focus: FocusState,

    // Result tree
    tree_nodes: Vec<ResultTreeNode>,
    tree_prefixes: Vec<String>,
    result_count: usize,

    cursor: usize,
    scroll_offset: usize,
    git_cache: Option<GitStatusCache>,
    last_list_area: Option<Rect>,
    last_modal_area: Option<Rect>,

    // Async
    search_receiver: Option<mpsc::Receiver<SearchResults>>,
    search_cancel: Option<Arc<AtomicBool>>,
    is_searching: bool,
    regex_error: Option<String>,

    // Track Esc state: first Esc from Results goes to input, second closes
    esc_from_results: bool,
}

impl std::fmt::Debug for TreeSearchModal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeSearchModal")
            .field("mode", &self.mode)
            .field("title", &self.title)
            .field("base_path", &self.base_path)
            .field("result_count", &self.result_count)
            .field("cursor", &self.cursor)
            .field("is_searching", &self.is_searching)
            .finish()
    }
}

impl TreeSearchModal {
    /// Create a new file search modal (Ctrl+F)
    pub fn new_file_search(title: impl Into<String>, base_path: PathBuf) -> Self {
        let git_cache = termide_git::get_git_status(&base_path);
        Self {
            mode: TreeSearchMode::FileSearch,
            title: title.into(),
            mask_input: TextInputHandler::new(),
            content_input: TextInputHandler::new(),
            base_path,
            max_file_size: 0,
            focus: FocusState::MaskInput,
            tree_nodes: Vec::new(),
            tree_prefixes: Vec::new(),
            result_count: 0,
            cursor: 0,
            scroll_offset: 0,
            git_cache,
            last_list_area: None,
            last_modal_area: None,
            search_receiver: None,
            search_cancel: None,
            is_searching: false,
            regex_error: None,
            esc_from_results: false,
        }
    }

    /// Create a new content search modal (Ctrl+Shift+F)
    pub fn new_content_search(
        title: impl Into<String>,
        base_path: PathBuf,
        max_file_size: u64,
    ) -> Self {
        let git_cache = termide_git::get_git_status(&base_path);
        Self {
            mode: TreeSearchMode::ContentSearch,
            title: title.into(),
            mask_input: TextInputHandler::new(),
            content_input: TextInputHandler::new(),
            base_path,
            max_file_size,
            focus: FocusState::MaskInput,
            tree_nodes: Vec::new(),
            tree_prefixes: Vec::new(),
            result_count: 0,
            cursor: 0,
            scroll_offset: 0,
            git_cache,
            last_list_area: None,
            last_modal_area: None,
            search_receiver: None,
            search_cancel: None,
            is_searching: false,
            regex_error: None,
            esc_from_results: false,
        }
    }

    /// Calculate dynamic modal width
    fn calculate_modal_width(&self, screen_width: u16) -> u16 {
        let title_width = self.title.len() as u16 + 4;
        let min_width = if self.mode == TreeSearchMode::ContentSearch {
            60u16
        } else {
            50u16
        };

        let max_path_width = self
            .tree_nodes
            .iter()
            .filter(|n| !n.is_dir)
            .map(|n| n.name.len() as u16 + (n.depth as u16 * 3) + 4)
            .max()
            .unwrap_or(40);

        calculate_modal_width(
            [title_width, min_width, max_path_width].into_iter(),
            screen_width,
            ModalWidthConfig::wide(),
        )
    }

    /// Max visible result nodes
    fn max_visible_nodes(&self) -> usize {
        match self.mode {
            TreeSearchMode::FileSearch => 15,
            TreeSearchMode::ContentSearch => 40, // more lines since items vary
        }
    }

    /// Start search in background thread
    fn start_search(&mut self) {
        let mask = self.mask_input.text().to_string();

        // Don't search with empty mask
        if mask.is_empty() {
            self.tree_nodes.clear();
            self.tree_prefixes.clear();
            self.result_count = 0;
            self.cursor = 0;
            self.scroll_offset = 0;
            self.is_searching = false;
            self.regex_error = None;
            return;
        }

        // For content search, also need content pattern
        if self.mode == TreeSearchMode::ContentSearch {
            let content = self.content_input.text().to_string();
            if content.is_empty() {
                self.tree_nodes.clear();
                self.tree_prefixes.clear();
                self.result_count = 0;
                self.cursor = 0;
                self.scroll_offset = 0;
                self.is_searching = false;
                self.regex_error = None;
                return;
            }

            // Validate regex
            if let Err(e) = Regex::new(&content) {
                self.regex_error = Some(e.to_string());
                self.tree_nodes.clear();
                self.tree_prefixes.clear();
                self.result_count = 0;
                self.is_searching = false;
                return;
            }
        }

        self.regex_error = None;

        // Cancel previous search
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }

        let cancel = Arc::new(AtomicBool::new(false));
        self.search_cancel = Some(cancel.clone());

        let (tx, rx) = mpsc::channel();
        let base_path = self.base_path.clone();
        let git_cache = self.git_cache.clone();
        let mode = self.mode;
        let max_file_size = self.max_file_size;
        let content_pattern = self.content_input.text().to_string();

        self.search_receiver = Some(rx);
        self.is_searching = true;

        std::thread::spawn(move || {
            let results = match mode {
                TreeSearchMode::FileSearch => {
                    let file_results =
                        search_files(&base_path, &mask, &cancel, git_cache.as_ref());
                    SearchResults::FileResults(file_results)
                }
                TreeSearchMode::ContentSearch => {
                    let content_results = search_content(
                        &base_path,
                        &mask,
                        &content_pattern,
                        &cancel,
                        git_cache.as_ref(),
                        max_file_size,
                    );
                    SearchResults::ContentResults(content_results)
                }
            };
            if !cancel.load(Ordering::Relaxed) {
                let _ = tx.send(results);
            }
        });
    }

    /// Check for search results
    fn check_search_results(&mut self) {
        if let Some(rx) = &self.search_receiver {
            match rx.try_recv() {
                Ok(results) => {
                    match results {
                        SearchResults::FileResults(items) => {
                            self.build_file_tree(items);
                        }
                        SearchResults::ContentResults(items) => {
                            self.build_content_tree(items);
                        }
                    }
                    self.cursor = 0;
                    self.scroll_offset = 0;
                    self.is_searching = false;
                    self.search_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.is_searching = false;
                    self.search_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    /// Build tree from file search results
    fn build_file_tree(&mut self, items: Vec<FileResult>) {
        self.result_count = items.len();
        let (nodes, prefixes) = build_tree_nodes(
            items
                .iter()
                .map(|i| TreeBuildItem {
                    relative_path: &i.relative_path,
                    full_path: &i.full_path,
                    git_status: i.git_status,
                    is_dir: i.is_dir,
                    content_match: None,
                })
                .collect(),
        );
        self.tree_nodes = nodes;
        self.tree_prefixes = prefixes;
    }

    /// Build tree from content search results
    fn build_content_tree(&mut self, items: Vec<ContentResult>) {
        self.result_count = items.len();
        let (nodes, prefixes) = build_tree_nodes(
            items
                .iter()
                .map(|i| TreeBuildItem {
                    relative_path: &i.relative_path,
                    full_path: &i.full_path,
                    git_status: i.git_status,
                    is_dir: false,
                    content_match: Some(ContentMatch {
                        line_number: i.line_number,
                        line_before: i.line_before.clone(),
                        matched_line: i.matched_line.clone(),
                        match_start: i.match_start,
                        match_end: i.match_end,
                        line_after: i.line_after.clone(),
                    }),
                })
                .collect(),
        );
        self.tree_nodes = nodes;
        self.tree_prefixes = prefixes;
    }

    /// Navigate to next result (skip dirs)
    fn next_result(&mut self) {
        if self.tree_nodes.is_empty() {
            return;
        }
        let start = self.cursor;
        let len = self.tree_nodes.len();
        let mut pos = (start + 1) % len;
        while pos != start {
            if !self.tree_nodes[pos].is_dir {
                self.cursor = pos;
                self.ensure_visible();
                return;
            }
            pos = (pos + 1) % len;
        }
    }

    /// Navigate to previous result (skip dirs)
    fn prev_result(&mut self) {
        if self.tree_nodes.is_empty() {
            return;
        }
        let start = self.cursor;
        let len = self.tree_nodes.len();
        let mut pos = if start == 0 { len - 1 } else { start - 1 };
        while pos != start {
            if !self.tree_nodes[pos].is_dir {
                self.cursor = pos;
                self.ensure_visible();
                return;
            }
            pos = if pos == 0 { len - 1 } else { pos - 1 };
        }
    }

    /// Navigate cursor up (including dirs)
    fn cursor_up(&mut self) {
        if self.tree_nodes.is_empty() {
            return;
        }
        if self.cursor > 0 {
            self.cursor -= 1;
        } else {
            self.cursor = self.tree_nodes.len() - 1;
        }
        self.ensure_visible();
    }

    /// Navigate cursor down (including dirs)
    fn cursor_down(&mut self) {
        if self.tree_nodes.is_empty() {
            return;
        }
        if self.cursor + 1 < self.tree_nodes.len() {
            self.cursor += 1;
        } else {
            self.cursor = 0;
        }
        self.ensure_visible();
    }

    /// Ensure cursor is visible in scroll area
    fn ensure_visible(&mut self) {
        let max_vis = self.max_visible_nodes();
        // Count visible lines from scroll_offset to cursor
        let lines_to_cursor = self.count_lines(self.scroll_offset, self.cursor);
        if lines_to_cursor >= max_vis {
            // Need to scroll down
            self.scroll_offset = self.find_scroll_for_cursor(max_vis);
        } else if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        }
    }

    /// Count display lines between two node indices
    fn count_lines(&self, from: usize, to: usize) -> usize {
        if to < from || from >= self.tree_nodes.len() {
            return 0;
        }
        let end = to.min(self.tree_nodes.len());
        let mut lines = 0;
        for i in from..end {
            lines += self.node_display_lines(i);
        }
        lines
    }

    /// How many display lines a node takes
    fn node_display_lines(&self, idx: usize) -> usize {
        if idx >= self.tree_nodes.len() {
            return 0;
        }
        let node = &self.tree_nodes[idx];
        if node.is_dir {
            1
        } else if self.mode == TreeSearchMode::ContentSearch && node.content_match.is_some() {
            4
        } else {
            1
        }
    }

    /// Find scroll offset to show cursor within max_vis lines
    fn find_scroll_for_cursor(&self, max_vis: usize) -> usize {
        // Walk backwards from cursor until we fill max_vis lines
        let mut lines = self.node_display_lines(self.cursor);
        let mut start = self.cursor;
        while start > 0 && lines < max_vis {
            start -= 1;
            lines += self.node_display_lines(start);
        }
        if lines > max_vis && start < self.cursor {
            start + 1
        } else {
            start
        }
    }

    /// Get selected result
    fn get_selected_result(&self) -> Option<TreeSearchResult> {
        let node = self.tree_nodes.get(self.cursor)?;
        if node.is_dir {
            return None;
        }
        match self.mode {
            TreeSearchMode::FileSearch => Some(TreeSearchResult::NavigateToFile(
                node.full_path.clone(),
            )),
            TreeSearchMode::ContentSearch => {
                let line = node
                    .content_match
                    .as_ref()
                    .map(|m| m.line_number)
                    .unwrap_or(1);
                Some(TreeSearchResult::OpenAtLine {
                    path: node.full_path.clone(),
                    line,
                })
            }
        }
    }

    /// Get style for git status
    fn get_git_style(&self, status: GitStatus, theme: &Theme) -> Style {
        match status {
            GitStatus::Ignored => Style::default()
                .fg(theme.disabled)
                .add_modifier(Modifier::DIM),
            GitStatus::Modified => Style::default().fg(theme.warning),
            GitStatus::Added => Style::default().fg(theme.success),
            GitStatus::Deleted => Style::default().fg(theme.error),
            GitStatus::Unmodified => Style::default().fg(theme.fg),
        }
    }

    /// Render input fields area
    fn render_inputs(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        match self.mode {
            TreeSearchMode::FileSearch => {
                // Single input with "Mask" label
                let input_block = Block::default()
                    .borders(Borders::ALL)
                    .title(" Mask ")
                    .border_style(Style::default().fg(if self.focus == FocusState::MaskInput {
                        theme.accented_fg
                    } else {
                        theme.disabled
                    }));
                let input_inner = input_block.inner(area);
                input_block.render(area, buf);

                render_input_field(
                    buf,
                    input_inner.x,
                    input_inner.y,
                    input_inner.width,
                    self.mask_input.text(),
                    self.mask_input.cursor_pos(),
                    self.mask_input.selection_range(),
                    self.focus != FocusState::Results,
                    theme,
                );
            }
            TreeSearchMode::ContentSearch => {
                // Two inputs: Mask and Content
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(3), Constraint::Length(3)])
                    .split(area);

                // Mask input
                let mask_block = Block::default()
                    .borders(Borders::ALL)
                    .title(" Mask ")
                    .border_style(Style::default().fg(
                        if self.focus == FocusState::MaskInput {
                            theme.accented_fg
                        } else {
                            theme.disabled
                        },
                    ));
                let mask_inner = mask_block.inner(chunks[0]);
                mask_block.render(chunks[0], buf);

                render_input_field(
                    buf,
                    mask_inner.x,
                    mask_inner.y,
                    mask_inner.width,
                    self.mask_input.text(),
                    self.mask_input.cursor_pos(),
                    self.mask_input.selection_range(),
                    self.focus == FocusState::MaskInput,
                    theme,
                );

                // Content input
                let content_block = Block::default()
                    .borders(Borders::ALL)
                    .title(" Find ")
                    .border_style(Style::default().fg(
                        if self.focus == FocusState::ContentInput {
                            theme.accented_fg
                        } else {
                            theme.disabled
                        },
                    ));
                let content_inner = content_block.inner(chunks[1]);
                content_block.render(chunks[1], buf);

                render_input_field(
                    buf,
                    content_inner.x,
                    content_inner.y,
                    content_inner.width,
                    self.content_input.text(),
                    self.content_input.cursor_pos(),
                    self.content_input.selection_range(),
                    self.focus == FocusState::ContentInput,
                    theme,
                );
            }
        }
    }

    /// Render result tree
    fn render_results(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        self.last_list_area = Some(area);

        if let Some(error) = &self.regex_error {
            let message = format!("Regex error: {}", error);
            let hint = Paragraph::new(message).style(Style::default().fg(theme.error));
            hint.render(area, buf);
            return;
        }

        if self.is_searching {
            let hint = Paragraph::new("Searching…").style(
                Style::default()
                    .fg(theme.accented_bg)
                    .add_modifier(Modifier::DIM),
            );
            hint.render(area, buf);
            return;
        }

        if self.tree_nodes.is_empty() {
            let message = if self.mask_input.is_empty() {
                match self.mode {
                    TreeSearchMode::FileSearch => "*.rs, src/**/*.ts, main.rs",
                    TreeSearchMode::ContentSearch => "Mask: *.rs   Find: fn main",
                }
            } else {
                "No matches found"
            };
            let hint = Paragraph::new(message).style(
                Style::default()
                    .fg(theme.accented_bg)
                    .add_modifier(Modifier::DIM),
            );
            hint.render(area, buf);
            return;
        }

        match self.mode {
            TreeSearchMode::FileSearch => self.render_file_results(area, buf, theme),
            TreeSearchMode::ContentSearch => self.render_content_results(area, buf, theme),
        }
    }

    /// Render file search results as tree
    fn render_file_results(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let max_vis = self.max_visible_nodes();
        let items: Vec<ListItem> = self
            .tree_nodes
            .iter()
            .enumerate()
            .skip(self.scroll_offset)
            .take(max_vis)
            .map(|(idx, node)| {
                let is_selected = idx == self.cursor && self.focus == FocusState::Results;
                let prefix = &self.tree_prefixes[idx];

                let style = if is_selected {
                    Style::default()
                        .fg(theme.fg)
                        .bg(theme.accented_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    self.get_git_style(node.git_status, theme)
                };

                let prefix_style = if is_selected {
                    style
                } else {
                    Style::default().fg(theme.disabled)
                };

                const DIR_ICON: &str = if cfg!(windows) { "►" } else { "▶" };
                let icon = if node.is_dir { DIR_ICON } else { " " };
                let dir_slash = if node.is_dir { "/" } else { "" };
                let display_name = format!("{}{}", dir_slash, node.name);

                let mut spans = Vec::new();
                if !prefix.is_empty() {
                    spans.push(Span::styled(prefix.clone(), prefix_style));
                }
                spans.push(Span::styled(icon, style));
                spans.push(Span::styled(" ", style));
                spans.push(Span::styled(display_name, style));

                // Pad to full width
                let text_len: usize = spans.iter().map(|s| s.content.len()).sum();
                let padding_len = (area.width as usize).saturating_sub(text_len);
                if padding_len > 0 {
                    spans.push(Span::styled(" ".repeat(padding_len), style));
                }

                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(items).style(Style::default().bg(theme.bg));
        list.render(area, buf);
    }

    /// Render content search results as tree with context lines
    fn render_content_results(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let max_vis = self.max_visible_nodes();
        let content_width = area.width as usize;
        let mut y = area.y;
        let mut lines_rendered = 0;

        // Editor-style colors
        let editor_bg = theme.bg;
        let line_num_style = Style::default().fg(theme.disabled).bg(editor_bg);
        let context_text_style = Style::default().fg(theme.fg).bg(editor_bg);
        let matched_text_style = Style::default().fg(theme.fg).bg(editor_bg);
        let highlight_style = Style::default().bg(theme.selected_bg).fg(theme.selected_fg);
        let separator_style = Style::default().fg(theme.disabled).bg(editor_bg);

        let line_num_width = 4usize;
        let separator = " │ ";
        let separator_len = separator.chars().count();
        let max_text_width = content_width.saturating_sub(line_num_width + separator_len);

        for (idx, node) in self.tree_nodes.iter().enumerate().skip(self.scroll_offset) {
            if lines_rendered >= max_vis || y >= area.y + area.height {
                break;
            }

            let is_selected = idx == self.cursor && self.focus == FocusState::Results;

            if node.is_dir {
                // Directory line: tree prefix + icon + name
                let prefix = &self.tree_prefixes[idx];
                let style = if is_selected {
                    Style::default()
                        .fg(theme.fg)
                        .bg(theme.accented_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg)
                };
                let prefix_style = if is_selected {
                    style
                } else {
                    Style::default().fg(theme.disabled)
                };

                const DIR_ICON: &str = if cfg!(windows) { "►" } else { "▶" };
                let text = format!("{}{} /{}", prefix, DIR_ICON, node.name);
                let padding =
                    " ".repeat(content_width.saturating_sub(text.chars().count()));
                buf.set_string(area.x, y, &text, Style::default());
                // Re-render with proper styles
                let mut x = area.x;
                if !prefix.is_empty() {
                    buf.set_string(x, y, prefix, prefix_style);
                    x += prefix.chars().count() as u16;
                }
                const DIR_ICON2: &str = if cfg!(windows) { "►" } else { "▶" };
                buf.set_string(x, y, &format!("{} /{}", DIR_ICON2, node.name), style);
                x += format!("{} /{}", DIR_ICON2, node.name).chars().count() as u16;
                buf.set_string(x, y, &padding, style);

                y += 1;
                lines_rendered += 1;
            } else if let Some(ref cm) = node.content_match {
                if y + 4 > area.y + area.height {
                    break;
                }

                // Line 1: path:line_number
                let prefix = &self.tree_prefixes[idx];
                let path_text = format!(
                    "{}{}:{}",
                    prefix, node.name, cm.line_number
                );
                let path_style = if is_selected {
                    Style::default()
                        .fg(theme.fg)
                        .bg(theme.accented_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg)
                };
                let padding =
                    " ".repeat(content_width.saturating_sub(path_text.chars().count()));
                buf.set_string(area.x, y, &path_text, path_style);
                buf.set_string(
                    area.x + path_text.chars().count() as u16,
                    y,
                    &padding,
                    path_style,
                );
                y += 1;

                // Helper to fill line background
                let fill_bg = |buf: &mut Buffer, row: u16| {
                    for col in 0..content_width {
                        buf.set_string(
                            area.x + col as u16,
                            row,
                            " ",
                            Style::default().bg(editor_bg),
                        );
                    }
                };

                // Line 2: previous line
                fill_bg(buf, y);
                if let Some(ref line_before) = cm.line_before {
                    let line_num = format!("{:>4}", cm.line_number - 1);
                    let content = truncate_from_start(line_before, max_text_width);
                    buf.set_string(area.x, y, &line_num, line_num_style);
                    buf.set_string(
                        area.x + line_num_width as u16,
                        y,
                        separator,
                        separator_style,
                    );
                    buf.set_string(
                        area.x + (line_num_width + separator_len) as u16,
                        y,
                        &content,
                        context_text_style,
                    );
                }
                y += 1;

                // Line 3: matched line with highlight
                fill_bg(buf, y);
                let line_num = format!("{:>4}", cm.line_number);
                buf.set_string(area.x, y, &line_num, line_num_style);
                buf.set_string(
                    area.x + line_num_width as u16,
                    y,
                    separator,
                    separator_style,
                );

                let content_start_x = area.x + (line_num_width + separator_len) as u16;
                let (display_line, match_start_g, match_end_g) = prepare_matched_line(
                    &cm.matched_line,
                    cm.match_start,
                    cm.match_end,
                    max_text_width,
                );

                let mut x = content_start_x;
                for (grapheme_idx, grapheme) in display_line.graphemes(true).enumerate() {
                    let style =
                        if grapheme_idx >= match_start_g && grapheme_idx < match_end_g {
                            highlight_style
                        } else {
                            matched_text_style
                        };
                    buf.set_string(x, y, grapheme, style);
                    x += unicode_width::UnicodeWidthStr::width(grapheme) as u16;
                }
                y += 1;

                // Line 4: next line
                fill_bg(buf, y);
                if let Some(ref line_after) = cm.line_after {
                    let line_num = format!("{:>4}", cm.line_number + 1);
                    let content = truncate_from_start(line_after, max_text_width);
                    buf.set_string(area.x, y, &line_num, line_num_style);
                    buf.set_string(
                        area.x + line_num_width as u16,
                        y,
                        separator,
                        separator_style,
                    );
                    buf.set_string(
                        area.x + (line_num_width + separator_len) as u16,
                        y,
                        &content,
                        context_text_style,
                    );
                }
                y += 1;

                lines_rendered += 4;
            } else {
                // File without content match (shouldn't happen in content mode, but handle gracefully)
                let prefix = &self.tree_prefixes[idx];
                let style = if is_selected {
                    Style::default()
                        .fg(theme.fg)
                        .bg(theme.accented_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    self.get_git_style(node.git_status, theme)
                };
                let text = format!("{}  {}", prefix, node.name);
                let padding =
                    " ".repeat(content_width.saturating_sub(text.chars().count()));
                buf.set_string(area.x, y, &text, style);
                buf.set_string(area.x + text.chars().count() as u16, y, &padding, style);
                y += 1;
                lines_rendered += 1;
            }
        }
    }
}

impl Modal for TreeSearchModal {
    type Result = TreeSearchResult;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        // Check for async results
        self.check_search_results();

        let modal_width = self.calculate_modal_width(area.width);

        // Calculate heights
        let input_height: u16 = match self.mode {
            TreeSearchMode::FileSearch => 3,
            TreeSearchMode::ContentSearch => 6,
        };

        // Count visible result lines
        let mut result_lines = 0u16;
        let max_vis = self.max_visible_nodes();
        for (idx, node) in self.tree_nodes.iter().enumerate().skip(self.scroll_offset) {
            if result_lines as usize >= max_vis {
                break;
            }
            let lines = if node.is_dir {
                1
            } else if self.mode == TreeSearchMode::ContentSearch && node.content_match.is_some() {
                4
            } else {
                1
            };
            result_lines += lines as u16;
            if idx > self.scroll_offset + 50 {
                break; // safety
            }
        }
        let list_height = if result_lines == 0 { 1 } else { result_lines };

        // Status line
        let status_height = 1u16;

        let modal_height =
            (2 + input_height + list_height + status_height).min(area.height - 2);

        let modal_area = centered_rect_with_size(modal_width, modal_height, area);
        self.last_modal_area = Some(modal_area);

        let inner = render_modal_block(modal_area, buf, &self.title, theme);

        // Split: inputs + results + status
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(input_height),
                Constraint::Min(1),
                Constraint::Length(status_height),
            ])
            .split(inner);

        // Render inputs
        self.render_inputs(chunks[0], buf, theme);

        // Render results
        self.render_results(chunks[1], buf, theme);

        // Render status line
        let status = if self.is_searching {
            "Searching…".to_string()
        } else if self.result_count > 0 {
            format!("{} found", self.result_count)
        } else if !self.mask_input.is_empty() {
            "No matches".to_string()
        } else {
            "Enter mask and press Enter".to_string()
        };
        let status_style = Style::default()
            .fg(theme.disabled)
            .add_modifier(Modifier::DIM);
        let status_para = Paragraph::new(status).style(status_style);
        status_para.render(chunks[2], buf);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>> {
        // Esc handling
        if key.code == KeyCode::Esc {
            if let Some(cancel) = self.search_cancel.take() {
                cancel.store(true, Ordering::Relaxed);
            }
            if self.focus == FocusState::Results && !self.esc_from_results {
                // First Esc from Results -> back to input
                self.focus = FocusState::MaskInput;
                self.esc_from_results = true;
                return Ok(None);
            }
            return Ok(Some(ModalResult::Cancelled));
        }

        // Reset esc tracking when not pressing Esc
        self.esc_from_results = false;

        match self.focus {
            FocusState::MaskInput | FocusState::ContentInput => {
                self.handle_input_key(key)
            }
            FocusState::Results => self.handle_results_key(key),
        }
    }

    fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        _modal_area: Rect,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        use crate::{check_mouse_click, MouseClickResult};

        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return Ok(None);
        }

        match check_mouse_click(
            mouse.column,
            mouse.row,
            self.last_modal_area,
            self.last_list_area,
            self.scroll_offset,
        ) {
            MouseClickResult::OutsideModal => Ok(Some(ModalResult::Cancelled)),
            MouseClickResult::OutsideList => Ok(None),
            MouseClickResult::OnListItem(clicked_index) => {
                if clicked_index < self.tree_nodes.len() {
                    self.cursor = clicked_index;
                    self.focus = FocusState::Results;
                    if let Some(result) = self.get_selected_result() {
                        return Ok(Some(ModalResult::Confirmed(result)));
                    }
                }
                Ok(None)
            }
        }
    }

    fn handle_paste(&mut self, text: &str) -> bool {
        match self.focus {
            FocusState::MaskInput => {
                self.mask_input.insert_str(text);
                true
            }
            FocusState::ContentInput => {
                self.content_input.insert_str(text);
                true
            }
            FocusState::Results => false,
        }
    }
}

impl TreeSearchModal {
    /// Handle key when focus is on an input field
    fn handle_input_key(
        &mut self,
        key: KeyEvent,
    ) -> Result<Option<ModalResult<TreeSearchResult>>> {
        // Enter/Tab/Shift+Tab -> start search
        match key.code {
            KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab => {
                self.start_search();
                if !self.tree_nodes.is_empty() {
                    self.focus = FocusState::Results;
                    // Tab -> first result, Shift+Tab -> last result
                    if key.code == KeyCode::BackTab {
                        // Find last non-dir
                        self.cursor = self.tree_nodes.len().saturating_sub(1);
                        while self.cursor > 0 && self.tree_nodes[self.cursor].is_dir {
                            self.cursor -= 1;
                        }
                    } else {
                        // Find first non-dir
                        self.cursor = 0;
                        while self.cursor < self.tree_nodes.len()
                            && self.tree_nodes[self.cursor].is_dir
                        {
                            self.cursor += 1;
                        }
                        if self.cursor >= self.tree_nodes.len() {
                            self.cursor = 0;
                        }
                    }
                    self.ensure_visible();
                }
                return Ok(None);
            }
            _ => {}
        }

        // Up/Down for ContentSearch: switch between fields when cursor at edge
        if self.mode == TreeSearchMode::ContentSearch {
            match key.code {
                KeyCode::Up if self.focus == FocusState::ContentInput => {
                    if self.content_input.cursor_pos() == 0 {
                        self.focus = FocusState::MaskInput;
                        return Ok(None);
                    }
                }
                KeyCode::Down if self.focus == FocusState::MaskInput => {
                    let text_len = self.mask_input.text().chars().count();
                    if self.mask_input.cursor_pos() >= text_len {
                        self.focus = FocusState::ContentInput;
                        return Ok(None);
                    }
                }
                _ => {}
            }
        }

        // Delegate to text input handler
        let handler = match self.focus {
            FocusState::MaskInput => &mut self.mask_input,
            FocusState::ContentInput => &mut self.content_input,
            _ => return Ok(None),
        };

        match handle_input_key(handler, key) {
            InputKeyResult::Handled | InputKeyResult::TextModified => Ok(None),
            InputKeyResult::NotHandled => Ok(None),
        }
    }

    /// Handle key when focus is on results
    fn handle_results_key(
        &mut self,
        key: KeyEvent,
    ) -> Result<Option<ModalResult<TreeSearchResult>>> {
        match key.code {
            KeyCode::Tab => {
                self.next_result();
                Ok(None)
            }
            KeyCode::BackTab => {
                self.prev_result();
                Ok(None)
            }
            KeyCode::Up => {
                self.cursor_up();
                Ok(None)
            }
            KeyCode::Down => {
                self.cursor_down();
                Ok(None)
            }
            KeyCode::Enter => {
                if let Some(result) = self.get_selected_result() {
                    Ok(Some(ModalResult::Confirmed(result)))
                } else {
                    Ok(None)
                }
            }
            KeyCode::PageUp => {
                for _ in 0..10 {
                    self.cursor_up();
                }
                Ok(None)
            }
            KeyCode::PageDown => {
                for _ in 0..10 {
                    self.cursor_down();
                }
                Ok(None)
            }
            KeyCode::Char(c) => {
                // Any printable char -> switch to input
                self.focus = FocusState::MaskInput;
                self.mask_input.insert_char(c);
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

// ─── Tree building ───────────────────────────────────────────────────────

struct TreeBuildItem<'a> {
    relative_path: &'a str,
    full_path: &'a Path,
    git_status: GitStatus,
    is_dir: bool,
    content_match: Option<ContentMatch>,
}

/// Build tree nodes and prefixes from flat sorted results
fn build_tree_nodes(items: Vec<TreeBuildItem<'_>>) -> (Vec<ResultTreeNode>, Vec<String>) {
    if items.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let mut nodes: Vec<ResultTreeNode> = Vec::new();
    let mut added_dirs: HashSet<PathBuf> = HashSet::new();

    for item in &items {
        let rel_path = Path::new(item.relative_path);
        let components: Vec<&std::ffi::OsStr> = rel_path.iter().collect();

        // Add ancestor directories
        for depth in 0..components.len().saturating_sub(1) {
            let dir_path: PathBuf = components[..=depth].iter().collect();
            if !added_dirs.contains(&dir_path) {
                added_dirs.insert(dir_path.clone());
                let dir_name = components[depth].to_string_lossy().into_owned();
                nodes.push(ResultTreeNode {
                    name: dir_name,
                    full_path: Path::new(item.full_path)
                        .ancestors()
                        .nth(components.len() - 1 - depth)
                        .unwrap_or(item.full_path)
                        .to_path_buf(),
                    depth,
                    is_dir: true,
                    git_status: GitStatus::Unmodified,
                    content_match: None,
                });
            }
        }

        // Add the item itself
        let depth = components.len().saturating_sub(1);
        let name = components
            .last()
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_default();

        nodes.push(ResultTreeNode {
            name,
            full_path: item.full_path.to_path_buf(),
            depth,
            is_dir: item.is_dir,
            git_status: item.git_status,
            content_match: item.content_match.clone(),
        });
    }

    // Sort: dirs first at each depth, then by name
    nodes.sort_by(|a, b| {
        // Sort by path components to maintain tree structure
        a.full_path.cmp(&b.full_path)
    });

    // Deduplicate dir nodes (same path)
    nodes.dedup_by(|a, b| a.is_dir && b.is_dir && a.full_path == b.full_path);

    // Compute tree prefixes
    let prefixes = compute_tree_prefixes(&nodes);

    (nodes, prefixes)
}

/// Compute tree prefixes for result nodes (similar to tree.rs::compute_prefixes)
fn compute_tree_prefixes(nodes: &[ResultTreeNode]) -> Vec<String> {
    if nodes.is_empty() {
        return Vec::new();
    }

    let max_depth = nodes.iter().map(|n| n.depth).max().unwrap_or(0);
    if max_depth == 0 {
        return vec![String::new(); nodes.len()];
    }

    let mut has_next_at_level = vec![false; max_depth + 1];
    let mut prefixes: Vec<String> = Vec::with_capacity(nodes.len());

    for node in nodes.iter().rev() {
        let depth = node.depth;

        if depth == 0 {
            has_next_at_level.fill(false);
            has_next_at_level[0] = true;
            prefixes.push(String::new());
            continue;
        }

        let mut prefix = String::with_capacity(depth * 3);
        for (lvl, has_next) in has_next_at_level[1..=depth].iter().enumerate() {
            let lvl = lvl + 1;
            if lvl == depth {
                if *has_next {
                    prefix.push_str(" ├─");
                } else {
                    prefix.push_str(" └─");
                }
            } else if *has_next {
                prefix.push_str(" │ ");
            } else {
                prefix.push_str("   ");
            }
        }
        prefixes.push(prefix);

        for val in &mut has_next_at_level[(depth + 1)..] {
            *val = false;
        }
        has_next_at_level[depth] = true;
    }

    prefixes.reverse();
    prefixes
}

// ─── Background search functions ─────────────────────────────────────────

/// Search files by glob mask
fn search_files(
    base_path: &Path,
    mask: &str,
    cancel: &AtomicBool,
    git_cache: Option<&GitStatusCache>,
) -> Vec<FileResult> {
    use ignore::WalkBuilder;

    // Parse glob pattern
    let has_path_sep = mask.contains('/') || mask.contains('\\');
    let has_wildcards = mask.contains('*') || mask.contains('?');

    let glob_pattern = if has_wildcards || has_path_sep {
        match glob::Pattern::new(mask) {
            Ok(g) => Some(g),
            Err(_) => return Vec::new(),
        }
    } else {
        // Plain name -> exact match by Pattern
        match glob::Pattern::new(mask) {
            Ok(g) => Some(g),
            Err(_) => return Vec::new(),
        }
    };

    let mask_lower = mask.to_lowercase();

    let mut results = Vec::new();

    let walker = WalkBuilder::new(base_path)
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .build();

    for entry in walker {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if path == base_path {
            continue;
        }

        let relative_path = path
            .strip_prefix(base_path)
            .map(|r| r.display().to_string())
            .unwrap_or_default();

        let matches = if has_path_sep {
            // Match against full relative path
            if let Some(ref glob) = glob_pattern {
                glob.matches(&relative_path)
            } else {
                false
            }
        } else if has_wildcards {
            // Match against file name
            let name = match path.file_name() {
                Some(n) => n.to_string_lossy(),
                None => continue,
            };
            if let Some(ref glob) = glob_pattern {
                glob.matches(&name)
            } else {
                false
            }
        } else {
            // Plain text: case-insensitive substring match on name
            let name = match path.file_name() {
                Some(n) => n.to_string_lossy(),
                None => continue,
            };
            name.to_lowercase().contains(&mask_lower)
        };

        if !matches {
            continue;
        }

        let is_dir = path.is_dir();
        let git_status = git_cache
            .map(|cache| {
                if is_dir {
                    cache.get_directory_status(&relative_path)
                } else {
                    cache.get_status(&relative_path)
                }
            })
            .unwrap_or(GitStatus::Unmodified);

        results.push(FileResult {
            full_path: path.to_path_buf(),
            relative_path,
            git_status,
            is_dir,
        });

        if results.len() >= MAX_RESULTS {
            break;
        }
    }

    results.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    results
}

/// Search content in files matching glob mask
fn search_content(
    base_path: &Path,
    mask: &str,
    content_pattern: &str,
    cancel: &AtomicBool,
    git_cache: Option<&GitStatusCache>,
    max_file_size: u64,
) -> Vec<ContentResult> {
    use ignore::WalkBuilder;

    let regex = match Regex::new(content_pattern) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    // Parse glob for file filtering
    let has_path_sep = mask.contains('/') || mask.contains('\\');
    let has_wildcards = mask.contains('*') || mask.contains('?');
    let glob_pattern = if has_wildcards || has_path_sep {
        glob::Pattern::new(mask).ok()
    } else {
        glob::Pattern::new(mask).ok()
    };
    let mask_lower = mask.to_lowercase();

    let min_size = content_pattern.len() as u64;
    let mut results = Vec::new();

    let walker = WalkBuilder::new(base_path)
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .build();

    for entry in walker {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if path.is_dir() {
            continue;
        }

        // Check file mask
        let relative_path = path
            .strip_prefix(base_path)
            .map(|r| r.display().to_string())
            .unwrap_or_default();

        let name_matches = if has_path_sep {
            glob_pattern
                .as_ref()
                .map(|g| g.matches(&relative_path))
                .unwrap_or(false)
        } else if has_wildcards {
            let name = match path.file_name() {
                Some(n) => n.to_string_lossy(),
                None => continue,
            };
            glob_pattern
                .as_ref()
                .map(|g| g.matches(&name))
                .unwrap_or(false)
        } else {
            let name = match path.file_name() {
                Some(n) => n.to_string_lossy(),
                None => continue,
            };
            name.to_lowercase().contains(&mask_lower)
        };

        if !name_matches {
            continue;
        }

        // Check file size and binary
        if should_skip_file(path, max_file_size, min_size) {
            continue;
        }

        // Read and search
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lines: Vec<&str> = content.lines().collect();
        let git_status = git_cache
            .map(|cache| cache.get_status(&relative_path))
            .unwrap_or(GitStatus::Unmodified);

        for (line_idx, line) in lines.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }

            if let Some(m) = regex.find(line) {
                let line_before = if line_idx > 0 {
                    Some(lines[line_idx - 1].to_string())
                } else {
                    None
                };
                let line_after = if line_idx + 1 < lines.len() {
                    Some(lines[line_idx + 1].to_string())
                } else {
                    None
                };

                results.push(ContentResult {
                    full_path: path.to_path_buf(),
                    relative_path: relative_path.clone(),
                    line_number: line_idx + 1,
                    line_before,
                    matched_line: line.to_string(),
                    match_start: m.start(),
                    match_end: m.end(),
                    line_after,
                    git_status,
                });

                if results.len() >= MAX_RESULTS {
                    return results;
                }
            }
        }

        if results.len() >= MAX_RESULTS {
            break;
        }
    }

    results
}

/// Check if file should be skipped
fn should_skip_file(path: &Path, max_size: u64, min_size: u64) -> bool {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let file_size = meta.len();
    if file_size < min_size {
        return true;
    }
    if max_size > 0 && file_size > max_size {
        return true;
    }
    is_binary_file(path)
}

