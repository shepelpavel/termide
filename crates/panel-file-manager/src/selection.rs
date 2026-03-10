use std::path::PathBuf;

use super::FileManager;
use termide_vfs::VfsPath;

impl FileManager {
    /// Check if entry at index is ".." (not selectable)
    fn is_parent_entry(&self, idx: usize) -> bool {
        self.entries.get(idx).is_some_and(|e| e.name == "..")
    }

    /// Insert index into selection, skipping ".."
    fn select_index(&mut self, idx: usize) {
        if !self.is_parent_entry(idx) {
            self.selection.items.insert(idx);
        }
    }

    /// Toggle index in selection, skipping ".."
    fn toggle_index(&mut self, idx: usize) {
        if self.is_parent_entry(idx) {
            return;
        }
        if self.selection.items.contains(&idx) {
            self.selection.items.remove(&idx);
        } else {
            self.selection.items.insert(idx);
        }
    }

    /// Toggle selection of current item
    pub(crate) fn toggle_selection(&mut self) {
        self.toggle_index(self.selected);
    }

    /// Select all files
    pub(crate) fn select_all(&mut self) {
        self.selection.items.clear();
        for i in 0..self.entries.len() {
            if let Some(entry) = self.entries.get(i) {
                if entry.name != ".." {
                    self.selection.items.insert(i);
                }
            }
        }
    }

    /// Move down with selection
    pub(crate) fn move_down_with_selection(&mut self) {
        self.select_index(self.selected);
        self.move_down();
    }

    /// Move up with selection
    pub(crate) fn move_up_with_selection(&mut self) {
        self.select_index(self.selected);
        self.move_up();
    }

    /// Page down with selection
    pub(crate) fn page_down_with_selection(&mut self) {
        let start = self.selected;
        let target =
            (self.selected + self.visible_height).min(self.entries.len().saturating_sub(1));
        for i in start..=target {
            self.select_index(i);
        }
        self.selected = target;
        self.adjust_scroll_offset(self.visible_height);
    }

    /// Page up with selection
    pub(crate) fn page_up_with_selection(&mut self) {
        let start = self.selected;
        let target = self.selected.saturating_sub(self.visible_height);
        for i in target..=start {
            self.select_index(i);
        }
        self.selected = target;
        self.scroll_offset = 0;
    }

    /// Select to beginning of list
    pub(crate) fn select_to_home(&mut self) {
        for i in 0..=self.selected {
            self.select_index(i);
        }
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Select to end of list
    pub(crate) fn select_to_end(&mut self) {
        let max_index = self.entries.len().saturating_sub(1);
        for i in self.selected..=max_index {
            self.select_index(i);
        }
        self.selected = max_index;
    }

    /// Get list of selected files/directories
    /// If nothing is selected, return current item under cursor
    pub fn get_selected_paths(&self) -> Vec<PathBuf> {
        if self.selection.items.is_empty() {
            // If no items are selected, return current one
            if let Some(entry) = self.entries.get(self.selected) {
                if entry.name != ".." {
                    return vec![self.current_path.join(&entry.name)];
                }
            }
            return Vec::new();
        }

        // Collect paths of selected items (pre-allocate capacity for efficiency)
        let mut paths = Vec::with_capacity(self.selection.items.len());
        for &idx in &self.selection.items {
            if let Some(entry) = self.entries.get(idx) {
                if entry.name != ".." {
                    paths.push(self.current_path.join(&entry.name));
                }
            }
        }
        paths
    }

    /// Get list of selected files/directories as VfsPath (for remote operations)
    /// If nothing is selected, return current item under cursor
    pub fn get_selected_vfs_paths(&self) -> Vec<VfsPath> {
        let base_path = self.vfs.current_path();

        if self.selection.items.is_empty() {
            // If no items are selected, return current one
            if let Some(entry) = self.entries.get(self.selected) {
                if entry.name != ".." {
                    return vec![base_path.join(&entry.name)];
                }
            }
            return Vec::new();
        }

        // Collect VFS paths of selected items (pre-allocate capacity for efficiency)
        let mut paths = Vec::with_capacity(self.selection.items.len());
        for &idx in &self.selection.items {
            if let Some(entry) = self.entries.get(idx) {
                if entry.name != ".." {
                    paths.push(base_path.join(&entry.name));
                }
            }
        }
        paths
    }

    /// Get count of selected items
    pub fn get_selected_count(&self) -> usize {
        self.selection.items.len()
    }

    /// Check if any selected entry is a directory
    /// If nothing is selected, check if current item under cursor is a directory
    pub fn has_selected_directories(&self) -> bool {
        if self.selection.items.is_empty() {
            // If no items are selected, check current one
            if let Some(entry) = self.entries.get(self.selected) {
                return entry.is_dir && entry.name != "..";
            }
            return false;
        }

        // Check if any selected item is a directory
        for &idx in &self.selection.items {
            if let Some(entry) = self.entries.get(idx) {
                if entry.is_dir && entry.name != ".." {
                    return true;
                }
            }
        }
        false
    }

    /// Clear file selection
    pub fn clear_selection(&mut self) {
        self.selection.items.clear();
    }

    /// Move down with toggle selection
    pub(crate) fn move_down_with_toggle(&mut self) {
        self.toggle_index(self.selected);
        self.move_down();
    }

    /// Move up with toggle selection
    pub(crate) fn move_up_with_toggle(&mut self) {
        self.toggle_index(self.selected);
        self.move_up();
    }

    /// Page down with toggle selection
    pub(crate) fn page_down_with_toggle(&mut self) {
        let start = self.selected;
        let target =
            (self.selected + self.visible_height).min(self.entries.len().saturating_sub(1));

        for i in start..=target {
            self.toggle_index(i);
        }

        self.selected = target;
        self.adjust_scroll_offset(self.visible_height);
    }

    /// Page up with toggle selection
    pub(crate) fn page_up_with_toggle(&mut self) {
        let start = self.selected;
        let target = self.selected.saturating_sub(self.visible_height);

        for i in target..=start {
            self.toggle_index(i);
        }

        self.selected = target;
        self.scroll_offset = 0;
    }
}
