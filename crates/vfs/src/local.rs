//! Local filesystem VFS provider.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use crate::error::{VfsError, VfsResult};
use crate::traits::{DiskSpace, VfsProvider};
use crate::types::{
    AuthMethod, ConnectOptions, ConnectionState, VfsEntry, VfsMetadata, VfsOperation, VfsPath,
};

/// Local filesystem provider.
///
/// This provider wraps the standard library's filesystem operations
/// and implements the VfsProvider trait for consistency with remote providers.
#[derive(Default)]
pub struct LocalFileSystem {
    /// Always connected for local filesystem.
    connected: bool,
}

impl LocalFileSystem {
    /// Create a new local filesystem provider.
    pub fn new() -> Self {
        Self { connected: true }
    }

    /// Convert VfsPath to local PathBuf, validating it's a local path.
    fn to_local_path(path: &VfsPath) -> VfsResult<&Path> {
        if !path.is_local() {
            return Err(VfsError::InvalidPath(format!(
                "Expected local path, got: {}",
                path
            )));
        }
        Ok(&path.path)
    }

    /// Convert fs::Metadata and path to VfsEntry.
    fn metadata_to_entry(name: &str, path: &VfsPath, metadata: fs::Metadata) -> VfsEntry {
        VfsEntry::new(name, path.clone(), VfsMetadata::from(metadata))
    }

    /// Read directory entries (internal helper).
    fn read_dir_entries(path: &Path) -> VfsResult<Vec<VfsEntry>> {
        let vfs_path = VfsPath::local(path);
        let mut entries = Vec::new();

        let read_dir = fs::read_dir(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                VfsError::NotFound {
                    path: path.to_path_buf(),
                }
            } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                VfsError::PermissionDenied {
                    path: path.to_path_buf(),
                }
            } else {
                VfsError::Io(e)
            }
        })?;

        for entry in read_dir {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let entry_path = vfs_path.join(&name);

            // Get metadata, handling symlinks
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("Failed to get metadata for {:?}: {}", entry.path(), e);
                    continue;
                }
            };

            entries.push(Self::metadata_to_entry(&name, &entry_path, metadata));
        }

        // Sort: directories first, then by name
        entries.sort_by(|a, b| match (a.is_dir(), b.is_dir()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        Ok(entries)
    }

    /// Copy directory recursively (internal helper).
    fn copy_dir_recursive(src: &Path, dst: &Path, depth: usize) -> VfsResult<()> {
        const MAX_DEPTH: usize = 100;

        if depth > MAX_DEPTH {
            return Err(VfsError::RemoteError {
                message: format!("Directory nesting too deep (> {})", MAX_DEPTH),
            });
        }

        fs::create_dir_all(dst)?;

        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let entry_path = entry.path();
            let dest_path = dst.join(entry.file_name());

            let metadata = fs::symlink_metadata(&entry_path)?;

            if metadata.is_symlink() {
                // Copy symlink
                #[cfg(unix)]
                {
                    let target = fs::read_link(&entry_path)?;
                    std::os::unix::fs::symlink(target, &dest_path)?;
                }
                #[cfg(not(unix))]
                {
                    // On Windows, copy as regular file
                    fs::copy(&entry_path, &dest_path)?;
                }
            } else if metadata.is_dir() {
                Self::copy_dir_recursive(&entry_path, &dest_path, depth + 1)?;
            } else {
                fs::copy(&entry_path, &dest_path)?;
            }
        }

        Ok(())
    }

    /// Delete directory recursively (internal helper).
    fn delete_dir_recursive(path: &Path) -> VfsResult<()> {
        fs::remove_dir_all(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                VfsError::NotFound {
                    path: path.to_path_buf(),
                }
            } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                VfsError::PermissionDenied {
                    path: path.to_path_buf(),
                }
            } else {
                VfsError::Io(e)
            }
        })
    }
}

