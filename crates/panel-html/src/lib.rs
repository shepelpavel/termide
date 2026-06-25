//! Rendered HTML preview panel (read-only).
//!
//! Opened for `.html`/`.htm` files via `F3` (view), this panel renders the
//! document as text pseudographics through the shared HTML engine (see
//! [`termide_html`]). `Enter`/`F4` open the source in the editor instead.
//!
//! The preview is interactive: a movable cursor, keyboard/mouse text selection
//! with copy-to-clipboard, clickable links and image placeholders (open in the
//! browser), incremental search (`Ctrl+F`), and vertical scrolling with line
//! wrapping. The configured toggle-view hotkey (or the `Edit` status chip) swaps
//! the panel in place for the editable source.

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use unicode_width::UnicodeWidthChar;

use termide_core::{
    Config, HotkeyTable, InputAction, KeyChord, LinkOpen, Panel, PanelEvent, RenderContext,
    SegmentKind, SessionPanel, StatusSegment, Theme, ThemeColors, WidthPreference,
};
use termide_html::render_html;
use termide_modal::{FindBar, FindBarAction, FindBarBtn, FindBarConfig, FindField};
use termide_richtext::{LinkSpan, Rendered};
use termide_ui::ScrollBar;

/// A cursor / selection position: `(line index, character column)`.
type Pos = (usize, usize);

/// A search match: `(line index, character column)`.
type Match = (usize, usize);

/// Rendered HTML viewer.
pub struct HtmlPanel {
    /// Path to the HTML file.
    file_path: PathBuf,
    /// Display title (filename).
    title: String,
    /// Raw HTML source.
    source: String,
    /// Error message if the file could not be read.
    error: Option<String>,

    /// Rendered output for `layout_width`; rebuilt when width or content changes.
    doc: Rendered,
    /// Width the current `doc` was laid out for.
    layout_width: u16,
    /// First visible line index (scroll offset).
    top: usize,
    /// Content area from the last render (for click mapping + paging).
    last_area: Rect,
    /// Cursor position (character column within the line).
    cursor: Pos,
    /// Selection anchor; `Some` while a selection is active.
    anchor: Option<Pos>,
    /// Origin of an in-progress mouse drag-selection.
    drag_from: Option<Pos>,

    /// Inline find bar, when open.
    find_bar: Option<FindBar>,
    /// Search matches (start of each occurrence).
    matches: Vec<Match>,
    /// Character length of the current search needle.
    match_len: usize,
    /// Index of the current match within `matches`.
    match_idx: usize,

    /// Cached theme colors.
    colors: ThemeColors,
    /// Full theme, cached for rendering the find bar.
    theme_full: Option<Theme>,
    /// Whether the active theme is light (for code highlighting).
    is_light: bool,
    /// Configurable hotkeys (toggle preview/source).
    hotkeys: HotkeyTable,
    /// Pointer of the last `Arc<Config>` used to build hotkeys.
    last_config_ptr: usize,
    /// Origin URL when the content was fetched (not read from a file). `None`
    /// for file-backed viewers. Used for the title, base-URL link resolution,
    /// and navigation; URL-backed viewers are not persisted across sessions.
    source_url: Option<String>,
    /// Browsing history (URLs) for an in-panel navigated viewer.
    history: Vec<String>,
    /// Current position within `history`.
    hist_idx: usize,
    /// Where a followed link opens by default (from config).
    open_links: LinkOpen,
}

impl HtmlPanel {
    /// A blank viewer with no content (file_path empty); fill via `set_file`
    /// or `from_source`.
    fn empty() -> Self {
        Self {
            file_path: PathBuf::new(),
            title: String::new(),
            source: String::new(),
            error: None,
            doc: Rendered {
                lines: Vec::new(),
                links: Vec::new(),
            },
            layout_width: 0,
            top: 0,
            last_area: Rect::default(),
            cursor: (0, 0),
            anchor: None,
            drag_from: None,
            find_bar: None,
            matches: Vec::new(),
            match_len: 0,
            match_idx: 0,
            colors: ThemeColors::default(),
            theme_full: None,
            is_light: false,
            hotkeys: HotkeyTable::default(),
            last_config_ptr: 0,
            source_url: None,
            history: Vec::new(),
            hist_idx: 0,
            open_links: LinkOpen::default(),
        }
    }

    /// Open an HTML file in the preview panel.
    pub fn new(path: PathBuf) -> anyhow::Result<Self> {
        let mut panel = Self::empty();
        panel.set_file(path);
        Ok(panel)
    }

