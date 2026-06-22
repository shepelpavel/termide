//! Binary file viewer panel (read-only hex/ASCII).
//!
//! Renders a binary file as a classic hex dump — `offset │ hex bytes │ ASCII
//! gutter` — in pure text pseudographics. The number of bytes per row adapts to
//! the panel width in 16-byte sections, so a wide panel shows 32/48/… bytes per
//! row. The file is read in windows on demand so large files are not loaded
//! fully into memory.
//!
//! The viewer is hex-only: the plain-text view of the same file is the editor.
//! `Ctrl+L` (or the `Text` chip in the status bar) swaps this panel in place
//! for a read-only editor; the editor's `Ctrl+L` swaps back to hex. The offset
//! and the `Hex│Text` toggle are contributed to the global status bar via
//! [`Panel::status_segments`].

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

/// Bytes per section; rows are laid out in whole sections (16, 32, 48, …).
const SECTION: u64 = 16;

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
    /// Byte offset of the first visible row (kept aligned to the row size).
    top_byte: u64,
    /// Last inner width/height seen in `render` (for column fitting + paging).
    last_inner: (u16, u16),
    /// Cached theme colors.
    theme: ThemeColors,
    /// Configurable hotkeys (toggle hex/text).
    hotkeys: HotkeyTable,
    /// Pointer of the last `Arc<Config>` used to build hotkeys.
    last_config_ptr: usize,
}

/// Bytes shown per row for the given inner width: as many whole 16-byte
/// sections as fit, at least one. Row layout is
/// `8 offset + 2 + n*3 hex + 1 + n ascii` ≈ `11 + 4n` columns.
fn bytes_per_row(width: u16) -> u64 {
    let usable = (width as i64 - 11).max(0);
    let fit = (usable / 4) as u64;
    (fit / SECTION).max(1) * SECTION
}

impl BinaryPanel {
    /// Open a binary file in the hex viewer.
    pub fn new(path: PathBuf) -> Result<Self> {
        let mut panel = Self {
            file_path: path.clone(),
            title: String::new(),
            file: None,
            len: 0,
            error: None,
            top_byte: 0,
            last_inner: (0, 0),
            theme: ThemeColors::default(),
            hotkeys: HotkeyTable::default(),
            last_config_ptr: 0,
        };
        panel.set_file(path);
        Ok(panel)
    }

    /// Point the panel at a file (also used to reuse an existing viewer).
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

    /// Bytes per row for the current width.
    fn cols(&self) -> u64 {
        bytes_per_row(self.last_inner.0)
    }

    /// Largest valid `top_byte`, aligned to `bpr`.
    fn max_top(&self, bpr: u64) -> u64 {
        if self.len == 0 {
            return 0;
        }
        (self.len.saturating_sub(1) / bpr) * bpr
    }

