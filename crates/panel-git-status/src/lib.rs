//! Git Status Panel for termide.
//!
//! Provides a panel for managing git operations: staging, unstaging, commits, push/pull.

#![allow(clippy::too_many_arguments)]

mod actions;
mod rendering;
pub mod tree;
mod types;

use types::FileTree;
pub use types::{Button, Section, Selection};

use std::any::Any;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use termide_config::{is_go_end, is_go_home, is_move_down, is_move_up, Config};
use termide_core::{
    CommandResult, HotkeyTable, Panel, PanelCommand, PanelEvent, RenderContext, SessionPanel,
    ThemeColors, WidthPreference,
};
use termide_git::{self as git, RepoManager, StagedFile, UnstagedFile};
use termide_modal::ActiveModal;
use termide_state::PendingAction;
use termide_theme::Theme;
use termide_ui::IndexClickTracker;

/// Git Status Panel
pub struct GitStatusPanel {
    /// Repository manager
    repo_manager: RepoManager,
    /// Current branch name
    branch: Option<String>,
    /// Available branches for current repo
    branches: Vec<String>,
    /// Ahead/behind counts
    ahead: usize,
    behind: usize,
    /// Unstaged files (modified + untracked)
    unstaged_files: Vec<UnstagedFile>,
    /// Staged files
    staged_files: Vec<StagedFile>,
    /// Current section
    current_section: Section,
    /// Cursor position as virtual line (0..total_virtual_lines, includes headers)
    cursor: usize,
    /// Selected button index
    selected_button: usize,
    /// Unified scroll offset for files area
    scroll_offset: usize,
    /// Cached viewport height for scroll calculations
    viewport_height: usize,
    /// Cached theme colors for rendering
    cached_theme: ThemeColors,
    /// Last render area (for mouse handling)
    last_area: Rect,
    /// Status message
    status_message: Option<String>,
    /// Is repo dropdown expanded
    repo_dropdown_open: bool,
    /// Is branch dropdown expanded
    branch_dropdown_open: bool,
    /// Cursor position in open dropdown
    dropdown_cursor: usize,
    // Layout zones for mouse handling
    /// Selector row Y position
    selector_y: u16,
    /// Branch selector X position (for mouse click detection)
    branch_selector_x: u16,
    /// Files area (combined unstaged + staged)
    files_area: Rect,
    /// Buttons row Y position
    buttons_y: u16,
    /// Cached height of the buttons area (may span multiple rows)
    cached_buttons_height: u16,
    /// Repo dropdown area (for mouse click detection)
    repo_dropdown_area: Option<Rect>,
    /// Branch dropdown area (for mouse click detection)
    branch_dropdown_area: Option<Rect>,
    /// Scroll offset in dropdown
    dropdown_scroll: usize,
    /// Stash button area (for dropdown anchoring)
    stash_button_area: Option<Rect>,
    /// Click tracker for double-click detection in files area
    click_tracker: IndexClickTracker,
    /// Modal request (for file properties)
    modal_request: Option<(termide_state::PendingAction, termide_modal::ActiveModal)>,
    /// Loading indicator flag
    is_loading: bool,
    /// Whether git operation (push/pull) is in progress
    git_operation_in_progress: bool,
    /// Current git operation name ("push" or "pull")
    current_operation: Option<String>,
    /// Spinner animation frame for Pushing/Pulling buttons
    spinner_frame: usize,
    /// Initial paths passed to the panel (for git init when no repo found)
    initial_paths: Vec<PathBuf>,
    /// Cached vim_mode setting for keyboard handling
    vim_mode: bool,
    /// Whether panel missed updates while collapsed (stale-on-collapse)
    is_stale: bool,
    /// Watched root registered with the watcher (None = not yet registered)
    watched_root: Option<PathBuf>,
    /// Tree state for unstaged files (modified + untracked)
    unstaged: FileTree,
    /// Tree state for staged files
    staged: FileTree,
    /// Pending initial fetch to update ahead/behind counts
    pending_init_fetch: bool,
    /// Cached stash count (for button label)
    stash_count: usize,
    /// Hotkey table for configurable keyboard shortcuts
    hotkeys: HotkeyTable,
    /// Pointer of the last Arc<Config> used to build hotkeys (skip rebuild when unchanged)
    last_config_ptr: usize,
}

/// Build HotkeyTable for the git status panel.
fn build_git_status_hotkey_table(config: &Config) -> HotkeyTable {
    let mut t = HotkeyTable::new();
    let kb = &config.git_status.keybindings;

    t.insert("stage", &kb.stage);
    t.insert("unstage", &kb.unstage);
    t.insert("view", &kb.view);
    t.insert("edit", &kb.edit);
    t.insert("info", &kb.info);
    t.insert("revert", &kb.revert);
    t.insert("refresh", &kb.refresh);
    t
}

impl GitStatusPanel {
    /// Create a new Git Status panel from a list of paths (from panels/session)
    pub fn new(paths: &[PathBuf]) -> Self {
        Self::create(RepoManager::new(paths), paths.to_vec())
    }

    /// Create panel for a specific repository
    pub fn new_for_repo(repo_path: PathBuf) -> Self {
        let initial_paths = vec![repo_path.clone()];
        Self::create(RepoManager::for_repo(repo_path), initial_paths)
    }

    fn create(repo_manager: RepoManager, initial_paths: Vec<PathBuf>) -> Self {
        let mut panel = Self {
            repo_manager,
            branch: None,
            branches: Vec::new(),
            ahead: 0,
            behind: 0,
            unstaged_files: Vec::new(),
            staged_files: Vec::new(),
            current_section: Section::RepoSelector,
            cursor: 0,
            selected_button: 0,
            scroll_offset: 0,
            viewport_height: 0,
            cached_theme: ThemeColors::default(),
            last_area: Rect::default(),
            status_message: None,
            repo_dropdown_open: false,
            branch_dropdown_open: false,
            dropdown_cursor: 0,
            selector_y: 0,
            branch_selector_x: 0,
            files_area: Rect::default(),
            buttons_y: 0,
            cached_buttons_height: 1,
            repo_dropdown_area: None,
            branch_dropdown_area: None,
            dropdown_scroll: 0,
            stash_button_area: None,
            click_tracker: IndexClickTracker::new(),
            modal_request: None,
            is_loading: false,
            git_operation_in_progress: false,
            current_operation: None,
            spinner_frame: 0,
            initial_paths,
            vim_mode: false,
            is_stale: false,
            watched_root: None,
            unstaged: FileTree::new(),
            staged: FileTree::new(),
            pending_init_fetch: true,
            stash_count: 0,
            hotkeys: HotkeyTable::default(),
            last_config_ptr: 0,
        };

        panel.refresh();
        panel
    }

