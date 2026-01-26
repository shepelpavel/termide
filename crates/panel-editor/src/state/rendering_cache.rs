//! Rendering cache state for the editor.

use std::collections::HashMap;

use ratatui::style::Color;
use termide_config::Config;
use termide_highlight::{global_highlighter, HighlightCache};
use termide_theme::Theme;

/// Cached wrap data for a single line.
#[derive(Clone, Debug)]
pub struct CachedWrapData {
    /// Number of visual rows this line occupies.
    pub visual_rows: usize,
    /// Grapheme indices where each new visual line starts.
    pub wrap_points: Vec<usize>,
}

/// Cached rendering state for the editor.
pub(crate) struct RenderingCache {
    /// Syntax highlighting cache.
    pub highlight: HighlightCache,
    /// Cached count of virtual lines (buffer lines + deletion markers).
    pub virtual_line_count: usize,
    /// Cached content width from last render.
    pub content_width: usize,
    /// Cached smart wrap setting from last render.
    pub use_smart_wrap: bool,
    /// Cache of wrap points for each line: line_index -> (visual_rows, wrap_points).
    wrap_cache: HashMap<usize, CachedWrapData>,
    /// Cumulative visual row counts: cumulative_rows[i] = total visual rows for lines 0..i.
    /// Used for O(1) lookup of visual row position.
    cumulative_visual_rows: Vec<usize>,
    /// Whether cumulative cache is valid.
    cumulative_valid: bool,
    /// Cached theme for rendering.
    pub theme: Theme,
    /// Cached config for rendering.
    pub config: Config,
}

impl Default for RenderingCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderingCache {
    /// Create new RenderingCache with defaults.
    pub fn new() -> Self {
        let theme = Theme::default();
        Self {
            // Note: is_light_theme and default_fg will be set correctly by prepare_render()
            highlight: HighlightCache::new(global_highlighter(), false, Color::White),
            virtual_line_count: 0,
            content_width: 0,
            use_smart_wrap: false,
            wrap_cache: HashMap::new(),
            cumulative_visual_rows: Vec::new(),
            cumulative_valid: false,
            theme,
            config: Config::default(),
        }
    }

    /// Create RenderingCache with large file optimization.
    pub fn new_large_file() -> Self {
        let theme = Theme::default();
        Self {
            // Note: is_light_theme and default_fg will be set correctly by prepare_render()
            highlight: HighlightCache::new(global_highlighter(), false, Color::White),
            virtual_line_count: 0,
            content_width: 0,
            use_smart_wrap: false,
            wrap_cache: HashMap::new(),
            cumulative_visual_rows: Vec::new(),
            cumulative_valid: false,
            theme,
            config: Config::default(),
        }
    }

    /// Update cached theme and config before render.
    pub fn prepare(&mut self, theme: &Theme, config: &Config) {
        self.theme = *theme;
        self.config = config.clone();
    }

    /// Get cached wrap data for a line, if available.
    pub fn get_wrap_data(&self, line: usize) -> Option<&CachedWrapData> {
        self.wrap_cache.get(&line)
    }

    /// Store wrap data for a line in cache.
    pub fn set_wrap_data(&mut self, line: usize, visual_rows: usize, wrap_points: Vec<usize>) {
        self.wrap_cache.insert(
            line,
            CachedWrapData {
                visual_rows,
                wrap_points,
            },
        );
        // Cumulative cache is no longer valid after individual line update
        self.cumulative_valid = false;
    }

    /// Invalidate wrap cache for a single line.
    pub fn invalidate_wrap_line(&mut self, line: usize) {
        self.wrap_cache.remove(&line);
        self.cumulative_valid = false;
    }

    /// Invalidate wrap cache for a range of lines (from start_line to end).
    pub fn invalidate_wrap_range(&mut self, start_line: usize) {
        self.wrap_cache.retain(|&line, _| line < start_line);
        self.cumulative_valid = false;
    }

