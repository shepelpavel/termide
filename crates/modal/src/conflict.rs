//! File conflict resolution modal dialog.

use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::base::{button_style, render_modal_block};

use termide_theme::Theme;

use crate::{centered_rect_with_size, Modal, ModalResult};

/// Conflict resolution options for copy/move operations
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConflictResolution {
    /// Overwrite existing file
    Overwrite,
    /// Skip this file
    Skip,
    /// Rename this file
    Rename,
    /// Overwrite and apply to all subsequent files
    OverwriteAll,
    /// Skip all subsequent files
    SkipAll,
    /// Rename all subsequent files
    RenameAll,
    /// Cancel the entire operation
    Cancel,
}

/// File conflict resolution modal window
#[derive(Debug)]
pub struct ConflictModal {
    title: String,
    dest_name: String,
    is_directory: bool,
    remaining_items: usize, // Number of items remaining in queue (excluding current)
    selected: usize,
    button_areas: Vec<Rect>,
}

impl ConflictModal {
    /// Create a conflict modal window
    pub fn new(source: &Path, destination: &Path, remaining_items: usize) -> Self {
        Self::with_counter(source, destination, remaining_items, 1, 1)
    }

    /// Create a conflict modal window with conflict counter.
    ///
    /// # Arguments
    /// * `source` - Source file path
    /// * `destination` - Destination file path (existing)
    /// * `remaining_items` - Number of items remaining in queue (excluding current)
    /// * `current_conflict` - Current conflict number (1-indexed)
    /// * `total_conflicts` - Total number of conflicts detected (or best estimate)
    fn with_counter(
        source: &Path,
        destination: &Path,
        remaining_items: usize,
        current_conflict: usize,
        total_conflicts: usize,
    ) -> Self {
        let dest_name = destination
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        // Check both source and destination: for remote paths (VFS URLs
        // stored as PathBuf), is_dir() always returns false, but the source
        // is a local path where is_dir() works correctly.
        let is_directory = source.is_dir() || destination.is_dir();

        // Build title with counter if there are multiple conflicts
        let t = termide_i18n::t();
        let base = if is_directory {
            t.conflict_directory_title()
        } else {
            t.conflict_file_title()
        };
        let title = if total_conflicts > 1 {
            format!("{} ({}/{})", base, current_conflict, total_conflicts)
        } else {
            base.to_string()
        };

        Self {
            title,
            dest_name,
            is_directory,
            remaining_items,
            selected: 0,
            button_areas: Vec::new(),
        }
    }

    fn get_resolution(&self) -> ConflictResolution {
        // If there are no remaining items, only 3 options available
        if self.remaining_items == 0 {
            match self.selected {
                0 => ConflictResolution::Overwrite,
                1 => ConflictResolution::Skip,
                _ => ConflictResolution::Rename,
            }
        } else {
            // All 6 options available
            match self.selected {
                0 => ConflictResolution::Overwrite,
                1 => ConflictResolution::Skip,
                2 => ConflictResolution::Rename,
                3 => ConflictResolution::OverwriteAll,
                4 => ConflictResolution::SkipAll,
                _ => ConflictResolution::RenameAll,
            }
        }
    }
}

impl Modal for ConflictModal {
    type Result = ConflictResolution;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let modal_width = 60;

        // Calculate height: top empty (1) + message (1) + empty (1) + buttons row1 (1) + buttons row2 (1 if needed) + bottom empty (1)
        let modal_height = if self.remaining_items == 0 {
            7 // 1 (top) + 1 (msg) + 1 (empty) + 1 (buttons) + 1 (bottom) + 2 (borders)
        } else {
            8 // 1 (top) + 1 (msg) + 1 (empty) + 2 (buttons) + 1 (bottom) + 2 (borders)
        };

        // Create centered area
        let modal_area = centered_rect_with_size(modal_width, modal_height, area);

        let inner = render_modal_block(modal_area, buf, &self.title, theme);

