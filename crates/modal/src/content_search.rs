//! Content search modal dialog for finding text in files using regex patterns.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use crate::base::render_modal_block;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;

use termide_core::util::is_binary_file;
use termide_git::{GitStatus, GitStatusCache};
use termide_theme::Theme;

use crate::{
    calculate_modal_width, centered_rect_with_size, CursorNavigation, Modal, ModalResult,
    ModalWidthConfig, TextInputHandler,
};

/// Maximum number of search results visible at once (each result = 4 lines)
const MAX_VISIBLE_RESULTS: usize = 10;

/// Maximum total results to collect
const MAX_RESULTS: usize = 200;

/// Debounce delay before starting search (longer due to heavier search)
const DEBOUNCE_DELAY: Duration = Duration::from_millis(300);

/// Content search result item
#[derive(Debug, Clone)]
pub struct ContentSearchResultItem {
    /// Full path to the file
    pub full_path: PathBuf,
    /// Path relative to base directory (for display)
    pub relative_path: String,
    /// Line number (1-based)
    pub line_number: usize,
    /// Previous line (if exists)
    pub line_before: Option<String>,
    /// The matched line
    pub matched_line: String,
    /// Match start position in line (byte offset)
    pub match_start: usize,
    /// Match end position in line (byte offset)
    pub match_end: usize,
    /// Next line (if exists)
    pub line_after: Option<String>,
    /// Git status for coloring
    pub git_status: GitStatus,
}

/// Content search modal window
pub struct ContentSearchModal {
    title: String,
    input_handler: TextInputHandler,
    base_path: PathBuf,
    max_file_size: u64,
    results: Vec<ContentSearchResultItem>,
    cursor: usize,
    scroll_offset: usize,
    git_cache: Option<GitStatusCache>,
    last_list_area: Option<Rect>,
    last_modal_area: Option<Rect>,

    // Async search state
    last_input_text: String,
    last_input_time: Option<Instant>,
    search_receiver: Option<mpsc::Receiver<Vec<ContentSearchResultItem>>>,
    search_cancel: Option<Arc<AtomicBool>>,
    is_searching: bool,
    regex_error: Option<String>,
}

impl std::fmt::Debug for ContentSearchModal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContentSearchModal")
            .field("title", &self.title)
            .field("base_path", &self.base_path)
            .field("results_count", &self.results.len())
            .field("cursor", &self.cursor)
            .field("is_searching", &self.is_searching)
            .finish()
    }
}

impl ContentSearchModal {
    /// Create a new content search modal
    pub fn new(title: impl Into<String>, base_path: PathBuf, max_file_size: u64) -> Self {
        // Get git status for the base path
        let git_cache = termide_git::get_git_status(&base_path);

        Self {
            title: title.into(),
            input_handler: TextInputHandler::new(),
            base_path,
            max_file_size,
            results: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            git_cache,
            last_list_area: None,
            last_modal_area: None,
            last_input_text: String::new(),
            last_input_time: None,
            search_receiver: None,
            search_cancel: None,
            is_searching: false,
            regex_error: None,
        }
    }

    /// Calculate dynamic modal width
    fn calculate_modal_width(&self, screen_width: u16) -> u16 {
        let title_width = self.title.len() as u16 + 4;
        let min_width = 60u16;

        // Find max result path width
        let max_path_width = self
            .results
            .iter()
            .map(|item| item.relative_path.len() as u16 + 10) // +10 for line number
            .max()
            .unwrap_or(50);

        calculate_modal_width(
            [title_width, min_width, max_path_width].into_iter(),
            screen_width,
            ModalWidthConfig::wide(),
        )
    }

