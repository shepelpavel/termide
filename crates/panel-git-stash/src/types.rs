//! Type definitions for Git Stash Panel.

/// Section of the Stash panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// [New] button in the header (focused by default on open)
    NewButton,
    /// Stash entries list
    List,
}