    /// Build a viewer over in-memory `source` (e.g. content fetched over HTTP),
    /// with `source_url` as its origin. Not read from, nor written to, disk.
    pub fn from_source(title: String, source: String, source_url: Option<String>) -> Self {
        let mut panel = Self::empty();
        panel.title = title;
        panel.source = source;
        if let Some(url) = &source_url {
            panel.history = vec![url.clone()];
            panel.hist_idx = 0;
        }
        panel.source_url = source_url;
        panel
    }

    /// Replace the content in place with a navigated document (link/history
    /// step). History is managed by the caller's navigation, not here.
    pub fn apply_fetched(&mut self, title: String, source: String, final_url: String) {
        self.title = title;
        self.source = source;
        self.source_url = Some(final_url);
        self.top = 0;
        self.cursor = (0, 0);
        self.anchor = None;
        self.matches.clear();
        self.layout_width = 0;
    }

    /// Resolve a link `href` against the current document URL (no-op for
    /// file-backed viewers or absolute links).
    fn resolve(&self, href: &str) -> String {
        if let Some(base) = &self.source_url {
            if let Ok(b) = url::Url::parse(base) {
                if let Ok(joined) = b.join(href) {
                    return joined.to_string();
                }
            }
        }
        href.to_string()
    }

    /// Follow a link. Non-web targets go to the external opener. Web links
    /// honor the `open_links` setting: `External` → browser; `Panel` →
    /// in-place navigation in a fetched view, or a new viewer otherwise.
    fn activate_link(&mut self, href: &str) -> Vec<PanelEvent> {
        let target = self.resolve(href);
        let is_web = target.starts_with("http://") || target.starts_with("https://");
        if !is_web || self.open_links == LinkOpen::External {
            return vec![PanelEvent::OpenExternal(PathBuf::from(target))];
        }
        if self.source_url.is_some() {
            self.history.truncate(self.hist_idx + 1);
            self.history.push(target.clone());
            self.hist_idx = self.history.len() - 1;
            vec![PanelEvent::NavigateUrl(target)]
        } else {
            vec![PanelEvent::OpenUrl(target)]
        }
    }

    /// Step back in history, re-fetching the previous page.
    fn go_back(&mut self) -> Vec<PanelEvent> {
        if self.hist_idx > 0 {
            self.hist_idx -= 1;
            return vec![PanelEvent::NavigateUrl(self.history[self.hist_idx].clone())];
        }
        vec![]
    }

    /// Step forward in history, re-fetching the next page.
    fn go_forward(&mut self) -> Vec<PanelEvent> {
        if self.hist_idx + 1 < self.history.len() {
            self.hist_idx += 1;
            return vec![PanelEvent::NavigateUrl(self.history[self.hist_idx].clone())];
        }
        vec![]
    }

    /// Point the panel at a new file, reloading its content.
    pub fn set_file(&mut self, path: PathBuf) {
        self.title = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        self.file_path = path;
        self.top = 0;
        self.cursor = (0, 0);
        self.anchor = None;
        self.matches.clear();
        match std::fs::read_to_string(&self.file_path) {
            Ok(s) => {
                self.source = s;
                self.error = None;
            }
            Err(e) => {
                self.source = String::new();
                self.error = Some(e.to_string());
            }
        }
        self.layout_width = 0; // force re-layout
    }

    fn line_count(&self) -> usize {
        self.doc.lines.len()
    }

    /// Plain text of a rendered line (concatenated span contents).
    fn line_text(&self, i: usize) -> String {
        self.doc
            .lines
            .get(i)
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .unwrap_or_default()
    }

    /// Character count of a rendered line.
    fn line_len(&self, i: usize) -> usize {
        self.line_text(i).chars().count()
    }

    fn viewport_height(&self) -> usize {
        self.last_area.height.max(1) as usize
    }

    fn max_top(&self) -> usize {
        self.line_count().saturating_sub(self.viewport_height())
    }

    fn relayout_if_needed(&mut self, width: u16) {
        if width == self.layout_width || width == 0 {
            return;
        }
        self.doc = render_html(&self.source, width, &self.colors, self.is_light);
        self.layout_width = width;
        self.clamp_cursor();
        self.run_search(); // re-locate matches against the new wrapping
        self.top = self.top.min(self.max_top());
    }