    /// Update repository list based on new paths from panels
    pub fn update_repos(&mut self, paths: &[PathBuf]) {
        if self.repo_manager.update(paths) {
            // Reset watched_root so the app watcher re-registers the new repo
            self.watched_root = None;
            self.refresh();
        }
    }

    /// Refresh git status
    pub fn refresh(&mut self) {
        self.is_loading = true;

        let repo = match self.repo_manager.current() {
            Some(r) => r.to_path_buf(),
            None => {
                // Try to re-discover repos (e.g. after external `git init`)
                if self.repo_manager.update(&self.initial_paths) {
                    if let Some(r) = self.repo_manager.current() {
                        r.to_path_buf()
                    } else {
                        self.is_loading = false;
                        return;
                    }
                } else {
                    self.is_loading = false;
                    return;
                }
            }
        };

        self.branch = git::get_current_branch(&repo);
        self.branches = git::get_all_branches(&repo);
        let (ahead, behind) = git::get_ahead_behind(&repo);
        self.ahead = ahead;
        self.behind = behind;
        self.unstaged_files = git::get_unstaged_files(&repo);
        self.staged_files = git::get_staged_files(&repo);
        self.stash_count = git::stash_list(&repo).len();

        // Sort by path
        self.unstaged_files.sort_by(|a, b| a.path.cmp(&b.path));
        self.staged_files.sort_by(|a, b| a.path.cmp(&b.path));

        self.rebuild_trees();

        // Adjust cursor to stay within bounds (cursor is virtual line)
        let max_cursor = self.total_virtual_lines().saturating_sub(1);
        if self.cursor > max_cursor {
            self.cursor = max_cursor;
        }
        // Ensure cursor is on a selectable line (direct calculation instead of loop)
        if !self.is_selectable_line(self.cursor) {
            // Find the nearest selectable line (either a file or header with files)
            self.cursor = self.find_nearest_selectable_line(self.cursor);
        }

        self.is_loading = false;
    }

    /// Lightweight refresh of only the data used by `title()`.
    /// Skips branch listing, sorting, and cursor adjustment.
    fn refresh_title_data(&mut self) {
        let repo = match self.repo_manager.current() {
            Some(r) => r.to_path_buf(),
            None => return,
        };

        self.branch = git::get_current_branch(&repo);
        let (ahead, behind) = git::get_ahead_behind(&repo);
        self.ahead = ahead;
        self.behind = behind;
        self.unstaged_files = git::get_unstaged_files(&repo);
        self.staged_files = git::get_staged_files(&repo);
    }

    /// Move to next section
    fn next_section(&mut self) {
        self.current_section = match self.current_section {
            Section::RepoSelector => Section::BranchSelector,
            Section::BranchSelector => {
                let total_files = self.unstaged_files.len() + self.staged_files.len();
                if total_files > 0 {
                    Section::Files
                } else {
                    Section::Buttons
                }
            }
            Section::Files => Section::Buttons,
            Section::Buttons => Section::RepoSelector,
        };
    }

    /// Move to previous section
    fn prev_section(&mut self) {
        self.current_section = match self.current_section {
            Section::RepoSelector => Section::Buttons,
            Section::BranchSelector => Section::RepoSelector,
            Section::Files => Section::BranchSelector,
            Section::Buttons => {
                let total_files = self.unstaged_files.len() + self.staged_files.len();
                if total_files > 0 {
                    Section::Files
                } else {
                    Section::BranchSelector
                }
            }
        };
    }

    /// Number of items in unstaged section (visible tree nodes)
    fn unstaged_item_count(&self) -> usize {
        self.unstaged.visible.len()
    }

    /// Number of items in staged section (visible tree nodes)
    fn staged_item_count(&self) -> usize {
        self.staged.visible.len()
    }