    /// Invalidate entire wrap cache (e.g., when content width changes).
    pub fn invalidate_wrap_cache(&mut self) {
        self.wrap_cache.clear();
        self.cumulative_visual_rows.clear();
        self.cumulative_valid = false;
    }

    /// Check if wrap settings match current parameters (for cache invalidation on settings change).
    pub fn wrap_settings_match(&self, content_width: usize, use_smart_wrap: bool) -> bool {
        self.content_width == content_width && self.use_smart_wrap == use_smart_wrap
    }

    /// Update wrap settings and invalidate cache if they changed.
    pub fn update_wrap_settings(&mut self, content_width: usize, use_smart_wrap: bool) {
        if !self.wrap_settings_match(content_width, use_smart_wrap) {
            self.invalidate_wrap_cache();
            self.content_width = content_width;
            self.use_smart_wrap = use_smart_wrap;
        }
    }

    // =========================================================================
    // Cumulative Visual Rows Cache
    // =========================================================================

    /// Build cumulative visual rows cache for the entire buffer.
    ///
    /// After calling this, `cumulative_visual_rows[i]` contains the total
    /// number of visual rows for lines 0..=i.
    ///
    /// This enables O(1) lookup for:
    /// - Total visual rows in the buffer
    /// - Visual row offset for any given buffer line
    pub fn build_cumulative_cache(&mut self, buffer: &termide_buffer::TextBuffer) {
        let line_count = buffer.line_count();
        self.cumulative_visual_rows.clear();
        self.cumulative_visual_rows.reserve(line_count);

        let mut cumulative = 0usize;

        for line_idx in 0..line_count {
            // Get visual rows from wrap cache, or compute and cache if missing
            let visual_rows = if let Some(cached) = self.wrap_cache.get(&line_idx) {
                cached.visual_rows
            } else if let Some(line_text) = buffer.line(line_idx) {
                let line_text = line_text.trim_end_matches('\n');
                let (visual_rows, wrap_points) = crate::word_wrap::get_line_wrap_points(
                    line_text,
                    self.content_width,
                    self.use_smart_wrap,
                );
                self.wrap_cache.insert(
                    line_idx,
                    CachedWrapData {
                        visual_rows,
                        wrap_points,
                    },
                );
                visual_rows
            } else {
                1 // Empty/non-existent line = 1 visual row
            };

            cumulative += visual_rows;
            self.cumulative_visual_rows.push(cumulative);
        }

        self.cumulative_valid = true;
    }

    /// Get total visual rows in the buffer (O(1) if cumulative cache is valid).
    ///
    /// Returns `None` if the cumulative cache is not valid.
    pub fn get_total_visual_rows(&self) -> Option<usize> {
        if self.cumulative_valid && !self.cumulative_visual_rows.is_empty() {
            Some(*self.cumulative_visual_rows.last().unwrap())
        } else {
            None
        }
    }

    /// Get the cumulative visual row count up to and including a given line.
    ///
    /// Returns the total number of visual rows for lines 0..=line.
    /// Returns `None` if the cumulative cache is not valid or line is out of bounds.
    pub fn get_cumulative_visual_rows(&self, line: usize) -> Option<usize> {
        if self.cumulative_valid && line < self.cumulative_visual_rows.len() {
            Some(self.cumulative_visual_rows[line])
        } else {
            None
        }
    }

    /// Get the starting visual row for a given buffer line (O(1) lookup).
    ///
    /// Returns the visual row index where the given buffer line starts.
    /// This is cumulative_visual_rows[line-1] for line > 0, or 0 for line 0.
    pub fn get_visual_row_for_line(&self, line: usize) -> Option<usize> {
        if !self.cumulative_valid {
            return None;
        }

        if line == 0 {
            Some(0)
        } else if line <= self.cumulative_visual_rows.len() {
            Some(self.cumulative_visual_rows[line - 1])
        } else {
            None
        }
    }

    /// Check if cumulative cache is valid.
    pub fn is_cumulative_valid(&self) -> bool {
        self.cumulative_valid
    }
}
