//! Mouse event handling for the Editor panel.
//!
//! This module contains mouse interaction logic including:
//! - Click handling (single, double, triple clicks)
//! - Drag selection
//! - Scroll wheel
//! - Ctrl+click for go-to-definition
//! - Popup interaction (completion, hover)

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use termide_buffer::{Cursor, Selection};
use termide_core::PanelEvent;

use crate::{git, rendering, selection, word_wrap, Editor};

/// Convert screen column to grapheme index, accounting for display widths.
///
/// Used for mouse click position conversion.
fn screen_col_to_grapheme_idx(text: &str, target_col: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    let mut col = 0;
    let mut last_idx = 0;
    for (idx, g) in text.graphemes(true).enumerate() {
        let w = g.width();
        if col + w > target_col {
            return idx;
        }
        col += w;
        last_idx = idx + 1;
    }
    last_idx
}

impl Editor {
    /// Handle mouse events within the editor panel.
    pub(crate) fn handle_mouse_event(
        &mut self,
        mouse: MouseEvent,
        panel_area: Rect,
    ) -> Vec<PanelEvent> {
        // Handle scroll - if any popup is open, scroll always scrolls popup
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                // Priority: completion popup > hover popup > editor
                if let Some(ref mut popup) = self.lsp.completion_popup {
                    popup.scroll_up(3);
                    return vec![];
                }
                if let Some(ref mut popup) = self.lsp.hover_popup {
                    popup.scroll_up(3);
                    return vec![];
                }
                // No popup - scroll editor by visual rows (accounts for word wrap)
                self.scroll_visual_rows_up(3);
                self.scroll_follows_cursor = false;
                return vec![];
            }
            MouseEventKind::ScrollDown => {
                // Priority: completion popup > hover popup > editor
                if let Some(ref mut popup) = self.lsp.completion_popup {
                    popup.scroll_down(3);
                    return vec![];
                }
                if let Some(ref mut popup) = self.lsp.hover_popup {
                    popup.scroll_down(3);
                    return vec![];
                }
                // No popup - scroll editor by visual rows (accounts for word wrap)
                self.scroll_visual_rows_down(3);
                self.scroll_follows_cursor = false;
                return vec![];
            }
            _ => {}
        }

        // Handle completion popup click
        if let Some(popup_rect) = self.lsp.popup_rect {
            let in_popup = mouse.column >= popup_rect.x
                && mouse.column < popup_rect.x + popup_rect.width
                && mouse.row >= popup_rect.y
                && mouse.row < popup_rect.y + popup_rect.height;

            if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                if in_popup {
                    // Click inside popup - select and accept
                    let row = (mouse.row - popup_rect.y) as usize;
                    if let Some(ref mut popup) = self.lsp.completion_popup {
                        popup.select_at_row(row);
                    }
                    self.accept_completion();
                    self.lsp.popup_rect = None;
                    self.input.click_tracker.skip_next_up = true;
                    return vec![];
                } else {
                    // Click outside popup - close it
                    self.lsp.completion_popup = None;
                    self.lsp.popup_rect = None;
                    // Fall through to normal mouse handling
                }
            }
        }

        // Handle hover popup click
        if let Some(popup_rect) = self.lsp.hover_popup_rect {
            let in_popup = mouse.column >= popup_rect.x
                && mouse.column < popup_rect.x + popup_rect.width
                && mouse.row >= popup_rect.y
                && mouse.row < popup_rect.y + popup_rect.height;

            if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                if in_popup {
                    // Click inside hover popup - do nothing (allow selection of text in future)
                    return vec![];
                } else {
                    // Click outside popup - close it
                    self.close_hover_popup();
                    // Fall through to normal mouse handling
                }
            }
        }

        let inner = Rect {
            x: panel_area.x + 1,
            y: panel_area.y + 1,
            width: panel_area.width.saturating_sub(2),
            height: panel_area.height.saturating_sub(2),
        };

        let line_number_width = rendering::LINE_NUMBER_WIDTH as u16;
        let content_x = inner.x + line_number_width;
        let content_y = inner.y;
        let content_width = inner.width.saturating_sub(line_number_width);
        let content_height = inner.height;

        // Save content bounds for auto-scroll in tick()
        self.input.content_bounds = Some((content_x, content_y, content_width, content_height));

        // Update last mouse position for auto-scroll
        self.input.last_mouse_position = Some((mouse.column, mouse.row));

        // Check if mouse is outside content area
        let is_outside_x = mouse.column < content_x || mouse.column >= content_x + content_width;
        let is_outside_y = mouse.row < content_y || mouse.row >= content_y + content_height;
        let is_outside = is_outside_x || is_outside_y;

        // For drag events during selection, allow extending beyond panel with clamping
        let is_drag = matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left));
        let has_selection = self.selection.is_some();

        if is_outside && !(is_drag && has_selection) {
            return vec![];
        }

        // Clamp coordinates to content area for out-of-bounds drag
        let (clamped_col, clamped_row) = if is_outside && is_drag && has_selection {
            // Auto-scroll if mouse is above or below content area
            // Use visual row methods for correct word-wrap support
            if mouse.row < content_y {
                self.scroll_visual_rows_up(1);
            } else if mouse.row >= content_y + content_height {
                self.scroll_visual_rows_down(1);
            }

            let col = mouse
                .column
                .clamp(content_x, content_x + content_width.saturating_sub(1));
            let row = mouse
                .row
                .clamp(content_y, content_y + content_height.saturating_sub(1));
            (col, row)
        } else {
            (mouse.column, mouse.row)
        };

        let rel_x = (clamped_col - content_x) as usize;
        let rel_y = (clamped_row - content_y) as usize;

        // Map visual row to buffer line, accounting for diagnostic virtual lines
        let (buffer_line, wrapped_offset, chunk_end, is_virtual_line) = if self.config.word_wrap {
            // In word wrap mode, use cached function that accounts for both
            // line wrapping and diagnostic virtual lines.
            // Account for top_visual_row_offset: when scrolled within a wrapped line,
            // rel_y=0 corresponds to visual row top_visual_row_offset of top_line.
            let effective_visual_row = rel_y + self.viewport.top_visual_row_offset;
            // Use cached content_width to ensure consistency with wrap cache
            let cached_width = self.render_cache.content_width;
            let use_smart_wrap = self.render_cache.use_smart_wrap;
            word_wrap::visual_row_to_buffer_position_cached(
                &mut self.render_cache,
                &self.buffer,
                effective_visual_row,
                self.viewport.top_line,
                cached_width,
                use_smart_wrap,
                &self.lsp.diagnostics,
            )
        } else {
            // Use virtual lines to correctly map visual row to buffer line
            // This accounts for diagnostic and deletion marker virtual lines
            if let Some(vline) = git::get_virtual_line_at_row(
                &self.buffer,
                &self.git.diff_cache,
                self.render_cache.config.editor.show_git_diff,
                &self.lsp.diagnostics,
                self.viewport.top_line,
                rel_y,
                content_width as usize,
            ) {
                match vline {
                    git::VirtualLine::Real(line_idx) => {
                        let line_len = self
                            .buffer
                            .line(line_idx)
                            .map(|s| {
                                use unicode_segmentation::UnicodeSegmentation;
                                s.trim_end_matches('\n').graphemes(true).count()
                            })
                            .unwrap_or(0);
                        (line_idx, 0, line_len, false)
                    }
                    // For virtual lines (diagnostics, deletion markers), use the associated line
                    git::VirtualLine::DeletionMarker(after_line, _) => (after_line, 0, 0, true),
                    git::VirtualLine::Diagnostic { line, .. } => {
                        let line_len = self
                            .buffer
                            .line(line)
                            .map(|s| {
                                use unicode_segmentation::UnicodeSegmentation;
                                s.trim_end_matches('\n').graphemes(true).count()
                            })
                            .unwrap_or(0);
                        (line, 0, line_len, true)
                    }
                }
            } else {
                // Fallback if no virtual line found
                let line_idx = self.viewport.top_line + rel_y;
                let line_len = self
                    .buffer
                    .line(line_idx)
                    .map(|s| {
                        use unicode_segmentation::UnicodeSegmentation;
                        s.trim_end_matches('\n').graphemes(true).count()
                    })
                    .unwrap_or(0);
                (line_idx, 0, line_len, false)
            }
        };

        // If mouse is on a virtual line (diagnostic/deletion), handle specially
        if is_virtual_line {
            // Click on virtual line - open diagnostics panel
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.cursor = Cursor::at(buffer_line, 0);
                self.scroll_follows_cursor = true;
                return vec![PanelEvent::OpenDiagnosticsPanel];
            }
            // For other mouse events on virtual lines, ignore
            return vec![];
        }

        let max_line = self.buffer.line_count().saturating_sub(1);
        let target_line = buffer_line.min(max_line);

        // Get line text for screen→grapheme conversion
        let line_text = self
            .buffer
            .line(target_line)
            .map(|s| s.trim_end_matches('\n').to_string())
            .unwrap_or_default();

        // Convert screen column to grapheme index
        let buffer_col = if self.config.word_wrap {
            // wrapped_offset is grapheme index where this visual line starts
            // chunk_end is grapheme index where this visual line ends (exclusive)
            // rel_x is screen column within this visual line
            // Get only the text for this visual line and convert rel_x to grapheme offset
            use unicode_segmentation::UnicodeSegmentation;
            let visual_line_len = chunk_end.saturating_sub(wrapped_offset);
            let segment: String = line_text
                .graphemes(true)
                .skip(wrapped_offset)
                .take(visual_line_len)
                .collect();
            let grapheme_in_segment = screen_col_to_grapheme_idx(&segment, rel_x);
            wrapped_offset + grapheme_in_segment
        } else {
            // Without wrap: convert absolute screen col to grapheme idx
            screen_col_to_grapheme_idx(&line_text, self.viewport.left_column + rel_x)
        };

        let line_len = self.buffer.line_len_graphemes(target_line);
        let target_col = buffer_col.min(line_len);

        self.handle_mouse_action(mouse, target_line, target_col)
    }

    /// Handle mouse action after coordinates are resolved.
    fn handle_mouse_action(
        &mut self,
        mouse: MouseEvent,
        target_line: usize,
        target_col: usize,
    ) -> Vec<PanelEvent> {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Handle Ctrl+Click for go-to-definition
                if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                    // Close any existing popups
                    self.lsp.hover_popup = None;
                    self.lsp.hover_popup_rect = None;
                    self.lsp.pending_ctrl_click = None;

                    // Store pending definition request - will be executed in tick() where LspManager is available
                    self.lsp.pending_definition_request = Some((target_line, target_col));
                    self.input.click_tracker.skip_next_up = true;
                    return vec![];
                }

                self.scroll_follows_cursor = true;
                self.close_search();

                // Start selection drag tracking for auto-scroll
                self.input.selection_drag_active = true;

                if self
                    .input
                    .click_tracker
                    .is_double_click(target_line, target_col)
                {
                    let temp_cursor = Cursor::at(target_line, target_col);
                    if let Some((new_selection, new_cursor)) =
                        selection::select_word(&self.buffer, &temp_cursor)
                    {
                        self.selection = Some(new_selection);
                        self.cursor = new_cursor;
                        self.input.click_tracker.skip_next_up = true;
                    }
                    self.input.click_tracker.reset();
                } else {
                    self.cursor = Cursor::at(target_line, target_col);
                    self.selection = Some(Selection::new(self.cursor, self.cursor));
                    self.input
                        .click_tracker
                        .record_click(target_line, target_col);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.scroll_follows_cursor = true;
                self.cursor = Cursor::at(target_line, target_col);
                if let Some(ref mut selection) = self.selection {
                    selection.active = self.cursor;
                }
                // Use word wrap aware scrolling when word wrap is enabled
                if self.config.word_wrap && self.render_cache.content_width > 0 {
                    self.ensure_cursor_visible_word_wrap(self.render_cache.content_height);
                } else {
                    self.viewport
                        .ensure_cursor_visible(&self.cursor, self.render_cache.virtual_line_count);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.scroll_follows_cursor = true;

                // End selection drag tracking
                self.input.selection_drag_active = false;
                self.input.last_mouse_position = None;

                if self.input.click_tracker.skip_next_up {
                    self.input.click_tracker.skip_next_up = false;
                    return vec![];
                }
                self.cursor = Cursor::at(target_line, target_col);
                if let Some(ref mut selection) = self.selection {
                    selection.active = self.cursor;
                    if selection.is_empty() {
                        self.selection = None;
                    }
                }
            }
            MouseEventKind::Moved => {
                // Track mouse position for hover functionality
                let new_pos = (mouse.column, mouse.row);
                let old_pos = self.lsp.last_mouse_position;

                if old_pos != Some(new_pos) {
                    // Don't close hover if mouse is inside hover popup
                    if self.lsp.hover_popup.is_some() {
                        if let Some(rect) = self.lsp.hover_popup_rect {
                            let in_popup = mouse.column >= rect.x
                                && mouse.column < rect.x + rect.width
                                && mouse.row >= rect.y
                                && mouse.row < rect.y + rect.height;
                            if !in_popup {
                                self.close_hover_popup();
                            }
                        }
                    }

                    // Schedule hover only if no popups open
                    if self.lsp.completion_popup.is_none() && self.lsp.hover_popup.is_none() {
                        self.lsp
                            .schedule_hover(target_line, target_col, mouse.column, mouse.row);
                    }
                }
            }
            _ => {}
        }

        vec![]
    }
}