    /// Start async content search
    fn start_search(&mut self) {
        let pattern = self.input_handler.text().to_string();

        // Don't search if pattern is empty
        if pattern.is_empty() {
            self.results.clear();
            self.cursor = 0;
            self.scroll_offset = 0;
            self.is_searching = false;
            self.regex_error = None;
            return;
        }

        // Validate regex pattern
        if let Err(e) = Regex::new(&pattern) {
            self.regex_error = Some(e.to_string());
            self.results.clear();
            self.is_searching = false;
            return;
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
        let max_file_size = self.max_file_size;

        self.search_receiver = Some(rx);
        self.is_searching = true;

        std::thread::spawn(move || {
            let results = search_content(
                &base_path,
                &pattern,
                &cancel,
                git_cache.as_ref(),
                max_file_size,
            );
            if !cancel.load(Ordering::Relaxed) {
                let _ = tx.send(results);
            }
        });
    }

    /// Check for search results and update state
    fn check_search_results(&mut self) {
        if let Some(rx) = &self.search_receiver {
            match rx.try_recv() {
                Ok(results) => {
                    self.results = results;
                    self.cursor = 0;
                    self.scroll_offset = 0;
                    self.is_searching = false;
                    self.search_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.is_searching = false;
                    self.search_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // Still searching
                }
            }
        }
    }

    /// Get the selected result
    fn get_selected_result(&self) -> Option<ContentSearchResultItem> {
        self.results.get(self.cursor).cloned()
    }

    /// Truncate line from start if too long (safely handles UTF-8)
    fn truncate_from_start(line: &str, max_chars: usize) -> String {
        let graphemes: Vec<&str> = line.graphemes(true).collect();
        if graphemes.len() > max_chars {
            let truncated: String = graphemes[..max_chars.saturating_sub(1)].concat();
            format!("{}…", truncated)
        } else {
            line.to_string()
        }
    }

    /// Convert byte positions to grapheme indices
    fn byte_to_grapheme_indices(line: &str, byte_start: usize, byte_end: usize) -> (usize, usize) {
        let mut grapheme_start = 0;
        let mut grapheme_end = 0;
        let mut byte_pos = 0;

        for (idx, grapheme) in line.graphemes(true).enumerate() {
            if byte_pos <= byte_start {
                grapheme_start = idx;
            }
            byte_pos += grapheme.len();
            if byte_pos <= byte_end {
                grapheme_end = idx + 1;
            }
        }

        (grapheme_start, grapheme_end)
    }

    /// Prepare matched line for display - center match if line is too long
    /// Returns (display_string, new_match_start_grapheme, new_match_end_grapheme)
    fn prepare_matched_line(
        line: &str,
        match_start: usize,
        match_end: usize,
        max_width: usize,
    ) -> (String, usize, usize) {
        let graphemes: Vec<&str> = line.graphemes(true).collect();

        // Convert byte positions to grapheme indices
        let (match_start_g, match_end_g) =
            Self::byte_to_grapheme_indices(line, match_start, match_end);

        if graphemes.len() <= max_width {
            // Line fits entirely
            return (line.to_string(), match_start_g, match_end_g);
        }

        // Need to truncate - center the match
        let ellipsis = "…";
        let ellipsis_len = 1;

        // Calculate window around match
        let match_center = (match_start_g + match_end_g) / 2;

        // Try to show as much context as possible while keeping match visible
        let available_for_text = max_width.saturating_sub(ellipsis_len * 2);

        // Center the match in available space
        let half_window = available_for_text / 2;
        let window_start = match_center.saturating_sub(half_window).min(match_start_g);
        let window_end = (window_start + available_for_text).min(graphemes.len());

        // Adjust window_start if window_end hit the end
        let window_start = if window_end == graphemes.len() {
            graphemes.len().saturating_sub(available_for_text)
        } else {
            window_start
        };

        let needs_left_ellipsis = window_start > 0;
        let needs_right_ellipsis = window_end < graphemes.len();

        let mut result = String::new();
        let mut new_match_start = match_start_g.saturating_sub(window_start);
        let mut new_match_end = match_end_g.saturating_sub(window_start);

        if needs_left_ellipsis {
            result.push_str(ellipsis);
            new_match_start += ellipsis_len;
            new_match_end += ellipsis_len;
        }

        result.push_str(&graphemes[window_start..window_end].concat());

        if needs_right_ellipsis {
            result.push_str(ellipsis);
        }

        let result_len = result.graphemes(true).count();
        (result, new_match_start, new_match_end.min(result_len))
    }
}

impl CursorNavigation for ContentSearchModal {
    fn results_len(&self) -> usize {
        self.results.len()
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn set_cursor(&mut self, pos: usize) {
        self.cursor = pos;
    }

    fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset;
    }

