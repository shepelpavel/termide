//! Miscellaneous panels for termide.
//!
//! This crate contains simple utility panels: help screen and journal.

pub mod help;
pub mod help_generator;
pub mod journal;

pub use help::HelpPanel;
pub use help_generator::HelpGenerator;
pub use journal::JournalPanel;
