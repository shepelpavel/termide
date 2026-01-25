// Allow clippy lints for file operations
#![allow(clippy::only_used_in_recursion)]

use anyhow::Result;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use super::FileManager;
use termide_ui::path_utils;

/// Progress information for file copy operation
#[derive(Debug, Clone)]
pub struct CopyProgress {
    pub bytes_copied: u64,
    pub total_bytes: u64,
}

/// Handle for an ongoing copy operation
pub struct CopyOperation {
    /// Receiver for operation completion result
    pub completion: mpsc::Receiver<Result<PathBuf>>,
    /// Receiver for progress updates
    pub progress: mpsc::Receiver<CopyProgress>,
    /// Flag to pause the operation
    pub pause_flag: Arc<AtomicBool>,
    /// Flag to cancel the operation
    pub cancel_flag: Arc<AtomicBool>,
}

impl std::fmt::Debug for CopyOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopyOperation")
            .field("pause_flag", &self.pause_flag.load(Ordering::Relaxed))
            .field("cancel_flag", &self.cancel_flag.load(Ordering::Relaxed))
            .finish()
    }
}

/// Copy a file with progress reporting (chunked copy in background thread)
pub fn copy_file_with_progress(source: &Path, dest: &Path) -> CopyOperation {
    let source = source.to_path_buf();
    let dest = dest.to_path_buf();

    let (tx_complete, rx_complete) = mpsc::channel();
    let (tx_progress, rx_progress) = mpsc::channel();

    let pause_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag = Arc::new(AtomicBool::new(false));

    let pause_flag_clone = Arc::clone(&pause_flag);
    let cancel_flag_clone = Arc::clone(&cancel_flag);

    thread::spawn(move || {
        let result = (|| -> Result<PathBuf> {
            // Open source file
            let mut source_file = fs::File::open(&source)?;
            let metadata = source_file.metadata()?;
            let total_bytes = metadata.len();

            // Create destination file
            let mut dest_file = fs::File::create(&dest)?;

            // Copy in chunks
            const CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks
            let mut buffer = vec![0u8; CHUNK_SIZE];
            let mut copied_bytes = 0u64;

            loop {
                // Check if operation was cancelled
                if cancel_flag_clone.load(Ordering::Relaxed) {
                    // Don't delete file - let user decide
                    drop(dest_file);
                    return Err(anyhow::anyhow!(
                        "Operation cancelled by user (partial file kept)"
                    ));
                }

                // Wait while paused
                while pause_flag_clone.load(Ordering::Relaxed) {
                    // Check for cancel while paused
                    if cancel_flag_clone.load(Ordering::Relaxed) {
                        drop(dest_file);
                        return Err(anyhow::anyhow!(
                            "Operation cancelled by user (partial file kept)"
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }

                let bytes_read = source_file.read(&mut buffer)?;
                if bytes_read == 0 {
                    break;
                }

                dest_file.write_all(&buffer[..bytes_read])?;
                copied_bytes += bytes_read as u64;

                // Send progress update (ignore errors if receiver dropped)
                let _ = tx_progress.send(CopyProgress {
                    bytes_copied: copied_bytes,
                    total_bytes,
                });
            }

            Ok(dest.clone())
        })();

        // Send completion result
        let _ = tx_complete.send(result);
    });

    CopyOperation {
        completion: rx_complete,
        progress: rx_progress,
        pause_flag,
        cancel_flag,
    }
}

/// Result of directory scan
#[derive(Debug, Clone)]
pub struct DirectoryScanResult {
    /// All files in the directory tree
    pub files: Vec<PathBuf>,
    /// Total size of all files in bytes
    pub total_bytes: u64,
}

/// Progress information during directory scan
#[derive(Debug, Clone)]
pub struct ScanProgress {
    /// Files found so far
    pub files_count: usize,
    /// Total bytes found so far
    pub total_bytes: u64,
    /// Current directory being scanned
    pub current_dir: PathBuf,
}

/// Handle for an ongoing scan operation
pub struct ScanOperation {
    /// Receiver for operation completion result
    pub completion: mpsc::Receiver<Result<DirectoryScanResult>>,
    /// Receiver for progress updates
    pub progress: mpsc::Receiver<ScanProgress>,
    /// Flag to cancel the operation
    pub cancel_flag: Arc<AtomicBool>,
}

impl std::fmt::Debug for ScanOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScanOperation")
            .field("cancel_flag", &self.cancel_flag.load(Ordering::Relaxed))
            .finish()
    }
}

/// Scan directory asynchronously with progress reporting
pub fn scan_directory_async(path: &Path) -> ScanOperation {
    let path = path.to_path_buf();

    let (tx_complete, rx_complete) = mpsc::channel();
    let (tx_progress, rx_progress) = mpsc::channel();

    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_clone = Arc::clone(&cancel_flag);

    thread::spawn(move || {
        let result = scan_directory_with_progress_inner(&path, &tx_progress, &cancel_flag_clone, 0);
        let _ = tx_complete.send(result);
    });

    ScanOperation {
        completion: rx_complete,
        progress: rx_progress,
        cancel_flag,
    }
}

/// Internal recursive scan with progress
fn scan_directory_with_progress_inner(
    path: &Path,
    tx_progress: &mpsc::Sender<ScanProgress>,
    cancel_flag: &Arc<AtomicBool>,
    depth: usize,
) -> Result<DirectoryScanResult> {
    const MAX_DEPTH: usize = termide_ui::constants::MAX_DIRECTORY_COPY_DEPTH;

    if depth > MAX_DEPTH {
        return Err(anyhow::anyhow!(
            "Directory nesting too deep (> {})",
            MAX_DEPTH
        ));
    }

    // Check for cancellation
    if cancel_flag.load(Ordering::Relaxed) {
        return Err(anyhow::anyhow!("Scan cancelled by user"));
    }

    let mut files = Vec::new();
    let mut total_bytes = 0u64;

    for entry in fs::read_dir(path)? {
        // Check for cancellation
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!("Scan cancelled by user"));
        }

        let entry = entry?;
        let entry_path = entry.path();
        let metadata = fs::symlink_metadata(&entry_path)?;

        if metadata.is_dir() && !metadata.is_symlink() {
            // Send progress before recursing
            let _ = tx_progress.send(ScanProgress {
                files_count: files.len(),
                total_bytes,
                current_dir: entry_path.clone(),
            });

            // Recursively scan subdirectory
            let sub_result = scan_directory_with_progress_inner(
                &entry_path,
                tx_progress,
                cancel_flag,
                depth + 1,
            )?;
            files.extend(sub_result.files);
            total_bytes += sub_result.total_bytes;
        } else if metadata.is_file() || metadata.is_symlink() {
            // Add file to list
            files.push(entry_path);
            if !metadata.is_symlink() {
                total_bytes += metadata.len();
            }
        }
    }

    // Send final progress for this directory
    let _ = tx_progress.send(ScanProgress {
        files_count: files.len(),
        total_bytes,
        current_dir: path.to_path_buf(),
    });

    Ok(DirectoryScanResult { files, total_bytes })
}

