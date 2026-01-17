//! LSP integration state for the editor.
//!
//! Tracks LSP-related state for a single editor instance:
//! - Language ID for current file
//! - Document version for sync
//! - Pending LSP requests (completion, hover, definition)
//! - Current diagnostics
//! - Active completion popup

use std::path::Path;
use std::sync::mpsc;

use lsp_types::{CompletionResponse, Diagnostic, GotoDefinitionResponse, Hover, Position};
use ratatui::layout::Rect;
use termide_lsp::{CompletionTriggerKind, LspManager};

use crate::completion_popup::CompletionPopup;

/// LSP state for a single editor instance.
pub struct LspState {
    /// Language ID for current file (e.g., "rust", "python")
    pub language_id: Option<String>,

    /// Document version for sync (incremented on each change)
    pub document_version: i32,

    /// Pending completion request receiver
    pub completion_rx: Option<mpsc::Receiver<Option<CompletionResponse>>>,

    /// Pending hover request receiver
    pub hover_rx: Option<mpsc::Receiver<Option<Hover>>>,

    /// Pending goto-definition request receiver
    pub definition_rx: Option<mpsc::Receiver<Option<GotoDefinitionResponse>>>,

    /// Current diagnostics for this file
    pub diagnostics: Vec<Diagnostic>,

    /// Whether LSP is enabled for this file
    pub enabled: bool,

    /// Whether LSP server is still starting (for spinner display)
    pub server_loading: bool,

    /// Flag indicating buffer has changed and LSP needs notification
    pub pending_change: bool,

    /// Active completion popup (if any)
    pub completion_popup: Option<CompletionPopup>,

    /// Flag indicating completion was requested (Ctrl+Space)
    pub completion_requested: bool,

    /// Column position where completion was triggered (start of word)
    /// Used to calculate prefix length when accepting completion
    pub completion_trigger_column: usize,

    /// Last rendered popup rect (for mouse hit testing)
    pub popup_rect: Option<Rect>,

    /// Timer for auto-completion delay (None if not scheduled)
    pub completion_timer: Option<std::time::Instant>,

    /// Last inserted character (for auto-completion scheduling)
    pub last_inserted_char: Option<char>,

    /// Status text to display next to spinner (e.g., "starting", "indexing")
    pub server_status_text: Option<String>,
}

impl LspState {
    /// Create new LSP state.
    pub fn new() -> Self {
        Self {
            language_id: None,
            document_version: 0,
            completion_rx: None,
            hover_rx: None,
            definition_rx: None,
            diagnostics: Vec::new(),
            enabled: false,
            server_loading: false,
            pending_change: false,
            completion_popup: None,
            completion_requested: false,
            completion_trigger_column: 0,
            popup_rect: None,
            completion_timer: None,
            last_inserted_char: None,
            server_status_text: None,
        }
    }

    /// Mark that the buffer has changed and needs LSP notification.
    pub fn mark_changed(&mut self) {
        if self.enabled {
            self.pending_change = true;
        }
    }

    /// Check if there's a pending change that needs LSP notification.
    pub fn has_pending_change(&self) -> bool {
        self.pending_change
    }

    /// Clear the pending change flag (after sending notification).
    pub fn clear_pending_change(&mut self) {
        self.pending_change = false;
    }

    /// Schedule auto-completion to trigger after delay.
    pub fn schedule_completion(&mut self) {
        if self.enabled && !self.server_loading {
            self.completion_timer = Some(std::time::Instant::now());
        }
    }

    /// Cancel scheduled auto-completion.
    pub fn cancel_completion_timer(&mut self) {
        self.completion_timer = None;
    }

    /// Check if auto-completion timer has expired and should trigger.
    /// Returns true if completion should be triggered now.
    pub fn check_completion_timer(&mut self, delay_ms: u64) -> bool {
        if let Some(start) = self.completion_timer {
            if start.elapsed().as_millis() >= delay_ms as u128 {
                self.completion_timer = None;
                return true;
            }
        }
        false
    }

    /// Initialize LSP for a file path.
    ///
    /// Detects language and enables LSP if a server is configured.
    pub fn init_for_file(&mut self, file_path: &Path, lsp_manager: &mut LspManager) {
        // Detect language from file extension
        self.language_id = LspManager::detect_language(file_path);

        if let Some(ref lang) = self.language_id {
            // Try to ensure server is running
            match lsp_manager.ensure_server(lang, file_path) {
                Ok(()) => {
                    self.enabled = true;
                    self.server_loading = true; // Server is starting, will be set to false when ready
                    log::info!("LSP enabled for {} ({:?})", lang, file_path);
                }
                Err(e) => {
                    log::debug!("LSP not available for {}: {}", lang, e);
                    self.enabled = false;
                    self.server_loading = false;
                }
            }
        }
    }

