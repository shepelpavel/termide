//! Scroll indicator utilities for panels.
//!
//! Provides functions to render scroll indicators on panel borders.

use ratatui::{buffer::Buffer, style::Style};

/// Render scroll indicators on the right edge of content area.
///
/// Displays ▲ at the top when there's content above (can scroll up)
/// and ▼ at the bottom when there's content below (can scroll down).
///
/// # Arguments
/// * `buf` - The buffer to render to
/// * `x` - X position for the indicator (typically right edge of content)
/// * `y_top` - Y position for top indicator
/// * `y_bottom` - Y position for bottom indicator
/// * `can_scroll_up` - Whether there's content above
/// * `can_scroll_down` - Whether there's content below
/// * `style` - Style for the indicators
pub fn render_scroll_indicators(
    buf: &mut Buffer,
    x: u16,
    y_top: u16,
    y_bottom: u16,
    can_scroll_up: bool,
    can_scroll_down: bool,
    style: Style,
) {
    if can_scroll_up {
        buf[(x, y_top)].set_symbol("▲").set_style(style);
    }

    if can_scroll_down {
        buf[(x, y_bottom)].set_symbol("▼").set_style(style);
    }
}

/// Scroll state for panels.
///
/// Tracks scroll position and total items for calculating indicators.
#[derive(Debug, Clone, Copy)]
pub struct ScrollState {
    /// Current scroll offset (first visible item index)
    pub offset: usize,
    /// Number of visible items
    pub visible: usize,
    /// Total number of items
    pub total: usize,
}

impl ScrollState {
    /// Create a new scroll state.
    pub fn new(offset: usize, visible: usize, total: usize) -> Self {
        Self {
            offset,
            visible,
            total,
        }
    }

    /// Check if there are items above the visible area.
    pub fn can_scroll_up(&self) -> bool {
        self.offset > 0
    }

    /// Check if there are items below the visible area.
    pub fn can_scroll_down(&self) -> bool {
        self.offset + self.visible < self.total
    }
}