/// Scan directory recursively to count files and total size
pub fn scan_directory(path: &Path) -> Result<DirectoryScanResult> {
    scan_directory_with_depth(path, 0)
}

/// Scan directory with depth limit
fn scan_directory_with_depth(path: &Path, depth: usize) -> Result<DirectoryScanResult> {
    const MAX_DEPTH: usize = termide_ui::constants::MAX_DIRECTORY_COPY_DEPTH;

    if depth > MAX_DEPTH {
        return Err(anyhow::anyhow!(
            "Directory nesting too deep (> {})",
            MAX_DEPTH
        ));
    }

    let mut files = Vec::new();
    let mut total_bytes = 0u64;

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        let metadata = fs::symlink_metadata(&entry_path)?;

        if metadata.is_dir() && !metadata.is_symlink() {
            // Recursively scan subdirectory
            let sub_result = scan_directory_with_depth(&entry_path, depth + 1)?;
            files.extend(sub_result.files);
            total_bytes += sub_result.total_bytes;
        } else if metadata.is_file() || metadata.is_symlink() {
            // Add file to list
            files.push(entry_path);
            if !metadata.is_symlink() {
                total_bytes += metadata.len();
            }
        }
    }

    Ok(DirectoryScanResult { files, total_bytes })
}

/// Progress for directory copy operation
#[derive(Debug, Clone)]
pub struct DirectoryCopyProgress {
    /// Total bytes copied so far (across all files)
    pub bytes_copied: u64,
    /// Total bytes to copy
    pub total_bytes: u64,
    /// Current file being copied
    pub current_file: PathBuf,
    /// Files completed
    pub files_completed: usize,
    /// Total files to copy
    pub total_files: usize,
}

