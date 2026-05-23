//! SFTP (SSH File Transfer Protocol) VFS provider.
//!
//! Uses `russh` + `russh-sftp` (pure-Rust SSH stack) so the workspace can
//! build statically against musl without pulling OpenSSL/libssh2.
//!
//! Internally an async tokio actor task owns the `SftpSession`. The
//! synchronous `VfsProvider` surface communicates with it through
//! `mpsc<Command>` + `oneshot<Reply>`, blocking the calling thread on a
//! global tokio runtime created lazily through `OnceLock`. From the
//! outside, callers see the same blocking API as before.
//!
//! Phase 2a scope: connect/disconnect, password/ssh-key/agent/auto auth,
//! basic file operations, recursive delete. Progress reporting and
//! pause/resume for download/upload are stubbed to plain transfer in
//! this phase — added in subsequent phases.
//!
//! NOTE: known_hosts verification is intentionally not enforced yet
//! (accept-all server keys) — matches previous ssh2 behavior. Hardening
//! this is a separate task.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use russh::client;
use russh::keys::{decode_secret_key, load_secret_key, PrivateKey, PrivateKeyWithHashAlg};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, FileType as SftpFileType, OpenFlags};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Runtime;
use tokio::sync::{mpsc as async_mpsc, oneshot};

use crate::error::{VfsError, VfsResult};
use crate::traits::{DiskSpace, VfsProvider};
use crate::types::{
    AuthMethod, ConnectOptions, ConnectionState, DownloadProgress, UploadProgress,
    VfsDownloadOperation, VfsEntry, VfsFileType, VfsMetadata, VfsOperation, VfsPath, VfsProtocol,
    VfsUploadOperation,
};

/// Default connection timeout in seconds (matches the previous ssh2-based impl).
const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Chunk size for chunked I/O operations (64KB) — matches old behavior.
const CHUNK_SIZE: usize = 64 * 1024;

// ============================================================================
// Global tokio runtime
// ============================================================================

static SFTP_RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn runtime() -> &'static Runtime {
    SFTP_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("vfs-sftp")
            .enable_all()
            .build()
            .expect("failed to build SFTP tokio runtime")
    })
}

/// Run a future to completion on the global SFTP runtime, blocking the
/// calling (sync) thread. Safe to call from any non-tokio thread.
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    runtime().block_on(fut)
}

// ============================================================================
// Actor: commands, replies, task loop
// ============================================================================

type Reply<T> = oneshot::Sender<VfsResult<T>>;

/// SFTP entry as crossed-over from the actor to the sync side.
/// Decoupled from russh_sftp's `DirEntry` so callers don't carry the dep.
struct ActorEntry {
    name: String,
    metadata: VfsMetadata,
}

enum SftpCommand {
    ListDir {
        path: PathBuf,
        reply: Reply<Vec<ActorEntry>>,
    },
    Stat {
        path: PathBuf,
        reply: Reply<VfsMetadata>,
    },
    Exists {
        path: PathBuf,
        reply: Reply<bool>,
    },
    Mkdir {
        path: PathBuf,
        reply: Reply<()>,
    },
    MkdirRecursive {
        path: PathBuf,
        reply: Reply<()>,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
        reply: Reply<()>,
    },
    Read {
        path: PathBuf,
        reply: Reply<Vec<u8>>,
    },
    Write {
        path: PathBuf,
        data: Vec<u8>,
        reply: Reply<()>,
    },
    /// Recursive delete (file or directory), with depth limit.
    DeleteRecursive {
        path: PathBuf,
        depth_limit: usize,
        reply: Reply<()>,
    },
    /// SFTP-side copy via streaming (no temp file).
    CopyFile {
        from: PathBuf,
        to: PathBuf,
        reply: Reply<()>,
    },
    /// Download a single remote file to a local path.
    DownloadFile {
        remote: PathBuf,
        local: PathBuf,
        reply: Reply<()>,
    },
    /// Upload a single local file to a remote path.
    UploadFile {
        local: PathBuf,
        remote: PathBuf,
        reply: Reply<()>,
    },
    /// Recursive download remote dir → local dir.
    DownloadDir {
        remote: PathBuf,
        local: PathBuf,
        reply: Reply<()>,
    },
    /// Recursive upload local dir → remote dir.
    UploadDir {
        local: PathBuf,
        remote: PathBuf,
        reply: Reply<()>,
    },
    /// Download with chunk-level progress and pause/cancel support.
    /// `remote` may be a file or directory; the actor walks recursively.
    DownloadWithProgress {
        remote: PathBuf,
        local: PathBuf,
        pause: Arc<AtomicBool>,
        cancel: Arc<AtomicBool>,
        progress_tx: std_mpsc::Sender<DownloadProgress>,
        reply: Reply<()>,
    },
    /// Upload with chunk-level progress and pause/cancel support.
    /// `local` may be a file or directory; the actor walks recursively.
    UploadWithProgress {
        local: PathBuf,
        remote: PathBuf,
        pause: Arc<AtomicBool>,
        cancel: Arc<AtomicBool>,
        progress_tx: std_mpsc::Sender<UploadProgress>,
        reply: Reply<()>,
    },
    /// Tear down the actor cleanly.
    Shutdown,
}

/// Handle to the long-lived SFTP actor task.
struct SftpHandle {
    cmd_tx: async_mpsc::Sender<SftpCommand>,
}

/// Per-command timeout for sync dispatches. The actor only processes
/// one command at a time, so if a previous transfer left it stuck on
/// the server (e.g. an open file handle that the server hasn't closed
/// yet after a cancel), all subsequent UI calls — `metadata`,
/// `exists`, `list_dir` — would otherwise block the UI thread
/// forever. This is a safety net, not the happy path.
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(30);

impl SftpHandle {
    /// Send a command and block for the reply on the SFTP runtime.
    fn dispatch<T, F>(&self, build: F) -> VfsResult<T>
    where
        F: FnOnce(Reply<T>) -> SftpCommand,
    {
        let (tx, rx) = oneshot::channel();
        let cmd = build(tx);
        block_on(async move {
            self.cmd_tx
                .send(cmd)
                .await
                .map_err(|_| VfsError::NotConnected)?;
            match tokio::time::timeout(DISPATCH_TIMEOUT, rx).await {
                Ok(Ok(res)) => res,
                Ok(Err(_)) => Err(VfsError::NotConnected),
                Err(_) => Err(VfsError::RemoteError {
                    message: "SFTP request timed out (server is unresponsive)".into(),
                }),
            }
        })
    }
}

