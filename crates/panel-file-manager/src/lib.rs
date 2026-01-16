//! File manager panel for termide.
//!
//! Provides a smart file manager with git integration, drag selection, and file operations.

mod file_info;
mod keyboard;
mod navigation;
mod operations;
mod rendering;
mod selection;
mod utils;

pub use file_info::FileInfo;

use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{buffer::Buffer, layout::Rect, prelude::Widget, widgets::Paragraph};
use std::any::Any;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;

use termide_config::{constants, Config, FileManagerSettings};
use termide_core::{
    util::is_binary_file, CommandResult, Panel, PanelCommand, PanelEvent, RenderContext,
    SessionPanel,
};
use termide_git::{get_git_status_async, GitStatus, GitStatusAsyncResult, GitStatusCache};
use termide_modal::{ActiveModal, ConfirmModal, ContentSearchModal, FileSearchModal, InputModal};
use termide_state::{DirSizeResult, PendingAction};
use termide_theme::Theme;
use termide_ui::{clipboard, path_utils, IndexClickTracker, ScrollBar};

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragMode {
    Select, // Shift+drag - selection
    Toggle, // Ctrl+drag - toggle selection
}

/// Selection state for file manager (multi-select and drag selection)
#[derive(Clone, Default)]
pub struct SelectionState {
    /// Set of selected items (indices)
    pub items: HashSet<usize>,
    /// Starting index for drag selection
    pub drag_start: Option<usize>,
    /// Drag mode (Shift/Ctrl)
    drag_mode: Option<DragMode>,
    /// Set of items already processed during current drag (to avoid re-toggling)
    pub dragged: HashSet<usize>,
}

impl SelectionState {
    /// Clear all selection
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Toggle item selection
    pub fn toggle(&mut self, index: usize) {
        if self.items.contains(&index) {
            self.items.remove(&index);
        } else {
            self.items.insert(index);
        }
    }

    /// Select an item
    pub fn select(&mut self, index: usize) {
        self.items.insert(index);
    }

    /// Deselect an item
    pub fn deselect(&mut self, index: usize) {
        self.items.remove(&index);
    }

    /// Check if item is selected
    pub fn is_selected(&self, index: usize) -> bool {
        self.items.contains(&index)
    }

    /// Get count of selected items
    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// Check if selection is empty
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Start shift-drag selection
    pub fn start_shift_drag(&mut self, index: usize) {
        self.dragged.clear();
        self.select(index);
        self.dragged.insert(index);
        self.drag_start = Some(index);
        self.drag_mode = Some(DragMode::Select);
    }

    /// Start ctrl-drag toggle selection
    pub fn start_ctrl_drag(&mut self, index: usize) {
        self.toggle(index);
        self.drag_start = Some(index);
        self.drag_mode = Some(DragMode::Toggle);
        self.dragged.clear();
        self.dragged.insert(index);
    }

    /// End drag selection
    pub fn end_drag(&mut self) {
        self.drag_start = None;
        self.drag_mode = None;
        self.dragged.clear();
    }

    /// Check if drag is active
    pub fn is_dragging(&self) -> bool {
        self.drag_mode.is_some()
    }

    /// Check if shift-drag mode
    pub fn is_shift_drag(&self) -> bool {
        self.drag_mode == Some(DragMode::Select)
    }

    /// Check if ctrl-drag mode
    pub fn is_ctrl_drag(&self) -> bool {
        self.drag_mode == Some(DragMode::Toggle)
    }

    /// Process drag over item (returns true if item was processed)
    pub fn process_drag(&mut self, index: usize) -> bool {
        if !self.dragged.contains(&index) {
            match self.drag_mode {
                Some(DragMode::Select) => {
                    self.select(index);
                    self.dragged.insert(index);
                    true
                }
                Some(DragMode::Toggle) => {
                    self.toggle(index);
                    self.dragged.insert(index);
                    true
                }
                None => false,
            }
        } else {
            false
        }
    }

