//! Miscellaneous panels for termide.
//!
//! This crate contains simple utility panels: help screen and log viewer.

pub mod help;
pub mod help_generator;
pub mod log_viewer;

pub use help::HelpPanel;
pub use help_generator::HelpGenerator;
pub use log_viewer::LogViewerPanel;
