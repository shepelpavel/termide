//! File manager panel for termide.
//!
//! Provides a smart file manager with git integration, drag selection, and file operations.

mod file_info;
mod file_search;
mod git_status;
mod keyboard;
mod navigation;
mod operations;
mod rendering;
mod selection;
mod tree;
mod utils;
mod vfs_state;

pub use file_info::FileInfo;
use navigation::NavigationState;
use selection::SelectionState;
use vfs_state::VfsState;

/// Case-insensitive string comparison without allocation.
fn cmp_ignore_case(a: &str, b: &str) -> std::cmp::Ordering {
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

/// Sort group key: 0 = directories, 1 = executable files, 2 = regular files.
fn sort_group(entry: &FileEntry) -> u8 {
    if entry.is_dir {
        0
    } else if entry.is_executable {
        1
    } else {
        2
    }
}

/// Sort entries: directories first, then executables, then regular files.
/// Within each group, sort alphabetically (case-insensitive).
fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        sort_group(a)
            .cmp(&sort_group(b))
            .then_with(|| cmp_ignore_case(&a.name, &b.name))
    });
}

use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{buffer::Buffer, layout::Rect, prelude::Widget, widgets::Paragraph};
use std::any::Any;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;

use termide_config::{constants, Config, FileManagerSettings};
use termide_core::{CommandResult, Panel, PanelCommand, PanelEvent, RenderContext, SessionPanel};
use termide_git::{get_git_status_async, GitStatus, GitStatusAsyncResult, GitStatusCache};
use termide_modal::{ActionButton, ActiveModal, ConfirmModal, InfoActionModal, InputModal};
use termide_state::{DirSizeResult, PendingAction};
use termide_theme::Theme;
use termide_ui::{clipboard, path_utils, IndexClickTracker, ScrollBar};
use termide_vfs::{VfsEntry, VfsFileType};

/// Smart file manager with advanced features
pub struct FileManager {
    current_path: PathBuf,
    /// Flat tree of all known entries (top-level + expanded subdirectories).
    tree_entries: Vec<tree::TreeEntry>,
    /// Indices into `tree_entries` of currently visible nodes (hides collapsed children).
    visible_indices: Vec<usize>,
    /// Tree-drawing prefixes (├─, └─, │) for each visible node.
    tree_prefixes: Vec<String>,
    /// Set of expanded directory paths (persists across reloads within session).
    expanded_dirs: HashSet<PathBuf>,
    /// Cursor position — index into `visible_indices`.
    selected: usize,
    scroll_offset: usize,
    /// Modal window request (action, modal)
    modal_request: Option<(PendingAction, ActiveModal)>,
    /// Visible area height (updated during rendering)
    visible_height: usize,
    /// Click tracker for double-click detection
    click_tracker: IndexClickTracker,
    /// Selection state (multi-select and drag)
    selection: SelectionState,
    /// Git status cache for the current directory
    git_status_cache: Option<GitStatusCache>,
    /// Channel receiver for async git status loading
    git_status_receiver: Option<mpsc::Receiver<GitStatusAsyncResult>>,
    /// Channel receiver for directory size calculation results (needs to be passed to AppState)
    pub dir_size_receiver: Option<mpsc::Receiver<DirSizeResult>>,
    /// Navigation state (cursor restoration, debouncing)
    navigation: NavigationState,
    /// Git repository root (None = not in git repo)
    /// Used for reference counting when navigating between directories
    git_root: Option<PathBuf>,
    /// Cached theme for rendering
    cached_theme: Theme,
    /// Cached config for rendering
    cached_config: FileManagerSettings,
    /// Cached vim_mode setting for keyboard handling
    vim_mode: bool,
    /// Cached VFS connection timeout in seconds
    cached_vfs_timeout_secs: u64,
    /// VFS state for network filesystem support
    vfs: VfsState,
    /// Whether panel is stale (collapsed, skipping background work)
    is_stale: bool,
    /// Whether to show hidden (dot) files
    show_hidden: bool,
    /// File/content search state (replaces TreeSearchModal results display)
    file_search: Option<file_search::FileSearchState>,
}

#[derive(Debug, Clone)]
pub(crate) struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub is_executable: bool,
    pub is_readonly: bool,
    pub git_status: GitStatus,
    pub size: Option<u64>,
    pub modified: Option<std::time::SystemTime>,
}

impl FileEntry {
    /// Create FileEntry from VfsEntry (for remote directories).
    pub fn from_vfs_entry(entry: VfsEntry) -> Self {
        Self {
            name: entry.name,
            is_dir: matches!(entry.metadata.file_type, VfsFileType::Directory),
            is_symlink: matches!(entry.metadata.file_type, VfsFileType::Symlink),
            is_executable: entry
                .metadata
                .permissions
                .map(|p| p & 0o111 != 0)
                .unwrap_or(false),
            is_readonly: entry.metadata.readonly,
            git_status: GitStatus::Unmodified, // Remote files don't have git status
            size: if matches!(entry.metadata.file_type, VfsFileType::File) {
                Some(entry.metadata.size)
            } else {
                None
            },
            modified: entry.metadata.modified,
        }
    }
}

impl FileManager {
    // ── Tree helpers ───────────────────────────────────────────────────

    /// Number of visible entries (used in place of old `entries.len()`).
    fn visible_count(&self) -> usize {
        self.visible_indices.len()
    }

    /// Get `FileEntry` at a visible index.
    fn entry_at(&self, vis_idx: usize) -> Option<&FileEntry> {
        let tree_idx = *self.visible_indices.get(vis_idx)?;
        Some(&self.tree_entries[tree_idx].file_entry)
    }

    /// Get `TreeEntry` at a visible index.
    fn tree_entry_at(&self, vis_idx: usize) -> Option<&tree::TreeEntry> {
        let tree_idx = *self.visible_indices.get(vis_idx)?;
        Some(&self.tree_entries[tree_idx])
    }

    /// Get full path of entry at a visible index.
    fn path_at(&self, vis_idx: usize) -> Option<&PathBuf> {
        let tree_idx = *self.visible_indices.get(vis_idx)?;
        Some(&self.tree_entries[tree_idx].full_path)
    }

    /// Recompute `visible_indices` and `tree_prefixes` from `tree_entries`.
    fn recompute_visible(&mut self) {
        self.visible_indices = tree::compute_visible(&self.tree_entries);
        self.tree_prefixes = tree::compute_prefixes(&self.tree_entries, &self.visible_indices);
    }

    /// Find visible index by entry name (top-level only for navigation restore).
    fn find_entry_index(&self, name: &str) -> Option<usize> {
        self.visible_indices
            .iter()
            .position(|&ti| self.tree_entries[ti].file_entry.name == name)
    }

    /// Create a new smart file manager
    pub fn new() -> Self {
        let current_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        Self::new_with_path(current_path)
    }

    /// Create a new smart file manager with the specified path
    pub fn new_with_path(current_path: PathBuf) -> Self {
        let vfs = VfsState::with_path(termide_vfs::VfsPath::local(&current_path), None);
        let mut fm = Self {
            current_path,
            tree_entries: Vec::new(),
            visible_indices: Vec::new(),
            tree_prefixes: Vec::new(),
            expanded_dirs: HashSet::new(),
            selected: 0,
            scroll_offset: 0,
            modal_request: None,
            visible_height: 10, // Default value, will be updated during rendering
            click_tracker: IndexClickTracker::new(),
            selection: SelectionState::default(),
            git_status_cache: None,
            git_status_receiver: None,
            dir_size_receiver: None,
            navigation: NavigationState::new(),
            git_root: None,
            cached_theme: Theme::default(),
            cached_config: FileManagerSettings::default(),
            vim_mode: false,
            cached_vfs_timeout_secs: 60, // Default, will be updated from config
            vfs,
            is_stale: false,
            show_hidden: true,
            file_search: None,
        };
        let _ = fm.load_directory();
        fm
    }

    /// Create a new FileManager at a VFS URL (for cloning remote panels)
    pub fn new_with_vfs_url(
        url: &str,
        vfs_manager: std::sync::Arc<termide_vfs::VfsManager>,
    ) -> anyhow::Result<Self> {
        let vfs_path = termide_vfs::parse_vfs_url(url)?;
        let vfs = VfsState::with_path(vfs_path, Some(vfs_manager));

        let mut fm = Self {
            current_path: PathBuf::from("/"), // Not used for remote
            tree_entries: Vec::new(),
            visible_indices: Vec::new(),
            tree_prefixes: Vec::new(),
            expanded_dirs: HashSet::new(),
            selected: 0,
            scroll_offset: 0,
            modal_request: None,
            visible_height: 10,
            click_tracker: IndexClickTracker::new(),
            selection: SelectionState::default(),
            git_status_cache: None,
            git_status_receiver: None,
            dir_size_receiver: None,
            navigation: NavigationState::new(),
            git_root: None,
            cached_theme: Theme::default(),
            cached_config: FileManagerSettings::default(),
            vim_mode: false,
            cached_vfs_timeout_secs: 60,
            vfs,
            is_stale: false,
            show_hidden: true,
            file_search: None,
        };

        // Start the directory listing operation for remote paths
        fm.vfs.start_list_dir();

        Ok(fm)
    }