/// Async actor task: owns the SftpSession and serves commands until the
/// channel closes or `Shutdown` is received.
async fn sftp_actor(sftp: SftpSession, mut rx: async_mpsc::Receiver<SftpCommand>) {
    while let Some(cmd) = rx.recv().await {
        match cmd {
            SftpCommand::ListDir { path, reply } => {
                let _ = reply.send(actor_list_dir(&sftp, &path).await);
            }
            SftpCommand::Stat { path, reply } => {
                let _ = reply.send(actor_stat(&sftp, &path).await);
            }
            SftpCommand::Exists { path, reply } => {
                let _ = reply.send(Ok(sftp.metadata(path_to_string(&path)).await.is_ok()));
            }
            SftpCommand::Mkdir { path, reply } => {
                let _ = reply.send(map_sftp_unit(sftp.create_dir(path_to_string(&path)).await));
            }
            SftpCommand::MkdirRecursive { path, reply } => {
                let _ = reply.send(actor_mkdir_recursive(&sftp, &path).await);
            }
            SftpCommand::Rename { from, to, reply } => {
                let _ = reply.send(map_sftp_unit(
                    sftp.rename(path_to_string(&from), path_to_string(&to))
                        .await,
                ));
            }
            SftpCommand::Read { path, reply } => {
                let _ = reply.send(actor_read_file(&sftp, &path).await);
            }
            SftpCommand::Write { path, data, reply } => {
                let _ = reply.send(actor_write_file(&sftp, &path, &data).await);
            }
            SftpCommand::DeleteRecursive {
                path,
                depth_limit,
                reply,
            } => {
                let _ = reply.send(actor_delete_recursive(&sftp, &path, depth_limit).await);
            }
            SftpCommand::CopyFile { from, to, reply } => {
                let _ = reply.send(actor_copy_file(&sftp, &from, &to).await);
            }
            SftpCommand::DownloadFile {
                remote,
                local,
                reply,
            } => {
                let _ = reply.send(actor_download_file(&sftp, &remote, &local).await);
            }
            SftpCommand::UploadFile {
                local,
                remote,
                reply,
            } => {
                let _ = reply.send(actor_upload_file(&sftp, &local, &remote).await);
            }
            SftpCommand::DownloadDir {
                remote,
                local,
                reply,
            } => {
                let _ = reply.send(actor_download_dir(&sftp, &remote, &local).await);
            }
            SftpCommand::UploadDir {
                local,
                remote,
                reply,
            } => {
                let _ = reply.send(actor_upload_dir(&sftp, &local, &remote).await);
            }
            SftpCommand::DownloadWithProgress {
                remote,
                local,
                pause,
                cancel,
                progress_tx,
                reply,
            } => {
                // Cancellation is now driven inside the transfer
                // (per-chunk tokio::select! against cancel_watch) so the
                // function can close remote file handles explicitly
                // before returning. Wrapping the whole future in an
                // outer select! would skip that cleanup and leave the
                // SFTP session in a state where the next operation
                // can't make progress until the server gives up.
                let result = actor_download_with_progress(
                    &sftp,
                    &remote,
                    &local,
                    &pause,
                    &cancel,
                    &progress_tx,
                )
                .await;
                let _ = reply.send(result);
            }
            SftpCommand::UploadWithProgress {
                local,
                remote,
                pause,
                cancel,
                progress_tx,
                reply,
            } => {
                let result = actor_upload_with_progress(
                    &sftp,
                    &local,
                    &remote,
                    &pause,
                    &cancel,
                    &progress_tx,
                )
                .await;
                let _ = reply.send(result);
            }
            SftpCommand::Shutdown => break,
        }
    }
    let _ = sftp.close().await;
}

// ============================================================================
// Actor operation primitives
// ============================================================================

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn map_sftp_err<E: std::fmt::Display>(e: E) -> VfsError {
    VfsError::Sftp(e.to_string())
}

fn map_sftp_unit<T, E: std::fmt::Display>(r: Result<T, E>) -> VfsResult<()> {
    r.map(|_| ()).map_err(map_sftp_err)
}

fn attrs_to_metadata(attrs: &FileAttributes) -> VfsMetadata {
    let file_type = match attrs.file_type() {
        SftpFileType::Dir => VfsFileType::Directory,
        SftpFileType::Symlink => VfsFileType::Symlink,
        SftpFileType::File => VfsFileType::File,
        _ => VfsFileType::Other,
    };

    let modified = attrs
        .mtime
        .map(|secs| UNIX_EPOCH + Duration::from_secs(secs as u64));

    VfsMetadata {
        file_type,
        size: attrs.size.unwrap_or(0),
        modified,
        created: None,
        accessed: attrs
            .atime
            .map(|secs| UNIX_EPOCH + Duration::from_secs(secs as u64)),
        readonly: attrs.permissions.is_some_and(|p| p & 0o200 == 0),
        permissions: attrs.permissions,
    }
}

async fn actor_list_dir(sftp: &SftpSession, path: &Path) -> VfsResult<Vec<ActorEntry>> {
    let entries = sftp
        .read_dir(path_to_string(path))
        .await
        .map_err(map_sftp_err)?;
    let mut out = Vec::new();
    for entry in entries {
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        out.push(ActorEntry {
            metadata: attrs_to_metadata(&entry.metadata()),
            name,
        });
    }
    Ok(out)
}

async fn actor_stat(sftp: &SftpSession, path: &Path) -> VfsResult<VfsMetadata> {
    let attrs = sftp
        .metadata(path_to_string(path))
        .await
        .map_err(map_sftp_err)?;
    Ok(attrs_to_metadata(&attrs))
}