    /// Select range of items
    pub fn select_range(&mut self, from: usize, to: usize) {
        let (start, end) = if from <= to { (from, to) } else { (to, from) };
        for i in start..=end {
            self.items.insert(i);
        }
    }

    /// Toggle range of items
    pub fn toggle_range(&mut self, from: usize, to: usize) {
        let (start, end) = if from <= to { (from, to) } else { (to, from) };
        for i in start..=end {
            self.toggle(i);
        }
    }
}

/// Navigation state for directory traversal (cursor restoration, debouncing).
///
/// Groups related navigation fields together:
/// - `previous_dir_name`: Saved directory name when going up (for cursor restoration)
/// - `navigating_down`: Flag signaling entry into subdirectory (cursor resets to 0)
/// - `last_reload_time`: Timestamp for debouncing rapid reload_directory() calls
#[derive(Clone, Default)]
pub struct NavigationState {
    /// Name of directory we came from (for cursor restoration when going up)
    pub previous_dir_name: Option<String>,
    /// Flag indicating we're navigating down into a subdirectory (cursor should reset to 0)
    pub navigating_down: bool,
    /// Last reload time for debouncing rapid reload_directory() calls
    pub last_reload_time: Option<std::time::Instant>,
}

impl NavigationState {
    /// Create a new navigation state
    pub const fn new() -> Self {
        Self {
            previous_dir_name: None,
            navigating_down: false,
            last_reload_time: None,
        }
    }

    /// Save directory name before going up (for cursor restoration)
    pub fn save_for_going_up(&mut self, dir_name: String) {
        self.previous_dir_name = Some(dir_name);
    }

    /// Clear saved name and set flag for going down into subdirectory
    pub fn prepare_for_going_down(&mut self) {
        self.previous_dir_name = None;
        self.navigating_down = true;
    }

    /// Take previous directory name (returns and clears it)
    pub fn take_previous_dir_name(&mut self) -> Option<String> {
        self.previous_dir_name.take()
    }

    /// Check and reset navigating_down flag, returns true if was navigating down
    pub fn check_and_reset_navigating_down(&mut self) -> bool {
        if self.navigating_down {
            self.navigating_down = false;
            true
        } else {
            false
        }
    }

    /// Check if reload should be skipped due to debounce
    /// Returns true if enough time has passed since last reload
    pub fn should_reload(&mut self, debounce_ms: u128) -> bool {
        let now = std::time::Instant::now();
        if let Some(last) = self.last_reload_time {
            if now.duration_since(last).as_millis() < debounce_ms {
                return false; // Skip rapid reloads
            }
        }
        self.last_reload_time = Some(now);
        true
    }
}

/// Smart file manager with advanced features
pub struct FileManager {
    current_path: PathBuf,
    entries: Vec<FileEntry>,
    selected: usize,
    scroll_offset: usize,
    /// Last displayed title (cached for [X] clicks)
    display_title: String,
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

impl FileManager {
    /// Create a new smart file manager
    pub fn new() -> Self {
        let current_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        Self::new_with_path(current_path)
    }

    /// Create a new smart file manager with the specified path
    pub fn new_with_path(current_path: PathBuf) -> Self {
        let display_title = current_path.display().to_string();
        let mut fm = Self {
            current_path,
            entries: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            display_title,
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
        };
        let _ = fm.load_directory();
        fm
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
            self.current_path = path;
            self.load_directory()
        } else if let Some(parent) = path.parent() {
            // If path is a file, navigate to its parent directory
            self.current_path = parent.to_path_buf();
            self.load_directory()
        } else {
            Ok(())
        }
    }

    /// Load the contents of the current directory
    pub fn load_directory(&mut self) -> Result<()> {
        // Invalidate git_root when navigating to a new directory
        // This triggers re-registration with watcher in check_watcher_events()
        self.git_root = None;
        self.load_directory_inner(false)
    }

