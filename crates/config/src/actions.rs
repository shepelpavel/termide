//! Actions directory scanning for user-defined scripts.
//!
//! Scans `~/.config/termide/actions/` for executable scripts and organizes them
//! into a registry for the Actions menu.

use std::path::PathBuf;

use super::get_config_dir;

/// A single action item (script file).
#[derive(Debug, Clone)]
pub struct ActionItem {
    /// Display name (filename before first dot).
    pub name: String,
    /// Full path to the script file.
    pub path: PathBuf,
    /// Whether this is a background action (contains `.bg.` in filename).
    pub is_background: bool,
    /// Whether this action shows result in modal (contains `.report.` in filename).
    pub is_report: bool,
}

/// A group of actions (subdirectory).
#[derive(Debug, Clone)]
pub struct ActionGroup {
    /// Group name (subdirectory name).
    pub name: String,
    /// Actions in this group.
    pub items: Vec<ActionItem>,
}

/// Registry of all available actions.
#[derive(Debug, Clone, Default)]
pub struct ActionsRegistry {
    /// Actions in the root directory.
    pub root_items: Vec<ActionItem>,
    /// Action groups (subdirectories).
    pub groups: Vec<ActionGroup>,
}

impl ActionItem {
    /// Create a new action item from a file path.
    fn from_path(path: PathBuf) -> Option<Self> {
        let file_name = path.file_name()?.to_str()?;

        // Check if this is a background action (contains .bg. in name)
        let is_background = file_name.contains(".bg.");

        // Check if this is a report action (contains .report. in name)
        let is_report = file_name.contains(".report.");

        // Extract display name (part before first dot)
        let name = file_name
            .split('.')
            .next()
            .filter(|s| !s.is_empty())?
            .to_string();

        Some(Self {
            name,
            path,
            is_background,
            is_report,
        })
    }
}

impl ActionsRegistry {
    /// Load actions from the actions directory.
    ///
    /// Returns None if the directory doesn't exist or can't be read.
    /// Returns empty registry if directory exists but is empty.
    pub fn load() -> Option<Self> {
        let actions_dir = get_config_dir().ok()?.join("actions");

        if !actions_dir.exists() {
            return Some(Self::default());
        }

        let mut registry = Self::default();

        let entries = std::fs::read_dir(&actions_dir).ok()?;

        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_file() && Self::is_executable(&path) {
                if let Some(item) = ActionItem::from_path(path) {
                    registry.root_items.push(item);
                }
            } else if path.is_dir() {
                // Only one level of subdirectories allowed
                if let Some(group) = Self::load_group(&path) {
                    if !group.items.is_empty() {
                        registry.groups.push(group);
                    }
                }
            }
        }

        // Sort for consistent ordering
        registry.root_items.sort_by(|a, b| a.name.cmp(&b.name));
        registry.groups.sort_by(|a, b| a.name.cmp(&b.name));

        Some(registry)
    }

    /// Load a group of actions from a subdirectory.
    fn load_group(dir: &PathBuf) -> Option<ActionGroup> {
        let name = dir.file_name()?.to_str()?.to_string();

        let entries = std::fs::read_dir(dir).ok()?;

        let mut items: Vec<ActionItem> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.is_file() && Self::is_executable(&path) {
                    ActionItem::from_path(path)
                } else {
                    None
                }
            })
            .collect();

        items.sort_by(|a, b| a.name.cmp(&b.name));

        Some(ActionGroup { name, items })
    }

    /// Check if a file is executable (has execute permission on Unix).
    #[cfg(unix)]
    fn is_executable(path: &PathBuf) -> bool {
        use std::os::unix::fs::PermissionsExt;

        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    fn is_executable(path: &PathBuf) -> bool {
        // On non-Unix, check for common script extensions
        let extensions = ["sh", "bat", "cmd", "ps1", "py", "rb", "pl"];
        path.extension()
            .and_then(OsStr::to_str)
            .map(|ext| extensions.contains(&ext.to_lowercase().as_str()))
            .unwrap_or(false)
    }

    /// Check if the registry has any actions.
    pub fn is_empty(&self) -> bool {
        self.root_items.is_empty() && self.groups.is_empty()
    }

    /// Get total number of actions (including those in groups).
    pub fn total_count(&self) -> usize {
        self.root_items.len() + self.groups.iter().map(|g| g.items.len()).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_item_from_path() {
        let item = ActionItem::from_path(PathBuf::from("/path/to/script.sh")).unwrap();
        assert_eq!(item.name, "script");
        assert!(!item.is_background);
        assert!(!item.is_report);

        let item = ActionItem::from_path(PathBuf::from("/path/to/deploy.bg.sh")).unwrap();
        assert_eq!(item.name, "deploy");
        assert!(item.is_background);
        assert!(!item.is_report);

        let item = ActionItem::from_path(PathBuf::from("/path/to/my.cool.script.bg.sh")).unwrap();
        assert_eq!(item.name, "my");
        assert!(item.is_background);
        assert!(!item.is_report);

        let item = ActionItem::from_path(PathBuf::from("/path/to/check.report.sh")).unwrap();
        assert_eq!(item.name, "check");
        assert!(!item.is_background);
        assert!(item.is_report);
    }

    #[test]
    fn test_action_item_no_extension() {
        let item = ActionItem::from_path(PathBuf::from("/path/to/myscript")).unwrap();
        assert_eq!(item.name, "myscript");
        assert!(!item.is_background);
        assert!(!item.is_report);
    }

    #[test]
    fn test_action_item_hidden_file() {
        // Hidden files (starting with dot) should have empty name before first dot
        let item = ActionItem::from_path(PathBuf::from("/path/to/.hidden"));
        assert!(item.is_none());
    }
}