    fn clamp_cursor(&mut self) {
        let lines = self.line_count();
        if lines == 0 {
            self.cursor = (0, 0);
            return;
        }
        self.cursor.0 = self.cursor.0.min(lines - 1);
        self.cursor.1 = self.cursor.1.min(self.line_len(self.cursor.0));
    }

    fn scroll_by(&mut self, delta: i32) {
        let max = self.max_top();
        self.top = (self.top as i64 + delta as i64).clamp(0, max as i64) as usize;
    }

    /// Keep the cursor line within the viewport.
    fn ensure_cursor_visible(&mut self) {
        let h = self.viewport_height();
        if self.cursor.0 < self.top {
            self.top = self.cursor.0;
        } else if self.cursor.0 >= self.top + h {
            self.top = self.cursor.0 + 1 - h;
        }
        self.top = self.top.min(self.max_top());
    }

    /// Move the cursor, optionally extending the selection.
    fn move_cursor(&mut self, to: Pos, extend: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = to;
        self.clamp_cursor();
        self.ensure_cursor_visible();
    }

    fn move_vertical(&mut self, delta: i32, extend: bool) {
        let lines = self.line_count();
        if lines == 0 {
            return;
        }
        let line = (self.cursor.0 as i64 + delta as i64).clamp(0, lines as i64 - 1) as usize;
        let col = self.cursor.1.min(self.line_len(line));
        self.move_cursor((line, col), extend);
    }

    fn move_horizontal(&mut self, forward: bool, extend: bool) {
        let (mut line, mut col) = self.cursor;
        if forward {
            if col < self.line_len(line) {
                col += 1;
            } else if line + 1 < self.line_count() {
                line += 1;
                col = 0;
            }
        } else if col > 0 {
            col -= 1;
        } else if line > 0 {
            line -= 1;
            col = self.line_len(line);
        }
        self.move_cursor((line, col), extend);
    }

    /// Normalized selection range `(start, end)` with `start <= end`.
    fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        let b = self.cursor;
        if a <= b {
            Some((a, b))
        } else {
            Some((b, a))
        }
    }

    /// Text of the current selection, or the cursor's line when none.
    fn selected_text(&self) -> String {
        let Some((start, end)) = self.selection() else {
            return self.line_text(self.cursor.0);
        };
        if start.0 == end.0 {
            return slice_chars(&self.line_text(start.0), start.1, end.1);
        }
        let mut out = String::new();
        for line in start.0..=end.0 {
            let text = self.line_text(line);
            let part = if line == start.0 {
                slice_chars(&text, start.1, text.chars().count())
            } else if line == end.0 {
                slice_chars(&text, 0, end.1)
            } else {
                text
            };
            out.push_str(&part);
            if line != end.0 {
                out.push('\n');
            }
        }
        out
    }

    /// The link under display column `col` on the given rendered line.
    fn link_at(&self, line: usize, col: u16) -> Option<&LinkSpan> {
        self.doc
            .links
            .iter()
            .find(|l| l.line == line && col >= l.start && col < l.end)
    }

    /// The link under the cursor (cursor column is a char index; compare in
    /// display columns, which match for the common ASCII case).
    fn link_under_cursor(&self) -> Option<&LinkSpan> {
        let (line, col) = self.cursor;
        let disp = char_col_to_display(&self.line_text(line), col);
        self.link_at(line, disp as u16)
    }

    // --- search ---------------------------------------------------------

    fn open_find(&mut self) {
        let mut bar = FindBar::new(FindBarConfig {
            fields: vec![FindField::Find],
            // Same button order as the editor: [Aa] Case, ◄ Prev, Next ►.
            buttons: vec![FindBarBtn::Case, FindBarBtn::Prev, FindBarBtn::Next],
        });
        // Seed the Find field from a single-line selection (the common
        // "Ctrl+F searches the current selection" behavior).
        let seed = self.selection().and_then(|(s, e)| {
            (s.0 == e.0 && s != e).then(|| slice_chars(&self.line_text(s.0), s.1, e.1))
        });
        if let Some(text) = seed {
            bar.set_text(FindField::Find, text);
        }
        bar.focus_first();
        self.find_bar = Some(bar);
        self.matches.clear();
        self.match_idx = 0;
        self.run_search();
    }

    fn close_find(&mut self) {
        self.find_bar = None;
        self.matches.clear();
    }

    /// Re-run the search and jump to the first match at/after the cursor.
    fn run_search(&mut self) {
        let Some(bar) = self.find_bar.as_ref() else {
            return;
        };
        let query = bar.find_text().to_string();
        let ci = !bar.case_sensitive();
        self.matches.clear();
        self.match_idx = 0;
        if query.is_empty() {
            if let Some(bar) = self.find_bar.as_mut() {
                bar.clear_match_info();
            }
            return;
        }
        self.match_len = query.chars().count();
        for line in 0..self.line_count() {
            for col in find_in_line(&self.line_text(line), &query, ci) {
                self.matches.push((line, col));
            }
        }
        // Prefer the first match at/after the current cursor.
        if let Some(idx) = self.matches.iter().position(|&m| m >= self.cursor) {
            self.match_idx = idx;
        }
        if let Some(bar) = self.find_bar.as_mut() {
            if self.matches.is_empty() {
                bar.set_match_info(0, 0);
            } else {
                bar.set_match_info(self.match_idx + 1, self.matches.len());
            }
        }
        self.jump_to_current_match();
    }

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
        if let Some(bar) = self.find_bar.as_mut() {
            bar.set_match_info(self.match_idx + 1, n);
        }
        self.jump_to_current_match();
    }

    fn jump_to_current_match(&mut self) {
        if let Some(&(line, col)) = self.matches.get(self.match_idx) {
            self.anchor = None;
            self.cursor = (line, col);
            self.clamp_cursor();
            self.ensure_cursor_visible();
        }
    }

    /// Build the "go to path" input request, seeded with this file's directory
    /// so relative entries resolve naturally.
    fn goto_path_event(&self) -> PanelEvent {
        let base = self
            .file_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let mut initial = base.display().to_string();
        if !initial.is_empty() {
            initial.push('/');
        }
        PanelEvent::ShowInput {
            prompt: "Go to path".to_string(),
            initial_value: initial,
            on_submit: InputAction::ViewPath { base_dir: base },
        }
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
}