    /// Send didOpen notification when file is opened.
    pub fn did_open(&mut self, file_path: &Path, content: &str, lsp_manager: &LspManager) {
        if !self.enabled {
            return;
        }

        if let Some(ref lang) = self.language_id {
            self.document_version = 1;
            lsp_manager.did_open(lang, file_path, content);
        }
    }

    /// Send didChange notification when content changes.
    pub fn did_change(&mut self, file_path: &Path, content: &str, lsp_manager: &LspManager) {
        if !self.enabled {
            return;
        }

        if let Some(ref lang) = self.language_id {
            self.document_version += 1;
            lsp_manager.did_change(lang, file_path, self.document_version, content);
        }
    }

    /// Send didClose notification when file is closed.
    pub fn did_close(&self, file_path: &Path, lsp_manager: &LspManager) {
        if !self.enabled {
            return;
        }

        if let Some(ref lang) = self.language_id {
            lsp_manager.did_close(lang, file_path);
        }
    }

    /// Request completion at position.
    pub fn request_completion(
        &mut self,
        file_path: &Path,
        line: usize,
        column: usize,
        trigger_kind: CompletionTriggerKind,
        trigger_character: Option<String>,
        lsp_manager: &LspManager,
    ) {
        if !self.enabled {
            return;
        }

        if let Some(ref lang) = self.language_id {
            let position = Position::new(line as u32, column as u32);
            self.completion_rx =
                lsp_manager.completion(lang, file_path, position, trigger_kind, trigger_character);
        }
    }

    /// Request hover info at position.
    pub fn request_hover(
        &mut self,
        file_path: &Path,
        line: usize,
        column: usize,
        lsp_manager: &LspManager,
    ) {
        if !self.enabled {
            return;
        }

        if let Some(ref lang) = self.language_id {
            let position = Position::new(line as u32, column as u32);
            self.hover_rx = lsp_manager.hover(lang, file_path, position);
        }
    }

    /// Request goto-definition at position.
    pub fn request_definition(
        &mut self,
        file_path: &Path,
        line: usize,
        column: usize,
        lsp_manager: &LspManager,
    ) {
        if !self.enabled {
            return;
        }

        if let Some(ref lang) = self.language_id {
            let position = Position::new(line as u32, column as u32);
            self.definition_rx = lsp_manager.goto_definition(lang, file_path, position);
        }
    }

    /// Poll for completion response (non-blocking).
    pub fn poll_completion(&mut self) -> Option<CompletionResponse> {
        if let Some(ref rx) = self.completion_rx {
            match rx.try_recv() {
                Ok(Some(response)) => {
                    self.completion_rx = None;
                    self.server_loading = false; // Server is ready since it responded
                    return Some(response);
                }
                Ok(None) => {
                    self.completion_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.completion_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // Still waiting
                }
            }
        }
        None
    }

    /// Poll for hover response (non-blocking).
    pub fn poll_hover(&mut self) -> Option<Hover> {
        if let Some(ref rx) = self.hover_rx {
            match rx.try_recv() {
                Ok(Some(response)) => {
                    self.hover_rx = None;
                    self.server_loading = false; // Server is ready since it responded
                    return Some(response);
                }
                Ok(None) => {
                    self.hover_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.hover_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // Still waiting
                }
            }
        }
        None
    }

    /// Poll for definition response (non-blocking).
    pub fn poll_definition(&mut self) -> Option<GotoDefinitionResponse> {
        if let Some(ref rx) = self.definition_rx {
            match rx.try_recv() {
                Ok(Some(response)) => {
                    self.definition_rx = None;
                    self.server_loading = false; // Server is ready since it responded
                    return Some(response);
                }
                Ok(None) => {
                    self.definition_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.definition_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // Still waiting
                }
            }
        }
        None
    }

    /// Update diagnostics for this file.
    pub fn update_diagnostics(&mut self, diagnostics: Vec<Diagnostic>) {
        self.diagnostics = diagnostics;
    }

    /// Get diagnostic at a specific line (if any).
    pub fn diagnostic_at_line(&self, line: usize) -> Option<&Diagnostic> {
        self.diagnostics
            .iter()
            .find(|d| d.range.start.line as usize == line)
    }

    /// Check if there are any diagnostics.
    pub fn has_diagnostics(&self) -> bool {
        !self.diagnostics.is_empty()
    }

    /// Get error count.
    pub fn error_count(&self) -> usize {
        use lsp_types::DiagnosticSeverity;
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
            .count()
    }

    /// Get warning count.
    pub fn warning_count(&self) -> usize {
        use lsp_types::DiagnosticSeverity;
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Some(DiagnosticSeverity::WARNING))
            .count()
    }
}

impl Default for LspState {
    fn default() -> Self {
        Self::new()
    }
}