        // Layout
        let chunks = if self.remaining_items == 0 {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Top empty line
                    Constraint::Length(1), // Message
                    Constraint::Length(1), // Empty line
                    Constraint::Length(1), // Buttons
                    Constraint::Length(1), // Bottom empty line
                ])
                .split(inner)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Top empty line
                    Constraint::Length(1), // Message
                    Constraint::Length(1), // Empty line
                    Constraint::Length(1), // Buttons row 1
                    Constraint::Length(1), // Buttons row 2
                    Constraint::Length(1), // Bottom empty line
                ])
                .split(inner)
        };

        // Conflict message
        let t = termide_i18n::t();
        let item_type = if self.is_directory {
            t.file_type_directory()
        } else {
            t.file_type_file()
        };
        let message = t.conflict_already_exists(item_type, &self.dest_name);
        let prompt = Paragraph::new(message)
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.fg));
        prompt.render(chunks[1], buf); // Changed from chunks[0] to chunks[1]

        // Render buttons
        let has_remaining = self.remaining_items > 0;
        let labels: Vec<&str> = if has_remaining {
            vec![
                t.conflict_overwrite(),
                t.conflict_skip(),
                t.conflict_rename(),
                t.conflict_overwrite_all(),
                t.conflict_skip_all(),
                t.conflict_rename_all(),
            ]
        } else {
            vec![
                t.conflict_overwrite(),
                t.conflict_skip(),
                t.conflict_rename(),
            ]
        };

        self.button_areas.clear();

        if !has_remaining {
            // Single row: [ Overwrite ]  [ Skip ]  [ Rename ]
            let mut button_spans = Vec::new();
            for (i, label) in labels.iter().enumerate() {
                if i > 0 {
                    button_spans.push(Span::raw("  "));
                }
                let style = button_style(i == self.selected, theme);
                button_spans.push(Span::styled(format!("[ {} ]", label), style));
            }

            let buttons_line = Line::from(button_spans);
            let buttons_para = Paragraph::new(buttons_line).alignment(Alignment::Center);
            buttons_para.render(chunks[3], buf); // Changed from chunks[2] to chunks[3]

            // Store button areas (approximate for mouse handling)
            self.button_areas = vec![chunks[3], chunks[3], chunks[3]];
        } else {
            // Row 1: [ Overwrite ]  [ Skip ]  [ Rename ]
            let mut row1_spans = Vec::new();
            for (i, label) in labels.iter().take(3).enumerate() {
                if i > 0 {
                    row1_spans.push(Span::raw("  "));
                }
                let style = button_style(i == self.selected, theme);
                row1_spans.push(Span::styled(format!("[ {} ]", label), style));
            }

            let row1_line = Line::from(row1_spans);
            let row1_para = Paragraph::new(row1_line).alignment(Alignment::Center);
            row1_para.render(chunks[3], buf); // Changed from chunks[2] to chunks[3]

            // Row 2: [ Overwrite All ]  [ Skip All ]  [ Rename All ]
            let mut row2_spans = Vec::new();
            for (i, label) in labels.iter().skip(3).enumerate() {
                if i > 0 {
                    row2_spans.push(Span::raw("  "));
                }
                let idx = i + 3;
                let style = button_style(idx == self.selected, theme);
                row2_spans.push(Span::styled(format!("[ {} ]", label), style));
            }

            let row2_line = Line::from(row2_spans);
            let row2_para = Paragraph::new(row2_line).alignment(Alignment::Center);
            row2_para.render(chunks[4], buf); // Changed from chunks[3] to chunks[4]

            // Store button areas (approximate)
            self.button_areas = vec![
                chunks[3], chunks[3], chunks[3], chunks[4], chunks[4], chunks[4],
            ];
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>> {
        // Calculate max index based on number of options
        let max_index = if self.remaining_items == 0 {
            2 // Only 3 options: Overwrite, Skip, Rename
        } else {
            5 // All 6 options
        };

        match key.code {
            KeyCode::Esc => Ok(Some(ModalResult::Cancelled)),
            KeyCode::Left | KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                } else {
                    self.selected = max_index; // Wrap around
                }
                Ok(None)
            }
            KeyCode::Right | KeyCode::Down => {
                if self.selected < max_index {
                    self.selected += 1;
                } else {
                    self.selected = 0; // Wrap around
                }
                Ok(None)
            }
            KeyCode::Tab => {
                // Tab moves forward
                if self.selected < max_index {
                    self.selected += 1;
                } else {
                    self.selected = 0; // Wrap around
                }
                Ok(None)
            }
            KeyCode::Home => {
                self.selected = 0;
                Ok(None)
            }
            KeyCode::End => {
                self.selected = max_index;
                Ok(None)
            }
            KeyCode::Enter => Ok(Some(ModalResult::Confirmed(self.get_resolution()))),
            _ => Ok(None),
        }
    }

    fn handle_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        _modal_area: Rect,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        use crossterm::event::MouseEventKind;

        // Only handle left button press
        if mouse.kind != MouseEventKind::Down(crossterm::event::MouseButton::Left) {
            return Ok(None);
        }

        // Check if click is within any button area and determine which button
        for (i, button_area) in self.button_areas.iter().enumerate() {
            if mouse.row >= button_area.y
                && mouse.row < button_area.y + button_area.height
                && mouse.column >= button_area.x
                && mouse.column < button_area.x + button_area.width
            {
                // Button clicked - select and confirm immediately
                self.selected = i;
                return Ok(Some(ModalResult::Confirmed(self.get_resolution())));
            }
        }

        Ok(None)
    }
}
