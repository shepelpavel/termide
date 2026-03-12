use std::path::PathBuf;

use super::FileManager;
use termide_vfs::VfsPath;

impl FileManager {
    /// Check if entry at visible index is ".." (not selectable)
    fn is_parent_entry(&self, vis_idx: usize) -> bool {
        self.entry_at(vis_idx).is_some_and(|e| e.name == "..")
    }

    /// Insert visible index into selection, skipping ".."
    fn select_index(&mut self, vis_idx: usize) {
        if !self.is_parent_entry(vis_idx) {
            self.selection.items.insert(vis_idx);
        }
    }

    /// Toggle visible index in selection, skipping ".."
    fn toggle_index(&mut self, vis_idx: usize) {
        if self.is_parent_entry(vis_idx) {
            return;
        }
        if self.selection.items.contains(&vis_idx) {
            self.selection.items.remove(&vis_idx);
        } else {
            self.selection.items.insert(vis_idx);
        }
    }

    /// Toggle selection of current item
    pub(crate) fn toggle_selection(&mut self) {
        self.toggle_index(self.selected);
    }

    /// Select all visible files
    pub(crate) fn select_all(&mut self) {
        self.selection.items.clear();
        for vis_idx in 0..self.visible_count() {
            if let Some(entry) = self.entry_at(vis_idx) {
                if entry.name != ".." {
                    self.selection.items.insert(vis_idx);
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
            (self.selected + self.visible_height).min(self.visible_count().saturating_sub(1));
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
        let max_index = self.visible_count().saturating_sub(1);
        for i in self.selected..=max_index {
            self.select_index(i);
        }
        self.selected = max_index;
    }

    /// Get list of selected files/directories as absolute paths.
    /// If nothing is selected, return current item under cursor.
    pub fn get_selected_paths(&self) -> Vec<PathBuf> {
        if self.selection.items.is_empty() {
            if let Some(te) = self.tree_entry_at(self.selected) {
                if te.file_entry.name != ".." {
                    return vec![te.full_path.clone()];
                }
            }
            return Vec::new();
        }

        let mut paths = Vec::with_capacity(self.selection.items.len());
        for &vis_idx in &self.selection.items {
            if let Some(te) = self.tree_entry_at(vis_idx) {
                if te.file_entry.name != ".." {
                    paths.push(te.full_path.clone());
                }
            }
        }
        paths
    }

    /// Get list of selected files/directories as VfsPath (for remote operations).
    /// If nothing is selected, return current item under cursor.
    pub fn get_selected_vfs_paths(&self) -> Vec<VfsPath> {
        let base_path = self.vfs.current_path();

        if self.selection.items.is_empty() {
            if let Some(entry) = self.entry_at(self.selected) {
                if entry.name != ".." {
                    return vec![base_path.join(&entry.name)];
                }
            }
            return Vec::new();
        }

        let mut paths = Vec::with_capacity(self.selection.items.len());
        for &vis_idx in &self.selection.items {
            if let Some(entry) = self.entry_at(vis_idx) {
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

    /// Check if any selected entry is a directory.
    /// If nothing is selected, check if current item under cursor is a directory.
    pub fn has_selected_directories(&self) -> bool {
        if self.selection.items.is_empty() {
            if let Some(entry) = self.entry_at(self.selected) {
                return entry.is_dir && entry.name != "..";
            }
            return false;
        }

        for &vis_idx in &self.selection.items {
            if let Some(entry) = self.entry_at(vis_idx) {
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
            (self.selected + self.visible_height).min(self.visible_count().saturating_sub(1));

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