    fn max_visible(&self) -> usize {
        MAX_VISIBLE_RESULTS
    }
}

/// Check if file should be skipped (too large, too small, or binary)
fn should_skip_file(path: &Path, max_size: u64, min_size: u64) -> bool {
    // Check file size
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let file_size = meta.len();

    // Skip empty files and files smaller than pattern length
    if file_size < min_size {
        return true;
    }

    // Skip files larger than max size
    if file_size > max_size {
        return true;
    }

    // Skip binary files
    is_binary_file(path)
}

/// Perform content search using regex (runs in background thread)
fn search_content(
    base_path: &Path,
    pattern: &str,
    cancel: &AtomicBool,
    git_cache: Option<&GitStatusCache>,
    max_file_size: u64,
) -> Vec<ContentSearchResultItem> {
    use ignore::WalkBuilder;

    let regex = match Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    // Minimum file size = pattern length (files smaller can't match)
    let min_size = pattern.len() as u64;

    let mut results = Vec::new();

    let walker = WalkBuilder::new(base_path)
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .build();

    for entry in walker {
        // Check cancellation
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();

        // Skip directories
        if path.is_dir() {
            continue;
        }

        // Skip if file should be skipped
        if should_skip_file(path, max_file_size, min_size) {
            continue;
        }

        // Read file content
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lines: Vec<&str> = content.lines().collect();
        let relative_path = path
            .strip_prefix(base_path)
            .map(|r| r.display().to_string())
            .unwrap_or_default();

        let git_status = git_cache
            .map(|cache| cache.get_status(&relative_path))
            .unwrap_or(GitStatus::Unmodified);

        // Search each line
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

                results.push(ContentSearchResultItem {
                    full_path: path.to_path_buf(),
                    relative_path: relative_path.clone(),
                    line_number: line_idx + 1, // 1-based
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

impl Modal for ContentSearchModal {
    type Result = ContentSearchResultItem;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        // Check for async search results
        self.check_search_results();

        // Check if input changed
        let current_text = self.input_handler.text().to_string();
        if current_text != self.last_input_text {
            self.last_input_text = current_text;
            self.last_input_time = Some(Instant::now());
        }

        // Check debounce timer and start search
        if let Some(time) = self.last_input_time {
            if time.elapsed() >= DEBOUNCE_DELAY {
                self.last_input_time = None;
                self.start_search();
            }
        }

        let modal_width = self.calculate_modal_width(area.width);

        // Height: border + input(3) + separator(1) + results (4 lines each) + border
        let visible_results = self.results.len().min(MAX_VISIBLE_RESULTS);
        let list_height = if visible_results == 0 {
            1
        } else {
            (visible_results * 4) as u16
        };
        let modal_height = (2 + 3 + 1 + list_height).min(area.height - 2);

        let modal_area = centered_rect_with_size(modal_width, modal_height, area);
        self.last_modal_area = Some(modal_area);

        let inner = render_modal_block(modal_area, buf, &self.title, theme);

        // Split into input field and results list
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Input
                Constraint::Min(1),    // Results
            ])
            .split(inner);

        // Render input field
        let input_line = Line::from(vec![
            Span::styled(
                self.input_handler.text_before_cursor(),
                Style::default().fg(theme.fg),
            ),
            Span::styled("█", Style::default().fg(theme.bg).bg(theme.fg)),
            Span::styled(
                self.input_handler.text_after_cursor(),
                Style::default().fg(theme.fg),
            ),
        ]);

        let input_paragraph = Paragraph::new(input_line)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.accented_fg)),
            )
            .style(Style::default().bg(theme.bg));
        input_paragraph.render(chunks[0], buf);

        // Render results list
        let list_area = chunks[1];
        self.last_list_area = Some(list_area);