    /// Navigate to a specific file - opens its parent directory and selects the file
    pub fn navigate_to_file(&mut self, path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            self.current_path = parent.to_path_buf();
            let _ = self.load_directory();

            // Find and select the file in the list
            if let Some(file_name) = path.file_name() {
                let name_str = file_name.to_string_lossy();
                if let Some(idx) = self.entries.iter().position(|e| e.name == name_str) {
                    self.selected = idx;
                    self.adjust_scroll_offset(self.visible_height);
                }
            }
        }
    }

    /// Select an entry by name in the current directory
    pub fn select_by_name(&mut self, name: &std::ffi::OsStr) {
        let name_str = name.to_string_lossy();
        if let Some(idx) = self.entries.iter().position(|e| e.name == name_str) {
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

        self.load_directory_inner(true)
    }

    /// Internal method to load directory with optional selection preservation
    fn load_directory_inner(&mut self, preserve_selection: bool) -> Result<()> {
        // Save current file name and index to restore position
        // Use previous_dir_name if navigating up, otherwise use current selection
        let current_name = self
            .navigation
            .take_previous_dir_name()
            .or_else(|| self.entries.get(self.selected).map(|e| e.name.clone()));
        let previous_index = self.selected;
        let previous_scroll_offset = self.scroll_offset;

        // Save names of selected files if we need to restore selection
        let selected_names: HashSet<String> = if preserve_selection {
            self.selection
                .items
                .iter()
                .filter_map(|&idx| self.entries.get(idx).map(|e| e.name.clone()))
                .collect()
        } else {
            HashSet::new()
        };

        self.entries.clear();
        self.selected = 0;
        self.scroll_offset = 0;
        // Clear selection state (will restore by names if preserve_selection)
        self.selection.clear();
        self.selection.end_drag();

        // Update displayed title (will be truncated during rendering if needed)
        self.display_title = self.current_path.display().to_string();

        // Start async git status loading (non-blocking)
        // Git status will be applied when check_git_status_async() is called
        // Only clear cache when navigating to new directory (not when reloading)
        if !preserve_selection {
            self.git_status_cache = None;
        }
        self.git_status_receiver = Some(get_git_status_async(self.current_path.clone()));

        // Add parent directory if not at root
        if self.current_path.parent().is_some() {
            self.entries.push(FileEntry {
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
        if let Ok(read_dir) = fs::read_dir(&self.current_path) {
            for entry in read_dir.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    let name = entry.file_name().to_string_lossy().to_string();

                    // Determine git status for this entry
                    let git_status = if metadata.is_dir() {
                        // For directories: check recursively for nested changes
                        self.git_status_cache
                            .as_ref()
                            .map(|cache| cache.get_directory_status(&name))
                            .unwrap_or(GitStatus::Unmodified)
                    } else {
                        // For files: use direct status
                        self.git_status_cache
                            .as_ref()
                            .map(|cache| cache.get_status(&name))
                            .unwrap_or(GitStatus::Unmodified)
                    };

                    // Check if this is a symlink (use symlink_metadata to not follow links)
                    let is_symlink = if let Ok(link_metadata) = fs::symlink_metadata(entry.path()) {
                        link_metadata.is_symlink()
                    } else {
                        false
                    };

                    // Check if file is executable (Unix permissions)
                    #[cfg(unix)]
                    let is_executable = {
                        use std::os::unix::fs::PermissionsExt;
                        metadata.permissions().mode() & 0o111 != 0
                    };
                    #[cfg(not(unix))]
                    let is_executable = false;

                    // Check if file is read-only (Unix permissions)
                    #[cfg(unix)]
                    let is_readonly = {
                        use std::os::unix::fs::PermissionsExt;
                        let mode = metadata.permissions().mode();
                        (mode & 0o200) == 0 // owner write bit
                    };
                    #[cfg(not(unix))]
                    let is_readonly = metadata.permissions().readonly();

                    // Get size (files only) and modification time
                    let size = if metadata.is_file() {
                        Some(metadata.len())
                    } else {
                        None
                    };
                    let modified = metadata.modified().ok();

                    self.entries.push(FileEntry {
                        name,
                        is_dir: metadata.is_dir(),
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
            log::warn!("Failed to read directory: {}", self.current_path.display());
        }

        // Note: deleted files are added when async git status completes (apply_git_statuses)

        // Sort: directories first, then files
        self.entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        // Restore selection by file names
        if !selected_names.is_empty() {
            for (idx, entry) in self.entries.iter().enumerate() {
                if selected_names.contains(&entry.name) {
                    self.selection.select(idx);
                }
            }
        }

        // Restore cursor position
        if self.navigation.check_and_reset_navigating_down() {
            // When entering a subdirectory, always start at first item ("..")
            self.selected = 0;
            self.scroll_offset = 0;
        } else if let Some(name) = current_name {
            if let Some(pos) = self.entries.iter().position(|e| e.name == name) {
                // Found file by name - restore to its position
                self.selected = pos;
            } else if !self.entries.is_empty() {
                // File not found (deleted) - use previous index or last available
                self.selected = previous_index.min(self.entries.len() - 1);
            }

            // Restore scroll_offset using real visible_height
            if self.visible_height > 0 {
                // If all items fit on screen - no scroll needed
                if self.entries.len() <= self.visible_height {
                    self.scroll_offset = 0;
                } else {
                    // Restore previous offset if still valid
                    let max_scroll = self.entries.len().saturating_sub(self.visible_height);
                    self.scroll_offset = previous_scroll_offset.min(max_scroll);
                }
                // Ensure cursor is visible
                self.adjust_scroll_offset(self.visible_height);
            }
            // If visible_height == 0, render() will recalculate on first draw
        }

        Ok(())
    }

    /// Get current directory path
    pub fn current_path(&self) -> &std::path::Path {
        &self.current_path
    }

    /// Enter directory or open file
    /// Returns `Some(PanelEvent::OpenFile)` if a file should be opened
    fn enter(&mut self) -> Option<PanelEvent> {
        if let Some(entry) = self.entries.get(self.selected) {
            // Prohibit operations on deleted files
            if entry.git_status == GitStatus::Deleted {
                return None;
            }

            if entry.name == ".." {
                // Save current directory name before going up
                if let Some(dir_name) = self.current_path.file_name() {
                    self.navigation
                        .save_for_going_up(dir_name.to_string_lossy().to_string());
                }
                if let Some(parent) = self.current_path.parent() {
                    self.current_path = parent.to_path_buf();
                    let _ = self.load_directory();
                }
            } else if entry.is_dir {
                self.navigation.prepare_for_going_down();
                self.current_path.push(&entry.name);
                let _ = self.load_directory();
            } else {
                // This is a file
                let file_path = self.current_path.join(&entry.name);

                // 1. Raster images → ImagePanel or xdg-open
                if is_raster_image(&entry.name) {
                    return Some(PanelEvent::PreviewMedia(file_path));
                }

                // 2. Vector images, video → always xdg-open
                if is_vector_image(&entry.name) || is_video(&entry.name) {
                    return Some(PanelEvent::OpenExternal(file_path));
                }

                // 3. Executable → run in terminal
                if entry.is_executable {
                    return Some(PanelEvent::ExecuteFile(file_path));
                }

                // 4. Binary files → xdg-open
                if is_binary_file(&file_path) {
                    return Some(PanelEvent::OpenExternal(file_path));
                }

                // 5. Text files → editor
                return Some(PanelEvent::OpenFile(file_path));
            }
        }
        None
    }

    /// Open file for editing (F4)
    /// Returns `Some(PanelEvent::OpenFile)` if a file should be opened
    fn edit_file(&mut self) -> Option<PanelEvent> {
        if let Some(entry) = self.entries.get(self.selected) {
            // Prohibit operations on deleted files
            if entry.git_status == GitStatus::Deleted {
                return None;
            }

            // Check that this is a file, not a directory and not ".."
            if !entry.is_dir && entry.name != ".." {
                let file_path = self.current_path.join(&entry.name);
                return Some(PanelEvent::OpenFile(file_path));
            }
        }
        None
    }

    /// View file without executing (F3)
    /// Similar to enter() but treats executables as text files
    fn view_file(&mut self) -> Option<PanelEvent> {
        if let Some(entry) = self.entries.get(self.selected) {
            // Prohibit operations on deleted files
            if entry.git_status == GitStatus::Deleted {
                return None;
            }

            // Directories and ".." - do nothing
            if entry.is_dir || entry.name == ".." {
                return None;
            }

            let file_path = self.current_path.join(&entry.name);

            // 1. Raster images → ImagePanel
            if is_raster_image(&entry.name) {
                return Some(PanelEvent::PreviewMedia(file_path));
            }

            // 2. Vector images, video → xdg-open
            if is_vector_image(&entry.name) || is_video(&entry.name) {
                return Some(PanelEvent::OpenExternal(file_path));
            }

            // 3. Binary files → xdg-open
            if is_binary_file(&file_path) {
                return Some(PanelEvent::OpenExternal(file_path));
            }

            // 4. Text files (including executables) → editor
            return Some(PanelEvent::OpenFile(file_path));
        }
        None
    }

    /// Force open file with system default application (Shift+Enter)
    fn open_external(&mut self) -> Option<PanelEvent> {
        if let Some(entry) = self.entries.get(self.selected) {
            // Prohibit operations on deleted files
            if entry.git_status == GitStatus::Deleted {
                return None;
            }

            // Directories and ".." - do nothing
            if entry.is_dir || entry.name == ".." {
                return None;
            }

            let file_path = self.current_path.join(&entry.name);
            return Some(PanelEvent::OpenExternal(file_path));
        }
        None
    }

    /// Format file size in human-readable format (public method for external use)
    pub fn format_size_static(bytes: u64) -> String {
        utils::format_size(bytes)
    }

    /// Check for async git status results and update entries if available.
    /// Returns true if git status was updated.
    pub fn check_git_status_async(&mut self) -> bool {
        let result = if let Some(ref rx) = self.git_status_receiver {
            rx.try_recv().ok()
        } else {
            None
        };

        if let Some(git_result) = result {
            // Verify the result is for the current directory
            if git_result.dir == self.current_path {
                self.git_status_cache = git_result.cache;
                self.git_status_receiver = None;

                // Re-apply git statuses to entries
                self.apply_git_statuses();
                return true;
            }
            // Result is for a different directory - discard it
            self.git_status_receiver = None;
        }
        false
    }

    /// Refresh git status without full directory reload.
    /// Used when git watcher detects repository changes (e.g., after commits).
    fn refresh_git_status(&mut self) {
        // Start async git status request for current directory
        self.git_status_receiver = Some(get_git_status_async(self.current_path.clone()));
    }

    /// Apply git statuses from cache to entries
    fn apply_git_statuses(&mut self) {
        for entry in &mut self.entries {
            if entry.name == ".." {
                continue;
            }

            entry.git_status = if entry.is_dir {
                self.git_status_cache
                    .as_ref()
                    .map(|cache| cache.get_directory_status(&entry.name))
                    .unwrap_or(GitStatus::Unmodified)
            } else {
                self.git_status_cache
                    .as_ref()
                    .map(|cache| cache.get_status(&entry.name))
                    .unwrap_or(GitStatus::Unmodified)
            };
        }

        // Also add deleted files that weren't in the directory listing
        if let Some(cache) = &self.git_status_cache {
            for deleted_name in cache.get_deleted_files() {
                // Skip if already in entries
                if self.entries.iter().any(|e| e.name == deleted_name) {
                    continue;
                }
                self.entries.push(FileEntry {
                    name: deleted_name,
                    is_dir: false,
                    is_symlink: false,
                    is_executable: false,
                    is_readonly: false,
                    git_status: GitStatus::Deleted,
                    size: None,
                    modified: None,
                });
            }

            // Re-sort if we added deleted files
            self.entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            });
        }
    }

    /// Check if git status is still loading
    pub fn is_git_status_loading(&self) -> bool {
        self.git_status_receiver.is_some()
    }
}

impl Panel for FileManager {
    fn name(&self) -> &'static str {
        "file_manager"
    }

    fn title(&self) -> String {
        if self.is_git_status_loading() {
            format!("{} {}", constants::LOADING_INDICATOR, self.display_title)
        } else {
            self.display_title.clone()
        }
    }

    fn prepare_render(&mut self, theme: &termide_theme::Theme, config: &Config) {
        self.cached_theme = *theme;
        self.cached_config = config.file_manager.clone();
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
        let _ = ctx; // Use ctx in future for theme/config
                     // Automatically update scroll offset
                     // area is already the inner content area (accordion drew outer border)
        let content_height = area.height as usize;
        self.visible_height = content_height; // Save for use in handle_key

        if self.selected >= self.scroll_offset + content_height {
            self.scroll_offset = self.selected - content_height + 1;
        } else if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        }

        // Get display path taking into account panel width
        self.display_title = self.get_display_title(area.width);

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
                self.entries.len(),
                &theme_colors,
                ctx.is_focused,
            );
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        use keyboard::FmCommand;

        // Translate Cyrillic to Latin for hotkeys
        let key = termide_keyboard::translate_hotkey(key);

        // Parse key event to command
        let command = FmCommand::from_key_event(key, &self.cached_config.keybindings);

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
                let max_scroll = self.entries.len().saturating_sub(visible_height);
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

                if clicked_index < self.entries.len() {
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
                        // Check for double click using ClickTracker
                        let is_double_click = self.click_tracker.is_double_click(&clicked_index);

                        if is_double_click {
                            // Double click - open file/directory
                            self.selected = clicked_index;
                            let event = self.enter();
                            // Reset click state
                            self.click_tracker.reset();
                            // Return event if file was opened
                            if let Some(e) = event {
                                return vec![e];
                            }
                        } else {
                            // Single click - select item
                            self.selected = clicked_index;
                            // Record click for double-click detection
                            self.click_tracker.record(clicked_index);
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

                    if current_index < self.entries.len() {
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
                        // If at repo root, reload directory to update .git metadata
                        if self.current_path == *git_root {
                            let _ = self.reload_directory();
                            return CommandResult::NeedsRedraw(true);
                        }
                        // Otherwise just refresh git status colors
                        self.refresh_git_status();
                        return CommandResult::NeedsRedraw(false);
                    }
                }
                CommandResult::None
            }
            // Commands not applicable to FileManager
            PanelCommand::CheckPendingGitDiff
            | PanelCommand::CheckGitDiffReceiver
            | PanelCommand::CheckExternalModification
            | PanelCommand::Resize { .. }
            | PanelCommand::GetModificationStatus
            | PanelCommand::Save
            | PanelCommand::CloseWithoutSaving
            | PanelCommand::SetGitOperationInProgress { .. } => CommandResult::None,
        }
    }

    fn needs_close_confirmation(&self) -> Option<String> {
        // FileManager doesn't store critical state by itself
        // Pending batch operations are checked in has_panels_requiring_confirmation()
        None
    }

    fn to_session(&self, _session_dir: &std::path::Path) -> Option<SessionPanel> {
        // Save file manager with current directory path
        Some(SessionPanel::FileManager {
            path: self.current_path.clone(),
        })
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
}

