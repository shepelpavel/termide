//! UI rendering components for termide.
//!
//! Provides reusable UI widgets and rendering utilities.

pub mod dropdown;
pub mod inline_selector;
pub mod language_dropdown;
pub mod menu;
pub mod panel_rendering;
pub mod status_bar;
pub mod theme_dropdown;

pub use dropdown::{
    get_actions_group_items, get_actions_items, get_options_items, get_sessions_items,
    get_tools_items, Dropdown, DropdownItem, OPTIONS_SUBMENU_ITEM_COUNT,
    SESSIONS_SUBMENU_ITEM_COUNT, TOOLS_SUBMENU_ITEM_COUNT,
};
pub use inline_selector::InlineSelector;
pub use language_dropdown::{find_current_language_index, LanguageDropdown};
pub use menu::{
    get_menu_item_x_position, get_menu_items, render_menu, resource_color, MenuRenderParams,
    ACTIONS_MENU_INDEX, MENU_ITEM_COUNT, OPTIONS_MENU_INDEX, SESSIONS_MENU_INDEX, TOOLS_MENU_INDEX,
};
pub use panel_rendering::{
    render_collapsed_panel, render_dividers, render_expanded_panel, ExpandedPanelParams,
};
pub use status_bar::{StatusBar, StatusBarParams};
pub use theme_dropdown::ThemeDropdown;