/// Create directory and all parents, ignoring "already exists" errors.
async fn actor_mkdir_recursive(sftp: &SftpSession, path: &Path) -> VfsResult<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        if current.as_os_str() == "/" {
            continue;
        }
        match sftp.create_dir(path_to_string(&current)).await {
            Ok(_) => {}
            Err(_) => {
                // Check whether it already exists as a directory.
                match sftp.metadata(path_to_string(&current)).await {
                    Ok(attrs) if matches!(attrs.file_type(), SftpFileType::Dir) => {}
                    Ok(_) => {
                        return Err(VfsError::Sftp(format!(
                            "Path '{}' exists but is not a directory",
                            current.display()
                        )));
                    }
                    Err(e) => {
                        return Err(VfsError::Sftp(format!(
                            "Failed to create remote directory '{}': {}",
                            current.display(),
                            e
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

async fn actor_read_file(sftp: &SftpSession, path: &Path) -> VfsResult<Vec<u8>> {
    let mut file = sftp
        .open(path_to_string(path))
        .await
        .map_err(map_sftp_err)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).await.map_err(map_sftp_err)?;
    Ok(buf)
}

async fn actor_write_file(sftp: &SftpSession, path: &Path, data: &[u8]) -> VfsResult<()> {
    let mut file = sftp
        .open_with_flags(
            path_to_string(path),
            OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::TRUNCATE,
        )
        .await
        .map_err(map_sftp_err)?;
    file.write_all(data).await.map_err(map_sftp_err)?;
    file.flush().await.map_err(map_sftp_err)?;
    file.shutdown().await.map_err(map_sftp_err)?;
    Ok(())
}

async fn actor_delete_recursive(
    sftp: &SftpSession,
    path: &Path,
    depth_limit: usize,
) -> VfsResult<()> {
    if depth_limit == 0 {
        return Err(VfsError::Sftp(format!(
            "delete recursion limit reached at {}",
            path.display()
        )));
    }
    let attrs = sftp
        .metadata(path_to_string(path))
        .await
        .map_err(map_sftp_err)?;
    if matches!(attrs.file_type(), SftpFileType::Dir) {
        let entries = sftp
            .read_dir(path_to_string(path))
            .await
            .map_err(map_sftp_err)?;
        for entry in entries {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let child = path.join(&name);
            Box::pin(actor_delete_recursive(sftp, &child, depth_limit - 1)).await?;
        }
        sftp.remove_dir(path_to_string(path))
            .await
            .map_err(map_sftp_err)?;
    } else {
        sftp.remove_file(path_to_string(path))
            .await
            .map_err(map_sftp_err)?;
    }
    Ok(())
}

/// SFTP has no native copy — stream read + write through chunks.
async fn actor_copy_file(sftp: &SftpSession, from: &Path, to: &Path) -> VfsResult<()> {
    let mut src = sftp
        .open(path_to_string(from))
        .await
        .map_err(map_sftp_err)?;
    let mut dst = sftp
        .open_with_flags(
            path_to_string(to),
            OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::TRUNCATE,
        )
        .await
        .map_err(map_sftp_err)?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = src.read(&mut buf).await.map_err(map_sftp_err)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).await.map_err(map_sftp_err)?;
    }
    dst.flush().await.map_err(map_sftp_err)?;
    dst.shutdown().await.map_err(map_sftp_err)?;
    Ok(())
}

async fn actor_download_file(sftp: &SftpSession, remote: &Path, local: &Path) -> VfsResult<()> {
    if let Some(parent) = local.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(VfsError::Io)?;
    }
    let mut src = sftp
        .open(path_to_string(remote))
        .await
        .map_err(map_sftp_err)?;
    let mut dst = tokio::fs::File::create(local).await.map_err(VfsError::Io)?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = src.read(&mut buf).await.map_err(map_sftp_err)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).await.map_err(VfsError::Io)?;
    }
    dst.flush().await.map_err(VfsError::Io)?;
    Ok(())
}

async fn actor_upload_file(sftp: &SftpSession, local: &Path, remote: &Path) -> VfsResult<()> {
    if let Some(parent) = remote.parent() {
        if parent.as_os_str() != "" && parent.as_os_str() != "/" {
            actor_mkdir_recursive(sftp, parent).await?;
        }
    }
    let mut src = tokio::fs::File::open(local).await.map_err(VfsError::Io)?;
    let mut dst = sftp
        .open_with_flags(
            path_to_string(remote),
            OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::TRUNCATE,
        )
        .await
        .map_err(map_sftp_err)?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = src.read(&mut buf).await.map_err(VfsError::Io)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).await.map_err(map_sftp_err)?;
    }
    dst.flush().await.map_err(map_sftp_err)?;
    dst.shutdown().await.map_err(map_sftp_err)?;
    Ok(())
}

async fn actor_download_dir(sftp: &SftpSession, remote: &Path, local: &Path) -> VfsResult<()> {
    tokio::fs::create_dir_all(local)
        .await
        .map_err(VfsError::Io)?;
    let entries = sftp
        .read_dir(path_to_string(remote))
        .await
        .map_err(map_sftp_err)?;
    for entry in entries {
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let remote_child = remote.join(&name);
        let local_child = local.join(&name);
        match entry.metadata().file_type() {
            SftpFileType::Dir => {
                Box::pin(actor_download_dir(sftp, &remote_child, &local_child)).await?;
            }
            _ => {
                actor_download_file(sftp, &remote_child, &local_child).await?;
            }
        }
    }
    Ok(())
}

async fn actor_upload_dir(sftp: &SftpSession, local: &Path, remote: &Path) -> VfsResult<()> {
    actor_mkdir_recursive(sftp, remote).await?;
    let mut entries = tokio::fs::read_dir(local).await.map_err(VfsError::Io)?;
    while let Some(entry) = entries.next_entry().await.map_err(VfsError::Io)? {
        let name = entry.file_name();
        let local_child = entry.path();
        let remote_child = remote.join(&name);
        let ft = entry.file_type().await.map_err(VfsError::Io)?;
        if ft.is_dir() {
            Box::pin(actor_upload_dir(sftp, &local_child, &remote_child)).await?;
        } else if ft.is_file() {
            actor_upload_file(sftp, &local_child, &remote_child).await?;
        }
    }
    Ok(())
}

// ============================================================================
// Chunk-level progress + pause/cancel for download and upload
// ============================================================================

/// Rolling state shared between top-level and recursive helpers so the
/// progress reports reflect the whole transfer, not just the current file.
struct DlState {
    total_files: usize,
    total_bytes: u64,
    files_done: usize,
    bytes_done: u64,
}

struct UlState {
    total_files: usize,
    total_bytes: u64,
    files_done: usize,
    bytes_done: u64,
}

