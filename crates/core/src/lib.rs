//! Core types and traits for termide panels.
//!
//! This crate provides the foundational abstractions for building panels
//! in termide without coupling them to the application state.

pub mod command;
pub mod event;
pub mod panel;
pub mod terminal_caps;
pub mod util;

pub use command::{CommandResult, PanelCommand};
pub use event::{
    ConfirmAction, ConflictResolution, Event, EventHandler, GitOperationType, InputAction,
    PanelEvent, SelectAction, SplitDirection, VimPanelDirection,
};
pub use panel::{
    Panel, PanelConfig, RenderContext, Searchable, SessionPanel, ThemeColors, WidthPreference,
};
pub use terminal_caps::{
    get_terminal_caps, init_icon_mode, init_terminal_caps, use_emoji_icons, ColorDepth,
    TerminalCaps,
};

// Re-export theme and config for convenience
pub use termide_config::Config;
pub use termide_theme::Theme;
