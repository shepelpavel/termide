//! Diagnostics panel for termide.
//!
//! Displays a list of all LSP diagnostics (errors, warnings, hints, info)
//! with navigation and filtering capabilities.

use std::any::Any;
use std::collections::HashMap;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lsp_types::{Diagnostic, DiagnosticSeverity};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use termide_config::{is_go_end, is_go_home, is_move_down, is_move_up};
use unicode_width::UnicodeWidthStr;

use termide_core::{Panel, PanelEvent, RenderContext, ThemeColors};
use termide_theme::Theme;
use termide_ui::ScrollBar;

/// Entry representing a single diagnostic item.
#[derive(Clone)]
pub struct DiagnosticEntry {
    /// File path
    pub file_path: PathBuf,
    /// Line number (0-indexed)
    pub line: u32,
    /// Column number (0-indexed)
    pub column: u32,
    /// Diagnostic severity
    pub severity: DiagnosticSeverity,
    /// Diagnostic message
    pub message: String,
    /// Source (e.g., "rustc", "clippy")
    pub source: Option<String>,
    /// Error code (if available)
    pub code: Option<String>,
}

impl DiagnosticEntry {
    /// Create from LSP Diagnostic.
    pub fn from_diagnostic(file_path: PathBuf, diag: &Diagnostic) -> Self {
        let code = diag.code.as_ref().map(|c| match c {
            lsp_types::NumberOrString::Number(n) => n.to_string(),
            lsp_types::NumberOrString::String(s) => s.clone(),
        });

        Self {
            file_path,
            line: diag.range.start.line,
            column: diag.range.start.character,
            severity: diag.severity.unwrap_or(DiagnosticSeverity::ERROR),
            message: diag.message.lines().next().unwrap_or("").to_string(),
            source: diag.source.clone(),
            code,
        }
    }

    /// Get severity icon.
    pub fn severity_icon(&self) -> char {
        match self.severity {
            DiagnosticSeverity::ERROR => '●',
            DiagnosticSeverity::WARNING => '▲',
            DiagnosticSeverity::INFORMATION => 'ℹ',
            DiagnosticSeverity::HINT => '○',
            _ => '○',
        }
    }
}

/// Severity filter options.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SeverityFilter {
    /// Show all diagnostics
    All,
    /// Show only errors
    Errors,
    /// Show errors and warnings
    ErrorsAndWarnings,
}

impl SeverityFilter {
    /// Check if severity matches filter.
    pub fn matches(&self, severity: DiagnosticSeverity) -> bool {
        match self {
            SeverityFilter::All => true,
            SeverityFilter::Errors => severity == DiagnosticSeverity::ERROR,
            SeverityFilter::ErrorsAndWarnings => {
                severity == DiagnosticSeverity::ERROR || severity == DiagnosticSeverity::WARNING
            }
        }
    }

    /// Get display text.
    pub fn display(&self) -> &'static str {
        match self {
            SeverityFilter::All => "All",
            SeverityFilter::Errors => "Errors",
            SeverityFilter::ErrorsAndWarnings => "E+W",
        }
    }

    /// Cycle to next filter.
    pub fn next(&self) -> Self {
        match self {
            SeverityFilter::All => SeverityFilter::Errors,
            SeverityFilter::Errors => SeverityFilter::ErrorsAndWarnings,
            SeverityFilter::ErrorsAndWarnings => SeverityFilter::All,
        }
    }
}

/// Diagnostics panel showing all LSP diagnostics.
pub struct DiagnosticsPanel {
    /// All diagnostics organized by file
    diagnostics_by_file: HashMap<PathBuf, Vec<DiagnosticEntry>>,
    /// Flattened list of all diagnostics (for display)
    all_diagnostics: Vec<DiagnosticEntry>,
    /// Currently selected index
    selected_index: usize,
    /// Scroll offset (top visible item)
    scroll_offset: usize,
    /// Current severity filter
    filter: SeverityFilter,
    /// Cached theme
    cached_theme: Theme,
    /// Last area height (for scroll calculations)
    last_height: usize,
    /// Cached vim_mode setting for keyboard handling
    vim_mode: bool,
}

impl DiagnosticsPanel {
    /// Create a new diagnostics panel.
    pub fn new(theme: &Theme) -> Self {
        Self {
            diagnostics_by_file: HashMap::new(),
            all_diagnostics: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            filter: SeverityFilter::All,
            cached_theme: *theme,
            last_height: 10,
            vim_mode: false,
        }
    }

    /// Update diagnostics for a specific file.
    pub fn update_diagnostics(&mut self, file_path: PathBuf, diagnostics: &[Diagnostic]) {
        let entries: Vec<DiagnosticEntry> = diagnostics
            .iter()
            .map(|d| DiagnosticEntry::from_diagnostic(file_path.clone(), d))
            .collect();

        if entries.is_empty() {
            self.diagnostics_by_file.remove(&file_path);
        } else {
            self.diagnostics_by_file.insert(file_path, entries);
        }

        self.rebuild_list();
    }