/// Bail out if cancel was requested; otherwise block while pause is set.
async fn wait_or_cancel(pause: &Arc<AtomicBool>, cancel: &Arc<AtomicBool>) -> VfsResult<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(VfsError::Cancelled);
    }
    while pause.load(Ordering::Relaxed) {
        if cancel.load(Ordering::Relaxed) {
            return Err(VfsError::Cancelled);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

/// Future that resolves when `cancel` becomes `true`. Used with
/// `tokio::select!` to interrupt in-flight server I/O the moment the
/// user cancels — otherwise a slow / stuck `src.read().await` would
/// pin the actor and block every subsequent VFS operation.
async fn cancel_watch(cancel: Arc<AtomicBool>) {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn actor_count_remote_files(
    sftp: &SftpSession,
    path: &Path,
    cancel: &Arc<AtomicBool>,
) -> VfsResult<(usize, u64)> {
    if cancel.load(Ordering::Relaxed) {
        return Err(VfsError::Cancelled);
    }
    let attrs = sftp
        .metadata(path_to_string(path))
        .await
        .map_err(map_sftp_err)?;
    if !matches!(attrs.file_type(), SftpFileType::Dir) {
        return Ok((1, attrs.size.unwrap_or(0)));
    }
    let entries = sftp
        .read_dir(path_to_string(path))
        .await
        .map_err(map_sftp_err)?;
    let mut count = 0;
    let mut bytes = 0u64;
    for entry in entries {
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let child = path.join(&name);
        let (c, b) = Box::pin(actor_count_remote_files(sftp, &child, cancel)).await?;
        count += c;
        bytes += b;
    }
    Ok((count, bytes))
}

fn count_local_files_sync(path: &Path, cancel: &Arc<AtomicBool>) -> VfsResult<(usize, u64)> {
    if cancel.load(Ordering::Relaxed) {
        return Err(VfsError::Cancelled);
    }
    let meta = std::fs::metadata(path).map_err(VfsError::Io)?;
    if !meta.is_dir() {
        return Ok((1, meta.len()));
    }
    let mut count = 0;
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(path).map_err(VfsError::Io)? {
        let entry = entry.map_err(VfsError::Io)?;
        let (c, b) = count_local_files_sync(&entry.path(), cancel)?;
        count += c;
        bytes += b;
    }
    Ok((count, bytes))
}

async fn actor_download_with_progress(
    sftp: &SftpSession,
    remote: &Path,
    local: &Path,
    pause: &Arc<AtomicBool>,
    cancel: &Arc<AtomicBool>,
    progress_tx: &std_mpsc::Sender<DownloadProgress>,
) -> VfsResult<()> {
    wait_or_cancel(pause, cancel).await?;
    let attrs = sftp
        .metadata(path_to_string(remote))
        .await
        .map_err(map_sftp_err)?;
    let is_dir = matches!(attrs.file_type(), SftpFileType::Dir);
    let (total_files, total_bytes) = if is_dir {
        actor_count_remote_files(sftp, remote, cancel).await?
    } else {
        (1usize, attrs.size.unwrap_or(0))
    };
    let mut state = DlState {
        total_files,
        total_bytes,
        files_done: 0,
        bytes_done: 0,
    };
    if is_dir {
        actor_dl_dir_with_progress(sftp, remote, local, pause, cancel, progress_tx, &mut state)
            .await?;
    } else {
        actor_dl_file_with_progress(sftp, remote, local, pause, cancel, progress_tx, &mut state)
            .await?;
    }
    Ok(())
}

async fn actor_dl_dir_with_progress(
    sftp: &SftpSession,
    remote: &Path,
    local: &Path,
    pause: &Arc<AtomicBool>,
    cancel: &Arc<AtomicBool>,
    progress_tx: &std_mpsc::Sender<DownloadProgress>,
    state: &mut DlState,
) -> VfsResult<()> {
    tokio::fs::create_dir_all(local)
        .await
        .map_err(VfsError::Io)?;
    let entries = sftp
        .read_dir(path_to_string(remote))
        .await
        .map_err(map_sftp_err)?;
    for entry in entries {
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let remote_child = remote.join(&name);
        let local_child = local.join(&name);
        match entry.metadata().file_type() {
            SftpFileType::Dir => {
                Box::pin(actor_dl_dir_with_progress(
                    sftp,
                    &remote_child,
                    &local_child,
                    pause,
                    cancel,
                    progress_tx,
                    state,
                ))
                .await?;
            }
            _ => {
                actor_dl_file_with_progress(
                    sftp,
                    &remote_child,
                    &local_child,
                    pause,
                    cancel,
                    progress_tx,
                    state,
                )
                .await?;
            }
        }
    }
    Ok(())
}

async fn actor_dl_file_with_progress(
    sftp: &SftpSession,
    remote: &Path,
    local: &Path,
    pause: &Arc<AtomicBool>,
    cancel: &Arc<AtomicBool>,
    progress_tx: &std_mpsc::Sender<DownloadProgress>,
    state: &mut DlState,
) -> VfsResult<()> {
    if let Some(parent) = local.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(VfsError::Io)?;
    }
    let file_total = sftp
        .metadata(path_to_string(remote))
        .await
        .map_err(map_sftp_err)?
        .size
        .unwrap_or(0);
    let src = sftp
        .open(path_to_string(remote))
        .await
        .map_err(map_sftp_err)?;
    let mut src = src;
    let mut dst = tokio::fs::File::create(local).await.map_err(VfsError::Io)?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut current_bytes = 0u64;
    let current_name = remote.file_name().map(|s| s.to_string_lossy().into_owned());

    // Emit an initial progress event so the UI knows we started.
    let _ = progress_tx.send(DownloadProgress {
        bytes_downloaded: state.bytes_done,
        total_bytes: state.total_bytes,
        current_file: current_name.clone(),
        files_downloaded: state.files_done,
        total_files: state.total_files,
        current_file_bytes: 0,
        current_file_total: file_total,
    });

    let mut cancelled = false;
    loop {
        if let Err(_e) = wait_or_cancel(pause, cancel).await {
            cancelled = true;
            break;
        }
        // tokio::select! against the cancel watcher around the SFTP read
        // so the server doesn't keep us blocked on a slow chunk after
        // the user has already cancelled.
        let read_res = tokio::select! {
            biased;
            _ = cancel_watch(Arc::clone(cancel)) => None,
            res = src.read(&mut buf) => Some(res),
        };
        let n = match read_res {
            Some(Ok(n)) => n,
            Some(Err(e)) => {
                let _ = tokio::time::timeout(Duration::from_secs(3), src.shutdown()).await;
                let _ = dst.flush().await;
                return Err(map_sftp_err(e));
            }
            None => {
                cancelled = true;
                break;
            }
        };
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).await.map_err(VfsError::Io)?;
        current_bytes += n as u64;
        state.bytes_done += n as u64;
        let _ = progress_tx.send(DownloadProgress {
            bytes_downloaded: state.bytes_done,
            total_bytes: state.total_bytes,
            current_file: current_name.clone(),
            files_downloaded: state.files_done,
            total_files: state.total_files,
            current_file_bytes: current_bytes,
            current_file_total: file_total,
        });
    }
    // Always close the remote read handle (bounded) so the SFTP actor
    // can move on to the next command — leaving open handles strung
    // along has caused subsequent ops to wedge until the server times
    // out.
    let _ = tokio::time::timeout(Duration::from_secs(3), src.shutdown()).await;
    let _ = dst.flush().await;
    if cancelled {
        return Err(VfsError::Cancelled);
    }
    state.files_done += 1;
    Ok(())
}

async fn actor_upload_with_progress(
    sftp: &SftpSession,
    local: &Path,
    remote: &Path,
    pause: &Arc<AtomicBool>,
    cancel: &Arc<AtomicBool>,
    progress_tx: &std_mpsc::Sender<UploadProgress>,
) -> VfsResult<()> {
    wait_or_cancel(pause, cancel).await?;
    let meta = std::fs::metadata(local).map_err(VfsError::Io)?;
    let is_dir = meta.is_dir();
    let (total_files, total_bytes) = if is_dir {
        count_local_files_sync(local, cancel)?
    } else {
        (1usize, meta.len())
    };
    let mut state = UlState {
        total_files,
        total_bytes,
        files_done: 0,
        bytes_done: 0,
    };
    if is_dir {
        actor_ul_dir_with_progress(sftp, local, remote, pause, cancel, progress_tx, &mut state)
            .await?;
    } else {
        actor_ul_file_with_progress(sftp, local, remote, pause, cancel, progress_tx, &mut state)
            .await?;
    }
    Ok(())
}

async fn actor_ul_dir_with_progress(
    sftp: &SftpSession,
    local: &Path,
    remote: &Path,
    pause: &Arc<AtomicBool>,
    cancel: &Arc<AtomicBool>,
    progress_tx: &std_mpsc::Sender<UploadProgress>,
    state: &mut UlState,
) -> VfsResult<()> {
    actor_mkdir_recursive(sftp, remote).await?;
    let mut entries = tokio::fs::read_dir(local).await.map_err(VfsError::Io)?;
    while let Some(entry) = entries.next_entry().await.map_err(VfsError::Io)? {
        let name = entry.file_name();
        let local_child = entry.path();
        let remote_child = remote.join(&name);
        let ft = entry.file_type().await.map_err(VfsError::Io)?;
        if ft.is_dir() {
            Box::pin(actor_ul_dir_with_progress(
                sftp,
                &local_child,
                &remote_child,
                pause,
                cancel,
                progress_tx,
                state,
            ))
            .await?;
        } else if ft.is_file() {
            actor_ul_file_with_progress(
                sftp,
                &local_child,
                &remote_child,
                pause,
                cancel,
                progress_tx,
                state,
            )
            .await?;
        }
    }
    Ok(())
}

async fn actor_ul_file_with_progress(
    sftp: &SftpSession,
    local: &Path,
    remote: &Path,
    pause: &Arc<AtomicBool>,
    cancel: &Arc<AtomicBool>,
    progress_tx: &std_mpsc::Sender<UploadProgress>,
    state: &mut UlState,
) -> VfsResult<()> {
    if let Some(parent) = remote.parent() {
        if parent.as_os_str() != "" && parent.as_os_str() != "/" {
            actor_mkdir_recursive(sftp, parent).await?;
        }
    }
    let file_total = std::fs::metadata(local).map(|m| m.len()).unwrap_or(0);
    let mut src = tokio::fs::File::open(local).await.map_err(VfsError::Io)?;
    let mut dst = sftp
        .open_with_flags(
            path_to_string(remote),
            OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::TRUNCATE,
        )
        .await
        .map_err(map_sftp_err)?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut current_bytes = 0u64;
    let current_name = local.file_name().map(|s| s.to_string_lossy().into_owned());

    let _ = progress_tx.send(UploadProgress {
        bytes_uploaded: state.bytes_done,
        total_bytes: state.total_bytes,
        current_file: current_name.clone(),
        files_uploaded: state.files_done,
        total_files: state.total_files,
        current_file_bytes: 0,
        current_file_total: file_total,
    });

    let mut cancelled = false;
    loop {
        if let Err(_e) = wait_or_cancel(pause, cancel).await {
            cancelled = true;
            break;
        }
        let n = match src.read(&mut buf).await {
            Ok(n) => n,
            Err(e) => {
                let _ = tokio::time::timeout(Duration::from_secs(3), dst.shutdown()).await;
                return Err(VfsError::Io(e));
            }
        };
        if n == 0 {
            break;
        }
        // tokio::select! around the SFTP write so cancel during a slow
        // network write doesn't pin the actor until the server times
        // out the channel.
        let write_res = tokio::select! {
            biased;
            _ = cancel_watch(Arc::clone(cancel)) => None,
            res = dst.write_all(&buf[..n]) => Some(res),
        };
        match write_res {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                let _ = tokio::time::timeout(Duration::from_secs(3), dst.shutdown()).await;
                return Err(map_sftp_err(e));
            }
            None => {
                cancelled = true;
                break;
            }
        }
        current_bytes += n as u64;
        state.bytes_done += n as u64;
        let _ = progress_tx.send(UploadProgress {
            bytes_uploaded: state.bytes_done,
            total_bytes: state.total_bytes,
            current_file: current_name.clone(),
            files_uploaded: state.files_done,
            total_files: state.total_files,
            current_file_bytes: current_bytes,
            current_file_total: file_total,
        });
    }
    // Always shutdown the remote write handle (bounded). For a
    // cancelled upload this is what makes the server flush its state
    // and stop blocking the next operation on this SFTP session.
    let _ = tokio::time::timeout(Duration::from_secs(3), dst.shutdown()).await;
    if cancelled {
        return Err(VfsError::Cancelled);
    }
    state.files_done += 1;
    Ok(())
}