    /// Get current selection based on cursor position (virtual line)
    fn get_selection(&self) -> Option<Selection> {
        let unstaged_header = 0;
        let unstaged_start = 1;
        let unstaged_end = unstaged_start + self.unstaged_item_count();
        let staged_header = unstaged_end;
        let staged_start = staged_header + 1;

        if self.cursor == unstaged_header && !self.unstaged_files.is_empty() {
            Some(Selection::UnstagedHeader)
        } else if self.cursor >= unstaged_start && self.cursor < unstaged_end {
            let idx = self.cursor - unstaged_start;
            if let Some(&tree_idx) = self.unstaged.visible.get(idx) {
                match self.unstaged.tree[tree_idx].kind {
                    tree::TreeNodeKind::Directory { .. } => Some(Selection::UnstagedDir(tree_idx)),
                    tree::TreeNodeKind::File { file_index, .. } => {
                        Some(Selection::UnstagedFile(file_index))
                    }
                }
            } else {
                None
            }
        } else if self.cursor == staged_header && !self.staged_files.is_empty() {
            Some(Selection::StagedHeader)
        } else if self.cursor >= staged_start {
            let idx = self.cursor - staged_start;
            if let Some(&tree_idx) = self.staged.visible.get(idx) {
                match self.staged.tree[tree_idx].kind {
                    tree::TreeNodeKind::Directory { .. } => Some(Selection::StagedDir(tree_idx)),
                    tree::TreeNodeKind::File { file_index, .. } => {
                        Some(Selection::StagedFile(file_index))
                    }
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Check if a virtual line is selectable (files and headers with buttons)
    fn is_selectable_line(&self, vline: usize) -> bool {
        let unstaged_end = 1 + self.unstaged_item_count();
        let staged_end = unstaged_end + 1 + self.staged_item_count();
        self.is_selectable_line_with_bounds(vline, unstaged_end, staged_end)
    }

    /// Check if a virtual line is selectable, using pre-calculated boundaries.
    /// Use this in loops to avoid recalculating counts every iteration.
    fn is_selectable_line_with_bounds(
        &self,
        vline: usize,
        unstaged_end: usize,
        staged_end: usize,
    ) -> bool {
        let unstaged_header = 0;
        let staged_header = unstaged_end;

        if vline == unstaged_header {
            !self.unstaged_files.is_empty()
        } else if vline == staged_header {
            !self.staged_files.is_empty()
        } else {
            vline > unstaged_header && vline < staged_end && vline != staged_header
        }
    }

    /// Check if there are any files (unstaged or staged)
    fn has_any_files(&self) -> bool {
        !self.unstaged_files.is_empty() || !self.staged_files.is_empty()
    }

    /// Get first selectable virtual line
    fn first_selectable_line(&self) -> usize {
        if !self.unstaged_files.is_empty() {
            0 // Unstaged header
        } else if !self.staged_files.is_empty() {
            1 // Staged header (vline = 1 when no unstaged files)
        } else {
            0
        }
    }

    /// Get last selectable virtual line
    fn last_selectable_line(&self) -> usize {
        let unstaged_end = 1 + self.unstaged_item_count();
        let staged_end = unstaged_end + 1 + self.staged_item_count();
        let total = self.total_virtual_lines();
        for vline in (0..total).rev() {
            if self.is_selectable_line_with_bounds(vline, unstaged_end, staged_end) {
                return vline;
            }
        }
        0
    }

    /// Find the nearest selectable line to the given position
    /// Prefers moving backward (up) when current line is not selectable
    fn find_nearest_selectable_line(&self, vline: usize) -> usize {
        let unstaged_end = 1 + self.unstaged_item_count();
        let staged_end = unstaged_end + 1 + self.staged_item_count();
        // Try moving backward first (more natural for cursor adjustment after refresh)
        for offset in 1..=vline {
            let target = vline - offset;
            if self.is_selectable_line_with_bounds(target, unstaged_end, staged_end) {
                return target;
            }
        }
        // If nothing found backward, try forward
        let total = self.total_virtual_lines();
        for target in (vline + 1)..total {
            if self.is_selectable_line_with_bounds(target, unstaged_end, staged_end) {
                return target;
            }
        }
        // Fallback to first selectable
        self.first_selectable_line()
    }

    /// Cursor position is the virtual line directly
    fn cursor_to_virtual_line(&self) -> usize {
        self.cursor
    }

    /// Ensure cursor is visible in viewport
    fn ensure_cursor_visible(&mut self) {
        if self.viewport_height == 0 {
            return;
        }
        let cursor_line = self.cursor_to_virtual_line();
        if cursor_line < self.scroll_offset {
            self.scroll_offset = cursor_line;
        } else if cursor_line >= self.scroll_offset + self.viewport_height {
            self.scroll_offset = cursor_line - self.viewport_height + 1;
        }
    }

    /// Get total virtual lines count (headers + items)
    fn total_virtual_lines(&self) -> usize {
        2 + self.unstaged_item_count() + self.staged_item_count()
    }

    /// Get selected files from the given section (staged or unstaged).
    fn get_selected_files(&self, staged: bool) -> Vec<PathBuf> {
        match self.get_selection() {
            Some(Selection::UnstagedFile(idx)) if !staged => self
                .unstaged_files
                .get(idx)
                .map(|f| f.path.clone())
                .into_iter()
                .collect(),
            Some(Selection::UnstagedDir(idx)) if !staged => {
                tree::collect_files_under(&self.unstaged.tree, idx)
            }
            Some(Selection::StagedFile(idx)) if staged => self
                .staged_files
                .get(idx)
                .map(|f| f.path.clone())
                .into_iter()
                .collect(),
            Some(Selection::StagedDir(idx)) if staged => {
                tree::collect_files_under(&self.staged.tree, idx)
            }
            _ => vec![],
        }
    }

    /// Build a section tree from file entries.
    fn build_section_tree(
        paths: &[(PathBuf, usize, char, bool)],
        collapsed: &HashSet<PathBuf>,
    ) -> (Vec<tree::TreeNode>, Vec<usize>, Vec<String>) {
        let entries: Vec<tree::FileEntry> = paths
            .iter()
            .map(|(path, index, status, untracked)| tree::FileEntry {
                path: path.clone(),
                index: *index,
                status: *status,
                untracked: *untracked,
            })
            .collect();
        let tree_nodes = tree::build_tree(&entries, collapsed);
        let visible = tree::compute_visible_nodes(&tree_nodes);
        let prefixes = tree::compute_tree_prefixes(&tree_nodes, &visible);
        (tree_nodes, visible, prefixes)
    }

    /// Rebuild tree data structures from current file lists.
    fn rebuild_trees(&mut self) {
        let unstaged_data: Vec<_> = self
            .unstaged_files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.clone(), i, f.status, f.untracked))
            .collect();
        let (tree, visible, prefixes) =
            Self::build_section_tree(&unstaged_data, &self.unstaged.collapsed);
        self.unstaged.tree = tree;
        self.unstaged.visible = visible;
        self.unstaged.prefixes = prefixes;

        let staged_data: Vec<_> = self
            .staged_files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.clone(), i, f.status, false))
            .collect();
        let (tree, visible, prefixes) =
            Self::build_section_tree(&staged_data, &self.staged.collapsed);
        self.staged.tree = tree;
        self.staged.visible = visible;
        self.staged.prefixes = prefixes;
    }