/// Handle for an ongoing directory copy operation
pub struct DirectoryCopyOperation {
    /// Receiver for operation completion result
    pub completion: mpsc::Receiver<Result<PathBuf>>,
    /// Receiver for progress updates
    pub progress: mpsc::Receiver<DirectoryCopyProgress>,
    /// Flag to pause the operation
    pub pause_flag: Arc<AtomicBool>,
    /// Flag to cancel the operation
    pub cancel_flag: Arc<AtomicBool>,
}

impl std::fmt::Debug for DirectoryCopyOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectoryCopyOperation")
            .field("pause_flag", &self.pause_flag.load(Ordering::Relaxed))
            .field("cancel_flag", &self.cancel_flag.load(Ordering::Relaxed))
            .finish()
    }
}

/// Copy a directory with progress reporting (background thread)
pub fn copy_directory_with_progress(source: &Path, dest: &Path) -> Result<DirectoryCopyOperation> {
    // Scan directory first to get total size
    let scan_result = scan_directory(source)?;

    let source = source.to_path_buf();
    let dest = dest.to_path_buf();

    let (tx_complete, rx_complete) = mpsc::channel();
    let (tx_progress, rx_progress) = mpsc::channel();

    let pause_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag = Arc::new(AtomicBool::new(false));

    let pause_flag_clone = Arc::clone(&pause_flag);
    let cancel_flag_clone = Arc::clone(&cancel_flag);

    let total_bytes = scan_result.total_bytes;
    let total_files = scan_result.files.len();

    thread::spawn(move || {
        let result = (|| -> Result<PathBuf> {
            // Create destination directory
            fs::create_dir_all(&dest)?;

            let mut bytes_copied = 0u64;
            let mut files_completed = 0usize;

            // Copy each file
            for file_path in &scan_result.files {
                // Check if operation was cancelled
                if cancel_flag_clone.load(Ordering::Relaxed) {
                    return Err(anyhow::anyhow!(
                        "Operation cancelled by user (partial directory kept)"
                    ));
                }

                // Wait while paused
                while pause_flag_clone.load(Ordering::Relaxed) {
                    if cancel_flag_clone.load(Ordering::Relaxed) {
                        return Err(anyhow::anyhow!(
                            "Operation cancelled by user (partial directory kept)"
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }

                // Calculate relative path and destination
                let rel_path = file_path
                    .strip_prefix(&source)
                    .map_err(|e| anyhow::anyhow!("Failed to strip prefix: {}", e))?;
                let file_dest = dest.join(rel_path);

                // Create parent directories if needed
                if let Some(parent) = file_dest.parent() {
                    fs::create_dir_all(parent)?;
                }

                // Send progress update for current file
                let _ = tx_progress.send(DirectoryCopyProgress {
                    bytes_copied,
                    total_bytes,
                    current_file: file_path.clone(),
                    files_completed,
                    total_files,
                });

                // Copy file (handle symlinks)
                let metadata = fs::symlink_metadata(file_path)?;
                if metadata.is_symlink() {
                    // Copy symlink
                    #[cfg(unix)]
                    {
                        let link_target = fs::read_link(file_path)?;
                        std::os::unix::fs::symlink(&link_target, &file_dest)?;
                    }
                    #[cfg(windows)]
                    {
                        let link_target = fs::read_link(file_path)?;
                        if link_target.is_dir() {
                            std::os::windows::fs::symlink_dir(&link_target, &file_dest)?;
                        } else {
                            std::os::windows::fs::symlink_file(&link_target, &file_dest)?;
                        }
                    }
                } else {
                    // Copy regular file with chunked progress
                    let file_size = metadata.len();
                    let mut source_file = fs::File::open(file_path)?;
                    let mut dest_file = fs::File::create(&file_dest)?;

                    const CHUNK_SIZE: usize = 1024 * 1024; // 1MB
                    let mut buffer = vec![0u8; CHUNK_SIZE];
                    let mut file_bytes_copied = 0u64;

                    loop {
                        // Check cancel/pause
                        if cancel_flag_clone.load(Ordering::Relaxed) {
                            return Err(anyhow::anyhow!(
                                "Operation cancelled by user (partial directory kept)"
                            ));
                        }
                        while pause_flag_clone.load(Ordering::Relaxed) {
                            if cancel_flag_clone.load(Ordering::Relaxed) {
                                return Err(anyhow::anyhow!(
                                    "Operation cancelled by user (partial directory kept)"
                                ));
                            }
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }

                        let bytes_read = source_file.read(&mut buffer)?;
                        if bytes_read == 0 {
                            break;
                        }

                        dest_file.write_all(&buffer[..bytes_read])?;
                        file_bytes_copied += bytes_read as u64;

                        // Send progress update
                        let _ = tx_progress.send(DirectoryCopyProgress {
                            bytes_copied: bytes_copied + file_bytes_copied,
                            total_bytes,
                            current_file: file_path.clone(),
                            files_completed,
                            total_files,
                        });
                    }

                    bytes_copied += file_size;
                }

                files_completed += 1;
            }

            // Final progress update
            let _ = tx_progress.send(DirectoryCopyProgress {
                bytes_copied,
                total_bytes,
                current_file: PathBuf::new(),
                files_completed,
                total_files,
            });

            Ok(dest.clone())
        })();

        let _ = tx_complete.send(result);
    });

    Ok(DirectoryCopyOperation {
        completion: rx_complete,
        progress: rx_progress,
        pause_flag,
        cancel_flag,
    })
}

/// Progress for delete operation
#[derive(Debug, Clone)]
pub struct DeleteProgress {
    /// Files deleted so far
    pub files_deleted: usize,
    /// Total files to delete
    pub total_files: usize,
    /// Current file being deleted
    pub current_path: PathBuf,
}

/// Handle for an ongoing delete operation
pub struct DeleteOperation {
    /// Receiver for operation completion result
    pub completion: mpsc::Receiver<Result<()>>,
    /// Receiver for progress updates
    pub progress: mpsc::Receiver<DeleteProgress>,
    /// Flag to cancel the operation
    pub cancel_flag: Arc<AtomicBool>,
}

impl std::fmt::Debug for DeleteOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeleteOperation")
            .field("cancel_flag", &self.cancel_flag.load(Ordering::Relaxed))
            .finish()
    }
}

/// Delete files/directories asynchronously with progress reporting
pub fn delete_paths_async(paths: Vec<PathBuf>) -> DeleteOperation {
    let (tx_complete, rx_complete) = mpsc::channel();
    let (tx_progress, rx_progress) = mpsc::channel();

    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_clone = Arc::clone(&cancel_flag);

    thread::spawn(move || {
        let result = delete_paths_inner(&paths, &tx_progress, &cancel_flag_clone);
        let _ = tx_complete.send(result);
    });

    DeleteOperation {
        completion: rx_complete,
        progress: rx_progress,
        cancel_flag,
    }
}

/// Internal delete with progress
fn delete_paths_inner(
    paths: &[PathBuf],
    tx_progress: &mpsc::Sender<DeleteProgress>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<()> {
    // First, count total files for progress
    let mut total_files = 0;
    for path in paths {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!("Delete cancelled by user"));
        }
        if path.is_dir() {
            total_files += count_files_in_directory(path, cancel_flag)?;
        } else {
            total_files += 1;
        }
    }

    // Now delete with progress
    let mut files_deleted = 0;
    for path in paths {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!("Delete cancelled by user"));
        }

        if path.is_dir() {
            delete_directory_with_progress(
                path,
                &mut files_deleted,
                total_files,
                tx_progress,
                cancel_flag,
            )?;
        } else {
            // Send progress
            let _ = tx_progress.send(DeleteProgress {
                files_deleted,
                total_files,
                current_path: path.clone(),
            });

            fs::remove_file(path)?;
            files_deleted += 1;
        }
    }

    // Send final progress
    let _ = tx_progress.send(DeleteProgress {
        files_deleted,
        total_files,
        current_path: PathBuf::new(),
    });

    Ok(())
}