    // ── File/content search methods ─────────────────────────────────────

    /// Start file glob search
    pub fn start_file_search(&mut self, glob_mask: &str) {
        let mut state = file_search::FileSearchState::new_file_glob(self.current_path.clone());
        state.start_file_search(glob_mask);
        self.file_search = Some(state);
    }

    /// Start content search (glob mask + regex pattern)
    pub fn start_content_search(&mut self, glob_mask: &str, regex_pattern: &str) {
        let max_file_size = self.cached_config.content_search_max_file_size_mb * 1024 * 1024;
        let mut state =
            file_search::FileSearchState::new_content(self.current_path.clone(), max_file_size);
        state.start_content_search(glob_mask, regex_pattern);
        self.file_search = Some(state);
    }

    /// Navigate to next search result
    pub fn search_next(&mut self) {
        if let Some(ref mut state) = self.file_search {
            state.next_result();
        }
    }

    /// Navigate to previous search result
    pub fn search_prev(&mut self) {
        if let Some(ref mut state) = self.file_search {
            state.prev_result();
        }
    }

    /// Close file search and return to normal tree view
    pub fn close_file_search(&mut self) {
        self.file_search = None;
    }

    /// Get match info for search modal display
    pub fn get_file_search_match_info(&self) -> Option<(usize, usize)> {
        self.file_search.as_ref().and_then(|s| s.get_match_info())
    }

    /// Whether file search is active
    pub fn is_file_search_active(&self) -> bool {
        self.file_search.is_some()
    }

    /// Close search and apply the selected result.
    /// For FileGlob: navigates to the file in the tree.
    /// For Content: returns `Some(PanelEvent::OpenFileAt { .. })` so the caller can open the file.
    pub fn close_search_with_selection(&mut self) -> Option<PanelEvent> {
        use file_search::SelectedSearchResult;
        let selection = self.file_search.as_ref()?.get_selected_result();
        self.close_file_search();
        match selection? {
            SelectedSearchResult::NavigateToFile(path) => {
                self.navigate_to_file(&path);
                None
            }
            SelectedSearchResult::OpenAtLine { path, line } => Some(PanelEvent::OpenFileAt {
                path,
                line,
                column: 0,
            }),
        }
    }

    /// Get the VfsManager Arc (for cloning panels)
    pub fn vfs_manager_arc(&self) -> std::sync::Arc<termide_vfs::VfsManager> {
        self.vfs.manager_arc()
    }

    /// Get the current directory
    pub fn get_current_directory(&self) -> PathBuf {
        self.current_path.clone()
    }

    /// Get the git repository root (None if not in a git repo)
    pub fn git_root(&self) -> Option<&PathBuf> {
        self.git_root.as_ref()
    }

    /// Get the currently watched root path (git_root or current_path for non-git)
    pub fn watched_root(&self) -> Option<&PathBuf> {
        self.git_root.as_ref()
    }

    /// Set the watched root path and whether it's a git repository
    pub fn set_watched_root(&mut self, root: Option<PathBuf>, is_git_repo: bool) {
        self.git_root = if is_git_repo { root } else { None };
    }

    /// Check if the watched root is a git repository
    pub fn is_watched_root_git_repo(&self) -> bool {
        self.git_root.is_some()
    }

    /// Check if absolute path is in a gitignored directory
    /// Uses cached git_status_cache to avoid spawning git processes
    pub fn is_path_ignored(&self, absolute_path: &std::path::Path) -> bool {
        // Need repo root (git_root) and git_status_cache
        let repo_root = match self.git_root.as_ref() {
            Some(root) => root,
            None => return false,
        };
        let cache = match self.git_status_cache.as_ref() {
            Some(cache) => cache,
            None => return false,
        };

        // Convert absolute path to repo-relative
        let relative_path = match absolute_path.strip_prefix(repo_root) {
            Ok(rel) => rel,
            Err(_) => return false,
        };

        // Check if this relative path is ignored
        cache.is_path_in_ignored(relative_path)
    }

    /// Take the watched root (for cleanup when closing)
    pub fn take_watched_root(&mut self) -> Option<PathBuf> {
        self.git_root.take()
    }

    /// Navigate to a specific directory
    pub fn navigate_to(&mut self, path: PathBuf) -> Result<()> {
        if path.is_dir() {
            self.current_path = path.clone();
            self.vfs.set_path(termide_vfs::VfsPath::local(path));
            self.load_directory()
        } else if let Some(parent) = path.parent() {
            // If path is a file, navigate to its parent directory
            self.current_path = parent.to_path_buf();
            self.vfs.set_path(termide_vfs::VfsPath::local(parent));
            self.load_directory()
        } else {
            Ok(())
        }
    }

    /// Navigate to a VFS URL (supports both local and remote paths).
    ///
    /// Examples:
    /// - `/home/user/documents` - local path
    /// - `sftp://user@host/path` - SFTP remote path
    /// - `ftp://host/path` - FTP remote path
    pub fn navigate_to_url(&mut self, url: &str) -> Result<()> {
        let vfs_path =
            termide_vfs::parse_vfs_url(url).map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;

        if vfs_path.is_local() {
            // Local path - use existing navigation
            self.navigate_to(vfs_path.path)
        } else {
            // Remote path - update VFS state and trigger connection/listing
            self.vfs
                .navigate_to(vfs_path.clone())
                .map_err(|e| anyhow::anyhow!("VFS navigation failed: {}", e))?;

            // If already connected, start listing (otherwise connection will trigger it)
            if !self.vfs.is_connecting() && !self.vfs.has_pending_operation() {
                self.vfs.start_list_dir();
            }

            // Don't update current_path yet - wait for listing to complete
            // The path will be synced when tick() succeeds
            Ok(())
        }
    }

    /// Get reference to VFS state (for network filesystem operations).
    pub fn vfs_state(&self) -> &VfsState {
        &self.vfs
    }

    /// Get mutable reference to VFS state.
    pub fn vfs_state_mut(&mut self) -> &mut VfsState {
        &mut self.vfs
    }

    /// Check if current path is a remote (network) filesystem.
    pub fn is_remote(&self) -> bool {
        self.vfs.is_remote()
    }

    /// Get display path (includes protocol for remote paths).
    pub fn display_path(&self) -> String {
        self.vfs.display_path()
    }

    /// Load the contents of the current directory
    pub fn load_directory(&mut self) -> Result<()> {
        // Preserve git_root when navigating within the same repo —
        // clearing it breaks OnGitUpdate/OnFsUpdate handlers.
        // Only clear when leaving the repo (navigate_to() handles re-registration).
        if let Some(ref root) = self.git_root {
            if !self.current_path.starts_with(root) {
                self.git_root = None;
            }
        }

        // Update debounce timestamp to prevent rapid subsequent reloads from being skipped
        self.navigation.last_reload_time = Some(std::time::Instant::now());

        self.load_directory_inner(false)
    }

    /// Force directory reload, bypassing debounce
    pub fn force_reload_directory(&mut self) -> Result<()> {
        // Preserve git_root within the same repo (same as load_directory)
        if let Some(ref root) = self.git_root {
            if !self.current_path.starts_with(root) {
                self.git_root = None;
            }
        }
        // Clear last_reload_time to bypass debounce
        self.navigation.last_reload_time = None;

        // For remote paths, invalidate cache and start async listing
        if self.vfs.is_remote() {
            self.vfs.invalidate_cache();
            self.vfs.start_list_dir();
            // Entries will be populated by tick() when VFS operation completes
            Ok(())
        } else {
            // Local paths can reload synchronously
            self.load_directory_inner(false)
        }
    }

