//! SFTP (SSH File Transfer Protocol) VFS provider.
//!
//! Uses the `ssh2` crate for SSH/SFTP connectivity.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ssh2::{Session, Sftp};

use crate::error::{VfsError, VfsResult};
use crate::traits::{DiskSpace, VfsProvider};
use crate::types::{
    AuthMethod, ConnectOptions, ConnectionState, DownloadProgress, VfsDownloadOperation, VfsEntry,
    VfsFileType, VfsMetadata, VfsOperation, VfsPath, VfsProtocol, VfsUploadOperation,
};

/// Default connection timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Inner connection state, protected by mutex for thread-safe updates.
struct SftpInner {
    /// Current connection state.
    state: ConnectionState,
    /// SSH session (Arc<Mutex<>> for thread-safe access in operations).
    session: Option<Arc<Mutex<Session>>>,
    /// SFTP channel (Arc<Mutex<>> for thread-safe access in operations).
    sftp: Option<Arc<Mutex<Sftp>>>,
    /// Home directory on remote system.
    home_dir: Option<String>,
    /// When connection started (for elapsed time display).
    connect_started: Option<Instant>,
    /// Cancellation flag for pending operations.
    cancelled: Arc<AtomicBool>,
}

impl SftpInner {
    fn new() -> Self {
        Self {
            state: ConnectionState::Disconnected,
            session: None,
            sftp: None,
            home_dir: None,
            connect_started: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// SFTP filesystem provider.
pub struct SftpProvider {
    /// SSH host.
    host: String,
    /// SSH port.
    port: u16,
    /// Username (None = use SSH config or current user).
    username: Option<String>,
    /// Thread-safe inner state.
    inner: Arc<Mutex<SftpInner>>,
}

impl SftpProvider {
    /// Create a new SFTP provider.
    pub fn new(host: &str, port: u16, username: Option<&str>) -> Self {
        Self {
            host: host.to_string(),
            port,
            username: username.map(String::from),
            inner: Arc::new(Mutex::new(SftpInner::new())),
        }
    }

    /// Get the effective username.
    fn effective_username(&self) -> String {
        self.username.clone().unwrap_or_else(|| {
            std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "root".to_string())
        })
    }

    /// Convert VfsPath to remote path string.
    fn to_remote_path(path: &VfsPath) -> VfsResult<PathBuf> {
        if !matches!(path.protocol, VfsProtocol::Sftp) {
            return Err(VfsError::InvalidPath(format!(
                "Expected SFTP path, got: {}",
                path
            )));
        }
        Ok(path.path.clone())
    }

    /// Check if connection is in progress.
    pub fn is_connecting(&self) -> bool {
        self.inner
            .lock()
            .map(|guard| guard.state == ConnectionState::Connecting)
            .unwrap_or(false)
    }

    /// Get elapsed time since connection started (for UI display).
    pub fn connection_elapsed(&self) -> Option<Duration> {
        self.inner
            .lock()
            .ok()
            .and_then(|guard| guard.connect_started.map(|s| s.elapsed()))
    }

    /// Cancel pending connection.
    pub fn cancel_connection(&self) {
        if let Ok(guard) = self.inner.lock() {
            guard.cancelled.store(true, Ordering::SeqCst);
        }
    }

    /// Get a clone of the SFTP handle for use in operations.
    fn get_sftp(&self) -> Option<Arc<Mutex<Sftp>>> {
        self.inner
            .lock()
            .ok()
            .and_then(|guard| guard.sftp.as_ref().map(Arc::clone))
    }

    /// Get a clone of the session handle.
    #[allow(dead_code)]
    fn get_session(&self) -> Option<Arc<Mutex<Session>>> {
        self.inner
            .lock()
            .ok()
            .and_then(|guard| guard.session.as_ref().map(Arc::clone))
    }

    /// Get the cached home directory.
    fn get_home_dir(&self) -> Option<String> {
        self.inner
            .lock()
            .ok()
            .and_then(|guard| guard.home_dir.clone())
    }

    /// Attempt authentication with available methods.
    fn authenticate(
        session: &Session,
        host: &str,
        username: &str,
        auth: &AuthMethod,
    ) -> VfsResult<()> {
        use crate::ssh_config::SshConfig;

        match auth {
            AuthMethod::None => {
                // Try no authentication (unlikely to work)
                session.userauth_agent(username).map_err(|e| {
                    VfsError::AuthenticationFailed(format!("Agent auth failed: {}", e))
                })?;
            }
            AuthMethod::Password(password) => {
                session.userauth_password(username, password).map_err(|e| {
                    VfsError::AuthenticationFailed(format!("Password auth failed: {}", e))
                })?;
            }
            AuthMethod::SshKey {
                private_key,
                passphrase,
            } => {
                session
                    .userauth_pubkey_file(username, None, private_key, passphrase.as_deref())
                    .map_err(|e| {
                        VfsError::AuthenticationFailed(format!("Key auth failed: {}", e))
                    })?;
            }
            AuthMethod::SshAgent => {
                session.userauth_agent(username).map_err(|e| {
                    VfsError::AuthenticationFailed(format!("Agent auth failed: {}", e))
                })?;
            }
            AuthMethod::Auto => {
                // Log SSH_AUTH_SOCK for debugging
                let ssh_auth_sock = std::env::var("SSH_AUTH_SOCK").ok();
                termide_logger::debug(format!(
                    "SFTP Auto auth: SSH_AUTH_SOCK = {:?}",
                    ssh_auth_sock
                ));

                // Load SSH config for host-specific settings
                let ssh_config = SshConfig::from_default_path();
                let host_config = ssh_config.as_ref().map(|c| c.get_host_config(host));

                if let Some(ref cfg) = host_config {
                    termide_logger::debug(format!(
                        "SFTP: SSH config for '{}': identity_files={:?}, identities_only={}",
                        host, cfg.identity_files, cfg.identities_only
                    ));
                } else {
                    termide_logger::debug(format!("SFTP: No SSH config found for '{}'", host));
                }

                // Try SSH agent first (unless IdentitiesOnly is set)
                let identities_only = host_config
                    .as_ref()
                    .map(|c| c.identities_only)
                    .unwrap_or(false);

                if !identities_only {
                    // Try to get detailed info from the agent
                    match session.agent() {
                        Ok(mut agent) => {
                            if let Err(e) = agent.connect() {
                                termide_logger::debug(format!("SFTP: Agent connect failed: {}", e));
                            } else if let Err(e) = agent.list_identities() {
                                termide_logger::debug(format!(
                                    "SFTP: Agent list_identities failed: {}",
                                    e
                                ));
                            } else {
                                // Log all identities in the agent
                                let mut identity_count = 0;
                                let identities = agent.identities().unwrap_or_default();
                                for identity in identities.iter() {
                                    identity_count += 1;
                                    termide_logger::debug(format!(
                                        "SFTP: Agent identity {}: comment='{}'",
                                        identity_count,
                                        identity.comment()
                                    ));
                                    // Try this specific identity
                                    match agent.userauth(username, identity) {
                                        Ok(()) => {
                                            termide_logger::debug(format!(
                                                "SFTP: Agent auth succeeded with identity '{}'",
                                                identity.comment()
                                            ));
                                            return Ok(());
                                        }
                                        Err(e) => {
                                            termide_logger::debug(format!(
                                                "SFTP: Agent identity '{}' rejected: {}",
                                                identity.comment(),
                                                e
                                            ));
                                        }
                                    }
                                }
                                termide_logger::debug(format!(
                                    "SFTP: Agent has {} identities, none worked",
                                    identity_count
                                ));
                            }
                            let _ = agent.disconnect();
                        }
                        Err(e) => {
                            termide_logger::debug(format!("SFTP: Failed to create agent: {}", e));
                        }
                    }
                }

                // Collect key files to try: SSH config keys first, then default keys
                let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                let ssh_dir = home.join(".ssh");

                let mut key_files: Vec<PathBuf> = Vec::new();

                // Add keys from SSH config first (higher priority)
                if let Some(ref cfg) = host_config {
                    for key_file in &cfg.identity_files {
                        if !key_files.contains(key_file) {
                            key_files.push(key_file.clone());
                        }
                    }
                }

                // Add default keys (lower priority)
                let default_keys = [
                    ssh_dir.join("id_ed25519"),
                    ssh_dir.join("id_rsa"),
                    ssh_dir.join("id_ecdsa"),
                    ssh_dir.join("id_dsa"),
                ];

                for key_file in default_keys {
                    if !key_files.contains(&key_file) {
                        key_files.push(key_file);
                    }
                }

                for key_file in &key_files {
                    if key_file.exists() {
                        termide_logger::debug(format!("SFTP: Trying key file {:?}", key_file));
                        match session.userauth_pubkey_file(username, None, key_file, None) {
                            Ok(()) => {
                                termide_logger::debug(format!(
                                    "SFTP: Key file auth succeeded with {:?}",
                                    key_file
                                ));
                                return Ok(());
                            }
                            Err(e) => {
                                termide_logger::debug(format!(
                                    "SFTP: Key file {:?} failed: {}",
                                    key_file, e
                                ));
                            }
                        }
                    }
                }

                return Err(VfsError::AuthenticationFailed(
                    "No authentication method succeeded. Password may be required.".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Convert ssh2 FileStat to VfsMetadata.
    fn stat_to_metadata(stat: &ssh2::FileStat) -> VfsMetadata {
        let file_type = if stat.is_dir() {
            VfsFileType::Directory
        } else if stat.is_file() {
            VfsFileType::File
        } else {
            VfsFileType::Other
        };

        let mut metadata = if file_type == VfsFileType::Directory {
            VfsMetadata::directory()
        } else {
            VfsMetadata::file(stat.size.unwrap_or(0))
        };

        // Set modification time
        if let Some(mtime) = stat.mtime {
            metadata.modified = Some(std::time::UNIX_EPOCH + Duration::from_secs(mtime));
        }

        // Set permissions
        if let Some(perms) = stat.perm {
            metadata = metadata.with_permissions(perms);
            metadata.readonly = (perms & 0o200) == 0;
        }

        metadata
    }

    /// Download directory recursively (internal helper, called within thread)
    fn download_directory_recursive(
        sftp: &std::sync::MutexGuard<'_, ssh2::Sftp>,
        remote_path: &Path,
        local_path: &Path,
    ) -> VfsResult<()> {
        Self::download_directory_recursive_with_progress(
            sftp,
            remote_path,
            local_path,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicBool::new(false)),
            None,
            &mut 0,
            &mut 0,
            0,
            0,
        )
    }

    /// Count files in remote directory recursively
    fn count_remote_files(
        sftp: &std::sync::MutexGuard<'_, ssh2::Sftp>,
        remote_path: &Path,
        cancel_flag: &Arc<AtomicBool>,
    ) -> VfsResult<(usize, u64)> {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(VfsError::Cancelled);
        }

        let entries = sftp
            .readdir(remote_path)
            .map_err(|e| VfsError::Sftp(format!("readdir failed: {}", e)))?;

        let mut file_count = 0;
        let mut total_bytes = 0u64;

        for (entry_path, stat) in entries {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(VfsError::Cancelled);
            }

            if let Some(name) = entry_path.file_name() {
                let name_str = name.to_string_lossy();
                if name_str == "." || name_str == ".." {
                    continue;
                }

                if stat.is_dir() {
                    let (sub_count, sub_bytes) =
                        Self::count_remote_files(sftp, &entry_path, cancel_flag)?;
                    file_count += sub_count;
                    total_bytes += sub_bytes;
                } else {
                    file_count += 1;
                    total_bytes += stat.size.unwrap_or(0);
                }
            }
        }

        Ok((file_count, total_bytes))
    }

    /// Download directory recursively with progress and pause/cancel support
    #[allow(clippy::too_many_arguments)]
    fn download_directory_recursive_with_progress(
        sftp: &std::sync::MutexGuard<'_, ssh2::Sftp>,
        remote_path: &Path,
        local_path: &Path,
        pause_flag: &Arc<AtomicBool>,
        cancel_flag: &Arc<AtomicBool>,
        tx_progress: Option<&mpsc::Sender<DownloadProgress>>,
        files_downloaded: &mut usize,
        bytes_downloaded: &mut u64,
        total_files: usize,
        total_bytes: u64,
    ) -> VfsResult<()> {
        // Check cancel
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(VfsError::Cancelled);
        }

        // Wait while paused
        while pause_flag.load(Ordering::Relaxed) {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(VfsError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        // Create local directory
        std::fs::create_dir_all(local_path).map_err(VfsError::Io)?;

        // List remote directory
        let entries = sftp
            .readdir(remote_path)
            .map_err(|e| VfsError::Sftp(format!("readdir failed: {}", e)))?;

        for (entry_path, stat) in entries {
            // Check cancel
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(VfsError::Cancelled);
            }

            // Wait while paused
            while pause_flag.load(Ordering::Relaxed) {
                if cancel_flag.load(Ordering::Relaxed) {
                    return Err(VfsError::Cancelled);
                }
                std::thread::sleep(Duration::from_millis(100));
            }

            if let Some(name) = entry_path.file_name() {
                let name_str = name.to_string_lossy();
                if name_str == "." || name_str == ".." {
                    continue;
                }

                let local_entry = local_path.join(name);

                if stat.is_dir() {
                    // Recurse into subdirectory
                    Self::download_directory_recursive_with_progress(
                        sftp,
                        &entry_path,
                        &local_entry,
                        pause_flag,
                        cancel_flag,
                        tx_progress,
                        files_downloaded,
                        bytes_downloaded,
                        total_files,
                        total_bytes,
                    )?;
                } else {
                    let file_size = stat.size.unwrap_or(0);

                    // Send progress update before downloading
                    if let Some(tx) = tx_progress {
                        let _ = tx.send(DownloadProgress {
                            bytes_downloaded: *bytes_downloaded,
                            total_bytes,
                            current_file: Some(name_str.to_string()),
                            files_downloaded: *files_downloaded,
                            total_files,
                            current_file_bytes: 0,
                            current_file_total: file_size,
                        });
                    }

                    // Download file with chunked reading
                    let mut remote_file = sftp
                        .open(&entry_path)
                        .map_err(|e| VfsError::Sftp(format!("open remote file failed: {}", e)))?;

                    // Create local file for writing
                    let mut local_file =
                        std::fs::File::create(&local_entry).map_err(VfsError::Io)?;

                    const CHUNK_SIZE: usize = 64 * 1024; // 64KB chunks
                    let mut buffer = vec![0u8; CHUNK_SIZE];
                    let mut current_file_bytes = 0u64;

                    loop {
                        // Check cancel
                        if cancel_flag.load(Ordering::Relaxed) {
                            return Err(VfsError::Cancelled);
                        }

                        // Wait while paused
                        while pause_flag.load(Ordering::Relaxed) {
                            if cancel_flag.load(Ordering::Relaxed) {
                                return Err(VfsError::Cancelled);
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }

                        let bytes_read = remote_file.read(&mut buffer).map_err(VfsError::Io)?;
                        if bytes_read == 0 {
                            break;
                        }

                        local_file
                            .write_all(&buffer[..bytes_read])
                            .map_err(VfsError::Io)?;
                        current_file_bytes += bytes_read as u64;

                        // Send progress update
                        if let Some(tx) = tx_progress {
                            let _ = tx.send(DownloadProgress {
                                bytes_downloaded: *bytes_downloaded + current_file_bytes,
                                total_bytes,
                                current_file: Some(name_str.to_string()),
                                files_downloaded: *files_downloaded,
                                total_files,
                                current_file_bytes,
                                current_file_total: file_size,
                            });
                        }
                    }

                    *files_downloaded += 1;
                    *bytes_downloaded += current_file_bytes;

                    // Send progress update after downloading
                    if let Some(tx) = tx_progress {
                        let _ = tx.send(DownloadProgress {
                            bytes_downloaded: *bytes_downloaded,
                            total_bytes,
                            current_file: None,
                            files_downloaded: *files_downloaded,
                            total_files,
                            current_file_bytes: 0,
                            current_file_total: 0,
                        });
                    }
                }
            }
        }

        Ok(())
    }
}

impl VfsProvider for SftpProvider {
    fn name(&self) -> &'static str {
        "sftp"
    }

    fn connection_state(&self) -> ConnectionState {
        self.inner
            .lock()
            .map(|guard| guard.state)
            .unwrap_or(ConnectionState::Failed)
    }

    fn connect(&mut self, options: ConnectOptions) -> VfsOperation<()> {
        // Check current state and update to Connecting
        {
            let mut guard = match self.inner.lock() {
                Ok(g) => g,
                Err(_) => {
                    return VfsOperation::error(VfsError::RemoteError {
                        message: "Failed to acquire lock".to_string(),
                    })
                }
            };

            if guard.state == ConnectionState::Connected {
                return VfsOperation::error(VfsError::AlreadyConnected);
            }

            // Reset cancellation flag for new connection
            guard.cancelled.store(false, Ordering::SeqCst);
            guard.state = ConnectionState::Connecting;
            guard.connect_started = Some(Instant::now());
        }

        let host = self.host.clone();
        let port = self.port;
        let username = self.effective_username();
        let auth = options.auth.clone();
        let timeout_secs = options.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
        let inner = Arc::clone(&self.inner);

        // Get cancellation token
        let cancelled = self
            .inner
            .lock()
            .map(|g| Arc::clone(&g.cancelled))
            .unwrap_or_else(|_| Arc::new(AtomicBool::new(false)));

        let (tx, rx) = mpsc::channel();

        // Connection happens in background thread
        thread::spawn(move || {
            let result = (|| -> VfsResult<(Session, Sftp, String)> {
                // Check cancellation before starting
                if cancelled.load(Ordering::SeqCst) {
                    return Err(VfsError::ConnectionFailed(
                        "Connection cancelled".to_string(),
                    ));
                }

                // Connect TCP
                let addr = format!("{}:{}", host, port);
                let tcp = TcpStream::connect_timeout(
                    &addr.parse().map_err(|e| {
                        VfsError::ConnectionFailed(format!("Invalid address: {}", e))
                    })?,
                    Duration::from_secs(timeout_secs),
                )
                .map_err(|e| VfsError::ConnectionFailed(format!("TCP connect failed: {}", e)))?;

                // Check cancellation after TCP connect
                if cancelled.load(Ordering::SeqCst) {
                    return Err(VfsError::ConnectionFailed(
                        "Connection cancelled".to_string(),
                    ));
                }

                tcp.set_read_timeout(Some(Duration::from_secs(timeout_secs)))
                    .ok();
                tcp.set_write_timeout(Some(Duration::from_secs(timeout_secs)))
                    .ok();

                // Create SSH session
                let mut session = Session::new().map_err(|e| {
                    VfsError::ConnectionFailed(format!("Failed to create session: {}", e))
                })?;

                session.set_tcp_stream(tcp);
                session.handshake().map_err(|e| {
                    VfsError::ConnectionFailed(format!("SSH handshake failed: {}", e))
                })?;

                // Check cancellation after handshake
                if cancelled.load(Ordering::SeqCst) {
                    return Err(VfsError::ConnectionFailed(
                        "Connection cancelled".to_string(),
                    ));
                }

                // Authenticate
                Self::authenticate(&session, &host, &username, &auth)?;

                if !session.authenticated() {
                    return Err(VfsError::AuthenticationFailed(
                        "Session not authenticated".to_string(),
                    ));
                }

                // Create SFTP channel
                let sftp = session.sftp().map_err(|e| {
                    VfsError::ConnectionFailed(format!("Failed to create SFTP channel: {}", e))
                })?;

                // Get home directory
                let home_dir = sftp
                    .realpath(Path::new("."))
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| format!("/home/{}", username));

                Ok((session, sftp, home_dir))
            })();

            // Update inner state with result
            match result {
                Ok((session, sftp, home_dir)) => {
                    if let Ok(mut guard) = inner.lock() {
                        // Wrap session and sftp in Arc<Mutex<>> for thread-safe operation access
                        guard.session = Some(Arc::new(Mutex::new(session)));
                        guard.sftp = Some(Arc::new(Mutex::new(sftp)));
                        guard.home_dir = Some(home_dir.clone());
                        guard.state = ConnectionState::Connected;
                        guard.connect_started = None;
                        termide_logger::info(format!(
                            "SFTP connected to {} (home: {})",
                            host, home_dir
                        ));
                    }
                    let _ = tx.send(Ok(()));
                }
                Err(e) => {
                    if let Ok(mut guard) = inner.lock() {
                        guard.state = ConnectionState::Failed;
                        guard.connect_started = None;
                    }
                    termide_logger::error(format!("SFTP connection failed: {}", e));
                    match tx.send(Err(e)) {
                        Ok(()) => termide_logger::info(
                            "SFTP: Error sent to channel successfully".to_string(),
                        ),
                        Err(send_err) => termide_logger::error(format!(
                            "SFTP: Failed to send error to channel: {:?}",
                            send_err
                        )),
                    }
                }
            }
        });

        VfsOperation::new(rx)
    }

    fn disconnect(&mut self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.sftp = None;
            guard.session = None;
            guard.state = ConnectionState::Disconnected;
            guard.home_dir = None;
            guard.connect_started = None;
        }
        termide_logger::info(format!("SFTP disconnected from {}", self.host));
    }

    fn list_dir(&self, path: &VfsPath) -> VfsOperation<Vec<VfsEntry>> {
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsOperation::error(VfsError::NotConnected),
        };

        let remote_path = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };

        let base_path = path.clone();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = (|| -> VfsResult<Vec<VfsEntry>> {
                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                let mut entries = Vec::new();
                let dir = sftp
                    .readdir(&remote_path)
                    .map_err(|e| VfsError::Sftp(format!("readdir failed: {}", e)))?;

                for (entry_path, stat) in dir {
                    let name = entry_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();

                    if name.is_empty() {
                        continue;
                    }

                    let entry_vfs_path = base_path.join(&name);
                    let metadata = Self::stat_to_metadata(&stat);

                    entries.push(VfsEntry::new(name, entry_vfs_path, metadata));
                }

                // Sort: directories first, then by name
                entries.sort_by(|a, b| match (a.is_dir(), b.is_dir()) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                });

                Ok(entries)
            })();

            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn create_dir(&self, path: &VfsPath) -> VfsOperation<()> {
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsOperation::error(VfsError::NotConnected),
        };

