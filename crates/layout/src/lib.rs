//! Panel layout management for termide.
//!
//! This crate provides panel layout management with accordion support:
//! - `PanelGroup` - vertical stack of panels with expandable accordion
//! - `LayoutManager` - horizontal arrangement of panel groups

pub mod layout_manager;
pub mod panel_group;

pub use layout_manager::{
    calculate_panel_rects, compute_drop_target, LayoutManager, PanelDropTarget,
};
pub use panel_group::PanelGroup;
