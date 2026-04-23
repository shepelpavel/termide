//! Panel-layout utilities for mouse hit-testing: divider drag, per-group
//! rectangle calculation, and coalesced scroll forwarding to panels.

use anyhow::Result;
use ratatui::layout::Rect;

use crate::app::App;

impl App {
    /// Handle click on divider to start resize drag.
    /// Returns true if click was on a divider.
    pub(in crate::app) fn handle_divider_click(&mut self, x: u16, y: u16) -> Result<bool> {
        let terminal_height = self.state.terminal.height;

        if let Some(divider_idx) =
            self.layout_manager
                .find_divider_at_position(x, y, terminal_height)
        {
            // Get current widths of adjacent groups
            let left_width = self
                .layout_manager
                .panel_groups
                .get(divider_idx)
                .and_then(|g| g.width)
                .unwrap_or(0);
            let right_width = self
                .layout_manager
                .panel_groups
                .get(divider_idx + 1)
                .and_then(|g| g.width)
                .unwrap_or(0);

            // Start drag
            self.state
                .ui
                .drag
                .start(divider_idx, x, left_width, right_width);
            self.state.needs_redraw = true;

            return Ok(true);
        }

        Ok(false)
    }

    /// Handle divider drag — track cursor position and draw ghost line.
    /// Only the ghost divider line is rendered (lightweight), while actual
    /// panel resize is deferred to mouse release.
    pub(in crate::app) fn handle_divider_drag(&mut self, current_x: u16) -> Result<()> {
        if self.state.ui.drag.last_applied_x == Some(current_x) {
            return Ok(());
        }
        self.state.ui.drag.last_applied_x = Some(current_x);
        // Trigger redraw for the ghost line — this is lightweight because
        // only ~2 columns change (ratatui diff rendering sends minimal data).
        self.state.needs_redraw = true;
        Ok(())
    }

    /// Handle divider drag end — apply final widths and redraw once.
    pub(in crate::app) fn handle_divider_drag_end(&mut self) -> Result<()> {
        // Apply the accumulated drag position as a single resize
        if let (Some(divider_idx), Some(final_x)) = (
            self.state.ui.drag.active_divider,
            self.state.ui.drag.last_applied_x,
        ) {
            let delta = final_x as i32 - self.state.ui.drag.start_x as i32;
            let (start_left, start_right) = self.state.ui.drag.start_widths;

            let min_width = self.state.config.general.min_panel_width;
            let total_width = start_left + start_right;

            let new_left = (start_left as i32 + delta)
                .max(min_width as i32)
                .min((total_width - min_width) as i32) as u16;
            let new_right = total_width - new_left;

            self.layout_manager
                .resize_groups(divider_idx, new_left, new_right);
        }

        self.state.ui.drag.end();
        self.state.needs_redraw = true;

        // Save session with new widths
        self.auto_save_session();

        Ok(())
    }

    /// Find the expanded panel group at the given screen coordinates.
    /// Returns `(group_idx, rect)` if an expanded panel contains the point.
    pub(in crate::app) fn find_expanded_panel_group_at(
        &self,
        x: u16,
        y: u16,
    ) -> Option<(usize, Rect)> {
        for (group_idx, _panel_idx, rect, is_expanded) in self.calculate_panel_rects() {
            if !is_expanded {
                continue;
            }
            if x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height {
                return Some((group_idx, rect));
            }
        }
        None
    }

    /// Calculate panel rectangles for mouse hit testing.
    /// Returns `Vec<(group_idx, panel_idx, rect, is_expanded)>`.
    pub(in crate::app) fn calculate_panel_rects(&self) -> Vec<(usize, usize, Rect, bool)> {
        let width = self.state.terminal.width;
        let height = self.state.terminal.height;
        let main_area = Rect {
            x: 0,
            y: 1,
            width,
            height: height.saturating_sub(2),
        };
        termide_layout::calculate_panel_rects(&self.layout_manager.panel_groups, main_area)
    }

    /// Handle coalesced scroll events (batched for performance).
    ///
    /// This method processes multiple scroll events that have been coalesced
    /// into a single event with a combined delta value.
    pub(in crate::app) fn handle_coalesced_scroll(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        delta: i32,
    ) -> Result<()> {
        // Track scroll timing for throttling heavy operations in Event::Tick
        self.state.last_mouse_scroll = Some(std::time::Instant::now());

        // When a modal is open, forward the wheel event to it instead of the
        // panel underneath — otherwise scrollable modals (InfoModal for the
        // report-script output, list modals, etc.) never see wheel input and
        // look broken to the user.
        if self.state.has_modal() {
            let kind = if delta < 0 {
                crossterm::event::MouseEventKind::ScrollUp
            } else if delta > 0 {
                crossterm::event::MouseEventKind::ScrollDown
            } else {
                return Ok(());
            };
            let modal_area = ratatui::layout::Rect {
                x: 0,
                y: 0,
                width: self.state.terminal.width,
                height: self.state.terminal.height,
            };
            // Coalescing lost per-step granularity; replay it so the modal's
            // per-tick scroll step (e.g. 3 lines in InfoModal) actually maps
            // to the physical notch count.
            let steps = delta.unsigned_abs();
            for _ in 0..steps {
                let ev = crossterm::event::MouseEvent { kind, ..mouse };
                self.handle_modal_mouse(ev, modal_area)?;
            }
            return Ok(());
        }

        self.forward_coalesced_scroll_to_panel(mouse, delta)
    }

    /// Forward coalesced scroll to the panel under the mouse cursor.
    fn forward_coalesced_scroll_to_panel(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        delta: i32,
    ) -> Result<()> {
        if let Some((group_idx, rect)) = self.find_expanded_panel_group_at(mouse.column, mouse.row)
        {
            if let Some(group) = self.layout_manager.panel_groups.get_mut(group_idx) {
                if let Some(panel) = group.expanded_panel_mut() {
                    let events = panel.handle_scroll(delta, rect);
                    self.process_panel_events(events)?;
                }
            }
        }

        Ok(())
    }
}