impl VfsProvider for LocalFileSystem {
    fn name(&self) -> &'static str {
        "local"
    }

    fn connection_state(&self) -> ConnectionState {
        if self.connected {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        }
    }

    fn connect(&mut self, _options: ConnectOptions) -> VfsOperation<()> {
        // Local filesystem is always connected
        self.connected = true;
        VfsOperation::ready(Ok(()))
    }

    fn disconnect(&mut self) {
        // No-op for local filesystem
        self.connected = false;
    }

    fn list_dir(&self, path: &VfsPath) -> VfsOperation<Vec<VfsEntry>> {
        let local_path = match Self::to_local_path(path) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };

        // Run in thread to match async pattern
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = Self::read_dir_entries(&local_path);
            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn create_dir(&self, path: &VfsPath) -> VfsOperation<()> {
        let local_path = match Self::to_local_path(path) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = fs::create_dir(&local_path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    VfsError::AlreadyExists { path: local_path }
                } else {
                    VfsError::Io(e)
                }
            });
            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn create_dir_all(&self, path: &VfsPath) -> VfsOperation<()> {
        let local_path = match Self::to_local_path(path) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = fs::create_dir_all(&local_path).map_err(VfsError::Io);
            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn exists(&self, path: &VfsPath) -> VfsOperation<bool> {
        let local_path = match Self::to_local_path(path) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };

        // Simple synchronous check, wrapped in operation for consistency
        VfsOperation::ready(Ok(local_path.exists()))
    }

    fn metadata(&self, path: &VfsPath) -> VfsOperation<VfsMetadata> {
        let local_path = match Self::to_local_path(path) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = fs::metadata(&local_path)
                .map(VfsMetadata::from)
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        VfsError::NotFound { path: local_path }
                    } else {
                        VfsError::Io(e)
                    }
                });
            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn read_file(&self, path: &VfsPath) -> VfsOperation<Vec<u8>> {
        let local_path = match Self::to_local_path(path) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = fs::read(&local_path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    VfsError::NotFound { path: local_path }
                } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                    VfsError::PermissionDenied { path: local_path }
                } else {
                    VfsError::Io(e)
                }
            });
            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn write_file(&self, path: &VfsPath, data: &[u8]) -> VfsOperation<()> {
        let local_path = match Self::to_local_path(path) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };

        let data = data.to_vec();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = fs::write(&local_path, data).map_err(|e| {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    VfsError::PermissionDenied { path: local_path }
                } else {
                    VfsError::Io(e)
                }
            });
            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn delete(&self, path: &VfsPath) -> VfsOperation<()> {
        let local_path = match Self::to_local_path(path) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = if local_path.is_dir() {
                fs::remove_dir(&local_path)
            } else {
                fs::remove_file(&local_path)
            }
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    VfsError::NotFound {
                        path: local_path.clone(),
                    }
                } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                    VfsError::PermissionDenied { path: local_path }
                } else {
                    VfsError::Io(e)
                }
            });
            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn delete_recursive(&self, path: &VfsPath) -> VfsOperation<()> {
        let local_path = match Self::to_local_path(path) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = Self::delete_dir_recursive(&local_path);
            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn rename(&self, from: &VfsPath, to: &VfsPath) -> VfsOperation<()> {
        let from_path = match Self::to_local_path(from) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };
        let to_path = match Self::to_local_path(to) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = fs::rename(&from_path, &to_path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    VfsError::NotFound { path: from_path }
                } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                    VfsError::PermissionDenied { path: from_path }
                } else {
                    VfsError::Io(e)
                }
            });
            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn copy(&self, from: &VfsPath, to: &VfsPath) -> VfsOperation<()> {
        let from_path = match Self::to_local_path(from) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };
        let to_path = match Self::to_local_path(to) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = if from_path.is_dir() {
                Self::copy_dir_recursive(&from_path, &to_path, 0)
            } else {
                fs::copy(&from_path, &to_path).map(|_| ()).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        VfsError::NotFound { path: from_path }
                    } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                        VfsError::PermissionDenied { path: from_path }
                    } else {
                        VfsError::Io(e)
                    }
                })
            };
            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn download(&self, remote: &VfsPath, local: &Path) -> VfsOperation<PathBuf> {
        // For local filesystem, "download" is just a copy
        let from_path = match Self::to_local_path(remote) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };
        let to_path = local.to_path_buf();

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = fs::copy(&from_path, &to_path)
                .map(|_| to_path.clone())
                .map_err(VfsError::Io);
            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn upload(&self, local: &Path, remote: &VfsPath) -> VfsOperation<()> {
        // For local filesystem, "upload" is just a copy
        let from_path = local.to_path_buf();
        let to_path = match Self::to_local_path(remote) {
            Ok(p) => p.to_path_buf(),
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = fs::copy(&from_path, &to_path)
                .map(|_| ())
                .map_err(VfsError::Io);
            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn supported_auth_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::None]
    }

    fn supports_recursive(&self) -> bool {
        true
    }

    fn home_dir(&self) -> Option<VfsPath> {
        dirs::home_dir().map(VfsPath::local)
    }

    fn disk_space(&self, path: &VfsPath) -> Option<DiskSpace> {
        #[cfg(unix)]
        {
            use std::ffi::CString;
            use std::mem::MaybeUninit;

            let local_path = Self::to_local_path(path).ok()?;
            let c_path = CString::new(local_path.to_str()?).ok()?;

            unsafe {
                let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
                if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) == 0 {
                    let stat = stat.assume_init();
                    let block_size = stat.f_frsize;
                    let total = stat.f_blocks * block_size;
                    let free = stat.f_bfree * block_size;
                    let used = total - free;
                    return Some(DiskSpace { total, free, used });
                }
            }
            None
        }

        #[cfg(not(unix))]
        {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::VfsFileType;
    use tempfile::TempDir;

    fn create_test_provider() -> (LocalFileSystem, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let provider = LocalFileSystem::new();
        (provider, temp_dir)
    }

    #[test]
    fn test_local_provider_name() {
        let provider = LocalFileSystem::new();
        assert_eq!(provider.name(), "local");
    }

    #[test]
    fn test_local_provider_always_connected() {
        let provider = LocalFileSystem::new();
        assert!(provider.is_connected());
    }

    #[test]
    fn test_list_dir() {
        let (provider, temp_dir) = create_test_provider();

        // Create test files
        fs::write(temp_dir.path().join("file1.txt"), "content1").unwrap();
        fs::write(temp_dir.path().join("file2.txt"), "content2").unwrap();
        fs::create_dir(temp_dir.path().join("subdir")).unwrap();

        let path = VfsPath::local(temp_dir.path());
        let entries = provider.list_dir(&path).recv().unwrap();

        assert_eq!(entries.len(), 3);
        // Directories should be first
        assert!(entries[0].is_dir());
        assert_eq!(entries[0].name, "subdir");
    }

    #[test]
    fn test_read_write_file() {
        let (provider, temp_dir) = create_test_provider();

        let file_path = VfsPath::local(temp_dir.path().join("test.txt"));
        let content = b"Hello, VFS!";

        // Write
        provider.write_file(&file_path, content).recv().unwrap();

        // Read
        let read_content = provider.read_file(&file_path).recv().unwrap();
        assert_eq!(read_content, content);
    }

    #[test]
    fn test_create_and_delete_dir() {
        let (provider, temp_dir) = create_test_provider();

        let dir_path = VfsPath::local(temp_dir.path().join("newdir"));

        // Create
        provider.create_dir(&dir_path).recv().unwrap();
        assert!(provider.exists(&dir_path).recv().unwrap());

        // Delete
        provider.delete(&dir_path).recv().unwrap();
        assert!(!provider.exists(&dir_path).recv().unwrap());
    }

    #[test]
    fn test_copy_file() {
        let (provider, temp_dir) = create_test_provider();

        let src = VfsPath::local(temp_dir.path().join("src.txt"));
        let dst = VfsPath::local(temp_dir.path().join("dst.txt"));

        provider.write_file(&src, b"copy me").recv().unwrap();
        provider.copy(&src, &dst).recv().unwrap();

        let content = provider.read_file(&dst).recv().unwrap();
        assert_eq!(content, b"copy me");
    }

    #[test]
    fn test_rename_file() {
        let (provider, temp_dir) = create_test_provider();

        let old = VfsPath::local(temp_dir.path().join("old.txt"));
        let new = VfsPath::local(temp_dir.path().join("new.txt"));

        provider.write_file(&old, b"content").recv().unwrap();
        provider.rename(&old, &new).recv().unwrap();

        assert!(!provider.exists(&old).recv().unwrap());
        assert!(provider.exists(&new).recv().unwrap());
    }

    #[test]
    fn test_metadata() {
        let (provider, temp_dir) = create_test_provider();

        let file_path = VfsPath::local(temp_dir.path().join("meta.txt"));
        provider.write_file(&file_path, b"12345").recv().unwrap();

        let meta = provider.metadata(&file_path).recv().unwrap();
        assert_eq!(meta.file_type, VfsFileType::File);
        assert_eq!(meta.size, 5);
    }

    #[test]
    fn test_home_dir() {
        let provider = LocalFileSystem::new();
        let home = provider.home_dir();
        assert!(home.is_some());
        assert!(home.unwrap().is_local());
    }
}
