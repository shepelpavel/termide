//! Input-related state for the editor.

use crate::click_tracker::ClickTracker;

/// Input-related state for the editor.
#[derive(Default)]
pub(crate) struct InputState {
    /// Mouse click tracking for double-click detection.
    pub click_tracker: ClickTracker,
    /// Preferred column for vertical navigation (maintains column across lines).
    pub preferred_column: Option<usize>,
    /// Left mouse button is currently held down during selection.
    pub selection_drag_active: bool,
    /// Last known mouse position (column, row) in screen coordinates.
    pub last_mouse_position: Option<(u16, u16)>,
    /// Content area bounds for auto-scroll checks: (x, y, width, height).
    pub content_bounds: Option<(u16, u16, u16, u16)>,
}

impl InputState {
    /// Create new InputState.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset preferred column (e.g., after horizontal movement).
    pub fn clear_preferred_column(&mut self) {
        self.preferred_column = None;
    }

    /// Set preferred column.
    pub fn set_preferred_column(&mut self, col: usize) {
        self.preferred_column = Some(col);
    }

    /// Get preferred column or current column.
    pub fn get_preferred_column(&self, current_col: usize) -> usize {
        self.preferred_column.unwrap_or(current_col)
    }
}
