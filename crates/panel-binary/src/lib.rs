//! Binary file viewer panel (read-only hex/ASCII).
//!
//! Renders a binary file as a classic hex dump — `offset │ hex bytes │ ASCII
//! gutter` — in pure text pseudographics. The number of bytes per row adapts to
//! the panel width in 16-byte sections, so a wide panel shows 32/48/… bytes per
//! row. The file is read in windows on demand so large files are not loaded
//! fully into memory.
//!
//! A byte cursor is shown in **both** the hex and ASCII zones at once (the
//! active zone is highlighted more strongly); `Tab` switches the active zone.
//! Shift+movement extends a selection and `Ctrl+C` copies it — as a hex string
//! when the cursor is in the hex zone, as text when it is in the ASCII zone.
//!
//! `Ctrl+L` (or the status-bar chip) toggles hex ↔ text. A real text file swaps
//! in place for a read-only editor (and the editor's `Ctrl+L` swaps back to
//! hex); a binary file — which the editor can't open as text — toggles an
//! in-panel lossy text view instead. `Ctrl+F` searches an ASCII substring or a
//! hex byte sequence, highlighting matches in both zones.
//!
//! Opened with `F4`, the panel is editable: typing overwrites the byte under
//! the cursor (two hex nibbles in the hex zone, a character in the ASCII zone)
//! without changing the file length. `Ctrl+S` saves after a confirmation,
//! backing the original up to `<file>.bak` first.

use std::any::Any;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};

use termide_core::{
    Config, HotkeyTable, KeyChord, Panel, PanelEvent, RenderContext, SegmentKind, SessionPanel,
    StatusSegment, Theme, ThemeColors, WidthPreference,
};
use termide_modal::{FindBar, FindBarAction, FindBarBtn, FindBarConfig, FindField};
use termide_ui::ScrollBar;

/// Bytes per section; rows are laid out in whole sections (16, 32, 48, …).
const SECTION: u64 = 16;

/// Upper bound on a single clipboard copy, so a huge selection can't allocate
/// without limit.
const MAX_COPY: u64 = 1 << 20;

/// Search scans at most this many bytes (from the start); larger files report
/// "file too large to search" rather than scanning unbounded.
const MAX_SEARCH_BYTES: u64 = 64 << 20;

/// Cap on collected match offsets.
const MAX_MATCHES: usize = 10_000;

/// Which column zone the cursor edits/navigates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    Hex,
    Ascii,
}

/// In-panel rendering mode. `Text` is a lossy plain-text view used for binary
/// files (a real text file swaps to the editor instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Hex,
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
    /// Byte offset of the first visible row (kept aligned to the row size).
    top_byte: u64,
    /// Cursor byte index (`0..len`).
    cursor: u64,
    /// Selection anchor; `Some` while a selection is active.
    anchor: Option<u64>,
    /// Active column zone (hex or ASCII).
    zone: Zone,
    /// In-panel rendering mode (hex dump or lossy text, for binary files).
    mode: ViewMode,
    /// Whether the file is open for editing (overwrite-in-place).
    editable: bool,
    /// Pending overwrites by absolute offset (applied on render, written on save).
    edits: std::collections::BTreeMap<u64, u8>,
    /// High nibble typed in the hex zone, awaiting the low nibble.
    pending_nibble: Option<u8>,
    /// Byte where the current mouse drag started (anchor for drag-selection).
    drag_from: Option<u64>,
    /// Inline find bar (ASCII / hex byte search), when open.
    find_bar: Option<FindBar>,
    /// Match start offsets from the last search.
    matches: Vec<u64>,
    /// Byte length of each search match (the needle length).
    match_len: usize,
    /// Index of the current match within `matches`.
    match_idx: usize,
    /// Last render area (absolute) for click mapping + paging.
    last_area: Rect,
    /// Cached theme colors.
    theme: ThemeColors,
    /// Full theme, cached for rendering the find bar.
    theme_full: Option<Theme>,
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

/// Parse a hex query (`"ff fe"` / `"fffe"`) into bytes; `None` if it has odd
/// digits or a non-hex character.
fn parse_hex(q: &str) -> Option<Vec<u8>> {
    let digits: String = q.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.is_empty() || !digits.len().is_multiple_of(2) {
        return None;
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).ok())
        .collect()
}