        let remote_path = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = (|| -> VfsResult<()> {
                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                sftp.mkdir(&remote_path, 0o755)
                    .map_err(|e| VfsError::Sftp(format!("mkdir failed: {}", e)))?;

                Ok(())
            })();

            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn create_dir_all(&self, path: &VfsPath) -> VfsOperation<()> {
        // SFTP doesn't have native mkdir -p, so we create directories one by one
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsOperation::error(VfsError::NotConnected),
        };

        let remote_path = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = (|| -> VfsResult<()> {
                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                let mut current = PathBuf::new();
                for component in remote_path.components() {
                    current.push(component);

                    // Check if exists
                    if sftp.stat(&current).is_err() {
                        // Doesn't exist, create it
                        sftp.mkdir(&current, 0o755).map_err(|e| {
                            VfsError::Sftp(format!("mkdir failed for {:?}: {}", current, e))
                        })?;
                    }
                }

                Ok(())
            })();

            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn exists(&self, path: &VfsPath) -> VfsOperation<bool> {
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsOperation::error(VfsError::NotConnected),
        };

        let remote_path = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = (|| -> VfsResult<bool> {
                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                Ok(sftp.stat(&remote_path).is_ok())
            })();

            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn metadata(&self, path: &VfsPath) -> VfsOperation<VfsMetadata> {
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsOperation::error(VfsError::NotConnected),
        };

