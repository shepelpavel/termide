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

        let Some(target) = self.compute_drop_target(x, y) else {
            return Ok(());
        };

        let available_width = self.state.terminal.width;
        match self.layout_manager.move_panel_to(
            src.group_idx,
            src.panel_idx,
            target,
            available_width,
        ) {
            Ok((_gi, _pi)) => {
                self.auto_save_session();
            }
            Err(e) => {
                self.show_error_modal(format!("Cannot move panel: {}", e));
            }
        }
        Ok(())
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
