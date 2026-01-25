//! VFS error types.

use std::io;
use std::path::PathBuf;
use thiserror::Error;

/// Virtual filesystem error type.
#[derive(Debug, Error)]
pub enum VfsError {
    /// I/O error during file operation.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Path parsing or validation error.
    #[error("Invalid path: {0}")]
    InvalidPath(String),

    /// URL parsing error.
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    /// Unsupported protocol scheme.
    #[error("Unsupported protocol: {0}")]
    UnsupportedProtocol(String),

    /// Connection error.
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Authentication error.
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Operation timeout.
    #[error("Operation timed out: {0}")]
    Timeout(String),

    /// File or directory not found.
    #[error("Not found: {path}")]
    NotFound { path: PathBuf },

    /// Permission denied.
    #[error("Permission denied: {path}")]
    PermissionDenied { path: PathBuf },

    /// Already exists (for create operations).
    #[error("Already exists: {path}")]
    AlreadyExists { path: PathBuf },

    /// Directory is not empty (for delete operations).
    #[error("Directory not empty: {path}")]
    DirectoryNotEmpty { path: PathBuf },

    /// Operation not supported by this provider.
    #[error("Operation not supported: {0}")]
    NotSupported(String),

    /// Provider is not connected.
    #[error("Not connected")]
    NotConnected,

    /// Operation was cancelled by user.
    #[error("Operation cancelled")]
    Cancelled,

    /// Provider is already connected.
    #[error("Already connected")]
    AlreadyConnected,

    /// Remote operation error with provider-specific details.
    #[error("Remote error: {message}")]
    RemoteError { message: String },

    /// SFTP-specific error.
    #[cfg(feature = "sftp")]
    #[error("SFTP error: {0}")]
    Sftp(String),

    /// FTP-specific error.
    #[cfg(feature = "ftp")]
    #[error("FTP error: {0}")]
    Ftp(String),

    /// SMB-specific error.
    #[cfg(feature = "smb")]
    #[error("SMB error: {0}")]
    Smb(String),

    /// NFS/FUSE mount error.
    #[cfg(feature = "nfs")]
    #[error("NFS mount error: {0}")]
    NfsMount(String),
}

/// Alias for VFS operation results.
pub type VfsResult<T> = Result<T, VfsError>;
