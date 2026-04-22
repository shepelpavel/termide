//! Log-specific syntax highlighting.
//!
//! Provides highlighting for log entries based on their level (DEBUG, INFO, WARN, ERROR).

use ratatui::style::{Color, Modifier, Style};
use std::borrow::Cow;
use std::collections::HashMap;

use termide_highlight::LineHighlighter;
use termide_theme::Theme;

/// Maximum cache size for log highlighting
const MAX_CACHE_SIZE: usize = 500;

/// Log highlighter that colors lines based on log level.
///
/// Parses log format: `[HH:MM:SS] LEVEL message`
pub struct LogHighlightCache {
    /// Cached highlighted segments: line_idx -> (segments, access_time)
    #[allow(clippy::type_complexity)]
    lines: HashMap<usize, (Vec<(Cow<'static, str>, Style)>, u64)>,
    /// Access counter for LRU eviction
    access_counter: u64,
    /// Theme for styling
    theme: Theme,
}

impl LogHighlightCache {
    /// Create a new log highlight cache with the given theme.
    pub fn new(theme: Theme) -> Self {
        Self {
            lines: HashMap::new(),
            access_counter: 0,
            theme,
        }
    }

    /// Update theme (invalidates cache).
    pub fn set_theme(&mut self, theme: Theme) {
        if self.theme != theme {
            self.theme = theme;
            self.lines.clear();
        }
    }

    /// Compute highlighting segments for a log line.
    fn compute_line_segments(&self, line_text: &str) -> Vec<(Cow<'static, str>, Style)> {
        // Parse log format: "[HH:MM:SS] LEVEL message"
        // Or continuation lines (start with spaces)

        if line_text.is_empty() {
            return vec![(Cow::Borrowed(""), Style::default())];
        }

        // Check if this is a timestamp line: starts with '['
        if line_text.starts_with('[') {
            self.parse_log_line(line_text)
        } else {
            // Continuation line or plain text - use default style
            vec![(
                Cow::Owned(line_text.to_string()),
                Style::default().fg(self.theme.fg),
            )]
        }
    }

    /// Parse a log line with timestamp and level.
    fn parse_log_line(&self, line_text: &str) -> Vec<(Cow<'static, str>, Style)> {
        let mut segments = Vec::new();

        // Find the closing bracket of timestamp
        let Some(bracket_end) = line_text.find(']') else {
            return vec![(
                Cow::Owned(line_text.to_string()),
                Style::default().fg(self.theme.fg),
            )];
        };

        // Timestamp: [HH:MM:SS]
        let timestamp = &line_text[..=bracket_end];
        let rest = &line_text[bracket_end + 1..];

        let timestamp_style = Style::default().fg(Color::DarkGray);
        segments.push((Cow::Owned(timestamp.to_string()), timestamp_style));

        // Skip space after timestamp
        let rest = rest.trim_start();
        if rest.is_empty() {
            return segments;
        }

        // Determine log level and style for the rest of the line
        // Note: level_text must match EXACTLY what strip_prefix removes (no extra spaces)
        let (level_text, message, level_style) = if let Some(msg) = rest.strip_prefix("TRACE") {
            ("TRACE", msg, Style::default().fg(Color::DarkGray))
        } else if let Some(msg) = rest.strip_prefix("DEBUG") {
            ("DEBUG", msg, Style::default().fg(Color::DarkGray))
        } else if let Some(msg) = rest.strip_prefix("INFO ") {
            ("INFO ", msg, Style::default().fg(self.theme.fg))
        } else if let Some(msg) = rest.strip_prefix("INFO") {
            ("INFO", msg, Style::default().fg(self.theme.fg))
        } else if let Some(msg) = rest.strip_prefix("WARN ") {
            (
                "WARN ",
                msg,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else if let Some(msg) = rest.strip_prefix("WARN") {
            (
                "WARN",
                msg,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else if let Some(msg) = rest.strip_prefix("ERROR") {
            (
                "ERROR",
                msg,
                Style::default()
                    .fg(self.theme.error)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            // Unknown format - return as plain text
            segments.push((Cow::Borrowed(" "), Style::default()));
            segments.push((
                Cow::Owned(rest.to_string()),
                Style::default().fg(self.theme.fg),
            ));
            return segments;
        };

        // Add space before level
        segments.push((Cow::Borrowed(" "), Style::default()));

        // Add level with its style (static str, no allocation)
        segments.push((Cow::Borrowed(level_text), level_style));

        // Add message with level's style (for consistency)
        if !message.is_empty() {
            segments.push((Cow::Owned(message.to_string()), level_style));
        }

        segments
    }

    /// Evict oldest entries from cache (LRU).
    ///
    /// Uses a threshold-based approach: remove all entries with access time
    /// below a calculated threshold. This avoids O(n log n) sorting.
    fn evict_lru(&mut self) {
        // Calculate threshold: keep entries accessed in the recent 80% of accesses
        let threshold = self
            .access_counter
            .saturating_sub(MAX_CACHE_SIZE as u64 * 4 / 5);

        // Use retain for efficient in-place removal
        self.lines.retain(|_, (_, access)| *access > threshold);
    }
}

impl LineHighlighter for LogHighlightCache {
    fn get_line_segments<'a>(
        &'a mut self,
        line_idx: usize,
        line_text: &'a str,
    ) -> &'a [(Cow<'a, str>, Style)] {
        self.access_counter += 1;

        // Update access time if cached
        if let Some((_, access_time)) = self.lines.get_mut(&line_idx) {
            *access_time = self.access_counter;
        } else {
            // Compute and cache
            let segments = self.compute_line_segments(line_text);

            if self.lines.len() >= MAX_CACHE_SIZE {
                self.evict_lru();
            }

            self.lines.insert(line_idx, (segments, self.access_counter));
        }

        &self
            .lines
            .get(&line_idx)
            .expect("line was just inserted or updated above")
            .0
    }

    fn invalidate_from(&mut self, line: usize) {
        // Use retain for efficient in-place removal (O(n) single pass)
        self.lines.retain(|&idx, _| idx < line);
    }

    fn invalidate_all(&mut self) {
        self.lines.clear();
    }

    fn has_syntax(&self) -> bool {
        // Always return true - we always highlight log levels
        true
    }
}
