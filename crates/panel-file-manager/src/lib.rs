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
mod vfs_state;

pub use file_info::FileInfo;
pub use vfs_state::VfsState;

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
use termide_modal::{
    ActionButton, ActiveModal, ConfirmModal, ContentSearchModal, FileSearchModal, InfoActionModal,
    InputModal,
};
use termide_state::{DirSizeResult, PendingAction};
use termide_theme::Theme;
use termide_ui::{clipboard, path_utils, IndexClickTracker, ScrollBar};
use termide_vfs::{VfsEntry, VfsFileType};

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragMode {
    Select, // Shift+drag - selection
    Toggle, // Ctrl+drag - toggle selection
}

/// How a file should be opened
#[derive(Debug, Clone, Copy, PartialEq)]
enum FileOpenMode {
    /// Open with default action (Enter): auto-detect type
    Default,
    /// Force open in editor (F4): treat everything as text
    ForceEdit,
    /// View mode (F3): similar to Default but executables are treated as text
    View,
    /// Open with system default app (Shift+Enter)
    External,
}

/// Determine the appropriate PanelEvent for opening a file based on its type and open mode.
/// Returns None if the operation should not proceed (e.g., deleted files, directories).
fn determine_file_open_event(
    entry: &FileEntry,
    file_path: &std::path::Path,
    mode: FileOpenMode,
) -> Option<PanelEvent> {
    // Prohibit operations on deleted files
    if entry.git_status == GitStatus::Deleted {
        return None;
    }

    // Directories and ".." - do nothing for file operations
    if entry.is_dir || entry.name == ".." {
        return None;
    }

    match mode {
        FileOpenMode::External => {
            // Always open with system default
            Some(PanelEvent::OpenExternal(file_path.to_path_buf()))
        }
        FileOpenMode::ForceEdit => {
            // Force open in editor regardless of type
            Some(PanelEvent::OpenFile(file_path.to_path_buf()))
        }
        FileOpenMode::Default | FileOpenMode::View => {
            // 1. Raster images → ImagePanel
            if is_raster_image(&entry.name) {
                return Some(PanelEvent::PreviewMedia(file_path.to_path_buf()));
            }

            // 2. Vector images, video → xdg-open
            if is_vector_image(&entry.name) || is_video(&entry.name) {
                return Some(PanelEvent::OpenExternal(file_path.to_path_buf()));
            }

            // 3. Executable → run in terminal (Default mode only)
            if mode == FileOpenMode::Default && entry.is_executable {
                return Some(PanelEvent::ExecuteFile(file_path.to_path_buf()));
            }

            // 4. Binary files → xdg-open
            if is_binary_file(file_path) {
                return Some(PanelEvent::OpenExternal(file_path.to_path_buf()));
            }

            // 5. Text files → editor
            Some(PanelEvent::OpenFile(file_path.to_path_buf()))
        }
    }
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
/// - `newly_created_item`: Name of newly created file/directory (for cursor positioning)
#[derive(Clone, Default)]
pub struct NavigationState {
    /// Name of directory we came from (for cursor restoration when going up)
    pub previous_dir_name: Option<String>,
    /// Flag indicating we're navigating down into a subdirectory (cursor should reset to 0)
    pub navigating_down: bool,
    /// Last reload time for debouncing rapid reload_directory() calls
    pub last_reload_time: Option<std::time::Instant>,
    /// Name of newly created item to navigate to after reload
    pub newly_created_item: Option<String>,
}

impl NavigationState {
    /// Create a new navigation state
    pub const fn new() -> Self {
        Self {
            previous_dir_name: None,
            navigating_down: false,
            last_reload_time: None,
            newly_created_item: None,
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

    /// Set newly created item name (for cursor navigation after reload)
    pub fn set_newly_created(&mut self, name: String) {
        self.newly_created_item = Some(name);
    }

    /// Take newly created item name (returns and clears it)
    pub fn take_newly_created(&mut self) -> Option<String> {
        self.newly_created_item.take()
    }
}

/// Smart file manager with advanced features
pub struct FileManager {
    current_path: PathBuf,
    entries: Vec<FileEntry>,
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
            entries: Vec::new(),
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
            entries: Vec::new(),
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
        };

        // Start the directory listing operation for remote paths
        fm.vfs.start_list_dir();

        Ok(fm)
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

        // For remote paths, invalidate cache and start async listing
        // Entries will be populated by tick() when VFS operation completes
        if self.vfs.is_remote() {
            self.vfs.invalidate_cache();
            self.vfs.start_list_dir();
            return Ok(());
        }

        self.load_directory_inner(true)
    }

    /// Update entries from VFS directory listing (for remote directories).
    fn update_entries_from_vfs(&mut self, vfs_entries: Vec<VfsEntry>) {
        // Save current state BEFORE clearing entries for cursor restoration
        let previous_index = self.selected;
        let previous_scroll_offset = self.scroll_offset;
        let current_name = self.entries.get(self.selected).map(|e| e.name.clone());

        self.entries.clear();
        self.selected = 0;
        self.scroll_offset = 0;
        self.selection.clear();

        // Add ".." entry for parent directory navigation (unless at root)
        if self.vfs.current_path().parent().is_some() {
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

        // Convert and add VFS entries
        let mut file_entries: Vec<FileEntry> = vfs_entries
            .into_iter()
            .map(FileEntry::from_vfs_entry)
            .collect();

        // Sort: directories first, then alphabetically by name (case-insensitive)
        file_entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        self.entries.extend(file_entries);

        // Clear git status (not applicable for remote files)
        self.git_status_cache = None;
        self.git_root = None;

        // Restore cursor position intelligently
        // Priority: newly created item > navigating down > normal restoration
        if let Some(created_name) = self.navigation.take_newly_created() {
            // Navigate to newly created/copied item (highest priority)
            if let Some(idx) = self.entries.iter().position(|e| e.name == created_name) {
                self.selected = idx;
                // Ensure cursor is visible in viewport
                if self.visible_height > 0 {
                    self.adjust_scroll_offset(self.visible_height);
                }
            } else if !self.entries.is_empty() {
                // Item not found (shouldn't happen) - stay on current or last item
                self.selected = previous_index.min(self.entries.len() - 1);
            }
        } else if self.navigation.check_and_reset_navigating_down() {
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
    }

    /// Internal method to load directory with optional selection preservation
    fn load_directory_inner(&mut self, preserve_selection: bool) -> Result<()> {
        // Sync VFS path with current_path for local paths
        // This ensures vfs.display_path() stays in sync with self.current_path
        // after navigation operations (enter, go parent, go home, etc.)
        if !self.vfs.is_remote() {
            self.vfs
                .set_path(termide_vfs::VfsPath::local(self.current_path.clone()));
        }

        // For remote paths, don't clear entries - keep showing current content while loading
        // update_entries_from_vfs() will replace them atomically when async operation completes
        if self.vfs.is_remote() {
            log::debug!("load_directory_inner: Starting async list for remote path (keeping current entries)");
            // Invalidate cache and start async directory listing
            // Entries will be populated by update_entries_from_vfs() when VFS operation completes
            self.vfs.invalidate_cache();
            self.vfs.start_list_dir();
            return Ok(());
        }

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

        // Read directory contents (local paths only - remote handled above)
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
        // Priority: newly created item > navigating down > normal restoration
        if let Some(created_name) = self.navigation.take_newly_created() {
            // Navigate to newly created item (highest priority)
            if let Some(idx) = self.entries.iter().position(|e| e.name == created_name) {
                self.selected = idx;
                // Ensure cursor is visible
                if self.visible_height > 0 {
                    self.adjust_scroll_offset(self.visible_height);
                }
            } else if !self.entries.is_empty() {
                // Item not found (shouldn't happen) - stay on current or last item
                self.selected = previous_index.min(self.entries.len() - 1);
            }
        } else if self.navigation.check_and_reset_navigating_down() {
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
        let entry = self.entries.get(self.selected)?;

        // Prohibit operations on deleted files
        if entry.git_status == GitStatus::Deleted {
            return None;
        }

        // Handle directory navigation
        if entry.name == ".." {
            // Save current directory name before going up
            if let Some(dir_name) = self.current_path.file_name() {
                self.navigation
                    .save_for_going_up(dir_name.to_string_lossy().to_string());
            }

            if self.vfs.is_remote() {
                // For remote paths, navigate through VFS
                self.vfs.navigate_up();
                // Don't update current_path yet - wait for listing to complete
                // The path will be synced when tick() succeeds
                self.vfs.start_list_dir();
            } else if let Some(parent) = self.current_path.parent() {
                self.current_path = parent.to_path_buf();
                let _ = self.load_directory();
            }
            return None;
        }

        if entry.is_dir {
            self.navigation.prepare_for_going_down();

            if self.vfs.is_remote() {
                // For remote paths, navigate through VFS
                self.vfs.navigate_down(&entry.name);
                // Don't update current_path yet - wait for listing to complete
                // The path will be synced when tick() succeeds
                self.vfs.start_list_dir();
            } else {
                self.current_path.push(&entry.name);
                let _ = self.load_directory();
            }
            return None;
        }

        // This is a file - check if remote
        if self.vfs.is_remote() {
            let vfs_path = self.vfs.current_path().join(&entry.name);
            return Some(PanelEvent::OpenRemoteFile(vfs_path.to_url_string()));
        }

        // Local path handling
        let file_path = self.current_path.join(&entry.name);
        determine_file_open_event(entry, &file_path, FileOpenMode::Default)
    }

    /// Open file for editing (F4)
    /// Returns `Some(PanelEvent::OpenFile)` if a file should be opened
    fn edit_file(&mut self) -> Option<PanelEvent> {
        let entry = self.entries.get(self.selected)?;

        // Prohibit operations on deleted files
        if entry.git_status == GitStatus::Deleted {
            return None;
        }

        // Directories and ".." - do nothing for file operations
        if entry.is_dir || entry.name == ".." {
            return None;
        }

        // Check if remote
        if self.vfs.is_remote() {
            let vfs_path = self.vfs.current_path().join(&entry.name);
            return Some(PanelEvent::OpenRemoteFile(vfs_path.to_url_string()));
        }

        let file_path = self.current_path.join(&entry.name);
        determine_file_open_event(entry, &file_path, FileOpenMode::ForceEdit)
    }

    /// View file without executing (F3)
    /// Similar to enter() but treats executables as text files
    fn view_file(&mut self) -> Option<PanelEvent> {
        let entry = self.entries.get(self.selected)?;

        // Prohibit operations on deleted files
        if entry.git_status == GitStatus::Deleted {
            return None;
        }

        // Directories and ".." - do nothing for file operations
        if entry.is_dir || entry.name == ".." {
            return None;
        }

        // Check if remote
        if self.vfs.is_remote() {
            let vfs_path = self.vfs.current_path().join(&entry.name);
            return Some(PanelEvent::OpenRemoteFile(vfs_path.to_url_string()));
        }

        let file_path = self.current_path.join(&entry.name);
        determine_file_open_event(entry, &file_path, FileOpenMode::View)
    }

    /// Force open file with system default application (Shift+Enter)
    fn open_external(&mut self) -> Option<PanelEvent> {
        let entry = self.entries.get(self.selected)?;
        let file_path = self.current_path.join(&entry.name);
        determine_file_open_event(entry, &file_path, FileOpenMode::External)
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
            let deleted_files = cache.get_deleted_files();
            if !deleted_files.is_empty() {
                // Build HashSet of existing entry names for O(1) lookup instead of O(n) per check
                let existing_names: HashSet<String> =
                    self.entries.iter().map(|e| e.name.clone()).collect();

                // Collect new entries to add (avoiding borrow conflict)
                let new_entries: Vec<FileEntry> = deleted_files
                    .into_iter()
                    .filter(|deleted_name| !existing_names.contains(deleted_name))
                    .map(|deleted_name| FileEntry {
                        name: deleted_name,
                        is_dir: false,
                        is_symlink: false,
                        is_executable: false,
                        is_readonly: false,
                        git_status: GitStatus::Deleted,
                        size: None,
                        modified: None,
                    })
                    .collect();

                // Only re-sort if we actually added deleted files
                if !new_entries.is_empty() {
                    self.entries.extend(new_entries);
                    self.entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                    });
                }
            }
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
        // Return full path, let smart_truncate_title() handle truncation
        // Use VFS display path for remote paths (includes protocol)
        let path = if self.is_remote() {
            self.display_path()
        } else {
            self.current_path.display().to_string()
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

    fn prepare_render(&mut self, theme: &termide_theme::Theme, config: &Config) {
        self.cached_theme = *theme;
        self.cached_config = config.file_manager.clone();
        self.vim_mode = config.general.vim_mode;
        self.cached_vfs_timeout_secs = config.vfs.connection_timeout_secs;
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
            let max_scroll = self.entries.len().saturating_sub(visible_height);
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
                        // If at repo root, reload directory to update .git metadata
                        if self.current_path == *git_root {
                            let _ = self.reload_directory();
                            return CommandResult::NeedsRedraw(true);
                        }
                        // Otherwise just refresh git status colors
                        self.refresh_git_status();
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
            | PanelCommand::SetGitOperationInProgress { .. } => CommandResult::None,
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
                self.move_down();
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
                                .map(|n| n.to_string_lossy().to_string())
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
                let action = PendingAction::FileSearch;
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
                let action = PendingAction::ContentSearch;
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
