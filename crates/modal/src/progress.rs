//! Progress display modal for long-running operations.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Widget},
};

use termide_config::constants::{SPINNER_FRAMES, SPINNER_FRAMES_COUNT};
use termide_theme::Theme;
use termide_ui::path_utils::truncate_left;

use crate::{base::button_style, centered_rect_with_size, Modal, ModalResult};

/// Progress modal window for showing operation progress
#[derive(Debug)]
pub struct ProgressModal {
    title: String,
    current: usize,
    total: usize,
    current_item: Option<String>,
    spinner_frame: usize,
    can_cancel: bool,
    /// Show visible Pause/Cancel buttons
    show_buttons: bool,
    /// Can user pause operation
    pause_enabled: bool,
    /// Current pause state
    paused: bool,
    /// 0=Pause, 1=Cancel (keyboard nav)
    selected_button: usize,
    /// Mouse click detection areas
    button_areas: Vec<Rect>,
    /// Display string for source path (formatted with user@host if remote)
    source_display: Option<String>,
    /// Display string for destination path
    dest_display: Option<String>,
    /// Byte-level progress: current file bytes copied
    current_file_bytes: u64,
    /// Byte-level progress: total file bytes
    total_file_bytes: u64,
    /// Transfer speed in bytes per second
    transfer_speed_bps: f64,
    /// Last update time for speed calculation
    last_update: Option<std::time::Instant>,
    /// Last byte count for speed calculation
    last_bytes: u64,
    /// Scanning mode: shows file count and total size during directory scan
    scanning_mode: bool,
    /// Files found during scan
    scan_files_count: usize,
    /// Total bytes found during scan
    scan_total_bytes: u64,
    /// Current directory being scanned
    scan_current_dir: Option<String>,
    /// Individual file progress: bytes downloaded of current file
    individual_file_bytes: u64,
    /// Individual file progress: total bytes of current file
    individual_file_total: u64,
}

impl ProgressModal {
    /// Create a new progress modal
    pub fn new(title: impl Into<String>, total: usize) -> Self {
        Self {
            title: title.into(),
            current: 0,
            total,
            current_item: None,
            spinner_frame: 0,
            can_cancel: false,
            show_buttons: false,
            pause_enabled: false,
            paused: false,
            selected_button: 0,
            button_areas: Vec::new(),
            source_display: None,
            dest_display: None,
            current_file_bytes: 0,
            total_file_bytes: 0,
            transfer_speed_bps: 0.0,
            last_update: None,
            last_bytes: 0,
            scanning_mode: false,
            scan_files_count: 0,
            scan_total_bytes: 0,
            scan_current_dir: None,
            individual_file_bytes: 0,
            individual_file_total: 0,
        }
    }

    /// Create a new cancellable progress modal
    pub fn new_cancellable(title: impl Into<String>, total: usize) -> Self {
        Self {
            title: title.into(),
            current: 0,
            total,
            current_item: None,
            spinner_frame: 0,
            can_cancel: true,
            show_buttons: false,
            pause_enabled: false,
            paused: false,
            selected_button: 0,
            button_areas: Vec::new(),
            source_display: None,
            dest_display: None,
            current_file_bytes: 0,
            total_file_bytes: 0,
            transfer_speed_bps: 0.0,
            last_update: None,
            last_bytes: 0,
            scanning_mode: false,
            scan_files_count: 0,
            scan_total_bytes: 0,
            scan_current_dir: None,
            individual_file_bytes: 0,
            individual_file_total: 0,
        }
    }

