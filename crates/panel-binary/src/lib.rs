//! Binary file viewer panel (read-only).
//!
//! Renders a binary file as a classic hex dump — `offset │ 16 hex bytes │
//! ASCII gutter` — in pure text pseudographics, or as a plain-text view of the
//! same bytes. The file is read in windows on demand so large files are not
//! loaded fully into memory. The offset and a clickable `Hex│Text` toggle are
//! contributed to the global status bar via [`Panel::status_segments`].

use std::any::Any;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
};

use termide_core::{
    Config, HotkeyTable, KeyChord, Panel, PanelEvent, RenderContext, SegmentKind, SessionPanel,
    StatusSegment, Theme, ThemeColors, WidthPreference,
};

/// Number of bytes shown per row in hex mode.
const HEX_COLS: u64 = 16;

/// How the bytes are presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    /// `offset │ hex │ ASCII` dump.
    Hex,
    /// Plain-text rendering (non-printable bytes shown as `·`).
    Text,
}

/// Read-only binary (hex/ASCII) viewer.
pub struct BinaryPanel {
    /// Path to the file.
    file_path: PathBuf,
    /// Display title (filename).
    title: String,
    /// Open handle for windowed reads; `None` if the file could not be opened.
    file: Option<File>,
    /// File length in bytes.
    len: u64,
    /// Error message if the file could not be opened.
    error: Option<String>,
    /// Current presentation mode.
    mode: ViewMode,
    /// Byte offset of the first visible row (kept aligned to the row size).
    top_byte: u64,
    /// Last inner width/height seen in `render` (for text wrapping + paging).
    last_inner: (u16, u16),
    /// Cached theme colors.
    theme: ThemeColors,
    /// Configurable hotkeys (toggle hex/text).
    hotkeys: HotkeyTable,
    /// Pointer of the last `Arc<Config>` used to build hotkeys.
    last_config_ptr: usize,
}

impl BinaryPanel {
    /// Open a binary file in the hex viewer.
    pub fn new(path: PathBuf) -> Result<Self> {
        let title = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("binary")
            .to_string();

        let (file, len, error) = match File::open(&path) {
            Ok(f) => {
                let len = f.metadata().map(|m| m.len()).unwrap_or(0);
                (Some(f), len, None)
            }
            Err(e) => (None, 0, Some(format!("Cannot open file: {e}"))),
        };

        Ok(Self {
            file_path: path,
            title,
            file,
            len,
            error,
            mode: ViewMode::Hex,
            top_byte: 0,
            last_inner: (0, 0),
            theme: ThemeColors::default(),
            hotkeys: HotkeyTable::default(),
            last_config_ptr: 0,
        })
    }

    /// Point the panel at a different file (reuse an existing viewer).
    pub fn set_file(&mut self, path: PathBuf) {
        self.title = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("binary")
            .to_string();
        match File::open(&path) {
            Ok(f) => {
                self.len = f.metadata().map(|m| m.len()).unwrap_or(0);
                self.file = Some(f);
                self.error = None;
            }
            Err(e) => {
                self.file = None;
                self.len = 0;
                self.error = Some(format!("Cannot open file: {e}"));
            }
        }
        self.file_path = path;
        self.top_byte = 0;
    }

    /// Bytes shown per row for the current mode.
    fn bytes_per_row(&self) -> u64 {
        match self.mode {
            ViewMode::Hex => HEX_COLS,
            // Text wraps at the panel width; fall back to a sane default before
            // the first render has recorded an area.
            ViewMode::Text => (self.last_inner.0 as u64).max(1),
        }
    }

    /// Largest valid `top_byte`, aligned to the row size.
    fn max_top(&self, bpr: u64) -> u64 {
        if self.len == 0 {
            return 0;
        }
        let last_row = (self.len.saturating_sub(1)) / bpr;
        last_row * bpr
    }

    /// Re-align and clamp `top_byte` after a mode/area/scroll change.
    fn clamp_scroll(&mut self) {
        let bpr = self.bytes_per_row();
        self.top_byte -= self.top_byte % bpr;
        let max_top = self.max_top(bpr);
        if self.top_byte > max_top {
            self.top_byte = max_top;
        }
    }

    /// Read up to `count` bytes starting at `start` from the file.
    fn read_window(&mut self, start: u64, count: usize) -> Vec<u8> {
        let Some(file) = self.file.as_mut() else {
            return Vec::new();
        };
        if start >= self.len {
            return Vec::new();
        }
        if file.seek(SeekFrom::Start(start)).is_err() {
            return Vec::new();
        }
        let want = count.min((self.len - start) as usize);
        let mut buf = vec![0u8; want];
        match file.read(&mut buf) {
            Ok(n) => {
                buf.truncate(n);
                buf
            }
            Err(_) => Vec::new(),
        }
    }