/// All start offsets where `needle` occurs in `hay` (capped at [`MAX_MATCHES`]).
/// `ci` does ASCII-case-insensitive matching.
fn find_all(hay: &[u8], needle: &[u8], ci: bool) -> Vec<u64> {
    let mut out = Vec::new();
    if needle.is_empty() || needle.len() > hay.len() {
        return out;
    }
    let eq = |a: &u8, b: &u8| {
        if ci {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    for i in 0..=hay.len() - needle.len() {
        if hay[i..i + needle.len()]
            .iter()
            .zip(needle)
            .all(|(a, b)| eq(a, b))
        {
            out.push(i as u64);
            if out.len() >= MAX_MATCHES {
                break;
            }
        }
    }
    out
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
            cursor: 0,
            anchor: None,
            zone: Zone::Hex,
            mode: ViewMode::Hex,
            editable: false,
            edits: std::collections::BTreeMap::new(),
            pending_nibble: None,
            drag_from: None,
            find_bar: None,
            matches: Vec::new(),
            match_len: 0,
            match_idx: 0,
            last_area: Rect::default(),
            theme: ThemeColors::default(),
            theme_full: None,
            hotkeys: HotkeyTable::default(),
            last_config_ptr: 0,
        };
        panel.set_file(path);
        Ok(panel)
    }

    /// Open a binary file for editing (overwrite-in-place).
    pub fn new_editable(path: PathBuf) -> Result<Self> {
        let mut panel = Self::new(path)?;
        panel.editable = true;
        Ok(panel)
    }

    /// Whether the buffer has unsaved overwrites.
    pub fn is_modified(&self) -> bool {
        !self.edits.is_empty()
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
        self.cursor = 0;
        self.anchor = None;
        self.edits.clear();
        self.pending_nibble = None;
    }

    /// Bytes per row for the current width and mode.
    fn cols(&self) -> u64 {
        match self.mode {
            ViewMode::Hex => bytes_per_row(self.last_area.width),
            ViewMode::Text => (self.last_area.width as u64).max(1),
        }
    }

    /// Largest valid `top_byte`, aligned to `bpr`.
    fn max_top(&self, bpr: u64) -> u64 {
        if self.len == 0 {
            return 0;
        }
        (self.len.saturating_sub(1) / bpr) * bpr
    }

    /// Clamp `top_byte` to an aligned, in-range value.
    fn clamp_top(&mut self) {
        let bpr = self.cols();
        self.top_byte -= self.top_byte % bpr;
        let max_top = self.max_top(bpr);
        if self.top_byte > max_top {
            self.top_byte = max_top;
        }
    }

    /// Scroll the view (not the cursor) by whole rows.
    fn scroll_rows(&mut self, rows: i64) {
        let bpr = self.cols() as i64;
        self.top_byte = (self.top_byte as i64 + rows.saturating_mul(bpr)).max(0) as u64;
        self.clamp_top();
    }

    /// Move the cursor by `delta` bytes, optionally extending the selection.
    fn move_cursor(&mut self, delta: i64, extend: bool) {
        if self.len == 0 {
            return;
        }
        self.pending_nibble = None;
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        let max = (self.len - 1) as i64;
        self.cursor = (self.cursor as i64 + delta).clamp(0, max) as u64;
        self.ensure_cursor_visible();
    }

    /// Jump the cursor to an absolute byte, optionally extending the selection.
    fn set_cursor(&mut self, byte: u64, extend: bool) {
        if self.len == 0 {
            return;
        }
        self.pending_nibble = None;
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = byte.min(self.len - 1);
        self.ensure_cursor_visible();
    }

    /// Scroll so the cursor's row is visible.
    fn ensure_cursor_visible(&mut self) {
        let bpr = self.cols();
        let rows = (self.last_area.height as u64).max(1);
        let cur_row = self.cursor / bpr;
        let top_row = self.top_byte / bpr;
        if cur_row < top_row {
            self.top_byte = cur_row * bpr;
        } else if cur_row >= top_row + rows {
            self.top_byte = (cur_row + 1 - rows) * bpr;
        }
        self.clamp_top();
    }

    /// Inclusive selected byte range (or just the cursor byte).
    fn sel_range(&self) -> (u64, u64) {
        match self.anchor {
            Some(a) => (a.min(self.cursor), a.max(self.cursor)),
            None => (self.cursor, self.cursor),
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
        let n = match file.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return Vec::new(),
        };
        buf.truncate(n);
        // Overlay unsaved edits that fall in this window.
        if !self.edits.is_empty() {
            let end = start + buf.len() as u64;
            for (&off, &b) in self.edits.range(start..end) {
                buf[(off - start) as usize] = b;
            }
        }
        buf
    }

    /// Copy the selection (or cursor byte) to the clipboard — as a hex string
    /// in the hex zone, as text in the ASCII zone.
    fn copy_selection(&mut self) -> Vec<PanelEvent> {
        let (s, e) = self.sel_range();
        let count = (e - s + 1).min(MAX_COPY) as usize;
        let bytes = self.read_window(s, count);
        let text = match self.zone {
            Zone::Hex => bytes
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" "),
            Zone::Ascii => String::from_utf8_lossy(&bytes).to_string(),
        };
        let _ = termide_clipboard::copy(&text);
        vec![PanelEvent::NeedsRedraw]
    }

    /// Toggle hex ↔ text. A real text file swaps in place for the editor; a
    /// binary file (which the editor can't open) toggles an in-panel lossy
    /// text view instead.
    fn toggle_view(&mut self) -> Vec<PanelEvent> {
        if !termide_core::util::is_binary_file(&self.file_path) {
            return vec![PanelEvent::SwapActiveToText(self.file_path.clone())];
        }
        self.mode = match self.mode {
            ViewMode::Hex => ViewMode::Text,
            ViewMode::Text => ViewMode::Hex,
        };
        self.clamp_top();
        vec![PanelEvent::NeedsRedraw]
    }

    /// Whether byte `gi` falls inside any search match.
    fn match_at(&self, gi: u64) -> Option<bool> {
        if self.match_len == 0 || self.matches.is_empty() {
            return None;
        }
        let mlen = self.match_len as u64;
        // Last match whose start is <= gi.
        let count = self.matches.partition_point(|&m| m <= gi);
        if count == 0 {
            return None;
        }
        let start = self.matches[count - 1];
        if gi < start + mlen {
            Some(count - 1 == self.match_idx)
        } else {
            None
        }
    }

    /// Current value of the byte at `off` (pending edit or on-disk).
    fn byte_value(&mut self, off: u64) -> u8 {
        if let Some(&b) = self.edits.get(&off) {
            return b;
        }
        self.read_window(off, 1).first().copied().unwrap_or(0)
    }

    /// Record an overwrite of the byte at `off`.
    fn set_byte(&mut self, off: u64, b: u8) {
        self.edits.insert(off, b);
    }

    /// Move the cursor one byte forward after entering a value (no selection).
    fn advance_cursor(&mut self) {
        self.pending_nibble = None;
        self.anchor = None;
        if self.cursor + 1 < self.len {
            self.cursor += 1;
            self.ensure_cursor_visible();
        }
    }

    /// Apply a hex digit in the hex zone (two nibbles per byte).
    fn edit_hex_nibble(&mut self, d: u8) {
        let cur = self.byte_value(self.cursor);
        self.anchor = None;
        match self.pending_nibble.take() {
            None => {
                self.set_byte(self.cursor, (d << 4) | (cur & 0x0f));
                self.pending_nibble = Some(d);
            }
            Some(hi) => {
                self.set_byte(self.cursor, (hi << 4) | d);
                self.advance_cursor();
            }
        }
    }

    /// Try to interpret `key` as an edit (hex digit in the hex zone, printable
    /// char in the ASCII zone). Returns true if it was consumed.
    fn try_edit(&mut self, key: crossterm::event::KeyEvent) -> bool {
        // Only plain (or Shift) character keys edit; Ctrl/Alt fall through.
        if !(key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) {
            return false;
        }
        let KeyCode::Char(c) = key.code else {
            return false;
        };
        match self.zone {
            Zone::Hex => match c.to_digit(16) {
                Some(d) => {
                    self.edit_hex_nibble(d as u8);
                    true
                }
                None => false,
            },
            Zone::Ascii => {
                if c.is_ascii() && (' '..='~').contains(&c) {
                    self.set_byte(self.cursor, c as u8);
                    self.advance_cursor();
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Write pending edits to disk after backing the original up to `<file>.bak`.
    pub fn save(&mut self) -> std::io::Result<()> {
        use std::io::Write;
        if self.edits.is_empty() {
            return Ok(());
        }
        let mut bak = self.file_path.clone().into_os_string();
        bak.push(".bak");
        std::fs::copy(&self.file_path, PathBuf::from(bak))?;

        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(&self.file_path)?;
        for (&off, &b) in &self.edits {
            f.seek(SeekFrom::Start(off))?;
            f.write_all(&[b])?;
        }
        f.flush()?;

        self.edits.clear();
        self.pending_nibble = None;
        // Refresh the read handle so reads reflect the saved bytes.
        self.file = File::open(&self.file_path).ok();
        Ok(())
    }

    /// Open the inline find bar (ASCII substring or hex byte sequence).
    fn open_find(&mut self) {
        let mut bar = FindBar::new(FindBarConfig {
            fields: vec![FindField::Find],
            // Prev/Next navigation, ASCII case toggle, and the hex-mode toggle.
            buttons: vec![
                FindBarBtn::Prev,
                FindBarBtn::Next,
                FindBarBtn::Case,
                FindBarBtn::Hex,
            ],
        });
        bar.focus_first();
        self.find_bar = Some(bar);
        self.matches.clear();
        self.match_idx = 0;
    }

    fn close_find(&mut self) {
        self.find_bar = None;
        self.matches.clear();
    }

    /// Parse the query into a search needle: hex bytes when the `[hex]` toggle
    /// is on, otherwise the raw ASCII bytes. Returns `(needle, case_insensitive)`.
    fn needle(&self) -> Option<(Vec<u8>, bool)> {
        let bar = self.find_bar.as_ref()?;
        let q = bar.find_text();
        if q.is_empty() {
            return None;
        }
        if bar.hex_mode() {
            parse_hex(q).map(|n| (n, false))
        } else {
            Some((q.as_bytes().to_vec(), !bar.case_sensitive()))
        }
    }

    /// Re-run the search and jump to the first match at/after the cursor.
    fn run_search(&mut self) {
        let parsed = self.needle();
        let too_large = self.len > MAX_SEARCH_BYTES;

        let Some((needle, ci)) = parsed else {
            self.matches.clear();
            if let Some(bar) = self.find_bar.as_mut() {
                bar.clear_match_info();
                let hex_bad = bar.hex_mode() && !bar.find_text().is_empty();
                bar.set_info_text(hex_bad.then(|| "invalid hex".to_string()));
            }
            return;
        };

        if too_large {
            self.matches.clear();
            if let Some(bar) = self.find_bar.as_mut() {
                bar.clear_match_info();
                bar.set_info_text(Some("file too large to search".to_string()));
            }
            return;
        }

        let cap = self.len.min(MAX_SEARCH_BYTES) as usize;
        let hay = self.read_window(0, cap);
        self.matches = find_all(&hay, &needle, ci);
        self.match_len = needle.len();
        self.match_idx = 0;

        if let Some(bar) = self.find_bar.as_mut() {
            bar.set_info_text(None);
            if self.matches.is_empty() {
                bar.set_match_info(0, 0);
            } else {
                bar.set_match_info(1, self.matches.len());
            }
        }
        if let Some(&off) = self.matches.first() {
            self.set_cursor(off, false);
        }
    }

    /// Step to the next/previous match and move the cursor there.
    fn step_match(&mut self, forward: bool) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        self.match_idx = if forward {
            (self.match_idx + 1) % n
        } else {
            (self.match_idx + n - 1) % n
        };
        let off = self.matches[self.match_idx];
        if let Some(bar) = self.find_bar.as_mut() {
            bar.set_match_info(self.match_idx + 1, n);
        }
        self.set_cursor(off, false);
    }

    fn handle_find_action(&mut self, action: FindBarAction) -> Vec<PanelEvent> {
        match action {
            FindBarAction::QueryChanged | FindBarAction::Refresh => self.run_search(),
            FindBarAction::Next | FindBarAction::Submit => self.step_match(true),
            FindBarAction::Previous => self.step_match(false),
            FindBarAction::Close => self.close_find(),
            _ => {}
        }
        vec![PanelEvent::NeedsRedraw]
    }

    /// Style for a byte cell, in the hex or ASCII representation.
    fn cell_style(&self, gi: u64, byte: u8, repr: Zone) -> Style {
        let mut st = Style::default().fg(if byte == 0 {
            self.theme.disabled
        } else {
            self.theme.fg
        });
        // Search-match highlight (same colours as the editor: current match in
        // the accent colour, other matches in the warning colour).
        match self.match_at(gi) {
            Some(true) => {
                st = st
                    .bg(self.theme.info)
                    .fg(self.theme.bg)
                    .add_modifier(Modifier::BOLD)
            }
            Some(false) => st = st.bg(self.theme.warning).fg(self.theme.bg),
            None => {}
        }
        let (s, e) = self.sel_range();
        let selected = self.anchor.is_some() && gi >= s && gi <= e;
        if selected {
            st = st.fg(self.theme.selection_fg).bg(self.theme.selection_bg);
        }
        // The cursor byte is shown inverse in both zones (consistent with the
        // editor's cursor); the active zone also gets BOLD so it's clear which
        // zone Tab will edit/navigate.
        if gi == self.cursor {
            st = st.add_modifier(Modifier::REVERSED);
            if repr == self.zone {
                st = st.add_modifier(Modifier::BOLD);
            }
        }
        st
    }

    /// Build one hex-dump row (`offset │ hex │ ASCII`) over `cols` columns.
    fn hex_row<'a>(&self, off: u64, bytes: &[u8], cols: u64) -> Line<'a> {
        let dim = Style::default().fg(self.theme.disabled);
        let off_style = Style::default().fg(self.theme.line_numbers);

        let mut spans: Vec<Span<'a>> = Vec::with_capacity(cols as usize * 2 + 4);
        spans.push(Span::styled(format!("{off:08X}"), off_style));
        spans.push(Span::styled("  ", dim));

        for i in 0..cols as usize {
            // Separator after the byte: a dim `│` every 8 bytes for orientation
            // (width-neutral — it replaces the inter-byte space), else a space.
            let last = i + 1 == cols as usize;
            let sep = if i % 8 == 7 && !last { "│" } else { " " };
            match bytes.get(i) {
                Some(&b) => {
                    let gi = off + i as u64;
                    spans.push(Span::styled(
                        format!("{b:02x}"),
                        self.cell_style(gi, b, Zone::Hex),
                    ));
                    spans.push(Span::styled(sep, dim));
                }
                None => {
                    spans.push(Span::styled("  ", dim));
                    spans.push(Span::styled(sep, dim));
                }
            }
        }

        spans.push(Span::styled(" ", dim));
        for (i, &b) in bytes.iter().enumerate() {
            let gi = off + i as u64;
            let ch = if (0x20..=0x7e).contains(&b) {
                (b as char).to_string()
            } else {
                "·".to_string()
            };
            spans.push(Span::styled(ch, self.cell_style(gi, b, Zone::Ascii)));
        }

        Line::from(spans)
    }

    /// Build one lossy plain-text row (non-printable bytes shown as `·`), one
    /// char per byte, with the same cursor/selection/match highlighting.
    fn text_row<'a>(&self, off: u64, bytes: &[u8]) -> Line<'a> {
        let mut spans: Vec<Span<'a>> = Vec::with_capacity(bytes.len());
        for (i, &b) in bytes.iter().enumerate() {
            let gi = off + i as u64;
            let ch = if (0x20..=0x7e).contains(&b) {
                (b as char).to_string()
            } else {
                "·".to_string()
            };
            spans.push(Span::styled(ch, self.cell_style(gi, b, Zone::Ascii)));
        }
        Line::from(spans)
    }

    /// Map a mouse event's absolute position to a byte + zone in the hex grid.
    fn byte_at_event(&self, event: &MouseEvent) -> Option<(u64, Zone)> {
        if event.column < self.last_area.x || event.row < self.last_area.y {
            return None;
        }
        self.byte_at(
            event.column - self.last_area.x,
            event.row - self.last_area.y,
        )
    }

    /// Map a click at panel-relative `(cx, cy)` to a byte + zone.
    fn byte_at(&self, cx: u16, cy: u16) -> Option<(u64, Zone)> {
        let cols = self.cols();
        let row = self.top_byte / cols + cy as u64;
        let row_start = row * cols;
        let cx = cx as u64;
        if self.mode == ViewMode::Text {
            if cx >= cols {
                return None;
            }
            return Some((
                (row_start + cx).min(self.len.saturating_sub(1)),
                Zone::Ascii,
            ));
        }
        let ascii_start = 11 + cols * 3; // 8 offset + 2 + cols*3 hex + 1 sep
        if cx >= ascii_start && cx < ascii_start + cols {
            let i = cx - ascii_start;
            return Some(((row_start + i).min(self.len.saturating_sub(1)), Zone::Ascii));
        }
        if (10..10 + cols * 3).contains(&cx) {
            let i = (cx - 10) / 3;
            return Some(((row_start + i).min(self.len.saturating_sub(1)), Zone::Hex));
        }
        None
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
        if self.editable && self.is_modified() {
            format!("{}*", self.title)
        } else {
            self.title.clone()
        }
    }

    fn prepare_render(&mut self, theme: &Theme, config: &Arc<Config>) {
        self.theme = ThemeColors::from(theme);
        self.theme_full = Some(*theme);
        let ptr = Arc::as_ptr(config) as usize;
        if self.last_config_ptr != ptr {
            self.last_config_ptr = ptr;
            let mut t = HotkeyTable::new();
            t.insert("toggle_hex", &config.viewer.keybindings.toggle_hex);
            self.hotkeys = t;
        }
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Find bar docked at the TOP with a separator below, matching the
        // editor / file manager.
        let mut hex_area = area;
        if let (Some(bar), Some(theme)) = (self.find_bar.as_mut(), self.theme_full.as_ref()) {
            let bar_h = bar.height().min(area.height);
            let bar_area = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: bar_h,
            };
            bar.render(bar_area, buf, theme, true);

            let mut used = bar_h;
            let sep_y = area.y + bar_h;
            if sep_y < area.y + area.height {
                let style = Style::default().fg(self.theme.disabled);
                for dx in 0..area.width {
                    buf[(area.x + dx, sep_y)].set_symbol("─").set_style(style);
                }
                used += 1;
            }
            hex_area = Rect {
                x: area.x,
                y: area.y + used,
                width: area.width,
                height: area.height.saturating_sub(used),
            };
        }
        self.last_area = hex_area;

        if let Some(ref err) = self.error {
            buf.set_string(
                hex_area.x,
                hex_area.y,
                err,
                Style::default().fg(self.theme.error),
            );
        } else if self.len == 0 {
            buf.set_string(
                hex_area.x,
                hex_area.y,
                "(empty file)",
                Style::default().fg(self.theme.disabled),
            );
        } else if hex_area.height > 0 {
            self.clamp_top();

            let bpr = self.cols();
            let visible_rows = hex_area.height as u64;
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
                    ViewMode::Hex => self.hex_row(off, bytes, bpr),
                    ViewMode::Text => self.text_row(off, bytes),
                };
                buf.set_line(hex_area.x, hex_area.y + row as u16, &line, hex_area.width);
            }

            // Scroll progress on the right border, like the other panels.
            let total_rows = self.len.div_ceil(bpr) as usize;
            let viewport = hex_area.height as usize;
            if let Some(border_x) = ctx.border_right_x {
                if ScrollBar::needs_scrollbar(viewport, total_rows) {
                    ScrollBar::render(
                        buf,
                        border_x,
                        hex_area.y,
                        hex_area.height,
                        (self.top_byte / bpr) as usize,
                        viewport,
                        total_rows,
                        &self.theme,
                        ctx.is_focused,
                    );
                }
            }
        }
    }

    fn status_segments(&self) -> Vec<StatusSegment> {
        if self.error.is_some() {
            return vec![];
        }
        let mut segs = vec![StatusSegment::new(
            format!(" {:#X} / {:#X} ", self.cursor, self.len),
            SegmentKind::Value,
        )];
        if let Some(a) = self.anchor {
            let n = a.max(self.cursor) - a.min(self.cursor) + 1;
            segs.push(StatusSegment::new(
                format!("({n} sel) "),
                SegmentKind::Label,
            ));
        }
        segs.push(StatusSegment::new("· ", SegmentKind::Label));
        // Cycle-on-click chip: shows the current view; clicking toggles it.
        let label = match self.mode {
            ViewMode::Hex => "[Hex]",
            ViewMode::Text => "[Text]",
        };
        segs.push(StatusSegment::clickable(
            label,
            SegmentKind::Active,
            "toggle",
        ));
        // RO = read-only; EDIT = editable (with `*` while there are unsaved edits).
        if self.editable {
            let edit = if self.is_modified() {
                " · EDIT*"
            } else {
                " · EDIT "
            };
            segs.push(StatusSegment::new(edit, SegmentKind::Warn));
        } else {
            segs.push(StatusSegment::new(" · RO ", SegmentKind::Label));
        }
        segs
    }

    fn handle_status_action(&mut self, action: &str) -> Vec<PanelEvent> {
        if action == "toggle" {
            return self.toggle_view();
        }
        vec![]
    }

    fn handle_key(&mut self, chord: KeyChord) -> Vec<PanelEvent> {
        let key = chord.raw;

        // While the find bar is open it owns input (Esc / Ctrl+F close it).
        if self.find_bar.is_some() {
            if key.code == KeyCode::Char('f') && key.modifiers == KeyModifiers::CONTROL {
                self.close_find();
                return vec![PanelEvent::NeedsRedraw];
            }
            let action = self.find_bar.as_mut().unwrap().handle_key(key);
            return match action {
                Some(a) => self.handle_find_action(a),
                None => vec![PanelEvent::NeedsRedraw],
            };
        }
        if key.code == KeyCode::Char('f') && key.modifiers == KeyModifiers::CONTROL {
            self.open_find();
            return vec![PanelEvent::NeedsRedraw];
        }

        if self.hotkeys.matches("toggle_hex", &key) {
            return self.toggle_view();
        }
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            return self.copy_selection();
        }

        // Edit mode: Ctrl+S asks to save; typed hex digits / chars overwrite
        // (handled before navigation so letters aren't treated as motions).
        if self.editable {
            if key.code == KeyCode::Char('s') && key.modifiers == KeyModifiers::CONTROL {
                if self.is_modified() {
                    let name = self.title.clone();
                    return vec![PanelEvent::ShowConfirm {
                        message: format!("Save {name}? A .bak backup will be created."),
                        on_confirm: termide_core::ConfirmAction::SaveBinary,
                    }];
                }
                return vec![];
            }
            if self.try_edit(key) {
                return vec![PanelEvent::NeedsRedraw];
            }
        }

        let cols = self.cols() as i64;
        let page = ((self.last_area.height as i64 - 1).max(1)) * cols;
        let extend = key.modifiers.contains(KeyModifiers::SHIFT);
        let ro = !self.editable; // vim-letter motions only when not editing
        match key.code {
            KeyCode::Tab => {
                self.zone = match self.zone {
                    Zone::Hex => Zone::Ascii,
                    Zone::Ascii => Zone::Hex,
                };
            }
            KeyCode::Left => self.move_cursor(-1, extend),
            KeyCode::Right => self.move_cursor(1, extend),
            KeyCode::Up => self.move_cursor(-cols, extend),
            KeyCode::Down => self.move_cursor(cols, extend),
            KeyCode::PageUp => self.move_cursor(-page, extend),
            KeyCode::PageDown => self.move_cursor(page, extend),
            KeyCode::Home => self.set_cursor(self.cursor - self.cursor % cols as u64, extend),
            KeyCode::End => {
                let row_end = self.cursor - self.cursor % cols as u64 + cols as u64 - 1;
                self.set_cursor(row_end, extend)
            }
            KeyCode::Char('h') if ro => self.move_cursor(-1, extend),
            KeyCode::Char('l') if ro => self.move_cursor(1, extend),
            KeyCode::Char('k') if ro => self.move_cursor(-cols, extend),
            KeyCode::Char('j') if ro => self.move_cursor(cols, extend),
            KeyCode::Char('g') if ro => self.set_cursor(0, extend),
            KeyCode::Char('G') if ro => self.set_cursor(self.len.saturating_sub(1), extend),
            KeyCode::Char('q') if ro => return vec![PanelEvent::ClosePanel],
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
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((byte, zone)) = self.byte_at_event(&event) {
                    self.zone = zone;
                    self.set_cursor(byte, false); // clears any selection
                    self.drag_from = Some(byte);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(from) = self.drag_from {
                    if let Some((byte, zone)) = self.byte_at_event(&event) {
                        self.zone = zone;
                        self.anchor = Some(from);
                        self.cursor = byte;
                        self.ensure_cursor_visible();
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.drag_from = None,
            _ => return vec![],
        }
        vec![PanelEvent::NeedsRedraw]
    }

    fn captures_escape(&self) -> bool {
        self.find_bar.is_some()
    }

    fn needs_close_confirmation(&self) -> Option<String> {
        if self.editable && self.is_modified() {
            Some("Unsaved changes in the hex editor".to_string())
        } else {
            None
        }
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

    fn panel_with(len: u64, w: u16, h: u16) -> BinaryPanel {
        let mut p = BinaryPanel::new(PathBuf::from("/dev/null")).unwrap();
        p.len = len;
        p.last_area = Rect::new(0, 0, w, h);
        p
    }

    #[test]
    fn bytes_per_row_rounds_to_16_byte_sections() {
        assert_eq!(bytes_per_row(80), 16);
        assert_eq!(bytes_per_row(140), 32);
        assert_eq!(bytes_per_row(10), 16);
    }

    #[test]
    fn hex_row_formats_offset_bytes_and_ascii() {
        let p = panel_with(0, 80, 10);
        let line = p.hex_row(0x10, b"Hi\x00\xff", 16);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("00000010"), "offset: {text:?}");
        assert!(text.contains("48 69 00 ff"), "hex: {text:?}");
        assert!(text.trim_end().ends_with("Hi··"), "ascii: {text:?}");
    }

    #[test]
    fn cursor_moves_and_clamps() {
        let mut p = panel_with(100, 80, 10); // 16 cols
        p.move_cursor(1, false);
        assert_eq!(p.cursor, 1);
        p.move_cursor(16, false);
        assert_eq!(p.cursor, 17);
        p.move_cursor(1000, false);
        assert_eq!(p.cursor, 99);
        p.move_cursor(-1000, false);
        assert_eq!(p.cursor, 0);
    }

    #[test]
    fn shift_movement_builds_selection_range() {
        let mut p = panel_with(100, 80, 10);
        p.set_cursor(10, false);
        assert_eq!(p.anchor, None);
        p.move_cursor(3, true); // extend to 13
        assert_eq!(p.sel_range(), (10, 13));
        p.move_cursor(-1, false); // plain move clears selection
        assert_eq!(p.anchor, None);
    }

    #[test]
    fn click_maps_to_hex_and_ascii_zones() {
        let p = panel_with(100, 80, 10); // 16 cols, ascii_start = 11+48 = 59
                                         // hex byte 2 at col 10 + 2*3 = 16
        assert_eq!(p.byte_at(16, 0), Some((2, Zone::Hex)));
        // ascii byte 2 at col 59 + 2 = 61
        assert_eq!(p.byte_at(61, 0), Some((2, Zone::Ascii)));
        // second visible row, hex byte 0
        assert_eq!(p.byte_at(10, 1), Some((16, Zone::Hex)));
    }

    #[test]
    fn parse_hex_accepts_pairs_rejects_garbage() {
        assert_eq!(parse_hex("ff fe"), Some(vec![0xff, 0xfe]));
        assert_eq!(parse_hex("FFFE"), Some(vec![0xff, 0xfe]));
        assert_eq!(parse_hex("f"), None); // odd
        assert_eq!(parse_hex("zz"), None); // non-hex
        assert_eq!(parse_hex(""), None);
    }

    #[test]
    fn find_all_locates_matches() {
        let hay = b"abXYabXY";
        assert_eq!(find_all(hay, b"XY", false), vec![2, 6]);
        // case-insensitive ASCII
        assert_eq!(find_all(b"AbcaBC", b"abc", true), vec![0, 3]);
        // hex bytes (exact)
        assert_eq!(
            find_all(&[0x00, 0xff, 0x00, 0xff], &[0x00, 0xff], false),
            vec![0, 2]
        );
        assert!(find_all(hay, b"ZZ", false).is_empty());
    }

    #[test]
    fn toggle_view_swaps_text_file_to_editor() {
        // /dev/null reads as empty → treated as text → swaps to the editor.
        let mut p = panel_with(10, 80, 10);
        assert!(matches!(
            p.toggle_view().as_slice(),
            [PanelEvent::SwapActiveToText(_)]
        ));
    }

    #[test]
    fn hex_nibble_editing_overwrites_byte_keeping_length() {
        let mut p = panel_with(100, 80, 10);
        p.editable = true;
        p.zone = Zone::Hex;
        p.cursor = 5;
        p.edit_hex_nibble(0xa); // high nibble
        p.edit_hex_nibble(0x3); // low nibble → 0xa3, advance
        assert_eq!(p.edits.get(&5), Some(&0xa3));
        assert_eq!(p.cursor, 6);
        assert_eq!(p.len, 100, "overwrite never changes length");
    }

    #[test]
    fn ascii_editing_sets_byte_and_advances() {
        use crossterm::event::{KeyEvent, KeyModifiers};
        let mut p = panel_with(100, 80, 10);
        p.editable = true;
        p.zone = Zone::Ascii;
        p.cursor = 2;
        assert!(p.try_edit(KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::NONE)));
        assert_eq!(p.edits.get(&2), Some(&b'Z'));
        assert_eq!(p.cursor, 3);
    }

    #[test]
    fn save_writes_edits_in_place_and_backs_up() {
        use crossterm::event::{KeyEvent, KeyModifiers};
        let path = std::env::temp_dir().join(format!("termide_hex_{}.bin", std::process::id()));
        std::fs::write(&path, b"ABCDE").unwrap();

        let mut p = BinaryPanel::new_editable(path.clone()).unwrap();
        p.zone = Zone::Ascii;
        p.cursor = 1;
        p.try_edit(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE)); // B -> X
        assert!(p.is_modified());
        p.save().unwrap();
        assert!(!p.is_modified());

        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"AXCDE",
            "edit written, length kept"
        );
        let mut bak = path.clone().into_os_string();
        bak.push(".bak");
        let bak = std::path::PathBuf::from(bak);
        assert_eq!(
            std::fs::read(&bak).unwrap(),
            b"ABCDE",
            "backup holds the original"
        );

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&bak).ok();
    }

    #[test]
    fn match_at_detects_current_and_other_matches() {
        let mut p = panel_with(100, 80, 10);
        p.matches = vec![10, 50];
        p.match_len = 3;
        p.match_idx = 1; // current = the match starting at 50
        assert_eq!(p.match_at(11), Some(false)); // inside first match, not current
        assert_eq!(p.match_at(50), Some(true)); // current match
        assert_eq!(p.match_at(52), Some(true));
        assert_eq!(p.match_at(53), None); // past it (len 3: 50,51,52)
        assert_eq!(p.match_at(0), None);
    }
}
