//! VT100/ANSI escape sequence parser implementation.
//!
//! This module provides the VtPerformer struct which implements the vte::Perform trait
//! to parse and handle VT100/ANSI escape sequences for terminal emulation.

#![allow(clippy::needless_range_loop)]

use std::io::Write;
use std::sync::{Arc, RwLock};
use vte::{Params, Perform};

use super::{
    csi_handlers::{handle_cursor_movement, handle_private_sequence, handle_sgr},
    Cell, TerminalScreen,
};

/// Batched screen operation to reduce mutex contention.
///
/// Instead of acquiring a lock for each character, we batch operations
/// and apply them all at once with a single lock.
#[derive(Clone)]
pub enum ScreenOp {
    PutChar(char),
    Newline,
    CarriageReturn,
    Backspace,
    Tab,
}

/// VT100 parser and performer.
///
/// Implements the vte::Perform trait to handle ANSI/VT100 escape sequences
/// and update the terminal screen state accordingly.
///
/// Uses batching to reduce lock contention: simple operations (print, execute)
/// are collected in a buffer and applied with a single lock via `flush()`.
pub struct VtPerformer {
    pub screen: Arc<RwLock<TerminalScreen>>,
    pub pending_backslash: bool,
    /// Buffer for batching screen operations
    pub pending_ops: Vec<ScreenOp>,
}

impl VtPerformer {
    /// Apply all pending operations with a single write lock.
    ///
    /// This significantly reduces lock contention when processing
    /// large amounts of terminal output (e.g., from Claude Code).
    pub fn flush(&mut self) {
        if self.pending_ops.is_empty() {
            return;
        }
        if let Ok(mut screen) = self.screen.write() {
            for op in self.pending_ops.drain(..) {
                match op {
                    ScreenOp::PutChar(ch) => screen.put_char(ch),
                    ScreenOp::Newline => screen.newline(),
                    ScreenOp::CarriageReturn => screen.carriage_return(),
                    ScreenOp::Backspace => screen.backspace(),
                    ScreenOp::Tab => screen.tab(),
                }
            }
            screen.dirty = true;
        }
    }
}

