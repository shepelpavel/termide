//! LSP server process management.
//!
//! Handles spawning the language server, communication via stdin/stdout,
//! and routing requests/responses through channels.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};
use lsp_types::{
    ClientCapabilities, CompletionContext, CompletionResponse, CompletionTriggerKind,
    GotoDefinitionResponse, Hover, InitializeParams, InitializeResult, Position,
    PublishDiagnosticsParams, ServerCapabilities, TextDocumentClientCapabilities,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, Uri, VersionedTextDocumentIdentifier,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Convert a file path to an LSP Uri
fn path_to_uri(path: &std::path::Path) -> Option<Uri> {
    let url = url::Url::from_file_path(path).ok()?;
    url.as_str().parse().ok()
}

use crate::protocol::{
    decode_header, encode_message, JsonRpcMessage, JsonRpcNotification, JsonRpcRequest,
    JsonRpcResponse, RequestId,
};

/// Configuration for a specific LSP server
#[derive(Debug, Clone, Default)]
pub struct LspServerConfig {
    /// Command to start the server
    pub command: String,
    /// Command arguments
    pub args: Vec<String>,
    /// File patterns to identify project root
    pub root_markers: Vec<String>,
}

/// Server status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerStatus {
    Starting,
    Indexing, // Initialized but background indexing in progress
    Running,
    ShuttingDown,
    Stopped,
}

/// Pending request tracking
type PendingRequests = Arc<Mutex<HashMap<RequestId, mpsc::Sender<Value>>>>;

/// LSP server instance
#[allow(dead_code)] // Fields will be used in future phases
pub struct LspServer {
    /// Language ID
    language_id: String,
    /// Workspace root
    workspace_root: PathBuf,
    /// Server process
    process: Child,
    /// Next request ID
    next_id: AtomicU64,
    /// Pending requests waiting for responses
    pending: PendingRequests,
    /// Writer thread handle
    writer_handle: Option<JoinHandle<()>>,
    /// Reader thread handle
    reader_handle: Option<JoinHandle<()>>,
    /// Channel to send messages to the writer thread
    writer_tx: mpsc::Sender<String>,
    /// Server status
    status: Arc<Mutex<ServerStatus>>,
    /// Server capabilities (after initialization)
    capabilities: Arc<Mutex<Option<ServerCapabilities>>>,
    /// Active progress tokens (server is indexing while non-empty)
    active_progress: Arc<Mutex<HashSet<String>>>,
}

impl LspServer {
    /// Start a new LSP server
    pub fn start(
        language_id: String,
        config: LspServerConfig,
        workspace_root: PathBuf,
        diagnostics_tx: mpsc::Sender<PublishDiagnosticsParams>,
    ) -> Result<Self> {
        log::info!(
            "Starting LSP server: {} {:?} in {:?}",
            config.command,
            config.args,
            workspace_root
        );
        log::info!("LSP: Starting {} for {:?}", config.command, workspace_root);

        let mut process = Command::new(&config.command)
            .args(&config.args)
            .current_dir(&workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                log::error!("LSP: Failed to start {}: {}", config.command, e);
                e
            })
            .with_context(|| format!("Failed to start LSP server: {}", config.command))?;

        let stdin = process.stdin.take().context("Failed to get stdin")?;
        let stdout = process.stdout.take().context("Failed to get stdout")?;
        let stderr = process.stderr.take().context("Failed to get stderr")?;

        // Stderr reader thread - captures server error output to journal
        let stderr_handle = {
            let lang = language_id.clone();
            thread::spawn(move || {
                Self::stderr_loop(stderr, &lang);
            })
        };
        // Detach stderr thread - we don't need to join it
        drop(stderr_handle);

        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let status = Arc::new(Mutex::new(ServerStatus::Starting));
        let capabilities = Arc::new(Mutex::new(None));
        let active_progress = Arc::new(Mutex::new(HashSet::new()));

