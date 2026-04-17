//! Submodules for mouse-event helpers.
//!
//! The main dispatcher stays in `mouse_handler.rs`; this directory contains
//! specialised helpers (resource-indicator builders, submenu click handlers,
//! divider drag) that each keep their own `impl App` block.

mod indicators;
mod layout;
mod submenu;
