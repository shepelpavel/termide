use super::FileManager;

/// Navigation state for directory traversal (cursor restoration, debouncing).
///
/// Groups related navigation fields together:
/// - `previous_dir_name`: Saved directory name when going up (for cursor restoration)
/// - `navigating_down`: Flag signaling entry into subdirectory (cursor resets to 0)
/// - `last_reload_time`: Timestamp for debouncing rapid reload_directory() calls
/// - `newly_created_item`: Name of newly created file/directory (for cursor positioning)
#[derive(Clone, Default)]
pub(crate) struct NavigationState {
    /// Name of directory we came from (for cursor restoration when going up)
    pub(crate) previous_dir_name: Option<String>,
    /// Flag indicating we're navigating down into a subdirectory (cursor should reset to 0)
    pub(crate) navigating_down: bool,
    /// Last reload time for debouncing rapid reload_directory() calls
    pub(crate) last_reload_time: Option<std::time::Instant>,
    /// Name of newly created item to navigate to after reload
    pub(crate) newly_created_item: Option<String>,
}

impl NavigationState {
    pub(crate) const fn new() -> Self {
        Self {
            previous_dir_name: None,
            navigating_down: false,
            last_reload_time: None,
            newly_created_item: None,
        }
    }

    pub(crate) fn save_for_going_up(&mut self, dir_name: String) {
        self.previous_dir_name = Some(dir_name);
    }

    pub(crate) fn prepare_for_going_down(&mut self) {
        self.previous_dir_name = None;
        self.navigating_down = true;
    }

    pub(crate) fn take_previous_dir_name(&mut self) -> Option<String> {
        self.previous_dir_name.take()
    }

    pub(crate) fn check_and_reset_navigating_down(&mut self) -> bool {
        if self.navigating_down {
            self.navigating_down = false;
            true
        } else {
            false
        }
    }

    pub(crate) fn should_reload(&mut self, debounce_ms: u128) -> bool {
        let now = std::time::Instant::now();
        if let Some(last) = self.last_reload_time {
            if now.duration_since(last).as_millis() < debounce_ms {
                return false;
            }
        }
        self.last_reload_time = Some(now);
        true
    }

    pub(crate) fn set_newly_created(&mut self, name: String) {
        self.newly_created_item = Some(name);
    }

    pub(crate) fn take_newly_created(&mut self) -> Option<String> {
        self.newly_created_item.take()
    }
}

impl FileManager {
    /// Move cursor down
    pub(crate) fn move_down(&mut self) {
        if self.selected < self.visible_count().saturating_sub(1) {
            self.selected += 1;
        }
    }

    /// Move cursor up
    pub(crate) fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Adjust scroll_offset for current item visibility
    pub(crate) fn adjust_scroll_offset(&mut self, visible_height: usize) {
        if self.selected >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected - visible_height + 1;
        } else if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        }
    }
}