/// Count files in a directory recursively
fn count_files_in_directory(path: &Path, cancel_flag: &Arc<AtomicBool>) -> Result<usize> {
    let mut count = 0;

    for entry in fs::read_dir(path)? {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!("Delete cancelled by user"));
        }

        let entry = entry?;
        let entry_path = entry.path();

        if entry_path.is_dir() {
            count += count_files_in_directory(&entry_path, cancel_flag)?;
        } else {
            count += 1;
        }
    }

    // Count the directory itself
    count += 1;

    Ok(count)
}

/// Delete directory recursively with progress
fn delete_directory_with_progress(
    path: &Path,
    files_deleted: &mut usize,
    total_files: usize,
    tx_progress: &mpsc::Sender<DeleteProgress>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<()> {
    // First delete contents
    for entry in fs::read_dir(path)? {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!("Delete cancelled by user"));
        }

        let entry = entry?;
        let entry_path = entry.path();

        // Send progress
        let _ = tx_progress.send(DeleteProgress {
            files_deleted: *files_deleted,
            total_files,
            current_path: entry_path.clone(),
        });

        if entry_path.is_dir() {
            delete_directory_with_progress(
                &entry_path,
                files_deleted,
                total_files,
                tx_progress,
                cancel_flag,
            )?;
        } else {
            fs::remove_file(&entry_path)?;
            *files_deleted += 1;
        }
    }

    // Then delete the directory itself
    fs::remove_dir(path)?;
    *files_deleted += 1;

    Ok(())
}

