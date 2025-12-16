//! File search modal dialog for finding files by glob patterns.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Widget},
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use termide_git::{GitStatus, GitStatusCache};
use termide_theme::Theme;

use crate::{
    calculate_modal_width, centered_rect_with_size, Modal, ModalResult, ModalWidthConfig,
    TextInputHandler,
};

/// Maximum number of search results visible at once
const MAX_VISIBLE_RESULTS: usize = 15;

/// Maximum total results to collect
const MAX_RESULTS: usize = 500;

/// Debounce delay before starting search
const DEBOUNCE_DELAY: Duration = Duration::from_millis(150);

/// Search result item
#[derive(Debug, Clone)]
pub struct SearchResultItem {
    /// Full path to the file/directory
    pub full_path: PathBuf,
    /// Path relative to base directory (for display)
    pub relative_path: String,
    /// Git status for coloring
    pub git_status: GitStatus,
    /// Whether this is a directory
    pub is_dir: bool,
}

/// File search modal window
pub struct FileSearchModal {
    title: String,
    input_handler: TextInputHandler,
    base_path: PathBuf,
    results: Vec<SearchResultItem>,
    cursor: usize,
    scroll_offset: usize,
    git_cache: Option<GitStatusCache>,
    last_list_area: Option<Rect>,

    // Async search state
    last_input_text: String,
    last_input_time: Option<Instant>,
    search_receiver: Option<mpsc::Receiver<Vec<SearchResultItem>>>,
    search_cancel: Option<Arc<AtomicBool>>,
    is_searching: bool,
}

impl std::fmt::Debug for FileSearchModal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSearchModal")
            .field("title", &self.title)
            .field("base_path", &self.base_path)
            .field("results_count", &self.results.len())
            .field("cursor", &self.cursor)
            .field("is_searching", &self.is_searching)
            .finish()
    }
}

impl FileSearchModal {
    /// Create a new file search modal
    pub fn new(title: impl Into<String>, base_path: PathBuf) -> Self {
        // Get git status for the base path
        let git_cache = termide_git::get_git_status(&base_path);

        Self {
            title: title.into(),
            input_handler: TextInputHandler::new(),
            base_path,
            results: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            git_cache,
            last_list_area: None,
            last_input_text: String::new(),
            last_input_time: None,
            search_receiver: None,
            search_cancel: None,
            is_searching: false,
        }
    }

    /// Calculate dynamic modal width
    fn calculate_modal_width(&self, screen_width: u16) -> u16 {
        let title_width = self.title.len() as u16 + 4;
        let min_width = 50u16;

        // Find max result path width
        let max_path_width = self
            .results
            .iter()
            .map(|item| item.relative_path.len() as u16 + 4)
            .max()
            .unwrap_or(40);

        calculate_modal_width(
            [title_width, min_width, max_path_width].into_iter(),
            screen_width,
            ModalWidthConfig::wide(),
        )
    }

    /// Start async file search
    fn start_search(&mut self) {
        let pattern = self.input_handler.text().to_string();

        // Don't search if pattern is empty
        if pattern.is_empty() {
            self.results.clear();
            self.cursor = 0;
            self.scroll_offset = 0;
            self.is_searching = false;
            return;
        }

        // Cancel previous search
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }

        let cancel = Arc::new(AtomicBool::new(false));
        self.search_cancel = Some(cancel.clone());

        let (tx, rx) = mpsc::channel();
        let base_path = self.base_path.clone();
        let git_cache = self.git_cache.clone();

        self.search_receiver = Some(rx);
        self.is_searching = true;

        std::thread::spawn(move || {
            let results = search_files(&base_path, &pattern, &cancel, git_cache.as_ref());
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

    /// Move cursor up
    fn cursor_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.adjust_scroll();
        }
    }

    /// Move cursor down
    fn cursor_down(&mut self) {
        if self.cursor < self.results.len().saturating_sub(1) {
            self.cursor += 1;
            self.adjust_scroll();
        }
    }

    /// Move to first result
    fn cursor_home(&mut self) {
        self.cursor = 0;
        self.scroll_offset = 0;
    }

    /// Move to last result
    fn cursor_end(&mut self) {
        self.cursor = self.results.len().saturating_sub(1);
        self.adjust_scroll();
    }

    /// Adjust scroll to keep cursor visible
    fn adjust_scroll(&mut self) {
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        } else if self.cursor >= self.scroll_offset + MAX_VISIBLE_RESULTS {
            self.scroll_offset = self.cursor - MAX_VISIBLE_RESULTS + 1;
        }
    }

    /// Get the selected result's full path
    fn get_selected_path(&self) -> Option<PathBuf> {
        self.results
            .get(self.cursor)
            .map(|item| item.full_path.clone())
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
            GitStatus::Unmodified => Style::default().fg(theme.bg),
        }
    }
}