    fn toggle_mode(&mut self) -> Vec<PanelEvent> {
        self.mode = match self.mode {
            ViewMode::Hex => ViewMode::Text,
            ViewMode::Text => ViewMode::Hex,
        };
        self.clamp_scroll();
        vec![PanelEvent::NeedsRedraw]
    }

    fn scroll_rows(&mut self, rows: i64) {
        let bpr = self.bytes_per_row() as i64;
        let delta = rows.saturating_mul(bpr);
        let new = (self.top_byte as i64 + delta).max(0) as u64;
        self.top_byte = new;
        self.clamp_scroll();
    }

    /// Build one hex-dump row (`offset │ hex │ ASCII`) as styled spans.
    fn hex_row<'a>(&self, off: u64, bytes: &[u8]) -> Line<'a> {
        let dim = Style::default().fg(self.theme.disabled);
        let fg = Style::default().fg(self.theme.fg);
        let off_style = Style::default().fg(self.theme.line_numbers);

        let mut spans: Vec<Span<'a>> = Vec::with_capacity(HEX_COLS as usize * 2 + 4);
        spans.push(Span::styled(format!("{off:08X}"), off_style));
        spans.push(Span::styled("  ", dim));

        for i in 0..HEX_COLS as usize {
            match bytes.get(i) {
                Some(&b) => {
                    let style = if b == 0 { dim } else { fg };
                    spans.push(Span::styled(format!("{b:02x} "), style));
                }
                None => spans.push(Span::styled("   ", dim)),
            }
            // Extra gap after the 8th byte for readability.
            if i == 7 {
                spans.push(Span::styled(" ", dim));
            }
        }

        spans.push(Span::styled(" ", dim));
        for &b in bytes {
            if (0x20..=0x7e).contains(&b) {
                spans.push(Span::styled((b as char).to_string(), fg));
            } else {
                spans.push(Span::styled("·", dim));
            }
        }

        Line::from(spans)
    }

    /// Build one plain-text row from `bytes` (non-printable shown as `·`).
    fn text_row<'a>(&self, bytes: &[u8]) -> Line<'a> {
        let fg = Style::default().fg(self.theme.fg);
        let s: String = bytes
            .iter()
            .map(|&b| {
                if (0x20..=0x7e).contains(&b) || b == b'\t' {
                    b as char
                } else {
                    '·'
                }
            })
            .collect();
        Line::from(Span::styled(s, fg))
    }
}