impl FileManager {
    /// Create a new file
    pub fn create_file(&mut self, name: String) -> Result<()> {
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

    /// Delete file or directory
    pub fn delete_path(&mut self, path: PathBuf) -> Result<()> {
        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
        self.load_directory()?;
        Ok(())
    }

    /// Copy file or directory
    pub fn copy_path(&mut self, source: PathBuf, destination: PathBuf) -> Result<()> {
        // Extract destination name to navigate to after copy
        let dest_name = source
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());

        if source.is_dir() {
            self.copy_directory_recursive(&source, &destination)?;
        } else {
            let dest_path = path_utils::resolve_destination_path(&source, &destination);
            fs::copy(&source, &dest_path)?;
        }

        // Navigate to newly copied item
        if let Some(name) = dest_name {
            self.navigation.set_newly_created(name);
        }
        self.load_directory()?;
        Ok(())
    }

    /// Recursively copy directory
    fn copy_directory_recursive(&self, source: &PathBuf, destination: &PathBuf) -> Result<()> {
        self.copy_directory_recursive_with_depth(source, destination, 0)
    }

    /// Recursively copy directory with depth limit
    fn copy_directory_recursive_with_depth(
        &self,
        source: &PathBuf,
        destination: &PathBuf,
        depth: usize,
    ) -> Result<()> {
        const MAX_DEPTH: usize = termide_ui::constants::MAX_DIRECTORY_COPY_DEPTH;

        if depth > MAX_DEPTH {
            return Err(anyhow::anyhow!(
                "Directory nesting too deep (> {})",
                MAX_DEPTH
            ));
        }

        // Create target directory if it doesn't exist
        if !destination.exists() {
            fs::create_dir_all(destination)?;
        }

        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let source_path = entry.path();
            let dest_path = destination.join(entry.file_name());

            // Check metadata without following symlinks
            let metadata = fs::symlink_metadata(&source_path)?;

            if metadata.is_symlink() {
                // Copy symlink as symlink (don't follow it)
                #[cfg(unix)]
                {
                    use std::os::unix::fs as unix_fs;
                    let link_target = fs::read_link(&source_path)?;
                    unix_fs::symlink(link_target, &dest_path)?;
                }
                #[cfg(not(unix))]
                {
                    // On Windows, just copy as file
                    fs::copy(&source_path, &dest_path)?;
                }
            } else if metadata.is_dir() {
                // Recursively copy directory with incremented depth counter
                self.copy_directory_recursive_with_depth(&source_path, &dest_path, depth + 1)?;
            } else {
                // Regular file
                fs::copy(&source_path, &dest_path)?;
            }
        }

        Ok(())
    }

    /// Move file or directory
    pub fn move_path(&mut self, source: PathBuf, destination: PathBuf) -> Result<()> {
        // Extract destination name to navigate to after move
        let dest_name = source
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());

        let dest_path = path_utils::resolve_destination_path(&source, &destination);

        // Try simple rename (works only within same filesystem)
        if fs::rename(&source, &dest_path).is_err() {
            // If that failed - copy and delete
            if source.is_dir() {
                self.copy_directory_recursive(&source, &dest_path)?;
                fs::remove_dir_all(&source)?;
            } else {
                fs::copy(&source, &dest_path)?;
                fs::remove_file(&source)?;
            }
        }

        // Navigate to newly moved item
        if let Some(name) = dest_name {
            self.navigation.set_newly_created(name);
        }
        self.load_directory()?;
        Ok(())
    }
}