    /// Toggle expand/collapse for a directory node.
    fn toggle_dir_expand(&mut self, is_unstaged: bool, tree_idx: usize) {
        let (tree, collapsed) = if is_unstaged {
            (&mut self.unstaged.tree, &mut self.unstaged.collapsed)
        } else {
            (&mut self.staged.tree, &mut self.staged.collapsed)
        };

        if matches!(tree[tree_idx].kind, tree::TreeNodeKind::Directory { .. }) {
            let path = tree[tree_idx].full_path.clone();
            if let tree::TreeNodeKind::Directory { ref mut expanded } = tree[tree_idx].kind {
                *expanded = !*expanded;
                if *expanded {
                    collapsed.remove(&path);
                } else {
                    collapsed.insert(path);
                }
            }
        }

        // Recompute visible nodes and prefixes
        if is_unstaged {
            self.unstaged.recompute_visible();
        } else {
            self.staged.recompute_visible();
        }

        // Clamp cursor
        let max_cursor = self.total_virtual_lines().saturating_sub(1);
        if self.cursor > max_cursor {
            self.cursor = max_cursor;
        }
    }

    // =========================================================================
    // Keyboard Navigation Helpers
    // =========================================================================

    /// Handle Up key navigation
    fn handle_up_key(&mut self) {
        match self.current_section {
            Section::RepoSelector => {
                if self.repo_dropdown_open && self.dropdown_cursor > 0 {
                    self.dropdown_cursor -= 1;
                }
            }
            Section::BranchSelector => {
                if self.branch_dropdown_open && self.dropdown_cursor > 0 {
                    self.dropdown_cursor -= 1;
                }
            }
            Section::Files => {
                let first = self.first_selectable_line();
                if self.cursor == first {
                    self.current_section = Section::BranchSelector;
                } else {
                    let mut new_cursor = self.cursor;
                    while new_cursor > 0 {
                        new_cursor -= 1;
                        if self.is_selectable_line(new_cursor) {
                            self.cursor = new_cursor;
                            self.ensure_cursor_visible();
                            break;
                        }
                    }
                }
            }
            Section::Buttons => {
                if self.has_any_files() {
                    self.current_section = Section::Files;
                    self.cursor = self.last_selectable_line();
                    self.ensure_cursor_visible();
                } else {
                    self.current_section = Section::BranchSelector;
                }
            }
        }
    }

    /// Handle Down key navigation
    fn handle_down_key(&mut self) {
        match self.current_section {
            Section::RepoSelector => {
                if self.repo_dropdown_open {
                    if self.dropdown_cursor + 1 < self.repo_manager.len() {
                        self.dropdown_cursor += 1;
                    }
                } else if self.has_any_files() {
                    self.current_section = Section::Files;
                    self.cursor = self.first_selectable_line();
                    self.ensure_cursor_visible();
                } else {
                    self.current_section = Section::Buttons;
                }
            }
            Section::BranchSelector => {
                if self.branch_dropdown_open {
                    if self.dropdown_cursor + 1 < self.branches.len() {
                        self.dropdown_cursor += 1;
                    }
                } else if self.has_any_files() {
                    self.current_section = Section::Files;
                    self.cursor = self.first_selectable_line();
                    self.ensure_cursor_visible();
                } else {
                    self.current_section = Section::Buttons;
                }
            }
            Section::Files => {
                let last = self.last_selectable_line();
                if self.cursor == last {
                    self.current_section = Section::Buttons;
                    let total = self.total_virtual_lines();
                    if total > self.viewport_height {
                        self.scroll_offset = total - self.viewport_height;
                    }
                } else {
                    let max = self.total_virtual_lines();
                    let mut new_cursor = self.cursor;
                    while new_cursor + 1 < max {
                        new_cursor += 1;
                        if self.is_selectable_line(new_cursor) {
                            self.cursor = new_cursor;
                            self.ensure_cursor_visible();
                            break;
                        }
                    }
                }
            }
            Section::Buttons => {
                // At bottom, do nothing
            }
        }
    }

    /// Handle Enter key
    fn handle_enter_key(&mut self) -> Vec<PanelEvent> {
        match self.current_section {
            Section::Files => {
                match self.get_selection() {
                    Some(Selection::UnstagedFile(_)) => self.do_stage(),
                    Some(Selection::StagedFile(_)) => self.do_unstage(),
                    Some(Selection::UnstagedDir(idx)) => self.toggle_dir_expand(true, idx),
                    Some(Selection::StagedDir(idx)) => self.toggle_dir_expand(false, idx),
                    _ => {}
                }
                vec![]
            }
            Section::RepoSelector => {
                if self.repo_dropdown_open {
                    if self.dropdown_cursor != self.repo_manager.selected_index() {
                        self.repo_manager.select(self.dropdown_cursor);
                        self.refresh();
                    }
                    self.repo_dropdown_open = false;
                } else {
                    self.repo_dropdown_open = true;
                    self.dropdown_cursor = self.repo_manager.selected_index();
                }
                vec![]
            }
            Section::BranchSelector => {
                if self.branch_dropdown_open {
                    self.switch_to_branch(self.dropdown_cursor);
                    self.branch_dropdown_open = false;
                } else {
                    self.branch_dropdown_open = true;
                    self.dropdown_cursor = self
                        .branches
                        .iter()
                        .position(|b| Some(b.as_str()) == self.branch.as_deref())
                        .unwrap_or(0);
                }
                vec![]
            }
            Section::Buttons => self.execute_button(),
        }
    }

    /// Check if click column hits the expand/collapse icon of a tree directory node.
    /// Returns `Some((is_unstaged, tree_idx))` if the click is on a directory icon.
    fn check_dir_icon_click(&self, vline: usize, relative_col: usize) -> Option<(bool, usize)> {
        let unstaged_start = 1;
        let unstaged_end = unstaged_start + self.unstaged_item_count();
        let staged_start = unstaged_end + 1;

        let (is_unstaged, visible_idx) = if vline >= unstaged_start && vline < unstaged_end {
            (true, vline - unstaged_start)
        } else if vline >= staged_start {
            (false, vline - staged_start)
        } else {
            return None;
        };

        let ft = if is_unstaged {
            &self.unstaged
        } else {
            &self.staged
        };
        let (tree_nodes, visible, prefixes) = (&ft.tree, &ft.visible, &ft.prefixes);

        let &tree_idx = visible.get(visible_idx)?;
        if !matches!(
            tree_nodes[tree_idx].kind,
            tree::TreeNodeKind::Directory { .. }
        ) {
            return None;
        }

        // Rendering layout: " {prefix}{arrow} /{name}"
        // Arrow icon is at column 1 + prefix_width
        let prefix_width = prefixes.get(visible_idx).map(|p| p.width()).unwrap_or(0);
        let icon_end = 1 + prefix_width + 1; // " " + prefix + arrow char

        if relative_col <= icon_end {
            Some((is_unstaged, tree_idx))
        } else {
            None
        }
    }