    /// Clear diagnostics for a specific file.
    pub fn clear_file(&mut self, file_path: &PathBuf) {
        self.diagnostics_by_file.remove(file_path);
        self.rebuild_list();
    }

    /// Clear all diagnostics.
    pub fn clear_all(&mut self) {
        self.diagnostics_by_file.clear();
        self.all_diagnostics.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Rebuild flattened list from diagnostics by file.
    fn rebuild_list(&mut self) {
        self.all_diagnostics.clear();

        // Sort files by path for consistent ordering
        let mut files: Vec<_> = self.diagnostics_by_file.keys().collect();
        files.sort();

        for file in files {
            if let Some(entries) = self.diagnostics_by_file.get(file) {
                // Sort by line number within file
                let mut sorted: Vec<_> = entries.clone();
                sorted.sort_by_key(|e| (e.line, e.column));
                self.all_diagnostics.extend(sorted);
            }
        }

        // Clamp selection
        if self.selected_index >= self.all_diagnostics.len() {
            self.selected_index = self.all_diagnostics.len().saturating_sub(1);
        }
    }

    /// Get filtered diagnostics.
    fn filtered_diagnostics(&self) -> Vec<(usize, &DiagnosticEntry)> {
        self.all_diagnostics
            .iter()
            .enumerate()
            .filter(|(_, e)| self.filter.matches(e.severity))
            .collect()
    }

    /// Get currently selected diagnostic entry.
    pub fn selected_entry(&self) -> Option<&DiagnosticEntry> {
        let filtered = self.filtered_diagnostics();
        filtered.get(self.selected_index).map(|(_, e)| *e)
    }

    /// Move selection up.
    fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
        self.ensure_visible();
    }

    /// Move selection down.
    fn select_next(&mut self) {
        let filtered = self.filtered_diagnostics();
        if self.selected_index + 1 < filtered.len() {
            self.selected_index += 1;
        }
        self.ensure_visible();
    }

    /// Ensure selected item is visible.
    fn ensure_visible(&mut self) {
        let content_height = self.last_height.saturating_sub(3); // Header + border

        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + content_height {
            self.scroll_offset = self.selected_index.saturating_sub(content_height - 1);
        }
    }

    /// Get error count.
    pub fn error_count(&self) -> usize {
        self.all_diagnostics
            .iter()
            .filter(|e| e.severity == DiagnosticSeverity::ERROR)
            .count()
    }

    /// Get warning count.
    pub fn warning_count(&self) -> usize {
        self.all_diagnostics
            .iter()
            .filter(|e| e.severity == DiagnosticSeverity::WARNING)
            .count()
    }

    /// Get total count.
    pub fn total_count(&self) -> usize {
        self.all_diagnostics.len()
    }
}

