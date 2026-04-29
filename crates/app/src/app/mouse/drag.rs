//! Panel drag-and-drop handlers: grab a panel by its top border, then
//! drop it onto another group or between groups to create a new one.

use anyhow::Result;

use termide_layout::PanelDropTarget;

use crate::app::App;

impl App {
    /// Update the drag cursor position on a `Drag(Left)` event. Promotes
    /// a pending drag to active once the threshold is crossed.
    pub(in crate::app) fn handle_panel_drag_move(&mut self, x: u16, y: u16) -> Result<()> {
        let became_active = self.state.ui.panel_drag.update_cursor(x, y);
        if became_active || self.state.ui.panel_drag.active {
            self.state.needs_redraw = true;
        }
        Ok(())
    }

    /// Finalise the drag on `Up(Left)`. If the drag never became active
    /// (threshold not crossed) we simply clear the state — the click path
    /// that fired on Down has already handled activation.
    ///
    /// Drop semantics:
    /// - In Split mode, dropping inside the source group's column is
    ///   interpreted as a vertical resize: the divider above the dragged
    ///   panel moves to the cursor, growing or shrinking the upper
    ///   neighbour. (The top panel of a group has no divider above and
    ///   falls through to the move logic.)
    /// - Any other drop (different column, between columns, or accordion
    ///   source) commits as a panel move via [`compute_drop_target`].
    pub(in crate::app) fn handle_panel_drag_end(&mut self, x: u16, y: u16) -> Result<()> {
        let was_active = self.state.ui.panel_drag.active;
        let source = self.state.ui.panel_drag.source;
        self.state.ui.panel_drag.cancel();
        self.state.needs_redraw = true;

        if !was_active {
            return Ok(());
        }

        let Some(src) = source else {
            return Ok(());
        };

        // Try resize-in-place first (Split + same column + has divider above).
        if self.try_resize_drag_in_split(src, x, y) {
            self.auto_save_session();
            return Ok(());
        }

        let Some(target) = self.compute_drop_target(x, y) else {
            return Ok(());
        };

        let available_width = self.state.terminal.width;
        let result = self
            .layout_manager
            .move_panel_to(src.group_idx, src.panel_idx, target, available_width)
            .map(|_| ());
        self.handle_layout_op("Cannot move panel", result);
        Ok(())
    }

    /// Returns `true` if the drag was reinterpreted as a vertical
    /// resize inside the source group (and applied). Caller falls
    /// through to the regular move logic when this returns `false`.
    fn try_resize_drag_in_split(
        &mut self,
        src: termide_state::PanelDragSource,
        x: u16,
        y: u16,
    ) -> bool {
        if src.panel_idx == 0 {
            // Top panel of a group has no divider above it — fall through
            // to the move logic so the user can still drag it elsewhere.
            return false;
        }

        // Restrict to the source group's horizontal extent so dragging
        // across columns continues to move the panel.
        let rects = self.calculate_panel_rects();
        let spans = termide_layout::group_spans_from_rects(&rects);
        let in_source_column = spans
            .iter()
            .find(|(gi, _, _)| *gi == src.group_idx)
            .is_some_and(|(_, left, right)| x >= *left && x < *right);
        if !in_source_column {
            return false;
        }

        let delta_y = y as i32 - src.start_y as i32;
        if delta_y == 0 {
            return true; // counted as resize (no-op), suppresses move
        }
        let area_height = self.state.terminal.height.saturating_sub(2);
        if let Some(group) = self.layout_manager.panel_groups.get_mut(src.group_idx) {
            group.resize_panel_divider(src.panel_idx - 1, delta_y, area_height);
        }
        true
    }

    /// Determine the drop target under the cursor, or `None` if the
    /// cursor is over an invalid zone (menu, status bar).
    pub(in crate::app) fn compute_drop_target(&self, x: u16, y: u16) -> Option<PanelDropTarget> {
        let height = self.state.terminal.height;
        if y == 0 || y + 1 >= height {
            return None;
        }
        let rects = self.calculate_panel_rects();
        termide_layout::compute_drop_target(&rects, x, y)
    }
}
