//! Scripts directory scanning for user-defined scripts.
//!
//! Scans `~/.local/share/termide/scripts/` for executable scripts and organizes them
//! into a registry for the Scripts menu.

#[cfg(not(unix))]
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::get_data_dir;

/// Unix permission bits indicating any execute permission (owner, group, other).
#[cfg(unix)]
const EXECUTABLE_MASK: u32 = 0o111;

/// A single script item (script file).
#[derive(Debug, Clone)]
pub struct ScriptItem {
    /// Display name (filename before first dot).
    pub name: String,
    /// Full path to the script file.
    pub path: PathBuf,
    /// Whether this is a background script (contains `.bg.` in filename).
    pub is_background: bool,
    /// Whether this script shows result in modal (contains `.report.` in filename).
    pub is_report: bool,
    /// Whether this script comes from a project-local `.termide/scripts/` directory.
    pub is_project: bool,
}

/// A group of scripts (subdirectory).
#[derive(Debug, Clone)]
pub struct ScriptGroup {
    /// Group name (subdirectory name).
    pub name: String,
    /// Scripts in this group.
    pub items: Vec<ScriptItem>,
    /// Whether this group comes from a project-local `.termide/scripts/` directory.
    pub is_project: bool,
}

/// Registry of all available scripts.
#[derive(Debug, Clone, Default)]
pub struct ScriptsRegistry {
    /// Scripts in the root directory.
    pub root_items: Vec<ScriptItem>,
    /// Script groups (subdirectories).
    pub groups: Vec<ScriptGroup>,
}

impl ScriptItem {
    /// Create a new script item from a file path.
    fn from_path(path: PathBuf) -> Option<Self> {
        let file_name = path.file_name()?.to_str()?;

        // Check if this is a background script (contains .bg. in name)
        let is_background = file_name.contains(".bg.");

        // Check if this is a report script (contains .report. in name)
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
            is_project: false,
        })
    }
}

impl ScriptsRegistry {
    /// Find a root-level script by name.
    pub fn find_script_by_name(&self, name: &str) -> Option<&ScriptItem> {
        self.root_items.iter().find(|s| s.name == name)
    }

    /// Load scripts from the global scripts directory.
    ///
    /// Returns None if the directory doesn't exist or can't be read.
    /// Returns empty registry if directory exists but is empty.
    pub fn load() -> Option<Self> {
        let scripts_dir = get_data_dir().ok()?.join("scripts");
        Self::load_from_dir(&scripts_dir)
    }

    /// Load scripts from a specific directory.
    pub fn load_from_dir(scripts_dir: &Path) -> Option<Self> {
        if !scripts_dir.exists() {
            return Some(Self::default());
        }

        let canonical_scripts_dir = std::fs::canonicalize(scripts_dir).ok()?;

        let mut registry = Self::default();

        let entries = std::fs::read_dir(scripts_dir).ok()?;

        for entry in entries.flatten() {
            let path = entry.path();

            // Validate symlinks: resolved path must stay within scripts_dir
            if path.is_symlink() {
                if let Ok(resolved) = std::fs::canonicalize(&path) {
                    if !resolved.starts_with(&canonical_scripts_dir) {
                        continue;
                    }
                } else {
                    continue; // broken symlink
                }
            }

            if path.is_file() && Self::is_executable(&path) {
                if let Some(item) = ScriptItem::from_path(path) {
                    registry.root_items.push(item);
                }
            } else if path.is_dir() {
                // Only one level of subdirectories allowed
                if let Some(group) = Self::load_group(&path, &canonical_scripts_dir) {
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

    /// Load and merge scripts from global and project-local directories.
    /// Project items appear first and are marked with `is_project = true`.
    pub fn load_merged(project_root: Option<&Path>) -> Option<Self> {
        let mut registry = Self::load().unwrap_or_default();

        if let Some(root) = project_root {
            let project_scripts_dir = root.join(".termide").join("scripts");
            if project_scripts_dir.exists() {
                if let Some(mut project) = Self::load_from_dir(&project_scripts_dir) {
                    // Mark all project items
                    for item in &mut project.root_items {
                        item.is_project = true;
                    }
                    for group in &mut project.groups {
                        group.is_project = true;
                        for item in &mut group.items {
                            item.is_project = true;
                        }
                    }
                    // Prepend project items before global
                    project.root_items.append(&mut registry.root_items);
                    project.groups.append(&mut registry.groups);
                    registry = project;
                }
            }
        }

        Some(registry)
    }

    /// Load a group of scripts from a subdirectory.
    fn load_group(dir: &PathBuf, canonical_scripts_dir: &std::path::Path) -> Option<ScriptGroup> {
        let name = dir.file_name()?.to_str()?.to_string();

        let entries = std::fs::read_dir(dir).ok()?;

        let mut items: Vec<ScriptItem> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();

                // Validate symlinks: resolved path must stay within scripts_dir
                if path.is_symlink() {
                    if let Ok(resolved) = std::fs::canonicalize(&path) {
                        if !resolved.starts_with(canonical_scripts_dir) {
                            return None;
                        }
                    } else {
                        return None; // broken symlink
                    }
                }

                if path.is_file() && Self::is_executable(&path) {
                    ScriptItem::from_path(path)
                } else {
                    None
                }
            })
            .collect();

        items.sort_by(|a, b| a.name.cmp(&b.name));

        Some(ScriptGroup {
            name,
            items,
            is_project: false,
        })
    }

    /// Check if a file is executable (has execute permission on Unix).
    #[cfg(unix)]
    fn is_executable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;

        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & EXECUTABLE_MASK != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    fn is_executable(path: &Path) -> bool {
        // On non-Unix, check for common script extensions
        let extensions = ["sh", "bat", "cmd", "ps1", "py", "rb", "pl"];
        path.extension()
            .and_then(OsStr::to_str)
            .map(|ext: &str| extensions.contains(&ext.to_lowercase().as_str()))
            .unwrap_or(false)
    }

    /// Check if the registry has any scripts.
    pub fn is_empty(&self) -> bool {
        self.root_items.is_empty() && self.groups.is_empty()
    }

    /// Get total number of scripts (including those in groups).
    pub fn total_count(&self) -> usize {
        self.root_items.len() + self.groups.iter().map(|g| g.items.len()).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_item_from_path() {
        let item = ScriptItem::from_path(PathBuf::from("/path/to/script.sh")).unwrap();
        assert_eq!(item.name, "script");
        assert!(!item.is_background);
        assert!(!item.is_report);

        let item = ScriptItem::from_path(PathBuf::from("/path/to/deploy.bg.sh")).unwrap();
        assert_eq!(item.name, "deploy");
        assert!(item.is_background);
        assert!(!item.is_report);

        let item = ScriptItem::from_path(PathBuf::from("/path/to/my.cool.script.bg.sh")).unwrap();
        assert_eq!(item.name, "my");
        assert!(item.is_background);
        assert!(!item.is_report);

        let item = ScriptItem::from_path(PathBuf::from("/path/to/check.report.sh")).unwrap();
        assert_eq!(item.name, "check");
        assert!(!item.is_background);
        assert!(item.is_report);
    }

    #[test]
    fn test_script_item_no_extension() {
        let item = ScriptItem::from_path(PathBuf::from("/path/to/myscript")).unwrap();
        assert_eq!(item.name, "myscript");
        assert!(!item.is_background);
        assert!(!item.is_report);
    }

    #[test]
    fn test_script_item_hidden_file() {
        // Hidden files (starting with dot) should have empty name before first dot
        let item = ScriptItem::from_path(PathBuf::from("/path/to/.hidden"));
        assert!(item.is_none());
    }
}
