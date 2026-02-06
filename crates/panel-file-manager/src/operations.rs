use anyhow::{bail, Result};
use std::fs;
use std::path::Component;

use super::FileManager;

/// Validate that a user-provided file/directory name does not escape the parent directory.
/// Rejects names containing `..`, absolute paths, and path separators.
fn validate_entry_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("Name cannot be empty");
    }

    let path = std::path::Path::new(name);

    // Reject absolute paths
    if path.is_absolute() {
        bail!("Absolute paths are not allowed");
    }

    // Reject any component that is `..` or contains path separators leading outside
    for component in path.components() {
        match component {
            Component::ParentDir => bail!("Path traversal ('..') is not allowed"),
            Component::RootDir | Component::Prefix(_) => {
                bail!("Absolute paths are not allowed")
            }
            _ => {}
        }
    }

    Ok(())
}

impl FileManager {
    /// Create a new file
    pub fn create_file(&mut self, name: String) -> Result<()> {
        validate_entry_name(&name)?;

        if self.vfs.is_remote() {
            // Remote path - use VFS
            let vfs_path = self.vfs.current_path();
            let new_path = vfs_path.join(&name);
            let operation = self.vfs.manager().write_file(&new_path, &[]);

            // Block until completion
            operation.recv()?;

            self.navigation.set_newly_created(name);
            self.load_directory()?;
        } else {
            // Local path - use std::fs
            let file_path = self.current_path.join(&name);
            fs::write(&file_path, "")?;
            // Navigate to newly created file
            self.navigation.set_newly_created(name);
            self.load_directory()?;
        }
        Ok(())
    }

    /// Create a new directory
    pub fn create_directory(&mut self, name: String) -> Result<()> {
        validate_entry_name(&name)?;

        if self.vfs.is_remote() {
            // Remote path - use VFS
            let vfs_path = self.vfs.current_path();
            let new_path = vfs_path.join(&name);
            let operation = self.vfs.manager().create_dir(&new_path);

            // Block until completion (sync behavior for UI)
            operation.recv()?;

            self.navigation.set_newly_created(name);
            self.load_directory()?;
        } else {
            // Local path - use std::fs
            let dir_path = self.current_path.join(&name);
            fs::create_dir(&dir_path)?;
            // Navigate to newly created directory
            self.navigation.set_newly_created(name);
            self.load_directory()?;
        }
        Ok(())
    }
}