        let remote_path = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = (|| -> VfsResult<VfsMetadata> {
                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                let stat = sftp
                    .stat(&remote_path)
                    .map_err(|e| VfsError::Sftp(format!("stat failed: {}", e)))?;

                Ok(Self::stat_to_metadata(&stat))
            })();

            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn read_file(&self, path: &VfsPath) -> VfsOperation<Vec<u8>> {
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsOperation::error(VfsError::NotConnected),
        };

        let remote_path = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = (|| -> VfsResult<Vec<u8>> {
                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                let mut file = sftp
                    .open(&remote_path)
                    .map_err(|e| VfsError::Sftp(format!("open failed: {}", e)))?;

                let mut contents = Vec::new();
                file.read_to_end(&mut contents).map_err(VfsError::Io)?;

                Ok(contents)
            })();

            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn write_file(&self, path: &VfsPath, data: &[u8]) -> VfsOperation<()> {
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsOperation::error(VfsError::NotConnected),
        };

        let remote_path = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };

        let data = data.to_vec();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = (|| -> VfsResult<()> {
                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                let mut file = sftp
                    .create(&remote_path)
                    .map_err(|e| VfsError::Sftp(format!("create failed: {}", e)))?;

                file.write_all(&data).map_err(VfsError::Io)?;

                Ok(())
            })();

            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn delete(&self, path: &VfsPath) -> VfsOperation<()> {
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsOperation::error(VfsError::NotConnected),
        };

        let remote_path = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = (|| -> VfsResult<()> {
                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                // Check if it's a directory
                let stat = sftp
                    .stat(&remote_path)
                    .map_err(|e| VfsError::Sftp(format!("stat failed: {}", e)))?;

                if stat.is_dir() {
                    sftp.rmdir(&remote_path)
                        .map_err(|e| VfsError::Sftp(format!("rmdir failed: {}", e)))?;
                } else {
                    sftp.unlink(&remote_path)
                        .map_err(|e| VfsError::Sftp(format!("unlink failed: {}", e)))?;
                }

                Ok(())
            })();

            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn delete_recursive(&self, path: &VfsPath) -> VfsOperation<()> {
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsOperation::error(VfsError::NotConnected),
        };

        let remote_path = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            fn delete_recursive_inner(sftp: &Sftp, path: &Path, depth: usize) -> VfsResult<()> {
                const MAX_DEPTH: usize = 100;

                if depth > MAX_DEPTH {
                    return Err(VfsError::RemoteError {
                        message: format!("Directory nesting too deep (> {})", MAX_DEPTH),
                    });
                }

                let stat = sftp
                    .stat(path)
                    .map_err(|e| VfsError::Sftp(format!("stat failed: {}", e)))?;

                if stat.is_dir() {
                    // List and delete contents first
                    let entries = sftp
                        .readdir(path)
                        .map_err(|e| VfsError::Sftp(format!("readdir failed: {}", e)))?;

                    for (entry_path, _) in entries {
                        delete_recursive_inner(sftp, &entry_path, depth + 1)?;
                    }

                    sftp.rmdir(path)
                        .map_err(|e| VfsError::Sftp(format!("rmdir failed: {}", e)))?;
                } else {
                    sftp.unlink(path)
                        .map_err(|e| VfsError::Sftp(format!("unlink failed: {}", e)))?;
                }

                Ok(())
            }

            let result = (|| -> VfsResult<()> {
                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                delete_recursive_inner(&sftp, &remote_path, 0)
            })();

            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn rename(&self, from: &VfsPath, to: &VfsPath) -> VfsOperation<()> {
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsOperation::error(VfsError::NotConnected),
        };

        let from_path = match Self::to_remote_path(from) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };
        let to_path = match Self::to_remote_path(to) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = (|| -> VfsResult<()> {
                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                sftp.rename(&from_path, &to_path, None)
                    .map_err(|e| VfsError::Sftp(format!("rename failed: {}", e)))?;

                Ok(())
            })();

            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn copy(&self, from: &VfsPath, to: &VfsPath) -> VfsOperation<()> {
        // SFTP doesn't have native copy - we need to read and write
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsOperation::error(VfsError::NotConnected),
        };

        let from_path = match Self::to_remote_path(from) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };
        let to_path = match Self::to_remote_path(to) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = (|| -> VfsResult<()> {
                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                // Read source file
                let mut src_file = sftp
                    .open(&from_path)
                    .map_err(|e| VfsError::Sftp(format!("open source failed: {}", e)))?;

                let mut contents = Vec::new();
                src_file.read_to_end(&mut contents).map_err(VfsError::Io)?;

                // Write to destination
                let mut dst_file = sftp
                    .create(&to_path)
                    .map_err(|e| VfsError::Sftp(format!("create destination failed: {}", e)))?;

                dst_file.write_all(&contents).map_err(VfsError::Io)?;

                Ok(())
            })();

            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn download(&self, remote: &VfsPath, local: &Path) -> VfsOperation<PathBuf> {
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsOperation::error(VfsError::NotConnected),
        };

        let remote_path = match Self::to_remote_path(remote) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };

        let local_path = local.to_path_buf();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = (|| -> VfsResult<PathBuf> {
                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                // Check if it's a directory or file
                let stat = sftp
                    .stat(&remote_path)
                    .map_err(|e| VfsError::Sftp(format!("stat failed: {}", e)))?;

                if stat.is_dir() {
                    // Recursive directory download
                    Self::download_directory_recursive(&sftp, &remote_path, &local_path)?;
                } else {
                    // Read remote file
                    let mut remote_file = sftp
                        .open(&remote_path)
                        .map_err(|e| VfsError::Sftp(format!("open remote failed: {}", e)))?;

                    let mut contents = Vec::new();
                    remote_file
                        .read_to_end(&mut contents)
                        .map_err(VfsError::Io)?;

                    // Write to local file
                    std::fs::write(&local_path, contents).map_err(VfsError::Io)?;
                }

                Ok(local_path)
            })();

            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn upload(&self, local: &Path, remote: &VfsPath) -> VfsOperation<()> {
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsOperation::error(VfsError::NotConnected),
        };

        let remote_path = match Self::to_remote_path(remote) {
            Ok(p) => p,
            Err(e) => return VfsOperation::error(e),
        };

        let local_path = local.to_path_buf();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = (|| -> VfsResult<()> {
                // Read local file
                let contents = std::fs::read(&local_path).map_err(VfsError::Io)?;

                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                // Write to remote file
                let mut remote_file = sftp
                    .create(&remote_path)
                    .map_err(|e| VfsError::Sftp(format!("create remote failed: {}", e)))?;

                remote_file.write_all(&contents).map_err(VfsError::Io)?;

                Ok(())
            })();

            let _ = tx.send(result);
        });

        VfsOperation::new(rx)
    }

    fn upload_with_progress(&self, local: &Path, remote: &VfsPath) -> VfsUploadOperation {
        use crate::types::UploadProgress;
        use std::io::Read;

        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsUploadOperation::error(VfsError::NotConnected),
        };

        let remote_path = match Self::to_remote_path(remote) {
            Ok(p) => p,
            Err(e) => return VfsUploadOperation::error(e),
        };

        let local_path = local.to_path_buf();
        let (tx_complete, rx_complete) = mpsc::channel();
        let (tx_progress, rx_progress) = mpsc::channel();

        thread::spawn(move || {
            let result = (|| -> VfsResult<()> {
                // Get file size
                let metadata = std::fs::metadata(&local_path).map_err(VfsError::Io)?;
                let total_bytes = metadata.len();

                // Open local file for reading
                let mut local_file = std::fs::File::open(&local_path).map_err(VfsError::Io)?;

                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                // Create remote file
                let mut remote_file = sftp
                    .create(&remote_path)
                    .map_err(|e| VfsError::Sftp(format!("create remote failed: {}", e)))?;

                // Write in chunks with progress reporting
                const CHUNK_SIZE: usize = 64 * 1024; // 64KB chunks
                let mut buffer = vec![0u8; CHUNK_SIZE];
                let mut bytes_uploaded: u64 = 0;

                loop {
                    let bytes_read = local_file.read(&mut buffer).map_err(VfsError::Io)?;
                    if bytes_read == 0 {
                        break;
                    }

                    remote_file
                        .write_all(&buffer[..bytes_read])
                        .map_err(VfsError::Io)?;
                    bytes_uploaded += bytes_read as u64;

                    // Send progress update
                    let _ = tx_progress.send(UploadProgress {
                        bytes_uploaded,
                        total_bytes,
                    });
                }

                Ok(())
            })();

            let _ = tx_complete.send(result);
        });

        VfsUploadOperation::new(rx_complete, rx_progress)
    }

    fn download_with_progress(&self, remote: &VfsPath, local: &Path) -> VfsDownloadOperation {
        let sftp = match self.get_sftp() {
            Some(s) => s,
            None => return VfsDownloadOperation::error(VfsError::NotConnected),
        };

        let remote_path = match Self::to_remote_path(remote) {
            Ok(p) => p,
            Err(e) => return VfsDownloadOperation::error(e),
        };

        let local_path = local.to_path_buf();

        let (tx_complete, rx_complete) = mpsc::channel();
        let (tx_progress, rx_progress) = mpsc::channel();
        let pause_flag = Arc::new(AtomicBool::new(false));
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let pause_flag_clone = Arc::clone(&pause_flag);
        let cancel_flag_clone = Arc::clone(&cancel_flag);

        thread::spawn(move || {
            let result = (|| -> VfsResult<PathBuf> {
                let sftp = sftp.lock().map_err(|_| VfsError::RemoteError {
                    message: "Failed to acquire SFTP lock".to_string(),
                })?;

                // Check if it's a directory or file
                let stat = sftp
                    .stat(&remote_path)
                    .map_err(|e| VfsError::Sftp(format!("stat failed: {}", e)))?;

                if stat.is_dir() {
                    // Count files first for progress
                    let (total_files, total_bytes) =
                        Self::count_remote_files(&sftp, &remote_path, &cancel_flag_clone)?;

                    // Send initial progress
                    let _ = tx_progress.send(DownloadProgress {
                        bytes_downloaded: 0,
                        total_bytes,
                        current_file: None,
                        files_downloaded: 0,
                        total_files,
                        current_file_bytes: 0,
                        current_file_total: 0,
                    });

                    // Recursive directory download with progress
                    let mut files_downloaded = 0;
                    let mut bytes_downloaded = 0u64;
                    Self::download_directory_recursive_with_progress(
                        &sftp,
                        &remote_path,
                        &local_path,
                        &pause_flag_clone,
                        &cancel_flag_clone,
                        Some(&tx_progress),
                        &mut files_downloaded,
                        &mut bytes_downloaded,
                        total_files,
                        total_bytes,
                    )?;
                } else {
                    // Single file download with chunked progress
                    let file_size = stat.size.unwrap_or(0);
                    let file_name = remote_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string());

                    // Send initial progress
                    let _ = tx_progress.send(DownloadProgress {
                        bytes_downloaded: 0,
                        total_bytes: file_size,
                        current_file: file_name.clone(),
                        files_downloaded: 0,
                        total_files: 1,
                        current_file_bytes: 0,
                        current_file_total: file_size,
                    });

                    // Open remote file
                    let mut remote_file = sftp
                        .open(&remote_path)
                        .map_err(|e| VfsError::Sftp(format!("open remote failed: {}", e)))?;

                    // Create local file
                    let mut local_file =
                        std::fs::File::create(&local_path).map_err(VfsError::Io)?;

                    // Chunked download with progress
                    const CHUNK_SIZE: usize = 64 * 1024; // 64KB chunks
                    let mut buffer = vec![0u8; CHUNK_SIZE];
                    let mut bytes_downloaded = 0u64;

                    loop {
                        // Check cancel
                        if cancel_flag_clone.load(Ordering::Relaxed) {
                            return Err(VfsError::Cancelled);
                        }

                        // Wait while paused
                        while pause_flag_clone.load(Ordering::Relaxed) {
                            if cancel_flag_clone.load(Ordering::Relaxed) {
                                return Err(VfsError::Cancelled);
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }

                        let bytes_read = remote_file.read(&mut buffer).map_err(VfsError::Io)?;
                        if bytes_read == 0 {
                            break;
                        }

                        local_file
                            .write_all(&buffer[..bytes_read])
                            .map_err(VfsError::Io)?;
                        bytes_downloaded += bytes_read as u64;

                        // Send progress update
                        let _ = tx_progress.send(DownloadProgress {
                            bytes_downloaded,
                            total_bytes: file_size,
                            current_file: file_name.clone(),
                            files_downloaded: 0,
                            total_files: 1,
                            current_file_bytes: bytes_downloaded,
                            current_file_total: file_size,
                        });
                    }

                    // Send final progress
                    let _ = tx_progress.send(DownloadProgress {
                        bytes_downloaded,
                        total_bytes: file_size,
                        current_file: None,
                        files_downloaded: 1,
                        total_files: 1,
                        current_file_bytes: bytes_downloaded,
                        current_file_total: file_size,
                    });
                }

                Ok(local_path)
            })();

            let _ = tx_complete.send(result);
        });

        VfsDownloadOperation::new(rx_complete, rx_progress, pause_flag, cancel_flag)
    }

    fn supported_auth_methods(&self) -> Vec<AuthMethod> {
        vec![
            AuthMethod::SshAgent,
            AuthMethod::SshKey {
                private_key: PathBuf::new(),
                passphrase: None,
            },
            AuthMethod::Password(String::new()),
            AuthMethod::Auto,
        ]
    }

    fn supports_recursive(&self) -> bool {
        true
    }

    fn home_dir(&self) -> Option<VfsPath> {
        self.get_home_dir().map(|h| {
            VfsPath::remote(VfsProtocol::Sftp, &self.host, &h)
                .with_port(self.port)
                .with_username(self.effective_username())
        })
    }

    fn disk_space(&self, _path: &VfsPath) -> Option<DiskSpace> {
        // SFTP extension for statvfs is not widely supported
        // Could implement via SSH exec of `df` command
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sftp_provider_creation() {
        let provider = SftpProvider::new("example.com", 22, Some("user"));
        assert_eq!(provider.name(), "sftp");
        assert_eq!(provider.connection_state(), ConnectionState::Disconnected);
        assert!(!provider.is_connected());
    }

    #[test]
    fn test_effective_username() {
        let provider = SftpProvider::new("host", 22, Some("testuser"));
        assert_eq!(provider.effective_username(), "testuser");

        let provider2 = SftpProvider::new("host", 22, None);
        // Should fall back to environment variable or "root"
        assert!(!provider2.effective_username().is_empty());
    }

    #[test]
    fn test_to_remote_path() {
        let sftp_path = VfsPath::remote(VfsProtocol::Sftp, "host", "/home/user/file.txt");
        let result = SftpProvider::to_remote_path(&sftp_path);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("/home/user/file.txt"));

        let local_path = VfsPath::local("/local/path");
        let result = SftpProvider::to_remote_path(&local_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_supported_auth_methods() {
        let provider = SftpProvider::new("host", 22, None);
        let methods = provider.supported_auth_methods();
        assert!(!methods.is_empty());
    }
}