        // Writer thread - sends messages to server
        let (writer_tx, writer_rx) = mpsc::channel::<String>();
        let writer_handle = {
            let status = status.clone();
            thread::spawn(move || {
                let mut stdin = stdin;
                while let Ok(msg) = writer_rx.recv() {
                    if *status.lock().unwrap() == ServerStatus::ShuttingDown {
                        break;
                    }
                    if let Err(e) = stdin.write_all(msg.as_bytes()) {
                        log::error!("Failed to write to LSP server: {}", e);
                        break;
                    }
                    if let Err(e) = stdin.flush() {
                        log::error!("Failed to flush LSP server stdin: {}", e);
                        break;
                    }
                }
            })
        };

        // Reader thread - receives messages from server
        let reader_handle = {
            let pending = pending.clone();
            let status = status.clone();
            let capabilities = capabilities.clone();
            let active_progress = active_progress.clone();
            let writer_tx = writer_tx.clone();
            thread::spawn(move || {
                Self::reader_loop(
                    stdout,
                    pending,
                    status,
                    capabilities,
                    active_progress,
                    diagnostics_tx,
                    writer_tx,
                );
            })
        };

        let mut server = Self {
            language_id,
            workspace_root: workspace_root.clone(),
            process,
            next_id: AtomicU64::new(1),
            pending,
            writer_handle: Some(writer_handle),
            reader_handle: Some(reader_handle),
            writer_tx,
            status,
            capabilities,
            active_progress,
        };

        // Send initialize request
        server.initialize(workspace_root)?;