    /// Navigate to a specific file - opens its parent directory and selects the file
    pub fn navigate_to_file(&mut self, path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            self.current_path = parent.to_path_buf();
            let _ = self.load_directory();

            // Find and select the file in the list
            if let Some(file_name) = path.file_name() {
                let name_str = file_name.to_string_lossy();
                if let Some(idx) = self.find_entry_index(&name_str) {
                    self.selected = idx;
                    self.adjust_scroll_offset(self.visible_height);
                }
            }
        }
    }

    /// Select an entry by name in the current directory
    pub fn select_by_name(&mut self, name: &std::ffi::OsStr) {
        let name_str = name.to_string_lossy();
        if let Some(idx) = self.find_entry_index(&name_str) {
            self.selected = idx;
            self.adjust_scroll_offset(self.visible_height);
        }
    }

    /// Reload directory preserving selection (with debounce to prevent rapid reloads)
    pub fn reload_directory(&mut self) -> Result<()> {
        const RELOAD_DEBOUNCE_MS: u128 = 300;

        // Debounce: skip if last reload was too recent
        if !self.navigation.should_reload(RELOAD_DEBOUNCE_MS) {
            return Ok(());
        }

        // For remote paths, invalidate cache and start async listing
        // Entries will be populated by tick() when VFS operation completes
        if self.vfs.is_remote() {
            self.vfs.invalidate_cache();
            self.vfs.start_list_dir();
            return Ok(());
        }

        self.load_directory_inner(true)
    }

    /// Build top-level `tree_entries` from a sorted list of `FileEntry`.
    fn build_top_level_tree(&self, entries: Vec<FileEntry>) -> Vec<tree::TreeEntry> {
        entries
            .into_iter()
            .map(|fe| {
                let full_path = if fe.name == ".." {
                    self.current_path
                        .parent()
                        .unwrap_or(&self.current_path)
                        .to_path_buf()
                } else {
                    self.current_path.join(&fe.name)
                };
                let expanded = if fe.is_dir && fe.name != ".." {
                    let is_expanded = self.expanded_dirs.contains(&full_path);
                    Some(is_expanded)
                } else {
                    None
                };
                tree::TreeEntry {
                    file_entry: fe,
                    full_path,
                    depth: 0,
                    expanded,
                }
            })
            .collect()
    }

    /// Read entries from a directory and return sorted `FileEntry` list.
    /// Does NOT add ".." entry — caller handles that.
    fn read_dir_entries(
        &self,
        dir_path: &std::path::Path,
        rel_prefix: &str,
    ) -> Result<Vec<FileEntry>> {
        let mut entries = Vec::new();

        if let Ok(read_dir) = fs::read_dir(dir_path) {
            for entry in read_dir.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    let name = entry.file_name().to_string_lossy().into_owned();

                    if !self.show_hidden && name.starts_with('.') {
                        continue;
                    }

                    let is_symlink = if let Ok(link_metadata) = fs::symlink_metadata(entry.path()) {
                        link_metadata.is_symlink()
                    } else {
                        false
                    };

                    let is_dir = if is_symlink {
                        fs::metadata(entry.path())
                            .map(|m| m.is_dir())
                            .unwrap_or(false)
                    } else {
                        metadata.is_dir()
                    };

                    // Build git-relative name: for top-level entries just the name,
                    // for nested entries: "subdir/name"
                    let git_name = if rel_prefix.is_empty() {
                        name.clone()
                    } else {
                        format!("{rel_prefix}/{name}")
                    };

                    let git_status = if is_dir {
                        self.git_status_cache
                            .as_ref()
                            .map(|cache| cache.get_directory_status(&git_name))
                            .unwrap_or(GitStatus::Unmodified)
                    } else {
                        self.git_status_cache
                            .as_ref()
                            .map(|cache| cache.get_status(&git_name))
                            .unwrap_or(GitStatus::Unmodified)
                    };

                    #[cfg(unix)]
                    let is_executable = {
                        use std::os::unix::fs::PermissionsExt;
                        metadata.permissions().mode() & 0o111 != 0
                    };
                    #[cfg(not(unix))]
                    let is_executable = false;

                    #[cfg(unix)]
                    let is_readonly = {
                        use std::os::unix::fs::PermissionsExt;
                        let mode = metadata.permissions().mode();
                        (mode & 0o200) == 0
                    };
                    #[cfg(not(unix))]
                    let is_readonly = metadata.permissions().readonly();

                    let size = if metadata.is_file() {
                        Some(metadata.len())
                    } else {
                        None
                    };
                    let modified = metadata.modified().ok();

                    entries.push(FileEntry {
                        name,
                        is_dir,
                        is_symlink,
                        is_executable,
                        is_readonly,
                        git_status,
                        size,
                        modified,
                    });
                }
            }
        } else {
            log::warn!("Failed to read directory: {}", dir_path.display());
        }

        sort_entries(&mut entries);
        Ok(entries)
    }

    /// Restore cursor position after entries reload.
    /// Priority: newly created item → navigating down → restore by name → fallback to index.
    fn restore_cursor(
        &mut self,
        current_name: Option<String>,
        previous_index: usize,
        previous_scroll_offset: usize,
    ) {
        let count = self.visible_count();
        if let Some(created_name) = self.navigation.take_newly_created() {
            if let Some(idx) = self.find_entry_index(&created_name) {
                self.selected = idx;
                if self.visible_height > 0 {
                    self.adjust_scroll_offset(self.visible_height);
                }
            } else if count > 0 {
                self.selected = previous_index.min(count - 1);
            }
        } else if self.navigation.check_and_reset_navigating_down() {
            self.selected = 0;
            self.scroll_offset = 0;
        } else if let Some(name) = current_name {
            if let Some(pos) = self.find_entry_index(&name) {
                self.selected = pos;
            } else if count > 0 {
                self.selected = previous_index.min(count - 1);
            }
            if self.visible_height > 0 {
                if count <= self.visible_height {
                    self.scroll_offset = 0;
                } else {
                    let max_scroll = count.saturating_sub(self.visible_height);
                    self.scroll_offset = previous_scroll_offset.min(max_scroll);
                }
                self.adjust_scroll_offset(self.visible_height);
            }
        }
    }

    /// Expand a directory at the given visible index, loading children lazily.
    pub(crate) fn expand_dir(&mut self, vis_idx: usize) {
        let tree_idx = match self.visible_indices.get(vis_idx) {
            Some(&idx) => idx,
            None => return,
        };
        if self.tree_entries[tree_idx].expanded != Some(false) {
            return; // not a collapsed dir
        }
        let dir_path = self.tree_entries[tree_idx].full_path.clone();
        let depth = self.tree_entries[tree_idx].depth;

        // Mark as expanded
        self.tree_entries[tree_idx].expanded = Some(true);
        self.expanded_dirs.insert(dir_path.clone());

        // Check if children are already loaded (next entry has depth > current)
        let next_idx = tree_idx + 1;
        let already_loaded =
            next_idx < self.tree_entries.len() && self.tree_entries[next_idx].depth > depth;

        if !already_loaded {
            // Load children from filesystem
            let rel_prefix = dir_path
                .strip_prefix(&self.current_path)
                .ok()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_string();
            if let Ok(children) = self.read_dir_entries(&dir_path, &rel_prefix) {
                let child_depth = depth + 1;
                let child_tree_entries: Vec<tree::TreeEntry> = children
                    .into_iter()
                    .map(|fe| {
                        let full_path = dir_path.join(&fe.name);
                        let expanded = if fe.is_dir {
                            let is_exp = self.expanded_dirs.contains(&full_path);
                            Some(is_exp)
                        } else {
                            None
                        };
                        tree::TreeEntry {
                            file_entry: fe,
                            full_path,
                            depth: child_depth,
                            expanded,
                        }
                    })
                    .collect();

                // Check if this directory was selected before expansion
                let dir_was_selected = self.selection.items.contains(&vis_idx);
                // Save selection by path before insert (indices will shift)
                let saved = self.save_selection_paths();
                let n = child_tree_entries.len();
                // Insert children after parent
                self.tree_entries
                    .splice(next_idx..next_idx, child_tree_entries);
                // Recursively expand any subdirs that were previously expanded
                self.expand_previously_expanded(next_idx, n);
                // Rebuild visible and restore selection
                self.recompute_visible();
                self.restore_selection_by_paths(&saved);
                // Cascade selection to descendants if directory was selected
                if dir_was_selected {
                    self.select_descendants(vis_idx);
                }
                return;
            }
        }

        // Already loaded — just toggle visibility
        let dir_was_selected = self.selection.items.contains(&vis_idx);
        let saved = self.save_selection_paths();
        self.recompute_visible();
        self.restore_selection_by_paths(&saved);
        if dir_was_selected {
            self.select_descendants(vis_idx);
        }
    }

    /// After inserting children, check if any subdirectories should be expanded
    /// (because they are in `expanded_dirs`).
    fn expand_previously_expanded(&mut self, start: usize, count: usize) {
        let mut i = start;
        let end = start + count;
        while i < end.min(self.tree_entries.len()) {
            if self.tree_entries[i].expanded == Some(true) {
                let dir_path = self.tree_entries[i].full_path.clone();
                let child_depth = self.tree_entries[i].depth + 1;
                let rel_prefix = dir_path
                    .strip_prefix(&self.current_path)
                    .ok()
                    .and_then(|p| p.to_str())
                    .unwrap_or("")
                    .to_string();
                if let Ok(children) = self.read_dir_entries(&dir_path, &rel_prefix) {
                    let child_entries: Vec<tree::TreeEntry> = children
                        .into_iter()
                        .map(|fe| {
                            let full_path = dir_path.join(&fe.name);
                            let expanded = if fe.is_dir {
                                let is_exp = self.expanded_dirs.contains(&full_path);
                                Some(is_exp)
                            } else {
                                None
                            };
                            tree::TreeEntry {
                                file_entry: fe,
                                full_path,
                                depth: child_depth,
                                expanded,
                            }
                        })
                        .collect();
                    let n = child_entries.len();
                    let insert_at = i + 1;
                    self.tree_entries
                        .splice(insert_at..insert_at, child_entries);
                    // Recurse into newly inserted range
                    self.expand_previously_expanded(insert_at, n);
                }
            }
            // Skip over any children we just inserted (they have depth > current)
            let current_depth = self.tree_entries[i].depth;
            i += 1;
            while i < self.tree_entries.len() && self.tree_entries[i].depth > current_depth {
                i += 1;
            }
        }
    }

    /// Collapse a directory at the given visible index.
    pub(crate) fn collapse_dir(&mut self, vis_idx: usize) {
        let tree_idx = match self.visible_indices.get(vis_idx) {
            Some(&idx) => idx,
            None => return,
        };
        if self.tree_entries[tree_idx].expanded != Some(true) {
            return; // not an expanded dir
        }

        // Mark as collapsed (children stay in tree_entries, just hidden by visibility)
        self.tree_entries[tree_idx].expanded = Some(false);
        self.expanded_dirs
            .remove(&self.tree_entries[tree_idx].full_path);

        let saved = self.save_selection_paths();
        self.recompute_visible();
        self.restore_selection_by_paths(&saved);
    }

    /// Toggle expand/collapse for a directory at the given visible index.
    pub(crate) fn toggle_expand(&mut self, vis_idx: usize) {
        let tree_idx = match self.visible_indices.get(vis_idx) {
            Some(&idx) => idx,
            None => return,
        };
        match self.tree_entries[tree_idx].expanded {
            Some(true) => self.collapse_dir(vis_idx),
            Some(false) => self.expand_dir(vis_idx),
            None => {} // not a directory
        }
    }

    /// Jump cursor to the parent directory node in the tree.
    /// Used when pressing Left on a non-directory or on a child of an expanded dir.
    fn jump_to_parent_dir(&mut self) {
        let tree_idx = match self.visible_indices.get(self.selected) {
            Some(&idx) => idx,
            None => return,
        };
        let current_depth = self.tree_entries[tree_idx].depth;
        if current_depth == 0 {
            return;
        }
        // Walk backwards in visible_indices to find the parent (first entry with depth < current)
        for vis_idx in (0..self.selected).rev() {
            let ti = self.visible_indices[vis_idx];
            if self.tree_entries[ti].depth < current_depth {
                self.selected = vis_idx;
                self.adjust_scroll_offset(self.visible_height);
                return;
            }
        }
    }

    /// Save selection as set of paths (survives tree rebuilds).
    fn save_selection_paths(&self) -> HashSet<PathBuf> {
        self.selection
            .items
            .iter()
            .filter_map(|&vis_idx| self.path_at(vis_idx).cloned())
            .collect()
    }

    /// Restore selection from saved paths after tree rebuild.
    fn restore_selection_by_paths(&mut self, saved: &HashSet<PathBuf>) {
        self.selection.items.clear();
        for (vis_idx, &tree_idx) in self.visible_indices.iter().enumerate() {
            if saved.contains(&self.tree_entries[tree_idx].full_path) {
                self.selection.items.insert(vis_idx);
            }
        }
    }

    /// Update entries from VFS directory listing (for remote directories).
    fn update_entries_from_vfs(&mut self, vfs_entries: Vec<VfsEntry>) {
        let previous_index = self.selected;
        let previous_scroll_offset = self.scroll_offset;
        let current_name = self.entry_at(self.selected).map(|e| e.name.clone());

        self.tree_entries.clear();
        self.selected = 0;
        self.scroll_offset = 0;
        self.selection.clear();

        let mut entries = Vec::new();

        // Add ".." entry for parent directory navigation (unless at root)
        if self.vfs.current_path().parent().is_some() {
            entries.push(FileEntry {
                name: "..".to_string(),
                is_dir: true,
                is_symlink: false,
                is_executable: false,
                is_readonly: false,
                git_status: GitStatus::Unmodified,
                size: None,
                modified: None,
            });
        }

        // Convert and add VFS entries
        let mut file_entries: Vec<FileEntry> = vfs_entries
            .into_iter()
            .map(FileEntry::from_vfs_entry)
            .filter(|e| self.show_hidden || !e.name.starts_with('.'))
            .collect();

        sort_entries(&mut file_entries);
        entries.extend(file_entries);

        self.tree_entries = self.build_top_level_tree(entries);
        self.recompute_visible();

        // Clear git status (not applicable for remote files)
        self.git_status_cache = None;
        self.git_root = None;

        self.restore_cursor(current_name, previous_index, previous_scroll_offset);
    }

    /// Internal method to load directory with optional selection preservation
    fn load_directory_inner(&mut self, preserve_selection: bool) -> Result<()> {
        // Sync VFS path with current_path for local paths
        if !self.vfs.is_remote() {
            self.vfs
                .set_path(termide_vfs::VfsPath::local(self.current_path.clone()));
        }

        // For remote paths, don't clear entries - keep showing current content while loading
        if self.vfs.is_remote() {
            log::debug!("load_directory_inner: Starting async list for remote path (keeping current entries)");
            self.vfs.invalidate_cache();
            self.vfs.start_list_dir();
            return Ok(());
        }

        // Save current file name and index to restore position
        let current_name = self
            .navigation
            .take_previous_dir_name()
            .or_else(|| self.entry_at(self.selected).map(|e| e.name.clone()));
        let previous_index = self.selected;
        let previous_scroll_offset = self.scroll_offset;

        // Save names of selected files if we need to restore selection
        let selected_names: HashSet<String> = if preserve_selection {
            self.selection
                .items
                .iter()
                .filter_map(|&vis_idx| self.entry_at(vis_idx).map(|e| e.name.clone()))
                .collect()
        } else {
            HashSet::new()
        };

        self.tree_entries.clear();
        self.selected = 0;
        self.scroll_offset = 0;
        self.selection.clear();
        self.selection.end_drag();

        // Start async git status loading (non-blocking)
        self.git_status_cache = None;
        self.git_status_receiver = Some(get_git_status_async(self.current_path.clone()));

        // Build entries list
        let mut entries = Vec::new();

        // Add parent directory if not at root
        if self.current_path.parent().is_some() {
            entries.push(FileEntry {
                name: "..".to_string(),
                is_dir: true,
                is_symlink: false,
                is_executable: false,
                is_readonly: false,
                git_status: GitStatus::Unmodified,
                size: None,
                modified: None,
            });
        }

        // Read directory contents
        let mut dir_entries = self.read_dir_entries(&self.current_path, "")?;
        entries.append(&mut dir_entries);

        // Build tree (top-level, respecting previously expanded dirs)
        self.tree_entries = self.build_top_level_tree(entries);

        // Expand any directories that were previously expanded
        self.load_expanded_subtrees();

        self.recompute_visible();

        // Restore selection by file names
        if !selected_names.is_empty() {
            for (vis_idx, &tree_idx) in self.visible_indices.iter().enumerate() {
                if selected_names.contains(&self.tree_entries[tree_idx].file_entry.name) {
                    self.selection.select(vis_idx);
                }
            }
        }

        self.restore_cursor(current_name, previous_index, previous_scroll_offset);

        Ok(())
    }

    /// After building top-level tree, load children for any expanded directories.
    fn load_expanded_subtrees(&mut self) {
        let mut i = 0;
        while i < self.tree_entries.len() {
            if self.tree_entries[i].expanded == Some(true) {
                let dir_path = self.tree_entries[i].full_path.clone();
                let child_depth = self.tree_entries[i].depth + 1;
                let rel_prefix = dir_path
                    .strip_prefix(&self.current_path)
                    .ok()
                    .and_then(|p| p.to_str())
                    .unwrap_or("")
                    .to_string();
                if let Ok(children) = self.read_dir_entries(&dir_path, &rel_prefix) {
                    let child_entries: Vec<tree::TreeEntry> = children
                        .into_iter()
                        .map(|fe| {
                            let full_path = dir_path.join(&fe.name);
                            let expanded = if fe.is_dir {
                                let is_exp = self.expanded_dirs.contains(&full_path);
                                Some(is_exp)
                            } else {
                                None
                            };
                            tree::TreeEntry {
                                file_entry: fe,
                                full_path,
                                depth: child_depth,
                                expanded,
                            }
                        })
                        .collect();
                    let insert_at = i + 1;
                    self.tree_entries
                        .splice(insert_at..insert_at, child_entries);
                    // Don't increment i — continue to process the newly inserted children
                    // (they may also need expansion)
                }
            }
            i += 1;
        }
    }

    /// Get current directory path
    pub fn current_path(&self) -> &std::path::Path {
        &self.current_path
    }

    /// Format file size in human-readable format (public method for external use)
    pub fn format_size_static(bytes: u64) -> String {
        utils::format_size(bytes)
    }
}