// ============================================================================
// SSH handshake / auth (runs on the runtime, ferries the session to actor)
// ============================================================================

/// Accept-all handler — matches previous ssh2 behavior (no known_hosts check).
struct AcceptAllHandler;

impl client::Handler for AcceptAllHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// Locate default SSH private key files for current user, in priority
/// order. Mirrors what the previous Auto auth would attempt.
fn default_key_files() -> Vec<PathBuf> {
    let mut keys = Vec::new();
    if let Some(home) = dirs::home_dir() {
        for name in ["id_ed25519", "id_rsa", "id_ecdsa", "id_dsa"] {
            let p = home.join(".ssh").join(name);
            if p.exists() {
                keys.push(p);
            }
        }
    }
    keys
}

/// Fallback username when none is provided: $USER → $USERNAME → "root".
fn fallback_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "root".to_string())
}

/// Expand a leading `~` in a path using `$HOME`. Returns the input
/// unchanged if there's no tilde or no home directory.
fn expand_tilde(p: &Path) -> PathBuf {
    let s = match p.to_str() {
        Some(s) => s,
        None => return p.to_path_buf(),
    };
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if s == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    p.to_path_buf()
}

/// Try every reasonable auth method until one succeeds. Stops at the
/// first success; returns AuthenticationFailed if none work.
///
/// `host` is used to look up Host-specific entries in `~/.ssh/config`
/// (IdentityFile / IdentitiesOnly) when `auth == Auto`.
async fn try_authenticate(
    session: &mut client::Handle<AcceptAllHandler>,
    host: &str,
    username: &str,
    auth: &AuthMethod,
    cancelled: &Arc<AtomicBool>,
) -> VfsResult<()> {
    let cancelled_check = || -> VfsResult<()> {
        if cancelled.load(Ordering::SeqCst) {
            Err(VfsError::ConnectionFailed("Connection cancelled".into()))
        } else {
            Ok(())
        }
    };

    cancelled_check()?;

    match auth {
        AuthMethod::None => {
            // Most permissive interpretation of "None": try SSH agent.
            try_auth_agent(session, username).await
        }
        AuthMethod::Password(password) => try_auth_password(session, username, password).await,
        AuthMethod::SshKey {
            private_key,
            passphrase,
        } => try_auth_keyfile(session, username, private_key, passphrase.as_deref()).await,
        AuthMethod::SshAgent => try_auth_agent(session, username).await,
        AuthMethod::Auto => {
            // Phase 2b: honor ~/.ssh/config IdentityFile / IdentitiesOnly.
            // Priority order matches OpenSSH's default behavior:
            //   1. SSH agent (unless IdentitiesOnly=yes)
            //   2. Keys from IdentityFile entries in ssh_config
            //   3. Default keys in ~/.ssh/id_*
            let host_cfg = crate::ssh_config::SshConfig::from_default_path()
                .map(|cfg| cfg.get_host_config(host))
                .unwrap_or_default();

            if !host_cfg.identities_only && try_auth_agent(session, username).await.is_ok() {
                return Ok(());
            }
            cancelled_check()?;

            // Try keys from ssh_config first (they're authoritative).
            for raw_key in &host_cfg.identity_files {
                let key_path = expand_tilde(raw_key);
                if !key_path.exists() {
                    continue;
                }
                if try_auth_keyfile(session, username, &key_path, None)
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
                cancelled_check()?;
            }

            // Fallback to default keys unless IdentitiesOnly forbids it.
            if !host_cfg.identities_only {
                for key_path in default_key_files() {
                    if try_auth_keyfile(session, username, &key_path, None)
                        .await
                        .is_ok()
                    {
                        return Ok(());
                    }
                    cancelled_check()?;
                }
            }

            Err(VfsError::AuthenticationFailed(
                "No authentication method succeeded (tried SSH agent and SSH keys). \
                 Provide a password or specific key."
                    .into(),
            ))
        }
    }
}

