//! Type definitions for Git Status Panel.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::tree;

/// Grouped tree state for one file section (unstaged or staged).
pub(crate) struct FileTree {
    pub tree: Vec<tree::TreeNode>,
    pub visible: Vec<usize>,
    pub prefixes: Vec<String>,
    pub collapsed: HashSet<PathBuf>,
}

impl FileTree {
    pub fn new() -> Self {
        Self {
            tree: Vec::new(),
            visible: Vec::new(),
            prefixes: Vec::new(),
            collapsed: HashSet::new(),
        }
    }

    /// Recompute `visible` and `prefixes` from the current `tree`.
    pub fn recompute_visible(&mut self) {
        self.visible = tree::compute_visible_nodes(&self.tree);
        self.prefixes = tree::compute_tree_prefixes(&self.tree, &self.visible);
    }
}

/// Section of the Git Status panel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Repository selector
    RepoSelector,
    /// Branch selector
    BranchSelector,
    /// Files list (both unstaged and staged)
    Files,
    /// Action buttons
    Buttons,
}

/// Current selection in the files area
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// Cursor on Unstaged header (selecting [Stage all] button)
    UnstagedHeader,
    /// Cursor on an unstaged file at given index
    UnstagedFile(usize),
    /// Cursor on an unstaged directory node (index into unstaged full tree)
    UnstagedDir(usize),
    /// Cursor on Staged header (selecting [Unstage all] button)
    StagedHeader,
    /// Cursor on a staged file at given index
    StagedFile(usize),
    /// Cursor on a staged directory node (index into staged full tree)
    StagedDir(usize),
}

/// Button in the Git Status panel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Show all diffs in Git Diff panel
    Diff,
    Commit,
    Pull,
    Push,
    /// Push operation in progress (shows spinner, click cancels)
    Pushing,
    /// Pull operation in progress (shows spinner, click cancels)
    Pulling,
    /// Initialize a new git repository
    Init,
}

impl Button {
    /// Get the label for this button
    pub fn label(&self, spinner_frame: usize) -> String {
        let t = termide_i18n::t();
        let spinner = termide_config::constants::SPINNER_FRAMES
            [spinner_frame % termide_config::constants::SPINNER_FRAMES.len()];
        match self {
            Button::Diff => t.git_action_diff().to_string(),
            Button::Commit => t.git_action_commit().to_string(),
            Button::Pull => t.git_action_pull().to_string(),
            Button::Push => t.git_action_push().to_string(),
            Button::Pushing => {
                format!("{} {}", spinner, t.git_pushing())
            }
            Button::Pulling => {
                format!("{} {}", spinner, t.git_pulling())
            }
            Button::Init => t.git_action_init().to_string(),
        }
    }
}