impl Panel for FileManager {
    fn name(&self) -> &'static str {
        "file_manager"
    }

    fn title(&self) -> String {
        // Return full path, let smart_truncate_title() handle truncation
        // Use VFS display path for remote paths (includes protocol)
        let path = if self.is_remote() {
            self.display_path()
        } else {
            termide_core::util::shorten_home_path(&self.current_path.display().to_string())
        };

        // Show spinner for VFS loading or git status loading
        if self.vfs.is_loading() {
            let spinner = constants::spinner_frame();
            format!("{} {}", spinner, path)
        } else if self.is_git_status_loading() {
            let spinner = constants::spinner_frame();
            format!("{} {} (git)", spinner, path)
        } else {
            path
        }
    }

    fn prepare_render(&mut self, theme: &termide_theme::Theme, config: std::sync::Arc<Config>) {
        self.cached_theme = *theme;
        self.cached_config = config.file_manager.clone();
        self.vim_mode = config.general.vim_mode;
        self.cached_vfs_timeout_secs = config.vfs.connection_timeout_secs;
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
        let content_height = area.height as usize;
        self.visible_height = content_height;

        // If file search is active, render search results instead of normal tree
        if let Some(ref search) = self.file_search {
            self.render_search_results(area, buf, search, &self.cached_theme);
            return;
        }

        if self.selected >= self.scroll_offset + content_height {
            self.scroll_offset = self.selected - content_height + 1;
        } else if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        }

        // Calculate available width for file names
        let content_width = area.width as usize;
        let items = self.get_items(
            content_height,
            content_width,
            &self.cached_theme,
            ctx.is_focused,
            &self.cached_config,
        );

        // Render file list content directly (accordion already drew border with title/buttons)
        let paragraph = Paragraph::new(items);

        paragraph.render(area, buf);

        // Render scrollbar on the right border
        if let Some(border_x) = ctx.border_right_x {
            let theme_colors = termide_core::ThemeColors::from(&self.cached_theme);
            ScrollBar::render(
                buf,
                border_x,
                area.y,
                area.height,
                self.scroll_offset,
                content_height,
                self.visible_count(),
                &theme_colors,
                ctx.is_focused,
            );
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        use keyboard::FmCommand;

        // Translate Cyrillic to Latin for hotkeys (Ctrl/Alt+key and special normalization)
        let key = termide_keyboard::translate_hotkey(key);
        // Also translate bare Cyrillic characters (panel has no text input at this level)
        let key = termide_keyboard::translate_all_chars(key);

        // Parse key event to command
        let command =
            FmCommand::from_key_event(key, &self.cached_config.keybindings, self.vim_mode);

        // Execute command
        self.execute_command(command)
    }

    fn handle_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        panel_area: Rect,
    ) -> Vec<PanelEvent> {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

        // Handle scroll first (works anywhere in panel)
        let visible_height = panel_area.height.saturating_sub(2) as usize;
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.scroll_offset = self.scroll_offset.saturating_sub(3);
                // Keep selected in visible area so render doesn't reset scroll
                if self.selected >= self.scroll_offset + visible_height {
                    self.selected = (self.scroll_offset + visible_height).saturating_sub(1);
                }
                return vec![];
            }
            MouseEventKind::ScrollDown => {
                let max_scroll = self.visible_count().saturating_sub(visible_height);
                self.scroll_offset = (self.scroll_offset + 3).min(max_scroll);
                // Keep selected in visible area so render doesn't reset scroll
                if self.selected < self.scroll_offset {
                    self.selected = self.scroll_offset;
                }
                return vec![];
            }
            MouseEventKind::Up(MouseButton::Left) => {
                // End drag - handle this ALWAYS, even if outside panel
                self.selection.end_drag();
                return vec![];
            }
            _ => {}
        }

        // Check that click is inside content area (not on borders)
        let inner_area = Rect {
            x: panel_area.x + 1,
            y: panel_area.y + 1,
            width: panel_area.width.saturating_sub(2),
            height: panel_area.height.saturating_sub(2),
        };

        // Check that click is inside inner area
        if mouse.column < inner_area.x
            || mouse.column >= inner_area.x + inner_area.width
            || mouse.row < inner_area.y
            || mouse.row >= inner_area.y + inner_area.height
        {
            return vec![];
        }

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Determine index of clicked item
                let relative_row = (mouse.row - inner_area.y) as usize;
                let clicked_index = self.scroll_offset + relative_row;

                if clicked_index < self.visible_count() {
                    // Check modifiers
                    if mouse.modifiers.contains(KeyModifiers::SHIFT) {
                        // Shift+click - select range from selected to clicked_index
                        let start = self.selected.min(clicked_index);
                        let end = self.selected.max(clicked_index);
                        self.selection.dragged.clear();
                        for i in start..=end {
                            self.selection.select(i);
                            self.selection.dragged.insert(i);
                        }
                        self.selected = clicked_index;
                        self.selection.drag_start = Some(clicked_index);
                        self.selection.start_shift_drag(clicked_index);
                    } else if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                        // Ctrl+click - toggle selection on clicked element
                        self.selection.toggle(clicked_index);
                        self.selected = clicked_index;
                        self.selection.start_ctrl_drag(clicked_index);
                    } else {
                        // Check if click is on the expand/collapse icon area for directories
                        let relative_col = (mouse.column - inner_area.x) as usize;
                        let is_dir_icon_click = if let Some(te) = self.tree_entry_at(clicked_index)
                        {
                            let prefix_width = self
                                .tree_prefixes
                                .get(clicked_index)
                                .map(|p| unicode_width::UnicodeWidthStr::width(p.as_str()))
                                .unwrap_or(0);
                            // Icon is at prefix_width + 1 (attr char) position
                            te.expanded.is_some() && relative_col <= prefix_width + 1
                        } else {
                            false
                        };

                        if is_dir_icon_click {
                            // Click on ▶/▼ icon — toggle expand/collapse
                            self.selected = clicked_index;
                            self.toggle_expand(clicked_index);
                            self.click_tracker.reset();
                        } else {
                            // Check for double click using ClickTracker
                            let is_double_click =
                                self.click_tracker.is_double_click(&clicked_index);

                            if is_double_click {
                                // Double click - open file/directory
                                self.selected = clicked_index;
                                let event = self.enter();
                                self.click_tracker.reset();
                                if let Some(e) = event {
                                    return vec![e];
                                }
                            } else {
                                // Single click - select item
                                self.selected = clicked_index;
                                self.click_tracker.record(clicked_index);
                            }
                        }
                        self.selection.end_drag();
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                // Handle drag only if there's an active drag mode
                if self.selection.is_dragging() {
                    let relative_row = (mouse.row - inner_area.y) as usize;
                    let current_index = self.scroll_offset + relative_row;

                    if current_index < self.visible_count() {
                        // Process drag will select or toggle based on drag mode
                        self.selection.process_drag(current_index);
                        self.selected = current_index;
                    }
                }
            }
            _ => {}
        }

        vec![]
    }

    fn handle_scroll(&mut self, delta: i32, panel_area: Rect) -> Vec<PanelEvent> {
        let lines = delta.unsigned_abs() as usize * 3; // 3 lines per scroll unit
        let visible_height = panel_area.height.saturating_sub(2) as usize;

        if delta < 0 {
            // Scroll up
            self.scroll_offset = self.scroll_offset.saturating_sub(lines);
            // Keep selected in visible area
            if self.selected >= self.scroll_offset + visible_height {
                self.selected = (self.scroll_offset + visible_height).saturating_sub(1);
            }
        } else {
            // Scroll down
            let max_scroll = self.visible_count().saturating_sub(visible_height);
            self.scroll_offset = (self.scroll_offset + lines).min(max_scroll);
            // Keep selected in visible area
            if self.selected < self.scroll_offset {
                self.selected = self.scroll_offset;
            }
        }
        vec![]
    }

    fn reload(&mut self) -> anyhow::Result<()> {
        // Reload directory contents (preserving selection)
        self.reload_directory()
    }

    fn handle_command(&mut self, cmd: PanelCommand<'_>) -> CommandResult {
        match cmd {
            // Return git repository root (enables registration with watcher)
            PanelCommand::GetRepoRoot => CommandResult::RepoRoot(self.git_root.clone()),
            PanelCommand::GetFsWatchInfo => CommandResult::FsWatchInfo {
                watched_root: self.git_root.clone(),
                current_path: self.current_path.clone(),
                is_git_repo: self.git_root.is_some(),
            },
            PanelCommand::SetFsWatchRoot { root, is_git_repo } => {
                self.git_root = if is_git_repo { root } else { None };
                CommandResult::None
            }
            PanelCommand::OnFsUpdate { changed_path } => {
                let current = self.current_path();

                // For git repos: reload on any change within current directory tree
                // (needed for git status color updates)
                // For non-git dirs: reload only for direct children
                let should_reload = if self.git_root.is_some() {
                    // Git repo: any change within current directory tree updates git status
                    // But skip gitignored paths (like target/) to avoid unnecessary reloads
                    changed_path.starts_with(current) && !self.is_path_ignored(changed_path)
                } else {
                    // Non-git: only direct children or current dir itself
                    changed_path.parent() == Some(current) || changed_path == current
                };

                if should_reload {
                    let _ = self.reload_directory();
                    return CommandResult::NeedsRedraw(true);
                }
                CommandResult::NeedsRedraw(false)
            }
            PanelCommand::Reload | PanelCommand::RefreshDirectory => {
                if self.reload_directory().is_ok() {
                    CommandResult::NeedsRedraw(true)
                } else {
                    CommandResult::NeedsRedraw(false)
                }
            }
            // Handle git status updates from unified watcher
            PanelCommand::OnGitUpdate { repo_paths } => {
                // Check if current directory is within one of the updated repositories
                if let Some(git_root) = &self.git_root {
                    let should_update = repo_paths
                        .iter()
                        .any(|p| git_root.starts_with(p) || p.starts_with(git_root));
                    if should_update {
                        // Reload directory to pick up new/deleted files and git status
                        let _ = self.reload_directory();
                        return CommandResult::NeedsRedraw(true);
                    }
                }
                CommandResult::None
            }
            PanelCommand::MarkStale => {
                // Remote panels don't depend on local fs/git events — never mark stale
                if !self.vfs.is_remote() {
                    self.is_stale = true;
                    return CommandResult::NeedsRedraw(true);
                }
                CommandResult::None
            }
            PanelCommand::RefreshIfStale => {
                if self.is_stale {
                    self.is_stale = false;
                    let _ = self.reload_directory();
                    // Only refresh git status for local panels
                    if !self.vfs.is_remote() {
                        self.refresh_git_status();
                    }
                    CommandResult::NeedsRedraw(true)
                } else {
                    CommandResult::None
                }
            }
            // Commands not applicable to FileManager
            PanelCommand::CheckPendingGitDiff
            | PanelCommand::CheckGitDiffReceiver
            | PanelCommand::CheckExternalModification
            | PanelCommand::Resize { .. }
            | PanelCommand::GetModificationStatus
            | PanelCommand::Save
            | PanelCommand::CloseWithoutSaving
            | PanelCommand::SetGitOperationInProgress { .. }
            | PanelCommand::UpdateRepoPaths { .. }
            | PanelCommand::Paste
            | PanelCommand::PasteText { .. } => CommandResult::None,
        }
    }

    fn needs_close_confirmation(&self) -> Option<String> {
        // FileManager doesn't store critical state by itself
        // Pending batch operations are checked in has_panels_requiring_confirmation()
        None
    }

    fn captures_escape(&self) -> bool {
        // Capture Escape when there's a pending VFS operation (e.g., connecting to remote)
        // This prevents the global handler from closing the panel
        self.vfs.has_pending_operation()
    }

    fn tick(&mut self) -> Vec<PanelEvent> {
        // --- Always drain async results (even when stale/collapsed) ---
        // VFS and git status receivers must be consumed to prevent stuck spinners.
        // IMPORTANT: never early-return before vfs.tick() — results must always be drained.

        let mut events = Vec::new();

        // Check for VFS connection timeout (cancel stuck connections)
        if let Some((status, Some(secs))) = self.vfs.connection_status_with_elapsed() {
            if secs >= self.cached_vfs_timeout_secs {
                log::warn!("VFS connection timeout after {}s", secs);
                if self.vfs.cancel_pending().is_some() {
                    self.current_path = self.vfs.path_buf();
                    let _ = self.load_directory();
                    if !self.is_stale {
                        let t = termide_i18n::t();
                        self.show_info_modal(
                            t.connection_timeout_title(),
                            t.connection_timeout_message(),
                        );
                        events.push(PanelEvent::ClearStatus);
                        events.push(PanelEvent::NeedsRedraw);
                        return events;
                    }
                }
            } else if !self.is_stale {
                // Show connection progress in status bar (no early return — must reach vfs.tick)
                events.push(PanelEvent::ShowMessage(format!("{} {}s", status, secs)));
            }
        }

        // Poll VFS operations for completion
        if let Some(result) = self.vfs.tick() {
            match result {
                Ok(entries) => {
                    self.current_path = self.vfs.path_buf();
                    self.update_entries_from_vfs(entries);
                }
                Err(e) => {
                    log::error!("VFS operation failed: {}", e);
                    self.current_path = self.vfs.path_buf();
                    let _ = self.load_directory();
                    if !self.is_stale {
                        let t = termide_i18n::t();
                        self.show_info_modal(t.connection_error_title(), &format!("{}", e));
                    }
                }
            }
            if !self.is_stale {
                events.push(PanelEvent::ClearStatus);
                events.push(PanelEvent::NeedsRedraw);
                return events;
            }
        }

        // Drain git status receiver — redraw if statuses changed
        if self.check_git_status_async() && !self.is_stale {
            events.push(PanelEvent::NeedsRedraw);
        }

        // Poll file search results
        if let Some(ref mut search) = self.file_search {
            if search.poll_results() {
                events.push(PanelEvent::NeedsRedraw);
            }
        }

        // Skip remaining work when collapsed (stale)
        if self.is_stale {
            return vec![];
        }

        events
    }

    fn to_session(&self, _session_dir: &std::path::Path) -> Option<SessionPanel> {
        // Save file manager with current directory path or VFS URL
        let path_or_url = self.display_path(); // Returns VFS URL for remote, local path for local

        // Defensive check: ensure remote paths include protocol
        if self.is_remote() && !path_or_url.contains("://") {
            log::warn!(
                "Session save WARNING: Remote path missing protocol. VfsPath details: protocol={:?}, host={:?}, path={:?}",
                self.vfs.current_path().protocol,
                self.vfs.current_path().host,
                self.vfs.current_path().path
            );

            // Try to reconstruct the URL manually
            let vfs_path = self.vfs.current_path();
            let reconstructed = if vfs_path.protocol.is_remote() {
                let mut url = format!("{}://", vfs_path.protocol.scheme());
                if let Some(ref user) = vfs_path.username {
                    url.push_str(user);
                    url.push('@');
                }
                if let Some(ref host) = vfs_path.host {
                    url.push_str(host);
                }
                if let Some(port) = vfs_path.port {
                    url.push(':');
                    url.push_str(&port.to_string());
                }
                url.push_str(&vfs_path.path.display().to_string());
                log::info!("Reconstructed URL: {}", url);
                url
            } else {
                log::error!("VfsPath.protocol is not remote but is_remote() returned true!");
                path_or_url
            };

            Some(SessionPanel::FileManager {
                path_or_url: reconstructed,
            })
        } else {
            log::debug!(
                "Session save: Saving path '{}' (is_remote={})",
                path_or_url,
                self.is_remote()
            );

            Some(SessionPanel::FileManager { path_or_url })
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn get_working_directory(&self) -> Option<PathBuf> {
        Some(self.current_path.clone())
    }

    fn get_working_directory_display(&self) -> Option<String> {
        // For remote paths, return the full URL; for local paths, return the path string
        Some(self.display_path())
    }
}

// Additional methods used by app layer (not part of Panel trait)
impl FileManager {
    /// Take modal window request (if any).
    pub fn take_modal_request(&mut self) -> Option<(PendingAction, ActiveModal)> {
        self.modal_request.take()
    }

    /// Set newly created item name for cursor navigation after reload
    pub fn set_newly_created(&mut self, name: String) {
        self.navigation.set_newly_created(name);
    }

    /// Show an information modal with a message and OK button.
    fn show_info_modal(&mut self, title: &str, message: &str) {
        let t = termide_i18n::t();
        let modal = InfoActionModal::new(
            title,
            vec![("".to_string(), message.to_string())],
            vec![ActionButton::new(t.modal_ok(), "ok")],
        );
        self.modal_request = Some((
            PendingAction::VfsMessage,
            ActiveModal::InfoAction(Box::new(modal)),
        ));
    }

    /// Execute a file manager command and return resulting events.
    fn execute_command(&mut self, command: keyboard::FmCommand) -> Vec<PanelEvent> {
        use keyboard::FmCommand;

        let mut events = Vec::new();

        match command {
            // Navigation
            FmCommand::MoveUp => self.move_up(),
            FmCommand::MoveDown => self.move_down(),
            FmCommand::PageUp => {
                self.selected = self.selected.saturating_sub(self.visible_height);
            }
            FmCommand::PageDown => {
                let max_index = self.visible_count().saturating_sub(1);
                self.selected = (self.selected + self.visible_height).min(max_index);
            }
            FmCommand::GoHome => {
                self.selected = 0;
                self.scroll_offset = 0;
            }
            FmCommand::GoEnd => {
                self.selected = self.visible_count().saturating_sub(1);
            }
            FmCommand::Enter => {
                if let Some(event) = self.enter() {
                    events.push(event);
                }
            }
            FmCommand::GoParent => {
                // Use VfsState for navigation (works for both local and remote paths)
                // navigate_up returns None if already at root - don't refresh in that case
                if let Some(dir_name) = self.vfs.navigate_up() {
                    self.navigation.save_for_going_up(dir_name);
                    // Sync local path with VfsState
                    self.current_path = self.vfs.path_buf();
                    let _ = self.load_directory();
                }
            }
            FmCommand::GoHomeDir => {
                if let Some(home) = dirs::home_dir() {
                    self.current_path = home;
                    let _ = self.load_directory();
                }
            }

            // Selection
            FmCommand::ToggleSelection => {
                self.toggle_selection();
            }
            FmCommand::SelectAll => self.select_all(),
            FmCommand::ClearSelection => {
                // If there's a pending VFS operation, cancel it instead of clearing selection
                if self.vfs.has_pending_operation() {
                    if let Some(message) = self.vfs.cancel_pending() {
                        // Sync FileManager path with VfsState
                        self.current_path = self.vfs.path_buf();
                        let _ = self.load_directory();
                        // Show cancellation modal
                        let t = termide_i18n::t();
                        self.show_info_modal(t.connection_cancelled_title(), &message);
                        events.push(PanelEvent::ClearStatus);
                    }
                } else {
                    self.selection.clear();
                }
            }
            FmCommand::CancelOperation => {
                // Explicitly cancel pending VFS operation
                if let Some(message) = self.vfs.cancel_pending() {
                    // Sync FileManager path with VfsState
                    self.current_path = self.vfs.path_buf();
                    let _ = self.load_directory();
                    // Show cancellation modal
                    let t = termide_i18n::t();
                    self.show_info_modal(t.connection_cancelled_title(), &message);
                    events.push(PanelEvent::ClearStatus);
                }
            }
            FmCommand::MoveUpWithSelection => self.move_up_with_selection(),
            FmCommand::MoveDownWithSelection => self.move_down_with_selection(),
            FmCommand::PageUpWithSelection => self.page_up_with_selection(),
            FmCommand::PageDownWithSelection => self.page_down_with_selection(),
            FmCommand::SelectToHome => self.select_to_home(),
            FmCommand::SelectToEnd => self.select_to_end(),
            FmCommand::MoveUpWithToggle => self.move_up_with_toggle(),
            FmCommand::MoveDownWithToggle => self.move_down_with_toggle(),
            FmCommand::PageUpWithToggle => self.page_up_with_toggle(),
            FmCommand::PageDownWithToggle => self.page_down_with_toggle(),

            // File operations
            FmCommand::NewFile => {
                let t = termide_i18n::t();
                let modal = InputModal::new(t.modal_create_file_title(), "");
                let action = PendingAction::CreateFile {
                    directory: self.current_path.clone(),
                };
                self.modal_request = Some((action, ActiveModal::Input(Box::new(modal))));
            }
            FmCommand::NewDirectory => {
                let t = termide_i18n::t();
                let modal = InputModal::new(t.modal_create_dir_title(), "");
                let action = PendingAction::CreateDirectory {
                    directory: self.current_path.clone(),
                };
                self.modal_request = Some((action, ActiveModal::Input(Box::new(modal))));
            }
            FmCommand::DeleteFiles => {
                if self.is_remote() {
                    // Remote delete - use VfsPath
                    let vfs_paths = self.get_selected_vfs_paths();
                    if !vfs_paths.is_empty() {
                        let t = termide_i18n::t();
                        let title = if vfs_paths.len() == 1 {
                            let file_name = vfs_paths[0]
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "file".to_string());
                            t.modal_delete_single_title(&file_name)
                        } else {
                            t.modal_delete_multiple_title(vfs_paths.len())
                        };
                        let modal = ConfirmModal::new(&title, "");
                        let action = PendingAction::DeleteRemotePath {
                            paths: vfs_paths,
                            vfs_manager: self.vfs_manager_arc(),
                        };
                        self.modal_request = Some((action, ActiveModal::Confirm(Box::new(modal))));
                    }
                } else {
                    // Local delete - use PathBuf
                    let paths = self.get_selected_paths();
                    if !paths.is_empty() {
                        let t = termide_i18n::t();
                        let title = if paths.len() == 1 {
                            let file_name = path_utils::get_file_name_str(&paths[0]);
                            t.modal_delete_single_title(file_name)
                        } else {
                            t.modal_delete_multiple_title(paths.len())
                        };
                        let modal = ConfirmModal::new(&title, "");
                        let action = PendingAction::DeletePath { paths };
                        self.modal_request = Some((action, ActiveModal::Confirm(Box::new(modal))));
                    }
                }
            }
            FmCommand::CopyFiles => {
                let paths = self.get_selected_paths();
                if !paths.is_empty() {
                    let default_dest = format!("{}/", self.current_path.display());
                    let t = termide_i18n::t();
                    let message = if paths.len() == 1 {
                        let name = path_utils::get_file_name_str(&paths[0]);
                        t.fm_copy_prompt(name)
                    } else {
                        format!("Copy {} items to:", paths.len())
                    };
                    let modal = InputModal::with_default("Copy", &message, &default_dest);
                    let action = PendingAction::CopyPath {
                        sources: paths,
                        target_directory: None,
                        create_symlink: false,
                    };
                    self.modal_request = Some((action, ActiveModal::Input(Box::new(modal))));
                }
            }
            FmCommand::MoveFiles => {
                let paths = self.get_selected_paths();
                if !paths.is_empty() {
                    let t = termide_i18n::t();
                    let (message, default_dest) = if paths.len() == 1 {
                        let name = path_utils::get_file_name_str(&paths[0]);
                        (t.fm_move_prompt(name), name.to_string())
                    } else {
                        (
                            format!("Move {} items to:", paths.len()),
                            format!("{}/", self.current_path.display()),
                        )
                    };
                    let modal = InputModal::with_default("Move", &message, &default_dest);
                    let action = PendingAction::MovePath {
                        sources: paths,
                        target_directory: None,
                    };
                    self.modal_request = Some((action, ActiveModal::Input(Box::new(modal))));
                }
            }
            FmCommand::RenameFile => {
                if let Some(te) = self.tree_entry_at(self.selected) {
                    let entry = &te.file_entry;
                    // Only allow renaming files and directories (not deleted or special entries)
                    if entry.git_status == GitStatus::Deleted {
                        return events;
                    }
                    let path = te.full_path.clone();
                    let filename = entry.name.clone();
                    let t = termide_i18n::t();
                    let modal = InputModal::with_default(
                        t.op_type_rename(),
                        t.fm_move_prompt(&filename),
                        &filename,
                    );
                    let action = PendingAction::MovePath {
                        sources: vec![path.clone()],
                        target_directory: path.parent().map(|p| p.to_path_buf()),
                    };
                    self.modal_request = Some((action, ActiveModal::Input(Box::new(modal))));
                }
            }
            FmCommand::EditFile => {
                if let Some(event) = self.edit_file() {
                    events.push(event);
                }
            }
            FmCommand::ViewFile => {
                if let Some(event) = self.view_file() {
                    events.push(event);
                }
            }
            FmCommand::OpenExternal => {
                if let Some(event) = self.open_external() {
                    events.push(event);
                }
            }

            // Search
            FmCommand::Search => {
                events.push(PanelEvent::ShowSearch {
                    mode: termide_core::SearchMode::FileGlob,
                    initial_query: None,
                });
            }
            FmCommand::SearchContent => {
                events.push(PanelEvent::ShowSearch {
                    mode: termide_core::SearchMode::Content,
                    initial_query: None,
                });
            }

            // Clipboard
            FmCommand::ClipboardCopy => {
                let paths = self.get_selected_paths();
                if !paths.is_empty() {
                    let text = paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    let _ = clipboard::copy(&text);
                }
            }
            FmCommand::ClipboardCut => {
                let paths = self.get_selected_paths();
                if !paths.is_empty() {
                    let text = paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    let _ = clipboard::cut(&text);
                }
            }
            FmCommand::ClipboardPaste => {
                if let Some(text) = clipboard::paste() {
                    let files: Vec<std::path::PathBuf> = text
                        .lines()
                        .filter(|line| !line.is_empty())
                        .map(std::path::PathBuf::from)
                        .filter(|path| path.exists())
                        .collect();

                    if !files.is_empty() {
                        let t = termide_i18n::t();
                        let message = t.fm_paste_confirm(
                            files.len(),
                            "Copy",
                            &self.current_path.display().to_string(),
                        );
                        let action = PendingAction::CopyPath {
                            sources: files,
                            target_directory: Some(self.current_path.clone()),
                            create_symlink: false,
                        };
                        let modal =
                            ConfirmModal::new(termide_i18n::t().modal_confirm_title(), &message);
                        self.modal_request = Some((action, ActiveModal::Confirm(Box::new(modal))));
                    }
                }
            }

            // Misc
            FmCommand::ShowFileInfo => self.show_file_info(),
            FmCommand::Refresh => {
                let _ = self.reload_directory();
            }
            FmCommand::ToggleHidden => {
                self.show_hidden = !self.show_hidden;
                let _ = self.reload_directory();
            }
            FmCommand::NextPanel => {
                let modal = ConfirmModal::new("", "");
                self.modal_request = Some((
                    PendingAction::NextPanel,
                    ActiveModal::Confirm(Box::new(modal)),
                ));
            }
            FmCommand::PrevPanel => {
                let modal = ConfirmModal::new("", "");
                self.modal_request = Some((
                    PendingAction::PrevPanel,
                    ActiveModal::Confirm(Box::new(modal)),
                ));
            }
            FmCommand::GoToPath => {
                // Open input modal to enter path or URL (supports sftp://, ftp://, etc.)
                let t = termide_i18n::t();
                let current_path = self.display_path();
                let modal =
                    InputModal::with_default(t.fm_goto_title(), t.fm_goto_prompt(), &current_path);
                let action = PendingAction::GoToPath {
                    current_directory: self.current_path.clone(),
                };
                self.modal_request = Some((action, ActiveModal::Input(Box::new(modal))));
            }

            FmCommand::SwitchDirectory => {
                return vec![PanelEvent::OpenDirectorySwitcher];
            }

            // Tree expand/collapse
            FmCommand::ExpandDir => {
                if let Some(te) = self.tree_entry_at(self.selected) {
                    if te.expanded == Some(false) {
                        self.expand_dir(self.selected);
                    }
                }
            }
            FmCommand::CollapseDir => {
                // If current item is an expanded dir, collapse it
                // If current item is inside an expanded subtree, jump to parent dir
                if let Some(te) = self.tree_entry_at(self.selected) {
                    if te.expanded == Some(true) {
                        self.collapse_dir(self.selected);
                    } else if te.depth > 0 {
                        // Navigate up to parent directory in tree
                        self.jump_to_parent_dir();
                    }
                }
            }

            // No operation
            FmCommand::None => {}
        }

        events
    }
}