impl Perform for VtPerformer {
    fn print(&mut self, ch: char) {
        // Filter control characters that shouldn't be displayed
        // (except printable characters)
        if ch.is_control() && ch != '\t' && ch != '\n' && ch != '\r' {
            return;
        }

        // Handle bash readline markers \[ and \]
        if self.pending_backslash {
            self.pending_backslash = false;
            // If backslash is followed by [ or ], skip both characters
            if ch == '[' || ch == ']' {
                return;
            }
            // Otherwise print deferred backslash and current character
            self.pending_ops.push(ScreenOp::PutChar('\\'));
            self.pending_ops.push(ScreenOp::PutChar(ch));
            return;
        }

        // If we encounter backslash, defer it
        if ch == '\\' {
            self.pending_backslash = true;
            return;
        }

        // Batch the operation instead of acquiring lock immediately
        self.pending_ops.push(ScreenOp::PutChar(ch));
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' | b'\r' | b'\x08' | b'\t' => {
                // Check if we're in sync_output mode
                // If not, apply immediately to prevent race condition with render
                let sync_output = self.screen.read().map(|s| s.sync_output).unwrap_or(false);

                if sync_output {
                    // During sync output, batch operations for efficiency
                    match byte {
                        b'\n' => self.pending_ops.push(ScreenOp::Newline),
                        b'\r' => self.pending_ops.push(ScreenOp::CarriageReturn),
                        b'\x08' => self.pending_ops.push(ScreenOp::Backspace),
                        b'\t' => self.pending_ops.push(ScreenOp::Tab),
                        _ => {}
                    }
                } else {
                    // Outside sync output, apply immediately to keep cursor position accurate
                    // First flush any pending ops
                    self.flush();
                    // Then apply this operation
                    // NOTE: Don't set dirty=true here! CR/LF/BS/TAB are cursor movement,
                    // not content changes. Setting dirty here causes render to show
                    // intermediate states between sync blocks (duplicate prompts bug).
                    if let Ok(mut screen) = self.screen.write() {
                        match byte {
                            b'\n' => screen.newline(),
                            b'\r' => screen.carriage_return(),
                            b'\x08' => screen.backspace(),
                            b'\t' => screen.tab(),
                            _ => {}
                        }
                        // dirty is NOT set - cursor position change without content change
                    }
                }
            }
            b'\x07' => {
                // Bell character - just forward to parent terminal
                print!("\x07");
                let _ = std::io::stdout().flush();
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, c: char) {
        // Single lock acquisition for both flush and CSI dispatch (critical optimization)
        // This eliminates double lock acquisition that occurred on every CSI sequence
        if let Ok(mut screen) = self.screen.write() {
            // Inline flush: apply pending operations with the same lock
            if !self.pending_ops.is_empty() {
                for op in self.pending_ops.drain(..) {
                    match op {
                        ScreenOp::PutChar(ch) => screen.put_char(ch),
                        ScreenOp::Newline => screen.newline(),
                        ScreenOp::CarriageReturn => screen.carriage_return(),
                        ScreenOp::Backspace => screen.backspace(),
                        ScreenOp::Tab => screen.tab(),
                    }
                }
                screen.dirty = true;
            }

            // Handle private sequences (start with '?')
            if !intermediates.is_empty() && intermediates[0] == b'?' {
                handle_private_sequence(&mut screen, params, c);
                return;
            }

            // Ignore other intermediate bytes
            if !intermediates.is_empty() {
                return;
            }
            // Try cursor movement commands first
            if handle_cursor_movement(&mut screen, params, c) {
                screen.dirty = true;
                return;
            }

            match c {
                'J' => {
                    // ED - Erase in Display
                    let param = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(0);
                    let (row, col) = screen.cursor;
                    let empty_cell = Cell {
                        ch: ' ',
                        style: screen.current_style,
                    };

                    match param {
                        0 => {
                            // Clear from cursor to end of screen
                            let buffer = screen.active_buffer_mut();
                            let buf_rows = buffer.len();

                            // Clear rest of current line
                            if row < buf_rows {
                                let buf_cols = buffer[row].len();
                                for i in col..buf_cols {
                                    buffer[row][i] = empty_cell;
                                }
                            }
                            // Clear all lines below
                            for r in (row + 1)..buf_rows {
                                let buf_cols = buffer[r].len();
                                for c in 0..buf_cols {
                                    buffer[r][c] = empty_cell;
                                }
                            }
                            // Force cache invalidation to show cleared content immediately
                            screen.force_cache_invalidation = true;
                        }
                        1 => {
                            // Clear from start of screen to cursor
                            let buffer = screen.active_buffer_mut();
                            let buf_rows = buffer.len();

                            // Clear all lines above
                            for r in 0..row.min(buf_rows) {
                                let buf_cols = buffer[r].len();
                                for c in 0..buf_cols {
                                    buffer[r][c] = empty_cell;
                                }
                            }
                            // Clear current line up to and including cursor
                            if row < buf_rows {
                                let buf_cols = buffer[row].len();
                                for i in 0..=col.min(buf_cols.saturating_sub(1)) {
                                    buffer[row][i] = empty_cell;
                                }
                            }
                            // Force cache invalidation to show cleared content immediately
                            screen.force_cache_invalidation = true;
                        }
                        2 => {
                            // Clear entire screen and move cursor to (0,0)
                            let buffer = screen.active_buffer_mut();
                            for row in buffer.iter_mut() {
                                row.fill(empty_cell);
                            }
                            // Move cursor to home position (compatibility with old behavior)
                            screen.cursor = (0, 0);
                            // Force cache invalidation to show cleared screen immediately
                            screen.force_cache_invalidation = true;
                        }
                        3 => {
                            // Clear entire screen and scrollback
                            let is_alt = screen.use_alt_screen;
                            let buffer = screen.active_buffer_mut();
                            for row in buffer.iter_mut() {
                                row.fill(empty_cell);
                            }
                            // Clear scrollback only for main screen
                            if !is_alt {
                                screen.scrollback.clear();
                            }
                            screen.cursor = (0, 0);
                            // Force cache invalidation to show cleared screen immediately
                            screen.force_cache_invalidation = true;
                        }
                        _ => {}
                    }
                }
                'K' => {
                    // EL - Erase in Line
                    let param = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(0);
                    let (row, col) = screen.cursor;
                    let empty_cell = Cell {
                        ch: ' ',
                        style: screen.current_style,
                    };

                    let buffer = screen.active_buffer_mut();
                    if row < buffer.len() {
                        let buf_cols = buffer[row].len();
                        match param {
                            0 => {
                                // From cursor to end of line
                                for i in col..buf_cols {
                                    buffer[row][i] = empty_cell;
                                }
                            }
                            1 => {
                                // From start of line to cursor (inclusive)
                                for i in 0..=col.min(buf_cols.saturating_sub(1)) {
                                    buffer[row][i] = empty_cell;
                                }
                            }
                            2 => {
                                // Entire line
                                for i in 0..buf_cols {
                                    buffer[row][i] = empty_cell;
                                }
                            }
                            _ => {}
                        }
                        // Force cache invalidation to show erased content immediately
                        screen.force_cache_invalidation = true;
                    }
                }
                'P' => {
                    // DCH - Delete Character
                    let n = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(1) as usize;
                    let (row, col) = screen.cursor;
                    let cols = screen.cols;
                    let empty_cell = Cell {
                        ch: ' ',
                        style: screen.current_style,
                    };

                    let buffer = screen.active_buffer_mut();
                    // Shift characters left from deleted position using copy_within (3-5x faster)
                    if col + n < cols {
                        buffer[row].copy_within(col + n..cols, col);
                    }

                    // Fill freed space with blanks
                    for i in (cols - n)..cols {
                        buffer[row][i] = empty_cell;
                    }
                    // Force cache invalidation after character deletion
                    screen.force_cache_invalidation = true;
                }
                'X' => {
                    // ECH - Erase Character
                    let n = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(1) as usize;
                    let (row, col) = screen.cursor;
                    let cols = screen.cols;
                    let empty_cell = Cell {
                        ch: ' ',
                        style: screen.current_style,
                    };

                    let buffer = screen.active_buffer_mut();
                    for i in col..(col + n).min(cols) {
                        buffer[row][i] = empty_cell;
                    }
                    // Force cache invalidation after character erasure
                    screen.force_cache_invalidation = true;
                }
                '@' => {
                    // ICH - Insert Character (shift characters right)
                    let n = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(1) as usize;
                    let (row, col) = screen.cursor;
                    let cols = screen.cols;
                    let empty_cell = Cell {
                        ch: ' ',
                        style: screen.current_style,
                    };

                    let buffer = screen.active_buffer_mut();
                    // Shift characters right using copy_within (3-5x faster)
                    if col + n < cols {
                        buffer[row].copy_within(col..cols - n, col + n);
                    }

                    // Insert blanks at freed positions
                    for i in col..(col + n).min(cols) {
                        buffer[row][i] = empty_cell;
                    }
                    // Force cache invalidation after character insertion
                    screen.force_cache_invalidation = true;
                }
                'L' => {
                    // IL - Insert Lines (insert blank lines within scroll region)
                    let n = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(1) as usize;
                    let row = screen.cursor.0;
                    let cols = screen.cols;
                    let bottom = screen.scroll_bottom;
                    let empty_cell = Cell {
                        ch: ' ',
                        style: screen.current_style,
                    };

                    // Only operate if cursor is within scroll region
                    if row <= bottom {
                        let effective_n = n.min(bottom - row + 1);
                        let buffer = screen.active_buffer_mut();

                        // Delete n lines from scroll_bottom
                        for _ in 0..effective_n {
                            if bottom < buffer.len() {
                                buffer.remove(bottom);
                            }
                        }
                        // Insert n blank lines at cursor position
                        for _ in 0..effective_n {
                            if row == 0 {
                                buffer.push_front(vec![empty_cell; cols]);
                            } else {
                                buffer.insert(row, vec![empty_cell; cols]);
                            }
                        }
                    }
                    // Ensure buffer size invariant after IL operation
                    screen.ensure_buffer_size();
                    // Force cache invalidation after line insertion
                    screen.force_cache_invalidation = true;
                }
                'M' => {
                    // DL - Delete Lines (delete lines within scroll region)
                    let n = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(1) as usize;
                    let row = screen.cursor.0;
                    let cols = screen.cols;
                    let bottom = screen.scroll_bottom;
                    let empty_cell = Cell {
                        ch: ' ',
                        style: screen.current_style,
                    };

                    // Only operate if cursor is within scroll region
                    if row <= bottom {
                        let effective_n = n.min(bottom - row + 1);
                        let buffer = screen.active_buffer_mut();

                        // Delete n lines at cursor position
                        for _ in 0..effective_n {
                            if row < buffer.len() {
                                if row == 0 {
                                    buffer.pop_front();
                                } else {
                                    buffer.remove(row);
                                }
                            }
                        }
                        // Insert n blank lines at scroll_bottom
                        for _ in 0..effective_n {
                            let insert_pos = bottom.min(buffer.len());
                            buffer.insert(insert_pos, vec![empty_cell; cols]);
                        }
                    }
                    // Ensure buffer size invariant after DL operation
                    screen.ensure_buffer_size();
                    // Force cache invalidation after line deletion
                    screen.force_cache_invalidation = true;
                }
                'S' => {
                    // SU - Scroll Up (scroll within region)
                    let n = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(1) as usize;
                    let cols = screen.cols;
                    let top = screen.scroll_top;
                    let bottom = screen.scroll_bottom;
                    let empty_cell = Cell {
                        ch: ' ',
                        style: screen.current_style,
                    };

                    let region_size = bottom.saturating_sub(top) + 1;
                    let effective_n = n.min(region_size);

                    // Full-screen scroll (use efficient VecDeque ops)
                    if top == 0 && bottom == screen.rows.saturating_sub(1) {
                        let buffer = screen.active_buffer_mut();
                        for _ in 0..effective_n {
                            if !buffer.is_empty() {
                                buffer.pop_front();
                            }
                            buffer.push_back(vec![empty_cell; cols]);
                        }
                    } else {
                        // Region scroll
                        let buffer = screen.active_buffer_mut();
                        for _ in 0..effective_n {
                            if top < buffer.len() {
                                buffer.remove(top);
                            }
                            let insert_pos = bottom.min(buffer.len());
                            buffer.insert(insert_pos, vec![empty_cell; cols]);
                        }
                    }
                    // Ensure buffer size invariant after SU operation
                    screen.ensure_buffer_size();
                    // Force cache invalidation after scroll up
                    screen.force_cache_invalidation = true;
                }
                'T' => {
                    // SD - Scroll Down (scroll within region)
                    let n = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(1) as usize;
                    let cols = screen.cols;
                    let rows = screen.rows;
                    let top = screen.scroll_top;
                    let bottom = screen.scroll_bottom;
                    let empty_cell = Cell {
                        ch: ' ',
                        style: screen.current_style,
                    };

                    let region_size = bottom.saturating_sub(top) + 1;
                    let effective_n = n.min(region_size);

                    // Full-screen scroll (use efficient VecDeque ops)
                    if top == 0 && bottom == rows.saturating_sub(1) {
                        let buffer = screen.active_buffer_mut();
                        for _ in 0..effective_n {
                            if buffer.len() >= rows {
                                buffer.pop_back();
                            }
                            buffer.push_front(vec![empty_cell; cols]);
                        }
                    } else {
                        // Region scroll
                        let buffer = screen.active_buffer_mut();
                        for _ in 0..effective_n {
                            if bottom < buffer.len() {
                                buffer.remove(bottom);
                            }
                            buffer.insert(top, vec![empty_cell; cols]);
                        }
                    }
                    // Ensure buffer size invariant after SD operation
                    screen.ensure_buffer_size();
                    // Force cache invalidation after scroll down
                    screen.force_cache_invalidation = true;
                }
                'm' => {
                    // SGR - set style (colors, bold, etc.)
                    handle_sgr(&mut screen, params);
                }
                's' => {
                    // Save cursor position
                    screen.save_cursor();
                }
                'u' => {
                    // Restore cursor position
                    screen.restore_cursor();
                }
                'r' => {
                    // DECSTBM - Set Top and Bottom Margins
                    let mut iter = params.iter();
                    let top = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                    let bottom = iter
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .map(|b| b as usize)
                        .unwrap_or(screen.rows);
                    screen.set_scroll_region(top, bottom);
                }
                'l' | 'h' => {
                    // Set/Reset Mode (ignore but don't break)
                }
                _ => {}
            }
            screen.dirty = true;
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        self.flush();
        if let Ok(mut screen) = self.screen.write() {
            match byte {
                b'D' => {
                    // IND - Index: move down, scroll at bottom margin
                    if screen.cursor.0 >= screen.scroll_bottom {
                        screen.scroll_up();
                    } else {
                        screen.cursor.0 += 1;
                    }
                }
                b'M' => {
                    // RI - Reverse Index: move up, scroll at top margin
                    if screen.cursor.0 <= screen.scroll_top {
                        screen.scroll_down_region();
                    } else {
                        screen.cursor.0 -= 1;
                    }
                }
                _ => {}
            }
            screen.dirty = true;
        }
    }
}
