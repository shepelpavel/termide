//! Panel header drag handlers: grab a panel by its top border, then
//! drop it on another panel/group to move/reorder/cross-group, or drop
//! on the source panel itself to resize the divider above.

use anyhow::Result;

use termide_layout::PanelDragIntent;

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
    /// Drop semantics (spatial classification):
    /// - Cursor over the source panel itself → resize the divider above
    ///   it (no-op for the top panel of a group, which has no divider
    ///   above).
    /// - Cursor over a different panel of the source group → reorder
    ///   inside the column (insert before/after the panel under the
    ///   cursor).
    /// - Cursor in another column or in a between-groups gutter → move
    ///   the panel across groups.
    /// - Cursor outside any valid zone → no-op.
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

        let rects = self.calculate_panel_rects();
        let intent =
            termide_layout::classify_panel_drag(&rects, src.group_idx, src.panel_idx, x, y);

        match intent {
            PanelDragIntent::ResizeAbove { divider_y } => {
                let area_height = self.state.terminal.height.saturating_sub(2);
                let upper_boundary_y = rects
                    .iter()
                    .find(|(gi, pi, _, _)| *gi == src.group_idx && *pi == src.panel_idx)
                    .map(|(_, _, rect, _)| rect.y);
                if let Some(boundary_y) = upper_boundary_y {
                    let delta = divider_y as i32 - boundary_y as i32;
                    if delta != 0 {
                        if let Some(group) = self.layout_manager.panel_groups.get_mut(src.group_idx)
                        {
                            group.resize_panel_divider(src.panel_idx - 1, delta, area_height);
                        }
                        self.auto_save_session();
                    }
                }
            }
            PanelDragIntent::Move(target) => {
                let available_width = self.state.terminal.width;
                let result = self
                    .layout_manager
                    .move_panel_to(src.group_idx, src.panel_idx, target, available_width)
                    .map(|_| ());
                self.handle_layout_op("Cannot move panel", result);
            }
            PanelDragIntent::Cancel => {}
        }
        Ok(())
    }
}