async fn try_auth_password(
    session: &mut client::Handle<AcceptAllHandler>,
    username: &str,
    password: &str,
) -> VfsResult<()> {
    let res = session
        .authenticate_password(username, password)
        .await
        .map_err(|e| VfsError::AuthenticationFailed(format!("password auth error: {e}")))?;
    if res.success() {
        Ok(())
    } else {
        Err(VfsError::AuthenticationFailed("Password rejected".into()))
    }
}

async fn try_auth_keyfile(
    session: &mut client::Handle<AcceptAllHandler>,
    username: &str,
    key_path: &Path,
    passphrase: Option<&str>,
) -> VfsResult<()> {
    let key = load_secret_key(key_path, passphrase).map_err(|e| {
        VfsError::AuthenticationFailed(format!(
            "Failed to load private key '{}': {}",
            key_path.display(),
            e
        ))
    })?;
    authenticate_with_key(session, username, key).await
}

async fn authenticate_with_key(
    session: &mut client::Handle<AcceptAllHandler>,
    username: &str,
    key: PrivateKey,
) -> VfsResult<()> {
    let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key), None);
    let res = session
        .authenticate_publickey(username, key_with_alg)
        .await
        .map_err(|e| VfsError::AuthenticationFailed(format!("publickey auth error: {e}")))?;
    if res.success() {
        Ok(())
    } else {
        Err(VfsError::AuthenticationFailed("Public key rejected".into()))
    }
}

#[cfg(unix)]
async fn try_auth_agent(
    session: &mut client::Handle<AcceptAllHandler>,
    username: &str,
) -> VfsResult<()> {
    use russh::keys::agent::client::AgentClient;
    if std::env::var_os("SSH_AUTH_SOCK").is_none() {
        return Err(VfsError::AuthenticationFailed(
            "SSH agent not available (SSH_AUTH_SOCK unset)".into(),
        ));
    }
    let mut agent = AgentClient::connect_env()
        .await
        .map_err(|e| VfsError::AuthenticationFailed(format!("agent connect failed: {e}")))?;
    let identities = agent
        .request_identities()
        .await
        .map_err(|e| VfsError::AuthenticationFailed(format!("agent identities failed: {e}")))?;
    for identity in identities {
        let pk = identity.public_key().into_owned();
        let res = session
            .authenticate_publickey_with(username, pk, None, &mut agent)
            .await;
        if let Ok(auth_result) = res {
            if auth_result.success() {
                return Ok(());
            }
        }
    }
    Err(VfsError::AuthenticationFailed(
        "SSH agent had no usable keys".into(),
    ))
}

#[cfg(not(unix))]
async fn try_auth_agent(
    _session: &mut client::Handle<AcceptAllHandler>,
    _username: &str,
) -> VfsResult<()> {
    Err(VfsError::AuthenticationFailed(
        "SSH agent not supported on this platform".into(),
    ))
}

/// Top-level async connect routine. Runs on the global runtime.
async fn do_connect(
    host: String,
    port: u16,
    username: String,
    options: ConnectOptions,
    cancelled: Arc<AtomicBool>,
) -> VfsResult<(SftpSession, Option<String>)> {
    if cancelled.load(Ordering::SeqCst) {
        return Err(VfsError::ConnectionFailed("Connection cancelled".into()));
    }
    let timeout_secs = options.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(timeout_secs * 5)),
        ..Default::default()
    });

    let handler = AcceptAllHandler;
    let addr = (host.as_str(), port);

    let connect_fut = client::connect(config, addr, handler);
    let mut session = tokio::time::timeout(Duration::from_secs(timeout_secs), connect_fut)
        .await
        .map_err(|_| VfsError::ConnectionFailed(format!("Connection to {host}:{port} timed out")))?
        .map_err(|e| VfsError::ConnectionFailed(format!("SSH connect failed: {e}")))?;

    if cancelled.load(Ordering::SeqCst) {
        return Err(VfsError::ConnectionFailed("Connection cancelled".into()));
    }

    try_authenticate(&mut session, &host, &username, &options.auth, &cancelled).await?;

    if cancelled.load(Ordering::SeqCst) {
        return Err(VfsError::ConnectionFailed("Connection cancelled".into()));
    }

    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| VfsError::ConnectionFailed(format!("SSH channel open failed: {e}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| VfsError::ConnectionFailed(format!("SFTP subsystem request failed: {e}")))?;

    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| VfsError::ConnectionFailed(format!("SFTP session init failed: {e}")))?;

    let home_dir = sftp
        .canonicalize(".")
        .await
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| Some(format!("/home/{username}")));

    Ok((sftp, home_dir))
}

// ============================================================================
// Public SftpProvider
// ============================================================================

struct SftpInner {
    state: ConnectionState,
    handle: Option<SftpHandle>,
    home_dir: Option<String>,
    connect_started: Option<Instant>,
    cancelled: Arc<AtomicBool>,
}