    /// Check if current click is a double-click on the same item
    fn check_double_click(&self, now: std::time::Instant, vline: usize) -> bool {
        self.click_tracker.is_double_click_at(now, &vline)
    }

    /// Reset double-click tracking state
    fn reset_click_state(&mut self) {
        self.click_tracker.reset();
    }

    /// Record click for double-click detection
    fn record_click(&mut self, now: std::time::Instant, vline: usize) {
        self.click_tracker.record_at(now, vline);
    }

    /// Take modal request for app to handle
    pub fn take_modal_request(&mut self) -> Option<(PendingAction, ActiveModal)> {
        self.modal_request.take()
    }

    /// Get disk space information for the current repository.
    pub fn get_disk_space_info(&self) -> Option<termide_system_monitor::DiskSpaceInfo> {
        self.repo_manager
            .current()
            .and_then(termide_system_monitor::get_disk_space_info)
    }
}

impl Panel for GitStatusPanel {
    fn name(&self) -> &'static str {
        "git_status"
    }

    fn width_preference(&self) -> WidthPreference {
        WidthPreference::PreferNarrow
    }

    fn title(&self) -> String {
        use termide_config::constants::spinner_frame;

        let t = termide_i18n::t();
        let repo_name = self
            .repo_manager
            .current()
            .map(git::get_repo_name)
            .unwrap_or_else(|| t.git_no_repo().to_string());
        let detached = t.git_branch_detached().to_string();
        let branch = self.branch.as_deref().unwrap_or(&detached);

        let uncommitted = self.unstaged_files.len() + self.staged_files.len();
        let status = format!("*{} ↑{} ↓{}", uncommitted, self.ahead, self.behind);

        if self.is_loading {
            let spinner = spinner_frame();
            format!(
                "{} {} ({}) {} ({})",
                spinner,
                repo_name,
                branch,
                status,
                t.git_status_loading()
            )
        } else {
            format!("{} ({}) {}", repo_name, branch, status)
        }
    }

    fn colorize_title(&self, truncated: &str, base_style: Style) -> Line<'static> {
        let markers: &[(&str, ratatui::style::Color)] = &[
            ("*", self.cached_theme.error),
            ("\u{2191}", self.cached_theme.success), // ↑
            ("\u{2193}", self.cached_theme.warning), // ↓
        ];

        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut rest = truncated;

        while !rest.is_empty() {
            // Find the earliest marker
            let mut earliest: Option<(usize, &str, ratatui::style::Color)> = None;
            for &(marker, color) in markers {
                if let Some(pos) = rest.find(marker) {
                    if earliest.is_none_or(|(e_pos, _, _)| pos < e_pos) {
                        earliest = Some((pos, marker, color));
                    }
                }
            }

            match earliest {
                Some((pos, marker, color)) => {
                    // Text before marker
                    if pos > 0 {
                        spans.push(Span::styled(rest[..pos].to_string(), base_style));
                    }
                    // Marker + following digits
                    let after_marker = &rest[pos + marker.len()..];
                    let digit_count = after_marker
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .count();
                    let end = pos + marker.len() + digit_count;
                    let value: usize = after_marker[..digit_count].parse().unwrap_or(0);
                    let marker_style = if value > 0 {
                        base_style.fg(color)
                    } else {
                        base_style
                    };
                    spans.push(Span::styled(rest[pos..end].to_string(), marker_style));
                    rest = &rest[end..];
                }
                None => {
                    spans.push(Span::styled(rest.to_string(), base_style));
                    break;
                }
            }
        }

        Line::from(spans)
    }

    fn prepare_render(&mut self, theme: &Theme, config: &std::sync::Arc<Config>) {
        self.cached_theme = ThemeColors::from(theme);
        self.vim_mode = config.general.vim_mode;
        let config_ptr = std::sync::Arc::as_ptr(config) as usize;
        if self.last_config_ptr != config_ptr {
            self.last_config_ptr = config_ptr;
            self.hotkeys = build_git_status_hotkey_table(config);
        }
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
        self.last_area = area;

        // Render content (border is handled by ui-render)
        self.render_content(area, buf, ctx.is_focused, ctx.border_right_x);
    }

    fn captures_escape(&self) -> bool {
        // Capture Escape when dropdown is open (don't close panel)
        self.repo_dropdown_open || self.branch_dropdown_open
    }

    fn handle_command(&mut self, cmd: PanelCommand<'_>) -> CommandResult {
        match cmd {
            PanelCommand::OnGitUpdate { repo_paths } => {
                // Check if current repo is in the updated list
                if let Some(current_repo) = self.repo_manager.current() {
                    let should_refresh = repo_paths
                        .iter()
                        .any(|p| current_repo.starts_with(p) || p.starts_with(current_repo));
                    if should_refresh {
                        self.refresh();
                        return CommandResult::NeedsRedraw(true);
                    }
                }
                CommandResult::NeedsRedraw(false)
            }
            PanelCommand::OnFsUpdate { changed_path } => {
                // Refresh on file changes within current repo
                if let Some(current_repo) = self.repo_manager.current() {
                    if changed_path.starts_with(current_repo) {
                        self.refresh();
                        return CommandResult::NeedsRedraw(true);
                    }
                }
                CommandResult::NeedsRedraw(false)
            }
            PanelCommand::SetGitOperationInProgress {
                in_progress,
                operation,
                spinner_frame,
            } => {
                let changed = self.git_operation_in_progress != in_progress
                    || self.current_operation != operation
                    || self.spinner_frame != spinner_frame;
                if changed {
                    self.git_operation_in_progress = in_progress;
                    self.current_operation = operation;
                    self.spinner_frame = spinner_frame;
                    // Adjust selected button if Push/Pull disappeared
                    let buttons = self.get_visible_buttons();
                    if self.selected_button >= buttons.len() {
                        self.selected_button = buttons.len().saturating_sub(1);
                    }
                    return CommandResult::NeedsRedraw(true);
                }
                CommandResult::NeedsRedraw(false)
            }
            PanelCommand::MarkStale => {
                self.is_stale = true;
                self.refresh_title_data();
                CommandResult::NeedsRedraw(true)
            }
            PanelCommand::Reload => {
                self.refresh();
                CommandResult::NeedsRedraw(true)
            }
            PanelCommand::RefreshIfStale => {
                if self.is_stale {
                    self.is_stale = false;
                    self.refresh();
                    CommandResult::NeedsRedraw(true)
                } else {
                    CommandResult::None
                }
            }
            PanelCommand::UpdateRepoPaths { paths } => {
                self.update_repos(&paths);
                CommandResult::NeedsRedraw(true)
            }
            PanelCommand::GetFsWatchInfo => {
                // Return watch info so the app watcher registers the repo root.
                // current_path is the repo root (used by app to call find_repo_root).
                let current_path = self
                    .repo_manager
                    .current()
                    .map(|p| p.to_path_buf())
                    .or_else(|| self.initial_paths.first().cloned())
                    .unwrap_or_default();
                CommandResult::FsWatchInfo {
                    watched_root: self.watched_root.clone(),
                    current_path,
                    is_git_repo: self.repo_manager.current().is_some(),
                }
            }
            PanelCommand::SetFsWatchRoot { root, .. } => {
                self.watched_root = root;
                CommandResult::None
            }
            _ => CommandResult::None,
        }
    }

    fn handle_key(&mut self, chord: termide_core::KeyChord) -> Vec<PanelEvent> {
        let key = chord.raw;
        // Clear status message on any key
        self.status_message = None;

        // Configurable actions via HotkeyTable
        if self.hotkeys.matches("stage", &key) {
            if self.current_section == Section::Files
                && matches!(
                    self.get_selection(),
                    Some(Selection::UnstagedFile(_)) | Some(Selection::UnstagedDir(_))
                )
            {
                self.do_stage();
            }
            return vec![];
        }

        if self.hotkeys.matches("unstage", &key) {
            if self.current_section == Section::Files
                && matches!(
                    self.get_selection(),
                    Some(Selection::StagedFile(_)) | Some(Selection::StagedDir(_))
                )
            {
                self.do_unstage();
            }
            return vec![];
        }

        if self.hotkeys.matches("view", &key) {
            if self.current_section == Section::Files
                && matches!(
                    self.get_selection(),
                    Some(Selection::UnstagedFile(_)) | Some(Selection::StagedFile(_))
                )
            {
                return self.open_file(false);
            }
            return vec![];
        }
        if self.hotkeys.matches("edit", &key) {
            if self.current_section == Section::Files
                && matches!(
                    self.get_selection(),
                    Some(Selection::UnstagedFile(_)) | Some(Selection::StagedFile(_))
                )
            {
                return self.open_file(true);
            }
            return vec![];
        }
        if self.hotkeys.matches("info", &key) {
            if self.current_section == Section::Files
                && matches!(
                    self.get_selection(),
                    Some(Selection::UnstagedFile(_)) | Some(Selection::StagedFile(_))
                )
            {
                return self.show_file_properties();
            }
            return vec![];
        }
        if self.hotkeys.matches("revert", &key) {
            if self.current_section == Section::Files
                && matches!(
                    self.get_selection(),
                    Some(Selection::UnstagedFile(_)) | Some(Selection::StagedFile(_))
                )
            {
                return self.initiate_revert();
            }
            return vec![];
        }
        if self.hotkeys.matches("refresh", &key) {
            self.refresh();
            self.status_message = Some(termide_i18n::t().git_refreshed().to_string());
            if let Some(repo) = self.repo_manager.current() {
                use termide_core::event::{GitOperationType, PanelEvent};
                return vec![PanelEvent::GitOperation {
                    operation: GitOperationType::Fetch,
                    repo_path: repo.to_path_buf(),
                }];
            }
            return vec![];
        }

        // Vim-aware navigation (j/k/g/G when vim_mode is enabled)
        if is_move_up(&key, self.vim_mode) {
            self.handle_up_key();
            return vec![];
        }
        if is_move_down(&key, self.vim_mode) {
            self.handle_down_key();
            return vec![];
        }
        if is_go_home(&key, self.vim_mode) && self.current_section == Section::Files {
            self.cursor = self.first_selectable_line();
            self.ensure_cursor_visible();
            return vec![];
        }
        if is_go_end(&key, self.vim_mode) && self.current_section == Section::Files {
            self.cursor = self.last_selectable_line();
            self.ensure_cursor_visible();
            return vec![];
        }

        match key.code {
            KeyCode::Tab => {
                self.next_section();
            }
            KeyCode::BackTab => {
                self.prev_section();
            }
            KeyCode::PageUp => {
                if self.current_section == Section::Files {
                    let page_size = self.viewport_height.max(1);
                    let mut new_cursor = self.cursor.saturating_sub(page_size);
                    while new_cursor > 0 && !self.is_selectable_line(new_cursor) {
                        new_cursor -= 1;
                    }
                    if self.is_selectable_line(new_cursor) {
                        self.cursor = new_cursor;
                    }
                    self.ensure_cursor_visible();
                }
            }
            KeyCode::PageDown => {
                if self.current_section == Section::Files {
                    let max = self.total_virtual_lines();
                    let page_size = self.viewport_height.max(1);
                    let target = (self.cursor + page_size).min(max.saturating_sub(1));
                    let mut new_cursor = target;
                    while new_cursor > self.cursor && !self.is_selectable_line(new_cursor) {
                        new_cursor -= 1;
                    }
                    if new_cursor > self.cursor && self.is_selectable_line(new_cursor) {
                        self.cursor = new_cursor;
                    }
                    self.ensure_cursor_visible();
                    if self.cursor == self.last_selectable_line() && max > self.viewport_height {
                        self.scroll_offset = max.saturating_sub(self.viewport_height);
                    }
                }
            }
            KeyCode::Home => {
                if self.current_section == Section::Files {
                    let unstaged_end = 1 + self.unstaged_item_count();
                    let staged_header = unstaged_end;
                    if self.cursor < staged_header {
                        if self.unstaged_item_count() > 0 {
                            self.cursor = 1;
                        } else {
                            self.cursor = 0;
                        }
                    } else if self.staged_item_count() > 0 {
                        self.cursor = staged_header + 1;
                    } else {
                        self.cursor = staged_header;
                    }
                    self.ensure_cursor_visible();
                }
            }
            KeyCode::End => {
                if self.current_section == Section::Files {
                    let unstaged_end = 1 + self.unstaged_item_count();
                    let staged_header = unstaged_end;
                    let staged_end = staged_header + 1 + self.staged_item_count();
                    if self.cursor < staged_header {
                        if self.unstaged_item_count() > 0 {
                            self.cursor = unstaged_end - 1;
                        } else {
                            self.cursor = 0;
                        }
                    } else if self.staged_item_count() > 0 {
                        self.cursor = staged_end - 1;
                    } else {
                        self.cursor = staged_header;
                    }
                    self.ensure_cursor_visible();
                }
            }
            KeyCode::Left => match self.current_section {
                Section::BranchSelector => {
                    self.current_section = Section::RepoSelector;
                }
                Section::Buttons => {
                    if self.selected_button > 0 {
                        self.selected_button -= 1;
                    }
                }
                Section::Files => match self.get_selection() {
                    Some(Selection::UnstagedDir(idx)) => {
                        if matches!(
                            self.unstaged.tree[idx].kind,
                            tree::TreeNodeKind::Directory { expanded: true }
                        ) {
                            self.toggle_dir_expand(true, idx);
                        }
                    }
                    Some(Selection::StagedDir(idx)) => {
                        if matches!(
                            self.staged.tree[idx].kind,
                            tree::TreeNodeKind::Directory { expanded: true }
                        ) {
                            self.toggle_dir_expand(false, idx);
                        }
                    }
                    _ => {}
                },
                _ => {}
            },
            KeyCode::Right => match self.current_section {
                Section::RepoSelector => {
                    self.current_section = Section::BranchSelector;
                }
                Section::Buttons => {
                    let max = self.get_visible_buttons().len().saturating_sub(1);
                    if self.selected_button < max {
                        self.selected_button += 1;
                    }
                }
                Section::Files => match self.get_selection() {
                    Some(Selection::UnstagedDir(idx)) => {
                        if matches!(
                            self.unstaged.tree[idx].kind,
                            tree::TreeNodeKind::Directory { expanded: false }
                        ) {
                            self.toggle_dir_expand(true, idx);
                        }
                    }
                    Some(Selection::StagedDir(idx)) => {
                        if matches!(
                            self.staged.tree[idx].kind,
                            tree::TreeNodeKind::Directory { expanded: false }
                        ) {
                            self.toggle_dir_expand(false, idx);
                        }
                    }
                    _ => {}
                },
                _ => {}
            },
            KeyCode::Enter => {
                return self.handle_enter_key();
            }
            KeyCode::Esc => {
                if self.branch_dropdown_open {
                    self.branch_dropdown_open = false;
                } else if self.repo_dropdown_open {
                    self.repo_dropdown_open = false;
                }
            }
            _ => {}
        }

        vec![]
    }

    fn handle_mouse(&mut self, event: MouseEvent, _panel_area: Rect) -> Vec<PanelEvent> {
        let col = event.column;
        let row = event.row;

        match event.kind {
            // Scroll handling - unified scroll for files area
            MouseEventKind::ScrollUp => {
                if self.is_in_rect(col, row, self.files_area) {
                    self.scroll_offset = self.scroll_offset.saturating_sub(3);
                }
            }
            MouseEventKind::ScrollDown => {
                if self.is_in_rect(col, row, self.files_area) {
                    let total_lines = self.total_virtual_lines();
                    let max_scroll = total_lines.saturating_sub(self.viewport_height);
                    self.scroll_offset = (self.scroll_offset + 3).min(max_scroll);
                }
            }

            // Click handling
            MouseEventKind::Down(MouseButton::Left) => {
                let now = std::time::Instant::now();

                // Check if click is in open repo dropdown
                if self.repo_dropdown_open {
                    if let Some(dropdown_area) = self.repo_dropdown_area {
                        if self.is_in_rect(col, row, dropdown_area) {
                            // Calculate which item was clicked (accounting for border)
                            let relative_row = row.saturating_sub(dropdown_area.y + 1) as usize;
                            let clicked_idx = self.dropdown_scroll + relative_row;
                            if clicked_idx < self.repo_manager.len()
                                && relative_row < dropdown_area.height.saturating_sub(2) as usize
                            {
                                self.repo_manager.select(clicked_idx);
                                self.refresh();
                            }
                            self.repo_dropdown_open = false;
                            return vec![];
                        }
                    }
                }

                // Check if click is in open branch dropdown
                if self.branch_dropdown_open {
                    if let Some(dropdown_area) = self.branch_dropdown_area {
                        if self.is_in_rect(col, row, dropdown_area) {
                            // Calculate which item was clicked (accounting for border)
                            let relative_row = row.saturating_sub(dropdown_area.y + 1) as usize;
                            let clicked_idx = self.dropdown_scroll + relative_row;
                            if clicked_idx < self.branches.len()
                                && relative_row < dropdown_area.height.saturating_sub(2) as usize
                            {
                                self.switch_to_branch(clicked_idx);
                            }
                            self.branch_dropdown_open = false;
                            return vec![];
                        }
                    }
                }

                // Check if click is in files area (unified)
                if self.is_in_rect(col, row, self.files_area) {
                    // Close any open dropdown
                    self.repo_dropdown_open = false;
                    self.branch_dropdown_open = false;

                    let relative_row = (row - self.files_area.y) as usize;
                    let relative_col = (col - self.files_area.x) as usize;
                    let vline = self.scroll_offset + relative_row;

                    // Virtual layout constants
                    let unstaged_files_start = 1;
                    let unstaged_files_end = unstaged_files_start + self.unstaged_item_count();
                    let staged_header_line = unstaged_files_end;
                    let staged_files_start = staged_header_line + 1;
                    let staged_files_end = staged_files_start + self.staged_item_count();

                    // Determine what was clicked
                    let unstaged_header_line = 0;

                    // Single click on directory icon → toggle expand/collapse
                    if let Some((is_unstaged, tree_idx)) =
                        self.check_dir_icon_click(vline, relative_col)
                    {
                        self.current_section = Section::Files;
                        self.cursor = vline;
                        self.toggle_dir_expand(is_unstaged, tree_idx);
                        self.reset_click_state();
                    } else if vline == unstaged_header_line && !self.unstaged_files.is_empty() {
                        // Clicked on unstaged header (with Stage all button)
                        self.current_section = Section::Files;
                        self.cursor = vline;
                        self.record_click(now, vline);
                    } else if vline >= unstaged_files_start && vline < unstaged_files_end {
                        // Clicked on unstaged item (file or dir name area)
                        self.current_section = Section::Files;
                        self.cursor = vline;
                        if self.check_double_click(now, vline) {
                            // Double-click: stage file
                            self.do_stage();
                            self.reset_click_state();
                        } else {
                            self.record_click(now, vline);
                        }
                    } else if vline == staged_header_line && !self.staged_files.is_empty() {
                        // Clicked on staged header (with Unstage all button)
                        self.current_section = Section::Files;
                        self.cursor = vline;
                        self.record_click(now, vline);
                    } else if vline >= staged_files_start && vline < staged_files_end {
                        // Clicked on staged item (file or dir name area)
                        self.current_section = Section::Files;
                        self.cursor = vline;
                        if self.check_double_click(now, vline) {
                            // Double-click: unstage file
                            self.do_unstage();
                            self.reset_click_state();
                        } else {
                            self.record_click(now, vline);
                        }
                    }
                    // Clicks on empty header lines are ignored
                }
                // Check if click is on selector row
                else if row == self.selector_y {
                    // Use saved branch_selector_x position for accurate detection
                    if col < self.branch_selector_x {
                        self.current_section = Section::RepoSelector;
                        // Toggle repo dropdown (close branch if open)
                        self.branch_dropdown_open = false;
                        self.repo_dropdown_open = !self.repo_dropdown_open;
                        if self.repo_dropdown_open {
                            self.dropdown_cursor = self.repo_manager.selected_index();
                        }
                    } else {
                        self.current_section = Section::BranchSelector;
                        // Toggle branch dropdown (close repo if open)
                        self.repo_dropdown_open = false;
                        self.branch_dropdown_open = !self.branch_dropdown_open;
                        if self.branch_dropdown_open {
                            self.dropdown_cursor = self
                                .branches
                                .iter()
                                .position(|b| Some(b.as_str()) == self.branch.as_deref())
                                .unwrap_or(0);
                        }
                    }
                    // Reset click state for non-file areas
                    self.reset_click_state();
                }
                // Check if click is on buttons area (may span multiple rows)
                else if row >= self.buttons_y && row < self.buttons_y + self.cached_buttons_height
                {
                    // Close any open dropdown
                    self.repo_dropdown_open = false;
                    self.branch_dropdown_open = false;

                    self.current_section = Section::Buttons;
                    // Calculate which button was clicked, accounting for wrapping
                    // Note: last_area is already content area (borders handled by ui-render)
                    let buttons = self.get_visible_buttons();
                    let content_x = self.last_area.x;
                    let content_width = self.last_area.width;
                    let mut btn_x = content_x;
                    let mut btn_y = self.buttons_y;
                    for (i, button) in buttons.iter().enumerate() {
                        let label = format!("[{}]", button.label(self.spinner_frame));
                        let btn_width = label.width() as u16;
                        if btn_x > content_x && btn_x + btn_width > content_x + content_width {
                            btn_y += 1;
                            btn_x = content_x;
                        }
                        if row == btn_y && col >= btn_x && col < btn_x + btn_width {
                            self.selected_button = i;
                            // Execute button action on click
                            return self.execute_button();
                        }
                        btn_x += btn_width + 1;
                    }
                    // Reset click state for non-file areas
                    self.reset_click_state();
                }
            }

            _ => {}
        }

        vec![]
    }

    fn handle_scroll(&mut self, delta: i32, _panel_area: Rect) -> Vec<PanelEvent> {
        let lines = delta.unsigned_abs() as usize * 3; // 3 lines per scroll unit
        if delta < 0 {
            // Scroll up
            self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        } else {
            // Scroll down
            let total_lines = self.total_virtual_lines();
            let max_scroll = total_lines.saturating_sub(self.viewport_height);
            self.scroll_offset = (self.scroll_offset + lines).min(max_scroll);
        }
        vec![]
    }

    fn to_session(&self, _session_dir: &Path) -> Option<SessionPanel> {
        self.repo_manager
            .current()
            .map(|repo| SessionPanel::GitStatus {
                repo_path: repo.to_path_buf(),
            })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn get_working_directory(&self) -> Option<PathBuf> {
        self.repo_manager.current().map(|p| p.to_path_buf())
    }

    fn tick(&mut self) -> Vec<PanelEvent> {
        // Trigger initial fetch once when panel is ready and has a repo
        if self.pending_init_fetch && !self.repo_manager.is_empty() {
            self.pending_init_fetch = false;
            if let Some(repo) = self.repo_manager.current() {
                use termide_core::event::{GitOperationType, PanelEvent};
                return vec![PanelEvent::GitOperation {
                    operation: GitOperationType::Fetch,
                    repo_path: repo.to_path_buf(),
                }];
            }
        }
        vec![]
    }
}