    /// Re-align and clamp `top_byte` after an area/scroll change.
    fn clamp_scroll(&mut self) {
        let bpr = self.cols();
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

    fn scroll_rows(&mut self, rows: i64) {
        let bpr = self.cols() as i64;
        let delta = rows.saturating_mul(bpr);
        self.top_byte = (self.top_byte as i64 + delta).max(0) as u64;
        self.clamp_scroll();
    }

    /// Event swapping this hex panel in place for a read-only text editor.
    fn swap_to_text(&self) -> Vec<PanelEvent> {
        vec![PanelEvent::SwapActiveToText(self.file_path.clone())]
    }

    /// Build one hex-dump row (`offset │ hex │ ASCII`) over `cols` columns.
    fn hex_row<'a>(&self, off: u64, bytes: &[u8], cols: u64) -> Line<'a> {
        let dim = Style::default().fg(self.theme.disabled);
        let fg = Style::default().fg(self.theme.fg);
        let off_style = Style::default().fg(self.theme.line_numbers);

        let mut spans: Vec<Span<'a>> = Vec::with_capacity(cols as usize * 2 + 4);
        spans.push(Span::styled(format!("{off:08X}"), off_style));
        spans.push(Span::styled("  ", dim));

        for i in 0..cols as usize {
            match bytes.get(i) {
                Some(&b) => {
                    let style = if b == 0 { dim } else { fg };
                    spans.push(Span::styled(format!("{b:02x} "), style));
                }
                None => spans.push(Span::styled("   ", dim)),
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

        // A width change may have left the scroll past the end / misaligned.
        self.clamp_scroll();

        let bpr = self.cols();
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
            let line = self.hex_row(off, bytes, bpr);
            buf.set_line(area.x, area.y + row as u16, &line, area.width);
        }
    }

    fn status_segments(&self) -> Vec<StatusSegment> {
        if self.error.is_some() {
            return vec![];
        }
        vec![
            StatusSegment::new(
                format!(" {:#X} / {:#X} ", self.top_byte, self.len),
                SegmentKind::Value,
            ),
            StatusSegment::new("· ", SegmentKind::Label),
            // Hex is the current view; Text swaps to the editor.
            StatusSegment::new("Hex", SegmentKind::Active),
            StatusSegment::new("│", SegmentKind::Label),
            StatusSegment::clickable("Text", SegmentKind::Inactive, "to_text"),
            StatusSegment::new(" · RO ", SegmentKind::Label),
        ]
    }

    fn handle_status_action(&mut self, action: &str) -> Vec<PanelEvent> {
        if action == "to_text" {
            return self.swap_to_text();
        }
        vec![]
    }

    fn handle_key(&mut self, chord: KeyChord) -> Vec<PanelEvent> {
        let key = chord.raw;
        if self.hotkeys.matches("toggle_hex", &key) {
            return self.swap_to_text();
        }
        let page = (self.last_inner.1 as i64 - 1).max(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_rows(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_rows(1),
            KeyCode::PageUp => self.scroll_rows(-page),
            KeyCode::PageDown => self.scroll_rows(page),
            KeyCode::Home | KeyCode::Char('g') => self.top_byte = 0,
            KeyCode::End | KeyCode::Char('G') => self.top_byte = self.max_top(self.cols()),
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
            MouseEventKind::Down(MouseButton::Left) => return vec![],
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

    fn panel_with(len: u64) -> BinaryPanel {
        let mut p = BinaryPanel::new(PathBuf::from("/dev/null")).unwrap();
        p.len = len;
        p
    }

    #[test]
    fn bytes_per_row_rounds_to_16_byte_sections() {
        // ~11 + 4n columns per row; rounds down to whole 16-byte sections.
        assert_eq!(bytes_per_row(80), 16); // (80-11)/4 = 17 → one section
        assert_eq!(bytes_per_row(140), 32); // (140-11)/4 = 32 → two sections
        assert_eq!(bytes_per_row(10), 16); // too narrow → at least one section
    }

    #[test]
    fn hex_row_formats_offset_bytes_and_ascii() {
        let p = panel_with(0);
        let line = p.hex_row(0x10, b"Hi\x00\xff", 16);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("00000010"), "offset: {text:?}");
        assert!(text.contains("48 69 00 ff"), "hex: {text:?}");
        assert!(text.trim_end().ends_with("Hi··"), "ascii: {text:?}");
    }

    #[test]
    fn scroll_is_row_aligned_and_clamped() {
        let mut p = panel_with(100); // 100 bytes
        p.last_inner = (80, 10); // 16 bytes/row
        p.scroll_rows(2);
        assert_eq!(p.top_byte, 32);
        p.scroll_rows(1000);
        assert_eq!(p.top_byte, 96); // last row start (96..100)
        p.scroll_rows(-1000);
        assert_eq!(p.top_byte, 0);
    }

    #[test]
    fn wide_panel_uses_more_columns() {
        let mut p = panel_with(200);
        p.last_inner = (140, 10); // 32 bytes/row
        assert_eq!(p.cols(), 32);
        p.scroll_rows(1);
        assert_eq!(p.top_byte, 32);
    }
}