        if let Some(error) = &self.regex_error {
            // Show regex error
            let message = format!("Regex error: {}", error);
            let hint = Paragraph::new(message).style(Style::default().fg(theme.error));
            hint.render(list_area, buf);
        } else if self.is_searching {
            // Show searching indicator
            let message = "Searching…";
            let hint = Paragraph::new(message).style(
                Style::default()
                    .fg(theme.accented_bg)
                    .add_modifier(Modifier::DIM),
            );
            hint.render(list_area, buf);
        } else if self.results.is_empty() {
            // Show "no results" or hint message
            let message = if self.input_handler.is_empty() {
                "Enter regex pattern"
            } else {
                "No matches found"
            };
            let hint = Paragraph::new(message).style(
                Style::default()
                    .fg(theme.accented_bg)
                    .add_modifier(Modifier::DIM),
            );
            hint.render(list_area, buf);
        } else {
            // Render results (4 lines per result)
            let mut y = list_area.y;
            let content_width = list_area.width as usize;

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

            for (idx, item) in self
                .results
                .iter()
                .enumerate()
                .skip(self.scroll_offset)
                .take(MAX_VISIBLE_RESULTS)
            {
                if y + 4 > list_area.y + list_area.height {
                    break;
                }

                let is_selected = idx == self.cursor;

                // Line 1: path:line_number
                let path_text = format!(
                    "{} {}:{}",
                    if is_selected { "▶" } else { " " },
                    item.relative_path,
                    item.line_number
                );
                let path_style = if is_selected {
                    Style::default()
                        .fg(theme.fg)
                        .bg(theme.accented_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg)
                };
                let padding = " ".repeat(content_width.saturating_sub(path_text.chars().count()));
                buf.set_string(list_area.x, y, &path_text, path_style);
                buf.set_string(
                    list_area.x + path_text.chars().count() as u16,
                    y,
                    &padding,
                    path_style,
                );
                y += 1;

                // Helper to fill line background
                let fill_bg = |buf: &mut Buffer, row: u16| {
                    for col in 0..content_width {
                        buf.set_string(
                            list_area.x + col as u16,
                            row,
                            " ",
                            Style::default().bg(editor_bg),
                        );
                    }
                };

                // Line 2: previous line (if exists)
                fill_bg(buf, y);
                if let Some(ref line_before) = item.line_before {
                    let line_num = format!("{:>4}", item.line_number - 1);
                    let content = Self::truncate_from_start(line_before, max_text_width);

                    buf.set_string(list_area.x, y, &line_num, line_num_style);
                    buf.set_string(
                        list_area.x + line_num_width as u16,
                        y,
                        separator,
                        separator_style,
                    );
                    buf.set_string(
                        list_area.x + (line_num_width + separator_len) as u16,
                        y,
                        &content,
                        context_text_style,
                    );
                }
                y += 1;

                // Line 3: matched line with highlighted match (centered if too long)
                fill_bg(buf, y);
                let line_num = format!("{:>4}", item.line_number);

                buf.set_string(list_area.x, y, &line_num, line_num_style);
                buf.set_string(
                    list_area.x + line_num_width as u16,
                    y,
                    separator,
                    separator_style,
                );

                let content_start_x = list_area.x + (line_num_width + separator_len) as u16;

                // Prepare line with centering if needed
                let (display_line, match_start_g, match_end_g) = Self::prepare_matched_line(
                    &item.matched_line,
                    item.match_start,
                    item.match_end,
                    max_text_width,
                );

                // Render character by character with highlight
                let mut x = content_start_x;
                for (grapheme_idx, grapheme) in display_line.graphemes(true).enumerate() {
                    let style = if grapheme_idx >= match_start_g && grapheme_idx < match_end_g {
                        highlight_style
                    } else {
                        matched_text_style
                    };
                    buf.set_string(x, y, grapheme, style);
                    x += unicode_width::UnicodeWidthStr::width(grapheme) as u16;
                }
                y += 1;

                // Line 4: next line (if exists)
                fill_bg(buf, y);
                if let Some(ref line_after) = item.line_after {
                    let line_num = format!("{:>4}", item.line_number + 1);
                    let content = Self::truncate_from_start(line_after, max_text_width);

                    buf.set_string(list_area.x, y, &line_num, line_num_style);
                    buf.set_string(
                        list_area.x + line_num_width as u16,
                        y,
                        separator,
                        separator_style,
                    );
                    buf.set_string(
                        list_area.x + (line_num_width + separator_len) as u16,
                        y,
                        &content,
                        context_text_style,
                    );
                }
                y += 1;
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>> {
        // Escape always cancels
        if key.code == KeyCode::Esc {
            // Cancel any running search
            if let Some(cancel) = self.search_cancel.take() {
                cancel.store(true, Ordering::Relaxed);
            }
            return Ok(Some(ModalResult::Cancelled));
        }

        match key.code {
            KeyCode::Enter => {
                // Confirm selection
                if let Some(result) = self.get_selected_result() {
                    Ok(Some(ModalResult::Confirmed(result)))
                } else {
                    Ok(None)
                }
            }
            KeyCode::Up => {
                self.cursor_up();
                Ok(None)
            }
            KeyCode::Down => {
                self.cursor_down();
                Ok(None)
            }
            KeyCode::PageUp => {
                self.cursor_page_up();
                Ok(None)
            }
            KeyCode::PageDown => {
                self.cursor_page_down();
                Ok(None)
            }
            KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor_home();
                Ok(None)
            }
            KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor_end();
                Ok(None)
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(None);
                }
                self.input_handler.insert_char(c);
                Ok(None)
            }
            KeyCode::Backspace => {
                self.input_handler.backspace();
                Ok(None)
            }
            KeyCode::Delete => {
                self.input_handler.delete();
                Ok(None)
            }
            KeyCode::Left => {
                self.input_handler.move_left();
                Ok(None)
            }
            KeyCode::Right => {
                self.input_handler.move_right();
                Ok(None)
            }
            KeyCode::Home => {
                self.input_handler.move_home();
                Ok(None)
            }
            KeyCode::End => {
                self.input_handler.move_end();
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        _modal_area: Rect,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        use crate::{check_mouse_click_with_item_height, MouseClickResult};

        // Only handle left button press
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return Ok(None);
        }

        // Content search items are 4 lines each
        const LINES_PER_ITEM: usize = 4;

        match check_mouse_click_with_item_height(
            mouse.column,
            mouse.row,
            self.last_modal_area,
            self.last_list_area,
            self.scroll_offset,
            LINES_PER_ITEM,
        ) {
            MouseClickResult::OutsideModal => Ok(Some(ModalResult::Cancelled)),
            MouseClickResult::OutsideList => Ok(None),
            MouseClickResult::OnListItem(clicked_index) => {
                if clicked_index < self.results.len() {
                    self.cursor = clicked_index;
                    // Return selected item on click
                    if let Some(result) = self.get_selected_result() {
                        return Ok(Some(ModalResult::Confirmed(result)));
                    }
                }
                Ok(None)
            }
        }
    }
}