        Ok(server)
    }

    /// Reader thread main loop
    fn reader_loop(
        stdout: std::process::ChildStdout,
        pending: PendingRequests,
        status: Arc<Mutex<ServerStatus>>,
        capabilities: Arc<Mutex<Option<ServerCapabilities>>>,
        active_progress: Arc<Mutex<HashSet<String>>>,
        diagnostics_tx: mpsc::Sender<PublishDiagnosticsParams>,
        writer_tx: mpsc::Sender<String>,
    ) {
        let mut reader = BufReader::new(stdout);
        let mut header = String::new();

        loop {
            if *status.lock().unwrap() == ServerStatus::ShuttingDown {
                break;
            }

            header.clear();

            // Read headers until empty line
            let mut content_length = 0;
            loop {
                header.clear();
                match reader.read_line(&mut header) {
                    Ok(0) => return, // EOF
                    Ok(_) => {
                        let trimmed = header.trim();
                        if trimmed.is_empty() {
                            break; // End of headers
                        }
                        if let Some(len) = decode_header(trimmed) {
                            content_length = len;
                        }
                    }
                    Err(e) => {
                        log::error!("Error reading LSP header: {}", e);
                        return;
                    }
                }
            }

            if content_length == 0 {
                continue;
            }

            // Read content
            let mut content = vec![0u8; content_length];
            if let Err(e) = std::io::Read::read_exact(&mut reader, &mut content) {
                log::error!("Error reading LSP content: {}", e);
                return;
            }

            // Parse message
            let content_str = match String::from_utf8(content) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Invalid UTF-8 in LSP message: {}", e);
                    continue;
                }
            };

            log::trace!("LSP recv: {}", content_str);

            match serde_json::from_str::<JsonRpcMessage>(&content_str) {
                Ok(JsonRpcMessage::Response(response)) => {
                    Self::handle_response(&pending, &capabilities, response);
                }
                Ok(JsonRpcMessage::Notification(notification)) => {
                    Self::handle_notification(
                        &diagnostics_tx,
                        &status,
                        &active_progress,
                        notification,
                    );
                }
                Ok(JsonRpcMessage::Request(request)) => {
                    // Server-initiated requests - respond with success
                    log::debug!("Received server request: {}", request.method);
                    Self::handle_server_request(&writer_tx, request);
                }
                Err(e) => {
                    log::error!("Failed to parse LSP message: {}", e);
                }
            }
        }
    }

    /// Stderr reader loop - captures server error output to journal
    fn stderr_loop(stderr: std::process::ChildStderr, lang: &str) {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            match line {
                Ok(line) if !line.is_empty() => {
                    // Log to journal as warning (not error, since LSP servers often
                    // write informational messages to stderr)
                    log::warn!("LSP [{}]: {}", lang, line);
                }
                Err(_) => break,
                _ => {}
            }
        }
    }

    /// Handle a response from the server
    fn handle_response(
        pending: &PendingRequests,
        capabilities: &Arc<Mutex<Option<ServerCapabilities>>>,
        response: JsonRpcResponse,
    ) {
        // Find and notify the waiting request
        let mut pending = pending.lock().unwrap();
        if let Some(tx) = pending.remove(&response.id) {
            if let Some(result) = response.result {
                // Try to parse as InitializeResult before sending
                // This avoids cloning the entire JSON value
                if let Ok(init_result) = serde_json::from_value::<InitializeResult>(result.clone())
                {
                    *capabilities.lock().unwrap() = Some(init_result.capabilities);
                    log::info!("LSP server initialized, waiting for indexing");
                    log::info!("LSP: Initialized, starting indexing...");
                }
                let _ = tx.send(result);
            } else if let Some(error) = response.error {
                log::warn!("LSP error {}: {}", error.code, error.message);
            }
        } else if let Some(result) = response.result {
            // No pending request - might be initialize response
            if let Ok(init_result) = serde_json::from_value::<InitializeResult>(result) {
                *capabilities.lock().unwrap() = Some(init_result.capabilities);
                log::info!("LSP server initialized, waiting for indexing");
                log::info!("LSP: Initialized, starting indexing...");
            }
        }
    }

    /// Handle a notification from the server
    fn handle_notification(
        diagnostics_tx: &mpsc::Sender<PublishDiagnosticsParams>,
        status: &Arc<Mutex<ServerStatus>>,
        active_progress: &Arc<Mutex<HashSet<String>>>,
        notification: JsonRpcNotification,
    ) {
        match notification.method.as_str() {
            "textDocument/publishDiagnostics" => {
                if let Some(params) = notification.params {
                    if let Ok(diagnostics) = serde_json::from_value(params) {
                        let _ = diagnostics_tx.send(diagnostics);
                    }
                }
            }
            "$/progress" => {
                log::debug!("Received $/progress notification");
                if let Some(params) = notification.params {
                    Self::handle_progress(status, active_progress, params);
                }
            }
            "window/logMessage" | "window/showMessage" => {
                // Log server messages
                if let Some(params) = notification.params {
                    log::debug!("LSP {}: {:?}", notification.method, params);
                }
            }
            _ => {
                log::trace!("Unhandled notification: {}", notification.method);
            }
        }
    }

    /// Handle $/progress notification
    fn handle_progress(
        status: &Arc<Mutex<ServerStatus>>,
        active_progress: &Arc<Mutex<HashSet<String>>>,
        params: Value,
    ) {
        use lsp_types::{NumberOrString, ProgressParams, ProgressParamsValue, WorkDoneProgress};

        if let Ok(progress) = serde_json::from_value::<ProgressParams>(params) {
            let token = match &progress.token {
                NumberOrString::String(s) => s.clone(),
                NumberOrString::Number(n) => n.to_string(),
            };

            match progress.value {
                ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(begin)) => {
                    active_progress.lock().unwrap().insert(token);
                    *status.lock().unwrap() = ServerStatus::Indexing;
                    log::info!("LSP: {}", begin.title);
                }
                ProgressParamsValue::WorkDone(WorkDoneProgress::Report(report)) => {
                    // Optional: could show percentage if available
                    if let Some(msg) = report.message {
                        log::debug!("LSP progress: {}", msg);
                    }
                }
                ProgressParamsValue::WorkDone(WorkDoneProgress::End(_)) => {
                    let mut progress = active_progress.lock().unwrap();
                    progress.remove(&token);
                    if progress.is_empty() {
                        drop(progress); // Release before acquiring status lock
                        *status.lock().unwrap() = ServerStatus::Running;
                        log::info!("LSP: Ready");
                    }
                }
            }
        }
    }

    /// Handle server-initiated request (respond with success)
    fn handle_server_request(writer_tx: &mpsc::Sender<String>, request: JsonRpcRequest) {
        // Respond with null/empty result for most server requests
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: Some(serde_json::Value::Null),
            error: None,
        };

        if let Ok(msg) = encode_message(&response) {
            let _ = writer_tx.send(msg);
            log::debug!("Responded to server request: {}", request.method);
        }
    }

    /// Send initialize request
    #[allow(deprecated)] // root_uri is deprecated but still widely used
    fn initialize(&mut self, workspace_root: PathBuf) -> Result<()> {
        let root_uri = path_to_uri(&workspace_root)
            .ok_or_else(|| anyhow::anyhow!("Invalid workspace path"))?;

        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(root_uri.clone()),
            capabilities: ClientCapabilities {
                text_document: Some(TextDocumentClientCapabilities {
                    completion: Some(lsp_types::CompletionClientCapabilities {
                        completion_item: Some(lsp_types::CompletionItemCapability {
                            snippet_support: Some(false),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    hover: Some(lsp_types::HoverClientCapabilities {
                        content_format: Some(vec![lsp_types::MarkupKind::Markdown]),
                        ..Default::default()
                    }),
                    definition: Some(lsp_types::GotoCapability::default()),
                    synchronization: Some(lsp_types::TextDocumentSyncClientCapabilities {
                        dynamic_registration: Some(false),
                        will_save: Some(false),
                        will_save_wait_until: Some(false),
                        did_save: Some(true),
                    }),
                    publish_diagnostics: Some(lsp_types::PublishDiagnosticsClientCapabilities {
                        related_information: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                window: Some(lsp_types::WindowClientCapabilities {
                    work_done_progress: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let _rx = self.send_request::<InitializeResult>("initialize", params)?;

        // Send initialized notification
        self.send_notification("initialized", serde_json::json!({}));

        // Set to Indexing - will become Running when all progress tokens complete
        *self.status.lock().unwrap() = ServerStatus::Indexing;
        Ok(())
    }

    /// Generate next request ID
    fn next_request_id(&self) -> RequestId {
        RequestId::Number(self.next_id.fetch_add(1, Ordering::SeqCst))
    }

    /// Send a request and return a receiver for the response
    fn send_request<T: DeserializeOwned + Send + 'static>(
        &self,
        method: &str,
        params: impl serde::Serialize,
    ) -> Result<mpsc::Receiver<Option<T>>> {
        let id = self.next_request_id();
        let request = JsonRpcRequest::new(id.clone(), method, Some(serde_json::to_value(params)?));

        let (tx, rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();

        // Store the sender for when response arrives
        self.pending.lock().unwrap().insert(id, tx);

        // Spawn thread to convert Value to T
        thread::spawn(move || {
            if let Ok(value) = rx.recv() {
                let result = serde_json::from_value::<T>(value).ok();
                let _ = result_tx.send(result);
            }
        });

        // Send the request
        let msg = encode_message(&request)?;
        log::trace!("LSP send: {}", msg);
        self.writer_tx.send(msg)?;

        Ok(result_rx)
    }

    /// Send a notification (no response expected)
    fn send_notification(&self, method: &str, params: impl serde::Serialize) {
        let notification = JsonRpcNotification::new(method, serde_json::to_value(params).ok());
        if let Ok(msg) = encode_message(&notification) {
            log::trace!("LSP send notification: {}", method);
            let _ = self.writer_tx.send(msg);
        }
    }

    /// Request completion at position
    pub fn completion(
        &self,
        uri: Uri,
        position: Position,
        trigger_kind: CompletionTriggerKind,
        trigger_character: Option<String>,
    ) -> mpsc::Receiver<Option<CompletionResponse>> {
        let params = lsp_types::CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position,
            },
            context: Some(CompletionContext {
                trigger_kind,
                trigger_character,
            }),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        self.send_request("textDocument/completion", params)
            .unwrap_or_else(|_| {
                let (_, rx) = mpsc::channel();
                rx
            })
    }

    /// Request hover at position
    pub fn hover(&self, uri: Uri, position: Position) -> mpsc::Receiver<Option<Hover>> {
        let params = lsp_types::HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position,
            },
            work_done_progress_params: Default::default(),
        };

        self.send_request("textDocument/hover", params)
            .unwrap_or_else(|_| {
                let (_, rx) = mpsc::channel();
                rx
            })
    }

    /// Request go-to-definition at position
    pub fn goto_definition(
        &self,
        uri: Uri,
        position: Position,
    ) -> mpsc::Receiver<Option<GotoDefinitionResponse>> {
        let params = lsp_types::GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        self.send_request("textDocument/definition", params)
            .unwrap_or_else(|_| {
                let (_, rx) = mpsc::channel();
                rx
            })
    }

    /// Send textDocument/didOpen notification
    pub fn did_open(&self, uri: Uri, language_id: String, text: String) {
        let params = lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri,
                language_id,
                version: 1,
                text,
            },
        };
        self.send_notification("textDocument/didOpen", params);
    }

    /// Send textDocument/didChange notification (full sync)
    pub fn did_change(&self, uri: Uri, version: i32, text: String) {
        let params = lsp_types::DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier { uri, version },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text,
            }],
        };
        self.send_notification("textDocument/didChange", params);
    }

    /// Send textDocument/didClose notification
    pub fn did_close(&self, uri: Uri) {
        let params = lsp_types::DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri },
        };
        self.send_notification("textDocument/didClose", params);
    }

    /// Send textDocument/didSave notification
    ///
    /// This triggers full project analysis in rust-analyzer and other LSP servers,
    /// which is necessary for detecting logical errors like unresolved modules.
    pub fn did_save(&self, uri: Uri, text: Option<String>) {
        let params = lsp_types::DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier { uri },
            text,
        };
        self.send_notification("textDocument/didSave", params);
    }

    /// Get current server status
    pub fn status(&self) -> ServerStatus {
        *self.status.lock().unwrap()
    }

    /// Check if server is effectively ready (Running, or Indexing with no active progress)
    pub fn is_ready(&self) -> bool {
        let status = *self.status.lock().unwrap();
        match status {
            ServerStatus::Running => true,
            ServerStatus::Indexing => {
                // If no active progress, consider it ready
                // (server might not support/send progress notifications)
                self.active_progress.lock().unwrap().is_empty()
            }
            _ => false,
        }
    }

    /// Check if server is actively indexing (has active progress tokens)
    pub fn is_indexing(&self) -> bool {
        let status = *self.status.lock().unwrap();
        status == ServerStatus::Indexing && !self.active_progress.lock().unwrap().is_empty()
    }

    /// Shutdown the server
    pub fn shutdown(mut self) {
        *self.status.lock().unwrap() = ServerStatus::ShuttingDown;

        // Send shutdown request
        let _ = self.send_request::<()>("shutdown", serde_json::json!(null));

        // Send exit notification
        self.send_notification("exit", serde_json::json!(null));

        // Wait for threads to finish
        if let Some(handle) = self.writer_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.reader_handle.take() {
            let _ = handle.join();
        }

        // Kill process if still running
        let _ = self.process.kill();
        let _ = self.process.wait();

        *self.status.lock().unwrap() = ServerStatus::Stopped;
    }
}