    /// Create an indeterminate progress modal (for operations without known total)
    pub fn indeterminate(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            current: 0,
            total: 0, // 0 means indeterminate
            current_item: Some(message.into()),
            spinner_frame: 0,
            can_cancel: false,
            show_buttons: false,
            pause_enabled: false,
            paused: false,
            selected_button: 0,
            button_areas: Vec::new(),
            source_display: None,
            dest_display: None,
            current_file_bytes: 0,
            total_file_bytes: 0,
            transfer_speed_bps: 0.0,
            last_update: None,
            last_bytes: 0,
            scanning_mode: false,
            scan_files_count: 0,
            scan_total_bytes: 0,
            scan_current_dir: None,
            individual_file_bytes: 0,
            individual_file_total: 0,
        }
    }

    /// Create progress modal with visible Pause/Cancel buttons
    pub fn new_with_controls(title: impl Into<String>, total: usize, pause_enabled: bool) -> Self {
        Self {
            title: title.into(),
            current: 0,
            total,
            current_item: None,
            spinner_frame: 0,
            can_cancel: true,
            show_buttons: true,
            pause_enabled,
            paused: false,
            selected_button: 0,
            button_areas: Vec::new(),
            source_display: None,
            dest_display: None,
            current_file_bytes: 0,
            total_file_bytes: 0,
            transfer_speed_bps: 0.0,
            last_update: None,
            last_bytes: 0,
            scanning_mode: false,
            scan_files_count: 0,
            scan_total_bytes: 0,
            scan_current_dir: None,
            individual_file_bytes: 0,
            individual_file_total: 0,
        }
    }

    /// Create progress modal with source/destination paths for copy/move operations
    pub fn new_copy_progress(
        title: impl Into<String>,
        total: usize,
        source_display: String,
        dest_display: String,
        pause_enabled: bool,
    ) -> Self {
        Self {
            title: title.into(),
            current: 0,
            total,
            current_item: None,
            spinner_frame: 0,
            can_cancel: true,
            show_buttons: true,
            pause_enabled,
            paused: false,
            selected_button: 0,
            button_areas: Vec::new(),
            source_display: Some(source_display),
            dest_display: Some(dest_display),
            current_file_bytes: 0,
            total_file_bytes: 0,
            transfer_speed_bps: 0.0,
            last_update: None,
            last_bytes: 0,
            scanning_mode: false,
            scan_files_count: 0,
            scan_total_bytes: 0,
            scan_current_dir: None,
            individual_file_bytes: 0,
            individual_file_total: 0,
        }
    }

    /// Create a scanning progress modal for directory scanning
    pub fn new_scanning(title: impl Into<String>, source_path: String) -> Self {
        Self {
            title: title.into(),
            current: 0,
            total: 0,
            current_item: None,
            spinner_frame: 0,
            can_cancel: true,
            show_buttons: true,
            pause_enabled: false,
            paused: false,
            selected_button: 0,
            button_areas: Vec::new(),
            source_display: Some(source_path),
            dest_display: None,
            current_file_bytes: 0,
            total_file_bytes: 0,
            transfer_speed_bps: 0.0,
            last_update: None,
            last_bytes: 0,
            scanning_mode: true,
            scan_files_count: 0,
            scan_total_bytes: 0,
            scan_current_dir: None,
            individual_file_bytes: 0,
            individual_file_total: 0,
        }
    }

    /// Create a delete progress modal
    pub fn new_delete_progress(total: usize, source_path: String) -> Self {
        Self {
            title: "Delete".into(),
            current: 0,
            total,
            current_item: None,
            spinner_frame: 0,
            can_cancel: true,
            show_buttons: true,
            pause_enabled: false,
            paused: false,
            selected_button: 0,
            button_areas: Vec::new(),
            source_display: Some(source_path),
            dest_display: None, // No destination for delete
            current_file_bytes: 0,
            total_file_bytes: 0, // No byte tracking for delete
            transfer_speed_bps: 0.0,
            last_update: None,
            last_bytes: 0,
            scanning_mode: false,
            scan_files_count: 0,
            scan_total_bytes: 0,
            scan_current_dir: None,
            individual_file_bytes: 0,
            individual_file_total: 0,
        }
    }

    /// Update delete progress
    pub fn update_delete_progress(&mut self, files_deleted: usize, total_files: usize) {
        self.current = files_deleted;
        self.total = total_files;
    }

    /// Check if operation is paused
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Toggle pause state
    pub fn toggle_pause(&mut self) {
        if self.pause_enabled {
            self.paused = !self.paused;
        }
    }

    /// Get selected button (0=Pause, 1=Cancel)
    pub fn selected_button(&self) -> usize {
        self.selected_button
    }

    /// Select next button (for keyboard nav)
    pub fn next_button(&mut self) {
        if self.show_buttons {
            self.selected_button = (self.selected_button + 1) % 2;
        }
    }

    /// Select previous button (for keyboard nav)
    pub fn prev_button(&mut self) {
        if self.show_buttons {
            self.selected_button = if self.selected_button == 0 {
                1 // Wrap to last button
            } else {
                self.selected_button - 1
            };
        }
    }

    /// Update progress
    pub fn update_progress(&mut self, current: usize, item: Option<String>) {
        self.current = current;
        self.current_item = item;
    }

    /// Update progress with new source/destination paths
    pub fn update_progress_with_paths(
        &mut self,
        current: usize,
        source_display: String,
        dest_display: String,
    ) {
        self.current = current;
        self.source_display = Some(source_display);
        self.dest_display = Some(dest_display);
        // Reset byte progress for new file
        self.current_file_bytes = 0;
        self.total_file_bytes = 0;
        self.transfer_speed_bps = 0.0;
        self.last_update = None;
        self.last_bytes = 0;
    }

    /// Update source and destination display (for batch uploads)
    pub fn update_source_dest(&mut self, source_display: String, dest_display: String) {
        self.source_display = Some(source_display);
        self.dest_display = Some(dest_display);
    }

    /// Update file-level byte progress
    pub fn update_file_progress(&mut self, bytes_copied: u64, total_bytes: u64) {
        let now = std::time::Instant::now();

        // Calculate transfer speed
        if let Some(last_update) = self.last_update {
            let elapsed = now.duration_since(last_update).as_secs_f64();
            if elapsed > 0.0 {
                let bytes_delta = bytes_copied.saturating_sub(self.last_bytes);
                self.transfer_speed_bps = bytes_delta as f64 / elapsed;
            }
        }

        self.current_file_bytes = bytes_copied;
        self.total_file_bytes = total_bytes;
        self.last_update = Some(now);
        self.last_bytes = bytes_copied;
    }

    /// Update directory copy progress (files count + bytes)
    pub fn update_directory_copy_progress(
        &mut self,
        files_completed: usize,
        total_files: usize,
        bytes_copied: u64,
        total_bytes: u64,
    ) {
        // Update file count
        self.current = files_completed;
        self.total = total_files;

        // Update byte progress with speed calculation
        self.update_file_progress(bytes_copied, total_bytes);
    }

    /// Update individual file progress (for chunked downloads)
    pub fn update_individual_file_progress(&mut self, current_bytes: u64, total_bytes: u64) {
        self.individual_file_bytes = current_bytes;
        self.individual_file_total = total_bytes;
    }

    /// Update scan progress during directory scanning
    pub fn update_scan_progress(
        &mut self,
        files_count: usize,
        total_bytes: u64,
        current_dir: Option<String>,
    ) {
        self.scan_files_count = files_count;
        self.scan_total_bytes = total_bytes;
        self.scan_current_dir = current_dir;
    }

    /// Check if modal is in scanning mode
    pub fn is_scanning(&self) -> bool {
        self.scanning_mode
    }

    /// Exit scanning mode and prepare for copy operation
    pub fn finish_scanning(
        &mut self,
        total_files: usize,
        total_bytes: u64,
        dest_display: String,
        title: impl Into<String>,
    ) {
        self.scanning_mode = false;
        self.title = title.into();
        self.total = total_files;
        self.current = 0;
        self.total_file_bytes = total_bytes;
        self.current_file_bytes = 0;
        self.dest_display = Some(dest_display);
        self.pause_enabled = true; // Enable Suspend button for copy/move
    }

    /// Advance the spinner frame counter (for animation)
    pub fn advance_spinner(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES_COUNT;
    }

    /// Get the current spinner character
    fn get_spinner_char(&self) -> &str {
        SPINNER_FRAMES[self.spinner_frame]
    }

    /// Calculate progress percentage based on file count
    fn progress_percentage(&self) -> u16 {
        if self.total == 0 {
            return 0;
        }
        ((self.current as f64 / self.total as f64) * 100.0) as u16
    }

    /// Check if individual file progress is being tracked
    fn has_individual_file_progress(&self) -> bool {
        self.individual_file_total > 0
    }
}