impl Panel for HtmlPanel {
    fn name(&self) -> &'static str {
        "html"
    }

    fn width_preference(&self) -> WidthPreference {
        WidthPreference::PreferWide
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn prepare_render(&mut self, theme: &Theme, config: &Arc<Config>) {
        let new_light = theme.is_light_theme();
        if new_light != self.is_light {
            self.layout_width = 0;
        }
        self.colors = ThemeColors::from(theme);
        self.theme_full = Some(*theme);
        self.is_light = new_light;

        let ptr = Arc::as_ptr(config) as usize;
        if self.last_config_ptr != ptr {
            self.last_config_ptr = ptr;
            let mut t = HotkeyTable::new();
            t.insert("toggle_view", &config.viewer.keybindings.toggle_view);
            self.hotkeys = t;
            self.open_links = config.viewer.open_links;
        }
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        buf.set_style(area, Style::default().fg(self.colors.fg).bg(self.colors.bg));

        // Find bar docked at the TOP with a separator below (like the editor).
        let mut content = area;
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
                let style = Style::default().fg(self.colors.disabled);
                for dx in 0..area.width {
                    buf[(area.x + dx, sep_y)].set_symbol("─").set_style(style);
                }
                used += 1;
            }
            content = Rect {
                x: area.x,
                y: area.y + used,
                width: area.width,
                height: area.height.saturating_sub(used),
            };
        }
        self.last_area = content;

        if let Some(err) = &self.error {
            let msg = ratatui::text::Line::styled(
                format!(" Cannot open: {err}"),
                Style::default().fg(self.colors.error),
            );
            buf.set_line(content.x, content.y, &msg, content.width);
            return;
        }

        // Reserve the rightmost column as a scrollbar gutter so wrapped text
        // never sits under the bar (keeps the layout stable frame-to-frame).
        let text_width = content.width.saturating_sub(1).max(1);
        self.relayout_if_needed(text_width);
        self.top = self.top.min(self.max_top());

        let sel = self.selection();
        for i in 0..(content.height as usize) {
            let line_idx = self.top + i;
            let Some(line) = self.doc.lines.get(line_idx) else {
                break;
            };
            let y = content.y + i as u16;
            buf.set_line(content.x, y, line, text_width);

            let text = self.line_text(line_idx);

            // Search matches.
            let match_style = Style::default().fg(self.colors.bg).bg(self.colors.warning);
            for &(ml, mc) in &self.matches {
                if ml != line_idx {
                    continue;
                }
                let x0 = char_col_to_display(&text, mc) as u16;
                let x1 = char_col_to_display(&text, mc + self.match_len) as u16;
                for dx in x0..x1.max(x0) {
                    if dx < text_width {
                        buf[(content.x + dx, y)].set_style(match_style);
                    }
                }
            }

            // Selection highlight (per display column).
            if let Some((s, e)) = sel {
                if line_idx >= s.0 && line_idx <= e.0 {
                    let c0 = if line_idx == s.0 { s.1 } else { 0 };
                    let c1 = if line_idx == e.0 {
                        e.1
                    } else {
                        text.chars().count()
                    };
                    let style = Style::default()
                        .fg(self.colors.selection_fg)
                        .bg(self.colors.selection_bg);
                    let x0 = char_col_to_display(&text, c0) as u16;
                    let x1 = char_col_to_display(&text, c1) as u16;
                    for dx in x0..x1.max(x0) {
                        if dx < text_width {
                            buf[(content.x + dx, y)].set_style(style);
                        }
                    }
                }
            }
        }

        // Cursor cell (only when focused, on a visible line, and not searching);
        // an unfocused preview shows no cursor.
        if ctx.is_focused
            && self.find_bar.is_none()
            && self.cursor.0 >= self.top
            && self.cursor.0 < self.top + content.height as usize
        {
            let y = content.y + (self.cursor.0 - self.top) as u16;
            let dx = char_col_to_display(&self.line_text(self.cursor.0), self.cursor.1) as u16;
            if dx < text_width {
                let style = Style::default().fg(self.colors.bg).bg(self.colors.cursor);
                buf[(content.x + dx, y)].set_style(style);
            }
        }

        // Vertical scrollbar on the panel's right border (replacing it), not one
        // column inside it — otherwise it reads as detached from the edge.
        ScrollBar::render(
            buf,
            ctx.border_right_x.unwrap_or(content.x + content.width - 1),
            content.y,
            content.height,
            self.top,
            content.height as usize,
            self.line_count(),
            &self.colors,
            ctx.is_focused,
        );
    }

    fn handle_key(&mut self, chord: KeyChord) -> Vec<PanelEvent> {
        let key = chord.raw;

        // While the find bar is open it owns input (Esc / Ctrl+F close it).
        if self.find_bar.is_some() {
            let ctrl_f = key.code == KeyCode::Char('f') && key.modifiers == KeyModifiers::CONTROL;
            if ctrl_f {
                self.close_find();
                return vec![PanelEvent::NeedsRedraw];
            }
            let action = self.find_bar.as_mut().unwrap().handle_key(key);
            return match action {
                Some(a) => self.handle_find_action(a),
                None => vec![PanelEvent::NeedsRedraw],
            };
        }

        // Below the find bar there is no text input, so match shortcuts against
        // the canonical (layout-normalized) key — `[`/`]`, `o`, `g`, … then work
        // regardless of the active keyboard layout (e.g. Cyrillic `х`/`ъ`).
        let key = chord.canonical;

        if self.hotkeys.matches("toggle_view", &key) {
            return vec![PanelEvent::SwapActiveToText(self.file_path.clone())];
        }
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = (self.viewport_height() as i32 - 1).max(1);

        // Ctrl+G: "go to path" — type a path to open it in the right viewer.
        if ctrl && key.code == KeyCode::Char('g') {
            return vec![self.goto_path_event()];
        }

        // Ctrl+R: re-read from disk (pick up external edits), keeping position.
        if ctrl && key.code == KeyCode::Char('r') {
            let (top, cursor) = (self.top, self.cursor);
            let _ = self.reload();
            self.cursor = cursor;
            self.clamp_cursor();
            self.top = top.min(self.max_top());
            return vec![PanelEvent::NeedsRedraw];
        }

        if ctrl && key.code == KeyCode::Char('f') {
            self.open_find();
            return vec![PanelEvent::NeedsRedraw];
        }
        if ctrl && key.code == KeyCode::Char('c') {
            let text = self.selected_text();
            if text.is_empty() {
                return vec![];
            }
            return vec![PanelEvent::CopyToClipboard(text)];
        }

        match key.code {
            // History back/forward in a navigated view.
            KeyCode::Char('[') | KeyCode::Backspace => return self.go_back(),
            KeyCode::Char(']') => return self.go_forward(),
            KeyCode::Up => self.move_vertical(-1, shift),
            KeyCode::Down | KeyCode::Char('j') => self.move_vertical(1, shift),
            KeyCode::Char('k') => self.move_vertical(-1, shift),
            KeyCode::Left | KeyCode::Char('h') => self.move_horizontal(false, shift),
            KeyCode::Right | KeyCode::Char('l') => self.move_horizontal(true, shift),
            KeyCode::PageUp => self.move_vertical(-page, shift),
            KeyCode::PageDown | KeyCode::Char(' ') => self.move_vertical(page, shift),
            KeyCode::Home => self.move_cursor((self.cursor.0, 0), shift),
            KeyCode::End => {
                let end = self.line_len(self.cursor.0);
                self.move_cursor((self.cursor.0, end), shift);
            }
            KeyCode::Char('g') => self.move_cursor((0, 0), shift),
            KeyCode::Char('G') => {
                let last = self.line_count().saturating_sub(1);
                self.move_cursor((last, 0), shift);
            }
            KeyCode::Enter => {
                if let Some(url) = self.link_under_cursor().map(|l| l.url.clone()) {
                    return self.activate_link(&url);
                }
                return vec![];
            }
            // Open the link under the cursor in the external browser, even when
            // the viewer would otherwise navigate it in place.
            KeyCode::Char('o') | KeyCode::Char('O') => {
                if let Some(url) = self.link_under_cursor().map(|l| l.url.clone()) {
                    return vec![PanelEvent::OpenExternal(PathBuf::from(self.resolve(&url)))];
                }
                return vec![];
            }
            KeyCode::Esc if self.anchor.is_some() => {
                self.anchor = None;
            }
            _ => return vec![],
        }
        vec![PanelEvent::NeedsRedraw]
    }

    fn handle_scroll(&mut self, delta: i32, _panel_area: Rect) -> Vec<PanelEvent> {
        self.scroll_by(delta);
        vec![PanelEvent::NeedsRedraw]
    }

    fn handle_mouse(&mut self, event: MouseEvent, _panel_area: Rect) -> Vec<PanelEvent> {
        // Route clicks on the find bar to it.
        if let Some(bar) = self.find_bar.as_mut() {
            if bar.click_hits_bar(event.column, event.row) {
                if let Some(action) = bar.handle_mouse(event) {
                    return self.handle_find_action(action);
                }
                return vec![PanelEvent::NeedsRedraw];
            }
        }

        // Map against the content area actually drawn in render (not the raw
        // panel area, which may include a header), so clicks land precisely.
        let area = self.last_area;
        if event.column < area.x || event.row < area.y {
            // Outside the content (e.g. the find bar zone) — ignore non-wheel.
            if !matches!(
                event.kind,
                MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            ) {
                return vec![];
            }
        }
        let rel_col = event.column.saturating_sub(area.x);
        let rel_row = event.row.saturating_sub(area.y);
        let line_idx = self.top + rel_row as usize;

        match event.kind {
            MouseEventKind::ScrollDown => {
                self.scroll_by(3);
            }
            MouseEventKind::ScrollUp => {
                self.scroll_by(-3);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if line_idx >= self.line_count() {
                    return vec![];
                }
                // Move the cursor to the click first, then open a link if the
                // click landed on one.
                let col = display_to_char_col(&self.line_text(line_idx), rel_col);
                self.cursor = (line_idx, col);
                self.anchor = None;
                self.drag_from = Some(self.cursor);
                if let Some(url) = self.link_at(line_idx, rel_col).map(|l| l.url.clone()) {
                    let mut evs = vec![PanelEvent::NeedsRedraw];
                    evs.extend(self.activate_link(&url));
                    return evs;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(origin) = self.drag_from {
                    if line_idx < self.line_count() {
                        let col = display_to_char_col(&self.line_text(line_idx), rel_col);
                        self.anchor = Some(origin);
                        self.cursor = (line_idx, col);
                        self.clamp_cursor();
                        self.ensure_cursor_visible();
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.drag_from = None;
                if self.anchor == Some(self.cursor) {
                    self.anchor = None; // a click, not a drag
                }
            }
            _ => return vec![],
        }
        vec![PanelEvent::NeedsRedraw]
    }

    fn captures_escape(&self) -> bool {
        self.find_bar.is_some() || self.anchor.is_some()
    }

    fn status_segments(&self) -> Vec<StatusSegment> {
        if self.error.is_some() {
            return vec![];
        }
        let sep = || StatusSegment::new(" │ ", SegmentKind::Label);
        let total = self.line_count().max(1);
        let pos = (self.cursor.0 + 1).min(total);
        // Same field order as the editor: View first, then Edit.
        vec![
            StatusSegment::new(" ", SegmentKind::Label),
            StatusSegment::new("View: ", SegmentKind::Label),
            StatusSegment::new("Rendered", SegmentKind::Value),
            sep(),
            StatusSegment::clickable("Edit: ", SegmentKind::Label, "edit_source"),
            StatusSegment::clickable("No", SegmentKind::Active, "edit_source"),
            sep(),
            StatusSegment::new("Line: ", SegmentKind::Label),
            StatusSegment::new(format!("{pos}/{total}"), SegmentKind::Value),
        ]
    }

    fn handle_status_action(&mut self, action: &str) -> Vec<PanelEvent> {
        match action {
            "edit_source" => vec![PanelEvent::SwapActiveToText(self.file_path.clone())],
            _ => vec![],
        }
    }

    fn reload(&mut self) -> anyhow::Result<()> {
        // URL-backed content has no file to re-read; leave it as-is.
        if self.source_url.is_some() {
            return Ok(());
        }
        let path = self.file_path.clone();
        self.set_file(path);
        Ok(())
    }

    fn to_session(&self, _session_dir: &Path) -> Option<SessionPanel> {
        // Only file-backed viewers persist; fetched URLs are not restored.
        if self.source_url.is_some() {
            return None;
        }
        Some(SessionPanel::Html {
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

/// Substring of `s` between character indices `[start, end)`.
fn slice_chars(s: &str, start: usize, end: usize) -> String {
    s.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

/// Display column at character index `col` (sum of preceding char widths).
fn char_col_to_display(s: &str, col: usize) -> usize {
    s.chars().take(col).map(|c| c.width().unwrap_or(0)).sum()
}

/// Character index at (or just past) display column `disp`.
fn display_to_char_col(s: &str, disp: u16) -> usize {
    let target = disp as usize;
    let mut acc = 0usize;
    for (i, c) in s.chars().enumerate() {
        if acc >= target {
            return i;
        }
        acc += c.width().unwrap_or(0);
    }
    s.chars().count()
}

/// Character indices where `needle` occurs in `line` (case-insensitive when `ci`).
fn find_in_line(line: &str, needle: &str, ci: bool) -> Vec<usize> {
    let hay: Vec<char> = line.chars().collect();
    let pat: Vec<char> = needle.chars().collect();
    let mut out = Vec::new();
    if pat.is_empty() || pat.len() > hay.len() {
        return out;
    }
    let eq = |a: char, b: char| {
        if ci {
            a.eq_ignore_ascii_case(&b) || a.to_lowercase().eq(b.to_lowercase())
        } else {
            a == b
        }
    };
    for i in 0..=hay.len() - pat.len() {
        if (0..pat.len()).all(|j| eq(hay[i + j], pat[j])) {
            out.push(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel_from(src: &str) -> HtmlPanel {
        let mut p = HtmlPanel {
            file_path: PathBuf::from("/x/page.html"),
            title: "page.html".to_string(),
            source: src.to_string(),
            error: None,
            doc: Rendered {
                lines: Vec::new(),
                links: Vec::new(),
            },
            layout_width: 0,
            top: 0,
            last_area: Rect::new(0, 0, 80, 10),
            cursor: (0, 0),
            anchor: None,
            drag_from: None,
            find_bar: None,
            matches: Vec::new(),
            match_len: 0,
            match_idx: 0,
            colors: ThemeColors::default(),
            theme_full: None,
            is_light: false,
            hotkeys: HotkeyTable::default(),
            last_config_ptr: 0,
            source_url: None,
            history: Vec::new(),
            hist_idx: 0,
            open_links: LinkOpen::Panel,
        };
        p.doc = render_html(src, 80, &p.colors, false);
        p.layout_width = 80;
        p
    }

    #[test]
    fn name_is_html() {
        let p = panel_from("<p>hi</p>");
        assert_eq!(p.name(), "html");
    }

    #[test]
    fn selection_copies_across_lines() {
        let mut p = panel_from("<p>alpha</p><p>beta</p>");
        p.cursor = (0, 0);
        p.anchor = Some((0, 0));
        let last = p.line_count() - 1;
        p.cursor = (last, p.line_len(last));
        let text = p.selected_text();
        assert!(text.contains("alpha"), "{text:?}");
        assert!(text.contains("beta"), "{text:?}");
    }

    #[test]
    fn edit_source_action_swaps_to_text() {
        let mut p = panel_from("<p>hi</p>");
        let evs = p.handle_status_action("edit_source");
        assert!(matches!(evs.as_slice(), [PanelEvent::SwapActiveToText(_)]));
    }

    #[test]
    fn to_session_round_trips_path() {
        let p = panel_from("<p>x</p>");
        match p.to_session(Path::new("/tmp")) {
            Some(SessionPanel::Html { path }) => {
                assert_eq!(path, PathBuf::from("/x/page.html"))
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn search_finds_matches_and_moves_cursor() {
        let mut p = panel_from("<p>one two</p><p>two three two</p>");
        p.open_find();
        p.find_bar
            .as_mut()
            .unwrap()
            .set_text(FindField::Find, "two".to_string());
        p.run_search();
        assert_eq!(p.matches.len(), 3, "{:?}", p.matches);
        assert_eq!(p.cursor, p.matches[p.match_idx]);
        let first = p.cursor;
        p.step_match(true);
        assert_ne!(p.cursor, first, "next match should move the cursor");
    }

    #[test]
    fn ctrl_g_requests_go_to_path() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut p = panel_from("<p>hi</p>");
        let evs = p.handle_key(KeyChord::identity(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        )));
        assert!(
            matches!(
                evs.as_slice(),
                [PanelEvent::ShowInput {
                    on_submit: InputAction::ViewPath { .. },
                    ..
                }]
            ),
            "{evs:?}"
        );
    }

    #[test]
    fn relative_link_resolves_and_navigates_in_place() {
        let mut p = HtmlPanel::from_source(
            "page".into(),
            "x".into(),
            Some("https://ex.com/dir/".into()),
        );
        let evs = p.activate_link("sub/x.html");
        match evs.as_slice() {
            [PanelEvent::NavigateUrl(u)] => assert_eq!(u, "https://ex.com/dir/sub/x.html"),
            _ => panic!("{evs:?}"),
        }
        assert_eq!(p.history.last().unwrap(), "https://ex.com/dir/sub/x.html");
    }

    #[test]
    fn file_backed_web_link_opens_new_panel_by_default() {
        // Default open_links = Panel: a web link from a file-backed view opens
        // a new viewer (not the external browser, not in place).
        let mut p = panel_from("<p>x</p>");
        let evs = p.activate_link("https://ex.com");
        assert!(
            matches!(evs.as_slice(), [PanelEvent::OpenUrl(_)]),
            "{evs:?}"
        );
    }

    #[test]
    fn external_setting_opens_browser() {
        let mut p = panel_from("<p>x</p>");
        p.open_links = LinkOpen::External;
        let evs = p.activate_link("https://ex.com");
        assert!(
            matches!(evs.as_slice(), [PanelEvent::OpenExternal(_)]),
            "{evs:?}"
        );
    }

    #[test]
    fn history_back_and_forward() {
        let mut p = HtmlPanel::from_source("p".into(), "x".into(), Some("https://ex.com/a".into()));
        p.activate_link("https://ex.com/b");
        p.activate_link("https://ex.com/c");
        let back = p.go_back();
        assert!(
            matches!(back.as_slice(), [PanelEvent::NavigateUrl(u)] if u == "https://ex.com/b"),
            "{back:?}"
        );
        assert!(
            matches!(p.go_back().as_slice(), [PanelEvent::NavigateUrl(u)] if u == "https://ex.com/a")
        );
        assert!(p.go_back().is_empty(), "no history before the first page");
        assert!(
            matches!(p.go_forward().as_slice(), [PanelEvent::NavigateUrl(u)] if u == "https://ex.com/b")
        );
    }

    #[test]
    fn bracket_history_works_on_cyrillic_layout() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut p = HtmlPanel::from_source("p".into(), "x".into(), Some("https://ex.com/a".into()));
        p.activate_link("https://ex.com/b"); // history [a, b], idx = 1
                                             // On a Russian layout the `[` key emits 'х'; the normalizer puts '[' in
                                             // `canonical`, which the viewer matches.
        let chord = KeyChord {
            raw: KeyEvent::new(KeyCode::Char('х'), KeyModifiers::NONE),
            canonical: KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE),
        };
        let evs = p.handle_key(chord);
        assert!(
            matches!(evs.as_slice(), [PanelEvent::NavigateUrl(u)] if u == "https://ex.com/a"),
            "{evs:?}"
        );
    }

    #[test]
    fn link_under_cursor_opens() {
        let p = panel_from("<p><a href=\"https://ex.com\">docs</a></p>");
        assert_eq!(p.doc.links.len(), 1, "{:?}", p.doc.links);
        assert_eq!(p.doc.links[0].url, "https://ex.com");
    }
}
