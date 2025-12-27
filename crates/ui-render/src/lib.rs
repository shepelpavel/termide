//! UI rendering components for termide.
//!
//! Provides reusable UI widgets and rendering utilities.

pub mod dropdown;
pub mod menu;
pub mod panel_rendering;
pub mod scroll_indicator;
pub mod status_bar;
pub mod theme_dropdown;

pub use dropdown::{
    get_git_items, get_preferences_items, get_sessions_items, Dropdown, DropdownItem,
    GIT_SUBMENU_ITEM_COUNT, SESSIONS_SUBMENU_ITEM_COUNT,
};
pub use menu::{
    get_menu_item_x_position, get_menu_items, render_menu, resource_color, MenuRenderParams,
    GIT_MENU_INDEX, MENU_ITEM_COUNT, PREFERENCES_MENU_INDEX, SESSIONS_MENU_INDEX,
};
pub use panel_rendering::{
    render_collapsed_panel, render_dividers, render_expanded_panel, ExpandedPanelParams,
};
pub use scroll_indicator::{render_scroll_indicators, ScrollState};
pub use status_bar::{StatusBar, StatusBarParams};
pub use theme_dropdown::ThemeDropdown;