/// Format bytes to human-readable string (MB, GB, etc.)
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Format bytes per second to human-readable string
fn format_speed(bytes_per_sec: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    if bytes_per_sec >= GB {
        format!("{:.1} GB/s", bytes_per_sec / GB)
    } else if bytes_per_sec >= MB {
        format!("{:.1} MB/s", bytes_per_sec / MB)
    } else if bytes_per_sec >= KB {
        format!("{:.1} KB/s", bytes_per_sec / KB)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}

/// Render custom progress bar with bracket delimiters
fn render_custom_progress_bar(current: usize, total: usize, width: usize) -> String {
    if total == 0 {
        return format!("[ {} ]", " ".repeat(width.saturating_sub(4)));
    }

    let bar_width = width.saturating_sub(4); // Subtract [ ] and spaces
    let filled_count = ((current as f64 / total as f64) * bar_width as f64) as usize;
    let empty_count = bar_width.saturating_sub(filled_count);

    format!(
        "[ {}{} ]",
        "█".repeat(filled_count),
        " ".repeat(empty_count)
    )
}

impl Modal for ProgressModal {
    type Result = bool; // true = continue, false = cancelled

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let modal_width = 60;
        // Adjust height based on mode and buttons
        let modal_height = if self.scanning_mode {
            // Scanning mode: 8 lines (padding + current dir + empty + stats + empty + buttons + padding)
            8
        } else if self.source_display.is_some() && self.dest_display.is_some() {
            // Copy/move layout (12 content lines + 2 borders)
            14
        } else if self.source_display.is_some() && self.dest_display.is_none() {
            // Delete layout: padding + source + progress bar + empty + files + empty + buttons + padding = 8 + 2 borders
            10
        } else if self.show_buttons {
            if self.total == 0 {
                7
            } else {
                9
            } // Indeterminate: 7, Determinate: 9
        } else if self.total == 0 {
            5
        } else {
            7
        }; // Indeterminate: 5, Determinate: 7

        let modal_area = centered_rect_with_size(modal_width, modal_height, area);

        // Clear the modal area to hide panels behind it
        Clear.render(modal_area, buf);

        // Render outer block
        let block = Block::default()
            .borders(Borders::ALL)
            .title(self.title.clone())
            .style(Style::default().fg(theme.fg).bg(theme.bg));
        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        // Scanning mode - show scan progress
        if self.scanning_mode {
            // Add 1 char horizontal padding on each side
            let padded_inner = Rect {
                x: inner.x + 1,
                y: inner.y,
                width: inner.width.saturating_sub(2),
                height: inner.height,
            };

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Empty padding
                    Constraint::Length(1), // Current directory being scanned
                    Constraint::Length(1), // Empty line
                    Constraint::Length(1), // Stats (files / size)
                    Constraint::Length(1), // Empty padding
                    Constraint::Length(1), // Cancel button
                ])
                .split(padded_inner);

            // Current directory being scanned (truncate from left)
            let dir_text = if let Some(ref current_dir) = self.scan_current_dir {
                let max_path_width = padded_inner.width as usize;
                truncate_left(current_dir, max_path_width)
            } else if let Some(ref source) = self.source_display {
                let max_path_width = padded_inner.width as usize;
                truncate_left(source, max_path_width)
            } else {
                String::new()
            };
            let dir_para = Paragraph::new(dir_text).style(Style::default().fg(theme.fg));
            dir_para.render(chunks[1], buf);

            // Stats: files count and total size
            let stats_text = format!(
                "Files: {}  |  Size: {}",
                self.scan_files_count,
                format_bytes(self.scan_total_bytes)
            );
            let stats_para = Paragraph::new(stats_text)
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.accented_fg));
            stats_para.render(chunks[3], buf);

            // Cancel button
            let cancel_style = button_style(true, theme);
            let cancel_span = Span::styled("[ Cancel ]", cancel_style);
            let cancel_line = Line::from(vec![cancel_span]);
            let cancel_para = Paragraph::new(cancel_line).alignment(Alignment::Center);
            cancel_para.render(chunks[5], buf);

            self.button_areas = vec![chunks[5]];
            return;
        }

        // Check if this is delete modal (source only, no destination)
        if let (Some(ref source), None) = (&self.source_display, &self.dest_display) {
            // Add 1 char horizontal padding on each side
            let padded_inner = Rect {
                x: inner.x + 1,
                y: inner.y,
                width: inner.width.saturating_sub(2),
                height: inner.height,
            };

            // Delete layout: simpler than copy/move (no Data/Speed)
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Empty padding
                    Constraint::Length(1), // Source line
                    Constraint::Length(1), // Progress bar
                    Constraint::Length(1), // Empty line
                    Constraint::Length(1), // Files: "Files: 3 / 10"
                    Constraint::Length(1), // Empty padding
                    Constraint::Length(1), // Buttons
                    Constraint::Length(1), // Empty padding
                ])
                .split(padded_inner);

            // Render source path
            let max_path_width = modal_width.saturating_sub(4) as usize;
            let source_para = Paragraph::new(truncate_left(source, max_path_width))
                .style(Style::default().fg(theme.fg));
            source_para.render(chunks[1], buf);

            // Render progress bar
            let bar_width = padded_inner.width as usize;
            let progress_bar_text = render_custom_progress_bar(self.current, self.total, bar_width);
            let bar_para = Paragraph::new(progress_bar_text)
                .alignment(Alignment::Left)
                .style(Style::default().fg(theme.accented_fg));
            bar_para.render(chunks[2], buf);

            // Files count
            let files_text = if self.total > 0 {
                format!("Files: {} / {}", self.current, self.total)
            } else {
                "Counting files...".to_string()
            };
            let files_para = Paragraph::new(files_text).style(Style::default().fg(theme.fg));
            files_para.render(chunks[4], buf);

            // Render Cancel button
            let cancel_style = button_style(true, theme);
            let cancel_span = Span::styled("[ Cancel ]", cancel_style);
            let cancel_line = Line::from(vec![cancel_span]);
            let cancel_para = Paragraph::new(cancel_line).alignment(Alignment::Center);
            cancel_para.render(chunks[6], buf);

            self.button_areas = vec![chunks[6]];
            return;
        }

        // Check if this is copy/move modal with path display
        if let (Some(ref source), Some(ref dest)) = (&self.source_display, &self.dest_display) {
            // Add 1 char horizontal padding on each side
            let padded_inner = Rect {
                x: inner.x + 1,
                y: inner.y,
                width: inner.width.saturating_sub(2),
                height: inner.height,
            };

            // New copy/move layout with vertical separation
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Empty padding
                    Constraint::Length(1), // Source line
                    Constraint::Length(1), // Destination line
                    Constraint::Length(1), // Progress bar
                    Constraint::Length(1), // Empty line
                    Constraint::Length(1), // Files: "Files: 3 / 10"
                    Constraint::Length(1), // Data: "Data: 45.2 MB / 120.5 MB"
                    Constraint::Length(1), // Speed: "Speed: 15.3 MB/s"
                    Constraint::Length(1), // Total progress: "Total progress: 37%"
                    Constraint::Length(1), // Empty padding
                    Constraint::Length(1), // Buttons
                    Constraint::Length(1), // Empty padding
                ])
                .split(padded_inner);

            // Render source path
            let max_path_width = modal_width.saturating_sub(12) as usize; // "Source: " = 8 chars + margin
            let source_text = format!("Source: {}", truncate_left(source, max_path_width));
            let source_para = Paragraph::new(source_text).style(Style::default().fg(theme.fg));
            source_para.render(chunks[1], buf);

            // Render destination path
            let dest_text = format!(
                "Destination: {}",
                truncate_left(dest, max_path_width.saturating_sub(4))
            );
            let dest_para = Paragraph::new(dest_text).style(Style::default().fg(theme.fg));
            dest_para.render(chunks[2], buf);

            // Render custom progress bar (full width of padded area)
            let bar_width = padded_inner.width as usize;

            // Use individual file progress if available for chunked downloads
            let progress_bar_text = if self.has_individual_file_progress() {
                // Show current file's progress bar (for chunked downloads)
                let filled_count = ((self.individual_file_bytes as f64
                    / self.individual_file_total as f64)
                    * (bar_width as f64 - 4.0)) as usize;
                let empty_count = (bar_width as usize)
                    .saturating_sub(4)
                    .saturating_sub(filled_count);
                format!(
                    "[ {}{} ]",
                    "█".repeat(filled_count),
                    " ".repeat(empty_count)
                )
            } else if self.total_file_bytes > 0 {
                // Fall back to overall byte-level progress bar
                let filled_count = ((self.current_file_bytes as f64 / self.total_file_bytes as f64)
                    * (bar_width as f64 - 4.0)) as usize;
                let empty_count = (bar_width as usize)
                    .saturating_sub(4)
                    .saturating_sub(filled_count);
                format!(
                    "[ {}{} ]",
                    "█".repeat(filled_count),
                    " ".repeat(empty_count)
                )
            } else {
                // Fall back to file count progress
                render_custom_progress_bar(self.current, self.total, bar_width as usize)
            };

            let bar_para = Paragraph::new(progress_bar_text)
                .alignment(Alignment::Left) // No centering
                .style(Style::default().fg(theme.accented_fg));
            bar_para.render(chunks[3], buf);

            // Line 1: Files count
            let files_text = if self.total > 0 {
                format!("Files: {} / {}", self.current, self.total)
            } else {
                String::new()
            };
            let files_para = Paragraph::new(files_text).style(Style::default().fg(theme.fg));
            files_para.render(chunks[5], buf);

            // Line 2: Data progress
            let data_text = if self.total_file_bytes > 0 {
                format!(
                    "Data: {} / {}",
                    format_bytes(self.current_file_bytes),
                    format_bytes(self.total_file_bytes)
                )
            } else {
                String::new()
            };
            let data_para = Paragraph::new(data_text).style(Style::default().fg(theme.fg));
            data_para.render(chunks[6], buf);

            // Line 3: Speed
            let speed_text = if self.transfer_speed_bps > 0.0 {
                format!("Speed: {}", format_speed(self.transfer_speed_bps))
            } else {
                String::new()
            };
            let speed_para = Paragraph::new(speed_text).style(Style::default().fg(theme.fg));
            speed_para.render(chunks[7], buf);

            // Line 4: Total progress percentage
            let progress_text = if self.total_file_bytes > 0 {
                let percentage = ((self.current_file_bytes as f64 / self.total_file_bytes as f64)
                    * 100.0) as u16;
                format!("Total progress: {}%", percentage)
            } else {
                String::new()
            };
            let progress_para = Paragraph::new(progress_text).style(Style::default().fg(theme.fg));
            progress_para.render(chunks[8], buf);

            // Render buttons
            let mut button_spans = Vec::new();

            let pause_text = if self.paused { "Resume" } else { "Suspend" };

            if self.pause_enabled {
                let pause_style = button_style(self.selected_button == 0, theme);
                button_spans.push(Span::styled(format!("[ {} ]", pause_text), pause_style));
                button_spans.push(Span::raw("  "));
            }

            let cancel_button_index = if self.pause_enabled { 1 } else { 0 };
            let cancel_style = button_style(self.selected_button == cancel_button_index, theme);
            button_spans.push(Span::styled("[ Abort ]".to_string(), cancel_style));

            let buttons_line = Line::from(button_spans);
            let buttons_para = Paragraph::new(buttons_line).alignment(Alignment::Center);
            buttons_para.render(chunks[10], buf);

            self.button_areas = vec![chunks[10], chunks[10]];
        } else {
            // Original layout (existing code)
            // Split into sections - different layouts for indeterminate vs determinate mode
            let chunks = if self.total == 0 {
                // Indeterminate mode - no progress bar, compact layout
                if self.show_buttons {
                    Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(1), // Empty line
                            Constraint::Length(1), // Current item (with spinner)
                            Constraint::Length(1), // Empty line
                            Constraint::Length(1), // Buttons row
                            Constraint::Length(1), // Empty line
                        ])
                        .split(inner)
                } else {
                    Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(1), // Empty line
                            Constraint::Length(1), // Current item (with spinner)
                            Constraint::Length(1), // Empty line
                        ])
                        .split(inner)
                }
            } else {
                // Determinate mode - includes progress bar
                if self.show_buttons {
                    Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(1), // Empty line
                            Constraint::Length(1), // Progress text
                            Constraint::Length(1), // Progress bar
                            Constraint::Length(1), // Empty line
                            Constraint::Length(1), // Current item
                            Constraint::Length(1), // Empty line
                            Constraint::Length(1), // Buttons row
                            Constraint::Length(1), // Empty line
                        ])
                        .split(inner)
                } else {
                    Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(1), // Empty line
                            Constraint::Length(1), // Progress text
                            Constraint::Length(1), // Progress bar
                            Constraint::Length(1), // Empty line
                            Constraint::Length(1), // Current item
                            Constraint::Length(1), // Empty line
                        ])
                        .split(inner)
                }
            };

            if self.total == 0 {
                // Indeterminate mode - show spinner with current item
                let text = if let Some(ref item) = self.current_item {
                    let max_item_width = modal_width as usize - 4; // Reserve space for spinner
                    let item_text = truncate_left(item, max_item_width);
                    format!("{} {}", self.get_spinner_char(), item_text)
                } else {
                    self.get_spinner_char().to_string()
                };

                let para = Paragraph::new(text)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(theme.fg));
                para.render(chunks[1], buf);
            } else {
                // Determinate mode - show progress
                let progress_text = format!(
                    "{} {} / {} {}",
                    self.get_spinner_char(),
                    self.current,
                    self.total,
                    if self.total > 1 { "files" } else { "file" }
                );
                let progress_para = Paragraph::new(progress_text)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(theme.fg));
                progress_para.render(chunks[1], buf);

                // Render progress bar
                let percentage = self.progress_percentage();
                let gauge = Gauge::default()
                    .gauge_style(
                        Style::default()
                            .fg(theme.accented_fg)
                            .bg(theme.bg)
                            .add_modifier(Modifier::BOLD),
                    )
                    .percent(percentage);
                gauge.render(chunks[2], buf);

                // Render current item if available
                if let Some(ref item) = self.current_item {
                    let max_item_width = modal_width as usize - 4;
                    let item_text = truncate_left(item, max_item_width);

                    let item_para = Paragraph::new(item_text)
                        .alignment(Alignment::Center)
                        .style(Style::default().fg(theme.disabled));
                    item_para.render(chunks[4], buf);
                }
            }

            // Render buttons if enabled
            if self.show_buttons {
                // Button chunk index differs based on mode
                let button_chunk_idx = if self.total == 0 {
                    3 // Indeterminate mode
                } else {
                    6 // Determinate mode
                };

                // Create button spans for inline rendering (prevents wide background highlight)
                let mut button_spans = Vec::new();

                // Determine pause/resume text
                let pause_text = if self.paused { "Resume" } else { "Pause" };

                // Render Pause/Resume button if enabled
                if self.pause_enabled {
                    let pause_style = button_style(self.selected_button == 0, theme);
                    button_spans.push(Span::styled(format!("[ {} ]", pause_text), pause_style));
                    button_spans.push(Span::raw("  ")); // Spacing between buttons
                }

                // Render Cancel button
                let cancel_button_index = if self.pause_enabled { 1 } else { 0 };
                let cancel_style = button_style(self.selected_button == cancel_button_index, theme);
                button_spans.push(Span::styled("[ Cancel ]".to_string(), cancel_style));

                // Render buttons as single centered line
                let buttons_line = Line::from(button_spans);
                let buttons_para = Paragraph::new(buttons_line).alignment(Alignment::Center);
                buttons_para.render(chunks[button_chunk_idx], buf);

                // Store button area for mouse click detection (all buttons share same chunk)
                self.button_areas = vec![chunks[button_chunk_idx], chunks[button_chunk_idx]];
            }
        } // end of else block for original layout
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>> {
        match key.code {
            KeyCode::Esc if self.can_cancel => {
                // User cancelled
                Ok(Some(ModalResult::Confirmed(false)))
            }

            // Button keyboard navigation (Tab/Right arrow - next button)
            KeyCode::Tab | KeyCode::Right if self.show_buttons => {
                self.next_button();
                Ok(None) // Don't close modal, just update button selection
            }

            // Button keyboard navigation (Shift+Tab/Left arrow - previous button)
            KeyCode::BackTab | KeyCode::Left if self.show_buttons => {
                self.prev_button();
                Ok(None) // Don't close modal, just update button selection
            }

            // Activate selected button (Enter key)
            KeyCode::Enter if self.show_buttons => {
                if self.selected_button == 0 && self.pause_enabled {
                    // Pause/Resume button
                    self.toggle_pause();
                    Ok(None) // Don't close modal on pause/resume
                } else {
                    // Cancel button
                    Ok(Some(ModalResult::Confirmed(false)))
                }
            }

            // Pause shortcut (P key)
            KeyCode::Char('p') | KeyCode::Char('P') if self.pause_enabled => {
                self.toggle_pause();
                Ok(None) // Don't close modal
            }

            _ => Ok(None),
        }
    }

    fn handle_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        _modal_area: Rect,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        use crossterm::event::{MouseButton, MouseEventKind};

        if !self.show_buttons {
            return Ok(None);
        }

        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
            // Check if clicked within button area
            if let Some(button_area) = self.button_areas.first() {
                let in_button_row = mouse.row >= button_area.y
                    && mouse.row < button_area.y + button_area.height
                    && mouse.column >= button_area.x
                    && mouse.column < button_area.x + button_area.width;

                if in_button_row {
                    if self.pause_enabled {
                        // Two buttons: [ Pause/Resume ]  [ Cancel ]
                        // Determine which button based on horizontal position
                        let center_x = button_area.x + button_area.width / 2;
                        if mouse.column < center_x {
                            // Clicked on left side - Pause/Resume button
                            self.toggle_pause();
                            return Ok(None); // Don't close modal
                        } else {
                            // Clicked on right side - Cancel button
                            return Ok(Some(ModalResult::Confirmed(false)));
                        }
                    } else {
                        // Only Cancel button - any click cancels
                        return Ok(Some(ModalResult::Confirmed(false)));
                    }
                }
            }
        }

        Ok(None)
    }
}