impl Default for FileManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use termide_core::{CommandResult, Panel, PanelCommand};

    fn create_file_manager_in_temp() -> (FileManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let fm = FileManager::new_with_path(temp_dir.path().to_path_buf());
        (fm, temp_dir)
    }

    #[test]
    fn test_file_manager_new() {
        let (fm, temp_dir) = create_file_manager_in_temp();
        assert_eq!(fm.current_path(), temp_dir.path());
    }

    #[test]
    fn test_handle_command_get_fs_watch_info() {
        let (mut fm, temp_dir) = create_file_manager_in_temp();

        let result = fm.handle_command(PanelCommand::GetFsWatchInfo);
        if let CommandResult::FsWatchInfo {
            current_path,
            is_git_repo,
            ..
        } = result
        {
            assert_eq!(current_path, temp_dir.path());
            assert!(!is_git_repo);
        } else {
            panic!("Expected FsWatchInfo result");
        }
    }

    #[test]
    fn test_handle_command_set_fs_watch_root() {
        let (mut fm, _temp_dir) = create_file_manager_in_temp();

        let root = PathBuf::from("/some/root");
        let result = fm.handle_command(PanelCommand::SetFsWatchRoot {
            root: Some(root.clone()),
            is_git_repo: true,
        });
        assert!(matches!(result, CommandResult::None));

        // Verify the root was set
        let info = fm.handle_command(PanelCommand::GetFsWatchInfo);
        if let CommandResult::FsWatchInfo {
            watched_root,
            is_git_repo,
            ..
        } = info
        {
            assert_eq!(watched_root, Some(root));
            assert!(is_git_repo);
        }
    }

    #[test]
    fn test_handle_command_refresh_directory() {
        let (mut fm, _temp_dir) = create_file_manager_in_temp();

        let result = fm.handle_command(PanelCommand::RefreshDirectory);
        assert!(result.needs_redraw());
    }

    #[test]
    fn test_handle_command_reload() {
        let (mut fm, _temp_dir) = create_file_manager_in_temp();

        let result = fm.handle_command(PanelCommand::Reload);
        assert!(result.needs_redraw());
    }

    #[test]
    fn test_handle_command_get_repo_root() {
        let (mut fm, _temp_dir) = create_file_manager_in_temp();

        // GetRepoRoot returns None when not in git repo
        let result = fm.handle_command(PanelCommand::GetRepoRoot);
        assert!(matches!(result, CommandResult::RepoRoot(None)));

        // Set git_root and verify it's returned
        fm.git_root = Some(PathBuf::from("/test/repo"));
        let result = fm.handle_command(PanelCommand::GetRepoRoot);
        if let CommandResult::RepoRoot(Some(root)) = result {
            assert_eq!(root, PathBuf::from("/test/repo"));
        } else {
            panic!("Expected RepoRoot result");
        }
    }

    #[test]
    fn test_handle_command_not_applicable() {
        let (mut fm, _temp_dir) = create_file_manager_in_temp();

        // Commands not applicable to FileManager should return None
        let result = fm.handle_command(PanelCommand::GetModificationStatus);
        assert!(matches!(result, CommandResult::None));

        let result = fm.handle_command(PanelCommand::Save);
        assert!(matches!(result, CommandResult::None));

        let result = fm.handle_command(PanelCommand::Resize { rows: 24, cols: 80 });
        assert!(matches!(result, CommandResult::None));
    }

    #[test]
    fn test_file_manager_panel_trait_title() {
        let (fm, temp_dir) = create_file_manager_in_temp();
        let title = fm.title();
        // Title may shorten home prefix to ~, so compare against both forms
        let full_path = temp_dir.path().display().to_string();
        let shortened = termide_core::util::shorten_home_path(&full_path);
        assert!(
            title.contains(&full_path) || title.contains(&shortened),
            "title {:?} should contain {:?} or {:?}",
            title,
            full_path,
            shortened,
        );
    }

    #[test]
    fn test_file_manager_panel_trait_needs_close_confirmation() {
        let (fm, _temp_dir) = create_file_manager_in_temp();
        // FileManager doesn't need close confirmation by default
        assert!(fm.needs_close_confirmation().is_none());
    }
}