impl Panel for DiagnosticsPanel {
    fn name(&self) -> &'static str {
        "diagnostics"
    }

    fn title(&self) -> String {
        let errors = self.error_count();
        let warnings = self.warning_count();
        if errors > 0 || warnings > 0 {
            format!("Diagnostics ({} errors, {} warnings)", errors, warnings)
        } else {
            "Diagnostics".to_string()
        }
    }

    fn prepare_render(&mut self, theme: &Theme, config: &termide_config::Config) {
        self.cached_theme = *theme;
        self.vim_mode = config.general.vim_mode;
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, _ctx: &RenderContext) {
        self.last_height = area.height as usize;
        let theme = &self.cached_theme;

        // Clear area
        let bg_style = Style::default().bg(theme.bg).fg(theme.fg);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].set_style(bg_style);
                buf[(x, y)].set_char(' ');
            }
        }

        // Render header with filter info
        if area.height > 1 {
            let header_y = area.top();
            let header_text = format!(
                " Filter: {} | {} items",
                self.filter.display(),
                self.filtered_diagnostics().len()
            );
            let header_style = Style::default().bg(theme.accented_bg).fg(theme.fg);

            for x in area.left()..area.right() {
                buf[(x, header_y)].set_style(header_style);
                buf[(x, header_y)].set_char(' ');
            }

            for (i, ch) in header_text.chars().enumerate() {
                let x = area.left() + i as u16;
                if x < area.right() {
                    buf[(x, header_y)].set_char(ch);
                }
            }
        }

        // Content area
        let content_top = area.top() + 1;
        let content_height = (area.height.saturating_sub(2)) as usize;

        let filtered = self.filtered_diagnostics();

        if filtered.is_empty() {
            // Show "No diagnostics" message
            let msg = "No diagnostics";
            let msg_y = content_top + content_height as u16 / 2;
            let msg_x = area.left() + (area.width.saturating_sub(msg.width() as u16)) / 2;

            let dim_style = Style::default().fg(theme.accented_fg);
            for (i, ch) in msg.chars().enumerate() {
                let x = msg_x + i as u16;
                if x < area.right() {
                    buf[(x, msg_y)].set_char(ch);
                    buf[(x, msg_y)].set_style(dim_style);
                }
            }
        } else {
            // Render diagnostic entries
            for (display_idx, (orig_idx, entry)) in filtered
                .iter()
                .skip(self.scroll_offset)
                .take(content_height)
                .enumerate()
            {
                let y = content_top + display_idx as u16;
                let is_selected = *orig_idx == self.selected_index;

                // Determine style
                let (line_style, icon_style) = if is_selected {
                    (
                        Style::default().bg(theme.selected_bg).fg(theme.selected_fg),
                        Style::default()
                            .bg(theme.selected_bg)
                            .fg(match entry.severity {
                                DiagnosticSeverity::ERROR => theme.error,
                                DiagnosticSeverity::WARNING => theme.warning,
                                _ => theme.accented_fg,
                            }),
                    )
                } else {
                    (
                        bg_style,
                        Style::default().bg(theme.bg).fg(match entry.severity {
                            DiagnosticSeverity::ERROR => theme.error,
                            DiagnosticSeverity::WARNING => theme.warning,
                            _ => theme.accented_fg,
                        }),
                    )
                };

                // Clear line
                for x in area.left()..area.right() {
                    buf[(x, y)].set_style(line_style);
                    buf[(x, y)].set_char(' ');
                }

                // Render icon
                let x = area.left() + 1;
                if x < area.right() {
                    buf[(x, y)].set_char(entry.severity_icon());
                    buf[(x, y)].set_style(icon_style);
                }

                // Render file:line:col
                let file_name = entry
                    .file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?");
                let location = format!(" {}:{}:{}", file_name, entry.line + 1, entry.column + 1);

                let loc_style = line_style.add_modifier(Modifier::BOLD);
                for (i, ch) in location.chars().enumerate() {
                    let x = area.left() + 3 + i as u16;
                    if x < area.right() {
                        buf[(x, y)].set_char(ch);
                        buf[(x, y)].set_style(loc_style);
                    }
                }

                // Render message (truncated)
                let msg_start = area.left() + 3 + location.width() as u16 + 1;
                let msg_max_width = area.right().saturating_sub(msg_start + 1) as usize;

                if msg_max_width > 3 {
                    let msg = if entry.message.width() > msg_max_width {
                        let mut truncated = String::new();
                        let mut width = 0;
                        for ch in entry.message.chars() {
                            let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
                            if width + ch_width + 1 > msg_max_width {
                                truncated.push('…');
                                break;
                            }
                            truncated.push(ch);
                            width += ch_width;
                        }
                        truncated
                    } else {
                        entry.message.clone()
                    };

                    for (i, ch) in msg.chars().enumerate() {
                        let x = msg_start + i as u16;
                        if x < area.right() {
                            buf[(x, y)].set_char(ch);
                            buf[(x, y)].set_style(line_style);
                        }
                    }
                }
            }
        }

        // Render scrollbar
        if filtered.len() > content_height && area.width > 2 {
            let scrollbar_x = area.right() - 1;
            let theme_colors = ThemeColors::from(theme);
            ScrollBar::render(
                buf,
                scrollbar_x,
                content_top,
                content_height as u16,
                self.scroll_offset,
                content_height,
                filtered.len(),
                &theme_colors,
                true, // is_focused
            );
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Vec<PanelEvent> {
        // Vim-aware navigation (j/k/g/G when vim_mode is enabled)
        if is_move_up(&key, self.vim_mode) {
            self.select_prev();
            return vec![];
        }
        if is_move_down(&key, self.vim_mode) {
            self.select_next();
            return vec![];
        }
        if is_go_home(&key, self.vim_mode) {
            self.selected_index = 0;
            self.scroll_offset = 0;
            return vec![];
        }
        if is_go_end(&key, self.vim_mode) {
            let filtered = self.filtered_diagnostics();
            self.selected_index = filtered.len().saturating_sub(1);
            self.ensure_visible();
            return vec![];
        }

        match key.code {
            KeyCode::PageUp => {
                let page_size = self.last_height.saturating_sub(3);
                for _ in 0..page_size {
                    self.select_prev();
                }
            }
            KeyCode::PageDown => {
                let page_size = self.last_height.saturating_sub(3);
                for _ in 0..page_size {
                    self.select_next();
                }
            }
            KeyCode::Enter => {
                // Navigate to selected diagnostic - open file (line navigation TODO)
                if let Some(entry) = self.selected_entry() {
                    return vec![PanelEvent::OpenFile(entry.file_path.clone())];
                }
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Toggle filter
                self.filter = self.filter.next();
                self.selected_index = 0;
                self.scroll_offset = 0;
            }
            _ => {}
        }
        vec![]
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) -> Vec<PanelEvent> {
        let content_top = area.top() + 1;

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.select_prev();
            }
            MouseEventKind::ScrollDown => {
                self.select_next();
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if mouse.row >= content_top && mouse.row < area.bottom() {
                    let click_offset = (mouse.row - content_top) as usize;
                    let filtered = self.filtered_diagnostics();
                    let new_idx = self.scroll_offset + click_offset;
                    if new_idx < filtered.len() {
                        self.selected_index = new_idx;
                    }
                }
            }
            _ => {}
        }
        vec![]
    }

    fn to_session(&self, _session_dir: &std::path::Path) -> Option<termide_core::SessionPanel> {
        // Diagnostics panel doesn't persist to session (it's dynamic)
        None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
