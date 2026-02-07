//! Journal panel based on Editor with read-only mode.
//!
//! Provides a full-featured journal/log viewer with:
//! - Cursor navigation and text selection
//! - Copy to clipboard
//! - Auto-scroll to new entries
//! - Log level highlighting (DEBUG, INFO, WARN, ERROR)

pub mod highlighting;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect};
use std::any::Any;

use termide_core::{Panel, PanelEvent, RenderContext, WidthPreference};
use termide_highlight::LineHighlighter;
use termide_logger::LogLevel;
use termide_panel_editor::{config::EditorConfig, Editor};
use termide_theme::Theme;

use highlighting::LogHighlightCache;

/// Log viewer panel with Editor-based text display.
pub struct JournalPanel {
    /// Internal editor in read-only mode
    editor: Editor,
    /// Custom highlighter for log levels
    highlight_cache: LogHighlightCache,
    /// Auto-scroll enabled (scroll to new entries)
    auto_scroll: bool,
    /// Number of log entries already synced to buffer
    last_synced_count: usize,
    /// Cached theme for rendering
    cached_theme: Theme,
    /// Cached config for rendering
    cached_config: termide_config::Config,
}

impl JournalPanel {
    /// Create a new log viewer panel.
    pub fn new(theme: &termide_theme::Theme) -> Self {
        // Create editor with view_only config
        let mut config = EditorConfig::view_only();
        config.syntax_highlighting = true; // Enable to use our custom highlighter

        let editor = Editor::with_config(config);
        let highlight_cache = LogHighlightCache::new(*theme);

        Self {
            editor,
            highlight_cache,
            auto_scroll: true,
            last_synced_count: 0,
            cached_theme: *theme,
            cached_config: termide_config::Config::default(),
        }
    }

    /// Sync log entries from logger to buffer.
    ///
    /// Uses incremental synchronization: only fetches new entries since
    /// the last sync, avoiding O(n) clone of all log entries.
    fn sync_logs(&mut self) {
        // First check if there are new entries (O(1) operation)
        let new_count = termide_logger::entry_count();

        if new_count > self.last_synced_count {
            // Only fetch entries we haven't synced yet
            let new_entries = termide_logger::get_entries_from(self.last_synced_count);

            // Get buffer access through editor
            let buffer = self.editor.buffer_mut();

            // Append new entries
            for entry in &new_entries {
                let level_text = match entry.level {
                    LogLevel::Debug => "DEBUG",
                    LogLevel::Info => "INFO ",
                    LogLevel::Warn => "WARN ",
                    LogLevel::Error => "ERROR",
                };

                let line = format!("[{}] {} {}\n", entry.timestamp, level_text, entry.message);
                buffer.append(&line);
            }

            // Invalidate highlight cache for new lines
            self.highlight_cache.invalidate_from(self.last_synced_count);

            self.last_synced_count = new_count;
        }
    }

    /// Access the inner editor (for search, modal requests, etc.).
    pub fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }

    /// Scroll to the end of the log (word-wrap aware).
    fn scroll_to_end(&mut self) {
        self.editor.scroll_to_document_end();
    }

    /// Check if currently at the end of the log.
    fn is_at_end(&self, content_height: usize) -> bool {
        let line_count = self.editor.buffer().line_count();
        let top_line = self.editor.viewport().top_line;
        // Consider "at end" if we can see the last line
        top_line + content_height >= line_count
    }
}

impl Panel for JournalPanel {
    fn name(&self) -> &'static str {
        "journal"
    }

    fn width_preference(&self) -> WidthPreference {
        WidthPreference::PreferWide
    }

    fn title(&self) -> String {
        termide_i18n::t().panel_journal().to_string()
    }

    fn prepare_render(&mut self, theme: &Theme, config: &termide_config::Config) {
        self.cached_theme = *theme;
        self.cached_config = config.clone();
        self.highlight_cache.set_theme(*theme);
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, _ctx: &RenderContext) {
        // Sync new log entries
        self.sync_logs();

        // Auto-scroll if enabled (word-wrap aware)
        if self.auto_scroll && area.height > 0 {
            self.scroll_to_end();
        }

        // Render using editor's rendering with our custom highlighter
        self.editor.render_with_highlighter(
            area,
            buf,
            &self.cached_theme,
            &self.cached_config,
            &mut self.highlight_cache,
        );
    }

    fn handle_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        // Check for auto-scroll toggle keys
        match key.code {
            // Disable auto-scroll on scroll up
            KeyCode::Up
            | KeyCode::Char('k')
            | KeyCode::PageUp
            | KeyCode::Home
            | KeyCode::Char('g') => {
                self.auto_scroll = false;
            }
            // Enable auto-scroll on scroll to end
            KeyCode::End | KeyCode::Char('G') => {
                self.auto_scroll = true;
            }
            _ => {}
        }

        // Delegate to editor for actual handling
        let _ = self.editor.handle_key(key);
        vec![]
    }

    fn tick(&mut self) -> Vec<PanelEvent> {
        if self.auto_scroll && termide_logger::entry_count() > self.last_synced_count {
            vec![PanelEvent::NeedsRedraw]
        } else {
            vec![]
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) -> Vec<PanelEvent> {
        // Delegate to editor first so viewport updates before we check position
        let _ = self.editor.handle_mouse(mouse, area);

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.auto_scroll = false;
            }
            MouseEventKind::ScrollDown => {
                let content_height = area.height as usize;
                if self.is_at_end(content_height) {
                    self.auto_scroll = true;
                }
            }
            _ => {}
        }

        vec![]
    }

    fn handle_scroll(&mut self, delta: i32, area: Rect) -> Vec<PanelEvent> {
        // Delegate to editor first so viewport updates before we check position
        let events = self.editor.handle_scroll(delta, area);

        if delta < 0 {
            self.auto_scroll = false;
        } else {
            let content_height = area.height as usize;
            if self.is_at_end(content_height) {
                self.auto_scroll = true;
            }
        }

        events
    }

    fn to_session(&self, _session_dir: &std::path::Path) -> Option<termide_core::SessionPanel> {
        Some(termide_core::SessionPanel::Journal)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Default for JournalPanel {
    fn default() -> Self {
        Self::new(&termide_theme::Theme::default())
    }
}
