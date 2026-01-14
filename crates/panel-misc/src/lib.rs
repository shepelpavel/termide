//! Miscellaneous panels for termide.
//!
//! This crate contains simple utility panels: welcome screen and log viewer.

pub mod log_viewer;
pub mod welcome;

pub use log_viewer::LogViewerPanel;
pub use welcome::WelcomePanel;