impl Panel for BinaryPanel {
    fn name(&self) -> &'static str {
        "binary"
    }

    fn width_preference(&self) -> WidthPreference {
        WidthPreference::PreferWide
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn prepare_render(&mut self, theme: &Theme, config: &Arc<Config>) {
        self.theme = ThemeColors::from(theme);
        let ptr = Arc::as_ptr(config) as usize;
        if self.last_config_ptr != ptr {
            self.last_config_ptr = ptr;
            let mut t = HotkeyTable::new();
            t.insert("toggle_hex", &config.viewer.keybindings.toggle_hex);
            self.hotkeys = t;
        }
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, _ctx: &RenderContext) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        self.last_inner = (area.width, area.height);

        if let Some(ref err) = self.error {
            buf.set_string(area.x, area.y, err, Style::default().fg(self.theme.error));
            return;
        }
        if self.len == 0 {
            buf.set_string(
                area.x,
                area.y,
                "(empty file)",
                Style::default().fg(self.theme.disabled),
            );
            return;
        }

        // A mode/area change may have left the scroll past the end.
        self.clamp_scroll();

        let bpr = self.bytes_per_row();
        let visible_rows = area.height as u64;
        let start = self.top_byte;
        let want = (visible_rows * bpr).min(self.len - start) as usize;
        let window = self.read_window(start, want);

        for row in 0..visible_rows {
            let row_start = (row * bpr) as usize;
            if row_start >= window.len() {
                break;
            }
            let row_end = (row_start + bpr as usize).min(window.len());
            let bytes = &window[row_start..row_end];
            let off = start + row * bpr;
            let line = match self.mode {
                ViewMode::Hex => self.hex_row(off, bytes),
                ViewMode::Text => self.text_row(bytes),
            };
            buf.set_line(area.x, area.y + row as u16, &line, area.width);
        }
    }

    fn status_segments(&self) -> Vec<StatusSegment> {
        if self.error.is_some() {
            return vec![];
        }
        let (hex_kind, text_kind) = match self.mode {
            ViewMode::Hex => (SegmentKind::Active, SegmentKind::Inactive),
            ViewMode::Text => (SegmentKind::Inactive, SegmentKind::Active),
        };
        vec![
            StatusSegment::new(
                format!(" {:#X} / {:#X} ", self.top_byte, self.len),
                SegmentKind::Value,
            ),
            StatusSegment::new("· ", SegmentKind::Label),
            StatusSegment::clickable("Hex", hex_kind, "toggle_hex"),
            StatusSegment::new("│", SegmentKind::Label),
            StatusSegment::clickable("Text", text_kind, "toggle_hex"),
            StatusSegment::new(" · RO ", SegmentKind::Label),
        ]
    }

    fn handle_status_action(&mut self, action: &str) -> Vec<PanelEvent> {
        if action == "toggle_hex" {
            return self.toggle_mode();
        }
        vec![]
    }

    fn handle_key(&mut self, chord: KeyChord) -> Vec<PanelEvent> {
        let key = chord.raw;
        if self.hotkeys.matches("toggle_hex", &key) {
            return self.toggle_mode();
        }
        let page = (self.last_inner.1 as i64 - 1).max(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_rows(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_rows(1),
            KeyCode::PageUp => self.scroll_rows(-page),
            KeyCode::PageDown => self.scroll_rows(page),
            KeyCode::Home | KeyCode::Char('g') => self.top_byte = 0,
            KeyCode::End | KeyCode::Char('G') => self.top_byte = self.max_top(self.bytes_per_row()),
            KeyCode::Char('q') => return vec![PanelEvent::ClosePanel],
            _ => return vec![],
        }
        vec![PanelEvent::NeedsRedraw]
    }

    fn handle_scroll(&mut self, delta: i32, _panel_area: Rect) -> Vec<PanelEvent> {
        self.scroll_rows(delta as i64);
        vec![PanelEvent::NeedsRedraw]
    }

    fn handle_mouse(&mut self, event: MouseEvent, _panel_area: Rect) -> Vec<PanelEvent> {
        match event.kind {
            MouseEventKind::ScrollUp => self.scroll_rows(-1),
            MouseEventKind::ScrollDown => self.scroll_rows(1),
            MouseEventKind::Down(MouseButton::Left) => {}
            _ => return vec![],
        }
        vec![PanelEvent::NeedsRedraw]
    }

    fn to_session(&self, _session_dir: &Path) -> Option<SessionPanel> {
        Some(SessionPanel::Binary {
            path: self.file_path.clone(),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn get_working_directory(&self) -> Option<PathBuf> {
        self.file_path.parent().map(|p| p.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel_with(bytes: &[u8]) -> BinaryPanel {
        let mut p = BinaryPanel::new(PathBuf::from("/dev/null")).unwrap();
        // Bypass the file: drive the formatting helpers directly.
        p.len = bytes.len() as u64;
        p
    }

    #[test]
    fn hex_row_formats_offset_bytes_and_ascii() {
        let p = panel_with(b"");
        let line = p.hex_row(0x10, b"Hi\x00\xff");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        // Offset, hex (incl. 00 and ff), then ASCII gutter with `·` for
        // non-printable bytes.
        assert!(text.starts_with("00000010"), "offset: {text:?}");
        assert!(text.contains("48 69 00 ff"), "hex: {text:?}");
        assert!(text.trim_end().ends_with("Hi··"), "ascii: {text:?}");
    }

    #[test]
    fn text_row_replaces_non_printable() {
        let p = panel_with(b"");
        let line = p.text_row(b"ab\x00\x07c");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "ab··c");
    }

    #[test]
    fn scroll_is_row_aligned_and_clamped() {
        let mut p = panel_with(&[0u8; 100]); // 100 bytes → rows of 16
        p.last_inner = (80, 10);
        p.scroll_rows(2);
        assert_eq!(p.top_byte, 32, "two hex rows down");
        // Cannot scroll past the last row start: last row = 96 (96..100).
        p.scroll_rows(1000);
        assert_eq!(p.top_byte, 96);
        p.scroll_rows(-1000);
        assert_eq!(p.top_byte, 0);
    }

    #[test]
    fn toggle_switches_mode() {
        let mut p = panel_with(&[0u8; 100]);
        assert_eq!(p.mode, ViewMode::Hex);
        p.toggle_mode();
        assert_eq!(p.mode, ViewMode::Text);
        p.toggle_mode();
        assert_eq!(p.mode, ViewMode::Hex);
    }
}