// Additional methods used by app layer (not part of Panel trait)
impl FileManager {
    /// Take modal window request (if any).
    pub fn take_modal_request(&mut self) -> Option<(PendingAction, ActiveModal)> {
        self.modal_request.take()
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
                let max_index = self.entries.len().saturating_sub(1);
                self.selected = (self.selected + self.visible_height).min(max_index);
            }
            FmCommand::GoHome => {
                self.selected = 0;
                self.scroll_offset = 0;
            }
            FmCommand::GoEnd => {
                self.selected = self.entries.len().saturating_sub(1);
            }
            FmCommand::Enter => {
                if let Some(event) = self.enter() {
                    events.push(event);
                }
            }
            FmCommand::GoParent => {
                if let Some(dir_name) = self.current_path.file_name() {
                    self.navigation
                        .save_for_going_up(dir_name.to_string_lossy().to_string());
                }
                if let Some(parent) = self.current_path.parent() {
                    self.current_path = parent.to_path_buf();
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
                self.move_down();
            }
            FmCommand::SelectAll => self.select_all(),
            FmCommand::ClearSelection => self.selection.clear(),
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
                    panel_index: 0,
                    directory: self.current_path.clone(),
                };
                self.modal_request = Some((action, ActiveModal::Input(Box::new(modal))));
            }
            FmCommand::NewDirectory => {
                let t = termide_i18n::t();
                let modal = InputModal::new(t.modal_create_dir_title(), "");
                let action = PendingAction::CreateDirectory {
                    panel_index: 0,
                    directory: self.current_path.clone(),
                };
                self.modal_request = Some((action, ActiveModal::Input(Box::new(modal))));
            }
            FmCommand::DeleteFiles => {
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
                    let action = PendingAction::DeletePath {
                        panel_index: 0,
                        paths,
                    };
                    self.modal_request = Some((action, ActiveModal::Confirm(Box::new(modal))));
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
                        panel_index: 0,
                        sources: paths,
                        target_directory: None,
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
                        panel_index: 0,
                        sources: paths,
                        target_directory: None,
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
            FmCommand::SearchFiles => {
                let t = termide_i18n::t();
                let modal = FileSearchModal::new(t.file_search_title(), self.current_path.clone());
                let action = PendingAction::FileSearch { panel_index: 0 };
                self.modal_request = Some((action, ActiveModal::FileSearch(Box::new(modal))));
            }
            FmCommand::SearchContent => {
                let t = termide_i18n::t();
                let max_file_size =
                    self.cached_config.content_search_max_file_size_mb * 1024 * 1024;
                let modal = ContentSearchModal::new(
                    t.content_search_title(),
                    self.current_path.clone(),
                    max_file_size,
                );
                let action = PendingAction::ContentSearch { panel_index: 0 };
                self.modal_request = Some((action, ActiveModal::ContentSearch(Box::new(modal))));
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
                            panel_index: 0,
                            sources: files,
                            target_directory: Some(self.current_path.clone()),
                        };
                        let modal = ConfirmModal::new("Confirm", &message);
                        self.modal_request = Some((action, ActiveModal::Confirm(Box::new(modal))));
                    }
                }
            }

            // Misc
            FmCommand::ShowFileInfo => self.show_file_info(),
            FmCommand::Refresh => {
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

/// Get file extension in lowercase
fn get_extension(filename: &str) -> String {
    filename
        .rsplit('.')
        .next()
        .map(|e| e.to_lowercase())
        .unwrap_or_default()
}

/// Check if file is a raster image supported by ImagePanel
fn is_raster_image(filename: &str) -> bool {
    matches!(
        get_extension(filename).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif"
    )
}

/// Check if file is a vector image (requires external viewer)
fn is_vector_image(filename: &str) -> bool {
    matches!(get_extension(filename).as_str(), "svg" | "ico")
}

/// Check if file is a video (requires external viewer)
fn is_video(filename: &str) -> bool {
    matches!(
        get_extension(filename).as_str(),
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v"
    )
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
        // Title should contain the directory path
        assert!(fm.title().contains(&temp_dir.path().display().to_string()));
    }

    #[test]
    fn test_file_manager_panel_trait_needs_close_confirmation() {
        let (fm, _temp_dir) = create_file_manager_in_temp();
        // FileManager doesn't need close confirmation by default
        assert!(fm.needs_close_confirmation().is_none());
    }
}