/// Perform file search using ignore crate (runs in background thread)
fn search_files(
    base_path: &Path,
    pattern: &str,
    cancel: &AtomicBool,
    git_cache: Option<&GitStatusCache>,
) -> Vec<SearchResultItem> {
    use ignore::WalkBuilder;

    // Determine search mode: glob for wildcards, substring for plain text
    let has_wildcards = pattern.contains('*') || pattern.contains('?');
    let glob_pattern = if has_wildcards {
        match glob::Pattern::new(pattern) {
            Ok(g) => Some(g),
            Err(_) => return Vec::new(),
        }
    } else {
        None
    };
    let pattern_lower = pattern.to_lowercase();

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

        // Skip the base path itself
        if path == base_path {
            continue;
        }

        // Match against file/dir name
        let name = match path.file_name() {
            Some(n) => n.to_string_lossy(),
            None => continue,
        };

        // Use glob matching for wildcards, case-insensitive substring for plain text
        let name_matches = if let Some(ref glob) = glob_pattern {
            glob.matches(&name)
        } else {
            name.to_lowercase().contains(&pattern_lower)
        };

        if !name_matches {
            continue;
        }

        let relative_path = path
            .strip_prefix(base_path)
            .map(|r| r.display().to_string())
            .unwrap_or_default();

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

        results.push(SearchResultItem {
            full_path: path.to_path_buf(),
            relative_path,
            git_status,
            is_dir,
        });

        if results.len() >= MAX_RESULTS {
            break;
        }
    }

    // Sort by relative path
    results.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    results
}

impl Modal for FileSearchModal {
    type Result = PathBuf;

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

        // Height: border + input(3) + separator(1) + results + border
        let visible_results = self.results.len().min(MAX_VISIBLE_RESULTS);
        let list_height = if visible_results == 0 {
            1
        } else {
            visible_results as u16
        };
        let modal_height = (2 + 3 + 1 + list_height).min(area.height - 2);

        let modal_area = centered_rect_with_size(modal_width, modal_height, area);
        Clear.render(modal_area, buf);

        // Create block with inverted colors
        let block = Block::default()
            .title(Span::styled(
                format!(" {} ", self.title),
                Style::default().fg(theme.bg).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.bg))
            .style(Style::default().bg(theme.fg));

        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

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
                Style::default().fg(theme.bg),
            ),
            Span::styled("█", Style::default().fg(theme.success)),
            Span::styled(
                self.input_handler.text_after_cursor(),
                Style::default().fg(theme.bg),
            ),
        ]);

        let input_paragraph = Paragraph::new(input_line)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.success)),
            )
            .style(Style::default().bg(theme.fg));
        input_paragraph.render(chunks[0], buf);

        // Render results list
        let list_area = chunks[1];
        self.last_list_area = Some(list_area);

        if self.is_searching {
            // Show searching indicator
            let message = "Searching...";
            let hint = Paragraph::new(message).style(
                Style::default()
                    .fg(theme.accented_bg)
                    .add_modifier(Modifier::DIM),
            );
            hint.render(list_area, buf);
        } else if self.results.is_empty() {
            // Show "no results" or hint message
            let message = if self.input_handler.is_empty() {
                "*.rs, test?.*"
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
            // Build list items
            let list_items: Vec<ListItem> = self
                .results
                .iter()
                .enumerate()
                .skip(self.scroll_offset)
                .take(MAX_VISIBLE_RESULTS)
                .map(|(idx, item)| {
                    let is_selected = idx == self.cursor;

                    let prefix = if is_selected { "▶ " } else { "  " };

                    let style = if is_selected {
                        Style::default()
                            .fg(theme.fg)
                            .bg(theme.accented_fg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        self.get_git_style(item.git_status, theme)
                    };

                    // Pad to full width
                    let text = format!("{}{}", prefix, item.relative_path);
                    let padding = " ".repeat((list_area.width as usize).saturating_sub(text.len()));

                    let line = Line::from(vec![
                        Span::styled(text, style),
                        Span::styled(padding, style),
                    ]);

                    ListItem::new(line)
                })
                .collect();

            let list = List::new(list_items).style(Style::default().bg(theme.fg));
            list.render(list_area, buf);
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
                if let Some(path) = self.get_selected_path() {
                    Ok(Some(ModalResult::Confirmed(path)))
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
                for _ in 0..MAX_VISIBLE_RESULTS {
                    self.cursor_up();
                }
                Ok(None)
            }
            KeyCode::PageDown => {
                for _ in 0..MAX_VISIBLE_RESULTS {
                    self.cursor_down();
                }
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
        // Only handle left button press
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return Ok(None);
        }

        let Some(list_area) = self.last_list_area else {
            return Ok(None);
        };

        // Check if click is within list area
        if mouse.row < list_area.y
            || mouse.row >= list_area.y + list_area.height
            || mouse.column < list_area.x
            || mouse.column >= list_area.x + list_area.width
        {
            return Ok(None);
        }

        // Calculate which item was clicked
        let relative_row = (mouse.row - list_area.y) as usize;
        let clicked_index = self.scroll_offset + relative_row;

        if clicked_index < self.results.len() {
            self.cursor = clicked_index;
            // Double click or single click to select
            if let Some(path) = self.get_selected_path() {
                return Ok(Some(ModalResult::Confirmed(path)));
            }
        }

        Ok(None)
    }
}