impl SftpInner {
    fn new() -> Self {
        Self {
            state: ConnectionState::Disconnected,
            handle: None,
            home_dir: None,
            connect_started: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// SFTP filesystem provider.
pub struct SftpProvider {
    host: String,
    port: u16,
    username: Option<String>,
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

    fn effective_username(&self) -> String {
        match &self.username {
            Some(u) if !u.is_empty() => u.clone(),
            _ => fallback_username(),
        }
    }

    fn to_remote_path(path: &VfsPath) -> VfsResult<PathBuf> {
        if !matches!(path.protocol, VfsProtocol::Sftp) {
            return Err(VfsError::InvalidPath(format!(
                "Expected SFTP path, got: {path}"
            )));
        }
        Ok(path.path.clone())
    }

    /// True while a connect attempt is in flight (for UI spinner).
    pub fn is_connecting(&self) -> bool {
        self.inner
            .lock()
            .map(|i| i.state == ConnectionState::Connecting)
            .unwrap_or(false)
    }

    /// Time elapsed since the current connect attempt started.
    pub fn connection_elapsed(&self) -> Option<Duration> {
        self.inner
            .lock()
            .ok()
            .and_then(|i| i.connect_started.map(|t| t.elapsed()))
    }

    /// Signal cancellation of the in-flight connect attempt.
    pub fn cancel_connection(&self) {
        if let Ok(i) = self.inner.lock() {
            i.cancelled.store(true, Ordering::SeqCst);
        }
    }

    fn get_handle(&self) -> VfsResult<SftpHandle> {
        let guard = self.inner.lock().map_err(|_| VfsError::RemoteError {
            message: "SFTP state poisoned".into(),
        })?;
        match &guard.handle {
            Some(h) => Ok(SftpHandle {
                cmd_tx: h.cmd_tx.clone(),
            }),
            None => Err(VfsError::NotConnected),
        }
    }

    fn dispatch_op<T, F>(&self, build: F) -> VfsOperation<T>
    where
        T: Send + 'static,
        F: FnOnce(Reply<T>) -> SftpCommand + Send + 'static,
    {
        let handle = match self.get_handle() {
            Ok(h) => h,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        let (tx, rx) = std_mpsc::channel();
        thread::spawn(move || {
            let res = handle.dispatch(build);
            let _ = tx.send(res);
        });
        VfsOperation::new(rx)
    }
}

impl Drop for SftpProvider {
    fn drop(&mut self) {
        // Best-effort shutdown of the actor.
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(handle) = inner.handle.take() {
                let _ = block_on(handle.cmd_tx.send(SftpCommand::Shutdown));
            }
            inner.state = ConnectionState::Disconnected;
        }
    }
}

// ============================================================================
// VfsProvider impl
// ============================================================================

impl VfsProvider for SftpProvider {
    fn name(&self) -> &'static str {
        "sftp"
    }

    fn connection_state(&self) -> ConnectionState {
        self.inner
            .lock()
            .map(|i| i.state)
            .unwrap_or(ConnectionState::Failed)
    }

    fn connect(&mut self, options: ConnectOptions) -> VfsOperation<()> {
        let cancelled = {
            let mut inner = match self.inner.lock() {
                Ok(g) => g,
                Err(_) => {
                    return VfsOperation::ready(Err(VfsError::RemoteError {
                        message: "SFTP state poisoned".into(),
                    }))
                }
            };
            if inner.state == ConnectionState::Connected {
                return VfsOperation::ready(Err(VfsError::RemoteError {
                    message: "Already connected".into(),
                }));
            }
            inner.state = ConnectionState::Connecting;
            inner.connect_started = Some(Instant::now());
            inner.cancelled = Arc::new(AtomicBool::new(false));
            inner.home_dir = None;
            inner.handle = None;
            Arc::clone(&inner.cancelled)
        };

        let host = self.host.clone();
        let port = self.port;
        let username = self.effective_username();
        let inner_arc = Arc::clone(&self.inner);

        let (tx, rx) = std_mpsc::channel();

        thread::spawn(move || {
            let result = block_on(do_connect(
                host.clone(),
                port,
                username.clone(),
                options,
                cancelled,
            ));

            match result {
                Ok((sftp, home_dir)) => {
                    let (cmd_tx, cmd_rx) = async_mpsc::channel::<SftpCommand>(32);
                    runtime().spawn(sftp_actor(sftp, cmd_rx));
                    if let Ok(mut inner) = inner_arc.lock() {
                        inner.state = ConnectionState::Connected;
                        inner.handle = Some(SftpHandle { cmd_tx });
                        inner.home_dir = home_dir;
                    }
                    let _ = tx.send(Ok(()));
                }
                Err(e) => {
                    if let Ok(mut inner) = inner_arc.lock() {
                        inner.state = ConnectionState::Failed;
                    }
                    let _ = tx.send(Err(e));
                }
            }
        });

        VfsOperation::new(rx)
    }

    fn disconnect(&mut self) {
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if let Some(handle) = inner.handle.take() {
            let _ = block_on(handle.cmd_tx.send(SftpCommand::Shutdown));
        }
        inner.state = ConnectionState::Disconnected;
        inner.home_dir = None;
    }

    fn list_dir(&self, path: &VfsPath) -> VfsOperation<Vec<VfsEntry>> {
        let remote_path = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        let parent = path.clone();
        let handle = match self.get_handle() {
            Ok(h) => h,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        let (tx, rx) = std_mpsc::channel();
        thread::spawn(move || {
            let res = handle.dispatch(|reply| SftpCommand::ListDir {
                path: remote_path,
                reply,
            });
            let entries = res.map(|raw| {
                let mut entries: Vec<VfsEntry> = raw
                    .into_iter()
                    .map(|e| {
                        let p = parent.join(&e.name);
                        VfsEntry::new(e.name, p, e.metadata)
                    })
                    .collect();
                entries.sort_by(|a, b| match (a.is_dir(), b.is_dir()) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                });
                entries
            });
            let _ = tx.send(entries);
        });
        VfsOperation::new(rx)
    }

    fn create_dir(&self, path: &VfsPath) -> VfsOperation<()> {
        let p = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        self.dispatch_op(move |reply| SftpCommand::Mkdir { path: p, reply })
    }

    fn create_dir_all(&self, path: &VfsPath) -> VfsOperation<()> {
        let p = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        self.dispatch_op(move |reply| SftpCommand::MkdirRecursive { path: p, reply })
    }

    fn exists(&self, path: &VfsPath) -> VfsOperation<bool> {
        let p = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        self.dispatch_op(move |reply| SftpCommand::Exists { path: p, reply })
    }

    fn metadata(&self, path: &VfsPath) -> VfsOperation<VfsMetadata> {
        let p = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        self.dispatch_op(move |reply| SftpCommand::Stat { path: p, reply })
    }

    fn read_file(&self, path: &VfsPath) -> VfsOperation<Vec<u8>> {
        let p = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        self.dispatch_op(move |reply| SftpCommand::Read { path: p, reply })
    }

    fn write_file(&self, path: &VfsPath, data: &[u8]) -> VfsOperation<()> {
        let p = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        let data = data.to_vec();
        self.dispatch_op(move |reply| SftpCommand::Write {
            path: p,
            data,
            reply,
        })
    }

    fn delete(&self, path: &VfsPath) -> VfsOperation<()> {
        // Match previous behavior: delete is recursive on SFTP.
        self.delete_recursive(path)
    }

    fn delete_recursive(&self, path: &VfsPath) -> VfsOperation<()> {
        let p = match Self::to_remote_path(path) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        self.dispatch_op(move |reply| SftpCommand::DeleteRecursive {
            path: p,
            depth_limit: crate::MAX_RECURSION_DEPTH,
            reply,
        })
    }

    fn rename(&self, from: &VfsPath, to: &VfsPath) -> VfsOperation<()> {
        let from_p = match Self::to_remote_path(from) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        let to_p = match Self::to_remote_path(to) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        self.dispatch_op(move |reply| SftpCommand::Rename {
            from: from_p,
            to: to_p,
            reply,
        })
    }

    fn copy(&self, from: &VfsPath, to: &VfsPath) -> VfsOperation<()> {
        let from_p = match Self::to_remote_path(from) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        let to_p = match Self::to_remote_path(to) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        self.dispatch_op(move |reply| SftpCommand::CopyFile {
            from: from_p,
            to: to_p,
            reply,
        })
    }

    fn download(&self, remote: &VfsPath, local: &Path) -> VfsOperation<PathBuf> {
        let remote_p = match Self::to_remote_path(remote) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        let local_path = local.to_path_buf();
        let result_local = local_path.clone();
        let handle = match self.get_handle() {
            Ok(h) => h,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        let (tx, rx) = std_mpsc::channel();
        thread::spawn(move || {
            let result: VfsResult<PathBuf> = (|| -> VfsResult<PathBuf> {
                let meta = handle.dispatch(|reply| SftpCommand::Stat {
                    path: remote_p.clone(),
                    reply,
                })?;
                if meta.file_type.is_dir() {
                    handle.dispatch(|reply| SftpCommand::DownloadDir {
                        remote: remote_p,
                        local: local_path,
                        reply,
                    })?;
                } else {
                    handle.dispatch(|reply| SftpCommand::DownloadFile {
                        remote: remote_p,
                        local: local_path,
                        reply,
                    })?;
                }
                Ok(result_local)
            })();
            let _ = tx.send(result);
        });
        VfsOperation::new(rx)
    }

    fn upload(&self, local: &Path, remote: &VfsPath) -> VfsOperation<()> {
        let remote_p = match Self::to_remote_path(remote) {
            Ok(p) => p,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        let local_path = local.to_path_buf();
        let handle = match self.get_handle() {
            Ok(h) => h,
            Err(e) => return VfsOperation::ready(Err(e)),
        };
        let (tx, rx) = std_mpsc::channel();
        thread::spawn(move || {
            let result: VfsResult<()> = (|| -> VfsResult<()> {
                let is_dir = std::fs::metadata(&local_path)
                    .map(|m| m.is_dir())
                    .unwrap_or(false);
                if is_dir {
                    handle.dispatch(|reply| SftpCommand::UploadDir {
                        local: local_path,
                        remote: remote_p,
                        reply,
                    })?;
                } else {
                    handle.dispatch(|reply| SftpCommand::UploadFile {
                        local: local_path,
                        remote: remote_p,
                        reply,
                    })?;
                }
                Ok(())
            })();
            let _ = tx.send(result);
        });
        VfsOperation::new(rx)
    }

    fn upload_with_progress(&self, local: &Path, remote: &VfsPath) -> VfsUploadOperation {
        let remote_p = match Self::to_remote_path(remote) {
            Ok(p) => p,
            Err(e) => return VfsUploadOperation::error(e),
        };
        let local_path = local.to_path_buf();
        let handle = match self.get_handle() {
            Ok(h) => h,
            Err(e) => return VfsUploadOperation::error(e),
        };
        let (completion_tx, completion_rx) = std_mpsc::channel();
        let (progress_tx, progress_rx) = std_mpsc::channel();
        let pause_flag = Arc::new(AtomicBool::new(false));
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let pause_for_actor = Arc::clone(&pause_flag);
        let cancel_for_actor = Arc::clone(&cancel_flag);

        thread::spawn(move || {
            let result = handle.dispatch(move |reply| SftpCommand::UploadWithProgress {
                local: local_path,
                remote: remote_p,
                pause: pause_for_actor,
                cancel: cancel_for_actor,
                progress_tx,
                reply,
            });
            let _ = completion_tx.send(result);
        });

        VfsUploadOperation::new(completion_rx, progress_rx, pause_flag, cancel_flag)
    }

    fn download_with_progress(&self, remote: &VfsPath, local: &Path) -> VfsDownloadOperation {
        let remote_p = match Self::to_remote_path(remote) {
            Ok(p) => p,
            Err(e) => return VfsDownloadOperation::error(e),
        };
        let local_path = local.to_path_buf();
        let result_local = local_path.clone();
        let handle = match self.get_handle() {
            Ok(h) => h,
            Err(e) => return VfsDownloadOperation::error(e),
        };
        let (completion_tx, completion_rx) = std_mpsc::channel();
        let (progress_tx, progress_rx) = std_mpsc::channel();
        let pause_flag = Arc::new(AtomicBool::new(false));
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let pause_for_actor = Arc::clone(&pause_flag);
        let cancel_for_actor = Arc::clone(&cancel_flag);

        thread::spawn(move || {
            let res: VfsResult<PathBuf> = handle
                .dispatch(move |reply| SftpCommand::DownloadWithProgress {
                    remote: remote_p,
                    local: local_path,
                    pause: pause_for_actor,
                    cancel: cancel_for_actor,
                    progress_tx,
                    reply,
                })
                .map(|()| result_local);
            let _ = completion_tx.send(res);
        });

        VfsDownloadOperation::new(completion_rx, progress_rx, pause_flag, cancel_flag)
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
        let home = self.inner.lock().ok()?.home_dir.clone()?;
        Some(
            VfsPath::remote(VfsProtocol::Sftp, &self.host, Path::new(&home))
                .with_port(self.port)
                .with_username(self.effective_username()),
        )
    }

    fn disk_space(&self, _path: &VfsPath) -> Option<DiskSpace> {
        // Could be implemented via SSH exec "df" but not used by termide yet.
        None
    }
}

// Decoding helper kept around for downstream use cases (e.g. inline keys).
#[allow(dead_code)]
fn decode_inline_key(pem: &str, passphrase: Option<&str>) -> VfsResult<PrivateKey> {
    decode_secret_key(pem, passphrase).map_err(|e| {
        VfsError::AuthenticationFailed(format!("Failed to decode inline private key: {e}"))
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sftp_provider_creation() {
        let provider = SftpProvider::new("example.com", 22, Some("alice"));
        assert_eq!(provider.name(), "sftp");
        assert_eq!(provider.connection_state(), ConnectionState::Disconnected);
    }

    #[test]
    fn test_effective_username() {
        let p1 = SftpProvider::new("h", 22, Some("alice"));
        assert_eq!(p1.effective_username(), "alice");

        let p2 = SftpProvider::new("h", 22, None);
        assert!(!p2.effective_username().is_empty());
    }

    #[test]
    fn test_to_remote_path() {
        let sftp_path = VfsPath::remote(VfsProtocol::Sftp, "example.com", Path::new("/var/log"));
        assert!(SftpProvider::to_remote_path(&sftp_path).is_ok());

        let local_path = VfsPath::local("/tmp");
        assert!(SftpProvider::to_remote_path(&local_path).is_err());
    }

    #[test]
    fn test_supported_auth_methods() {
        let provider = SftpProvider::new("h", 22, None);
        let methods = provider.supported_auth_methods();
        assert!(!methods.is_empty());
    }
}
