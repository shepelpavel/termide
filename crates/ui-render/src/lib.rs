//! UI rendering components for termide.
//!
//! Provides reusable UI widgets and rendering utilities.

pub mod dropdown;
pub mod menu;
pub mod panel_rendering;
pub mod status_bar;
pub mod theme_dropdown;

pub use dropdown::{get_preferences_items, Dropdown, DropdownItem};
pub use menu::{
    get_menu_item_x_position, get_menu_items, render_menu, resource_color, MenuRenderParams,
    MENU_ITEM_COUNT, PREFERENCES_MENU_INDEX,
};
pub use panel_rendering::{render_collapsed_panel, render_expanded_panel, ExpandedPanelParams};
pub use status_bar::{StatusBar, StatusBarParams};
pub use theme_dropdown::ThemeDropdown;
