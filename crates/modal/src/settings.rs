//! Settings modal with tabbed interface for editing application configuration.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Block, Borders, Clear, Widget},
};
use termide_config::Config;
use termide_config::KeyBinding;
use termide_config::LspServerSettings;
use termide_i18n as i18n;
use termide_theme::Theme;

use crate::{base::button_style, Modal, ModalResult};

/// Truncate `s` to at most `max_chars` Unicode scalar values, safe for UTF-8 slicing.
fn truncate_str(s: &str, max_chars: usize) -> &str {
    if let Some((idx, _)) = s.char_indices().nth(max_chars) {
        &s[..idx]
    } else {
        s
    }
}

mod fields;
mod kb;

use fields::{
    cycle_enum_backward, cycle_enum_forward, fields_for_tab, get_field_value, toggle_field,
    ContentRow, FieldDescriptor, FieldType,
};
use kb::{format_key_event, get_kb_value, kb_binding_names, set_kb_value, KB_SECTIONS};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Which settings tab is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Editor,
    FileManager,
    Terminal,
    Lsp,
    Logging,
    Vfs,
    Keybindings,
}

/// Top-level leaf tabs in the sidebar (excluding the Keybindings group).
const TOP_LEVEL_TABS: [SettingsTab; 7] = [
    SettingsTab::General,
    SettingsTab::Editor,
    SettingsTab::FileManager,
    SettingsTab::Terminal,
    SettingsTab::Lsp,
    SettingsTab::Logging,
    SettingsTab::Vfs,
];

/// Sidebar width in columns.
const MODAL_SIDEBAR_WIDTH: u16 = 18;

impl SettingsTab {
    fn label(self) -> String {
        let t = i18n::t();
        match self {
            SettingsTab::General => t.settings_tab_general().to_string(),
            SettingsTab::Editor => t.settings_tab_editor().to_string(),
            SettingsTab::FileManager => t.settings_tab_file_manager().to_string(),
            SettingsTab::Terminal => t.settings_tab_terminal().to_string(),
            SettingsTab::Lsp => t.settings_tab_lsp().to_string(),
            SettingsTab::Logging => t.settings_tab_logging().to_string(),
            SettingsTab::Vfs => t.settings_tab_vfs().to_string(),
            SettingsTab::Keybindings => t.settings_tab_keybindings().to_string(),
        }
    }
}

/// Which UI zone has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusArea {
    Sidebar,
    Content,
    Buttons,
}

/// A single visible row in the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarRow {
    /// Top-level leaf — activating sets `active_tab`.
    Leaf(SettingsTab),
    /// Expandable "Keybindings" group header.
    KbGroupHeader,
    /// Keybindings subsection (index into `KB_SECTIONS`, 0..7).
    KbChild(usize),
}

/// LSP tab sub-mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LspMode {
    /// Normal field browsing + server list.
    Fields,
    /// Editing an LSP server (new or existing).
    ServerEdit,
}

/// Keybindings tab sub-mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KbMode {
    /// Browsing bindings for the active section — user picks one to rebind.
    Bindings,
    /// Capturing a keypress for the selected binding.
    Capturing,
}

/// Result returned when the settings modal closes.
///
/// `Apply` boxes `Config` because it's large (~3.6 KB) and infrequent —
/// keeping the enum small avoids bloating every `ModalResult` carrier.
#[derive(Debug)]
pub enum SettingsResult {
    /// User clicked "Apply & Save" — apply and persist the config.
    Apply(Box<Config>),
    /// User clicked "Cancel" (or Esc from tab bar).
    Cancel,
}

/// Bottom buttons.
const BUTTON_APPLY: usize = 0;
const BUTTON_RESET: usize = 1;

/// Get localized button labels.
fn button_labels() -> [String; 3] {
    let t = i18n::t();
    [
        t.settings_btn_apply().to_string(),
        t.settings_btn_reset().to_string(),
        t.settings_btn_cancel().to_string(),
    ]
}

// ---------------------------------------------------------------------------
// SettingsModal
// ---------------------------------------------------------------------------

/// Full-screen settings modal with tabs, scrollable fields, and action buttons.
#[derive(Debug)]
pub struct SettingsModal {
    /// Working copy of config (mutated in-place; only saved on Apply).
    config: Config,

    // --- Tab state ---
    active_tab: SettingsTab,

    // --- Sidebar state ---
    /// Cursor index into `visible_sidebar_rows()`.
    sidebar_cursor: usize,
    /// Vertical scroll offset for the sidebar.
    sidebar_scroll: usize,
    /// Whether the Keybindings group is expanded.
    keybindings_expanded: bool,

    // --- Focus ---
    focus: FocusArea,
    /// Which field row is focused (within the current tab's content).
    field_cursor: usize,
    /// Vertical scroll offset for the content area.
    content_scroll: usize,

    // --- Editing ---
    /// True when a text/number field is being edited inline.
    editing: bool,
    /// Current edit buffer for text/number fields.
    edit_buffer: String,

    // --- LSP server management ---
    lsp_mode: LspMode,
    /// Index of the server being edited (None = adding new).
    lsp_edit_index: Option<usize>,
    /// Sorted server language names for stable indexing.
    lsp_server_keys: Vec<String>,
    /// Inline edit form for LSP server: [language, command, args, root_markers].
    lsp_edit_fields: [String; 4],
    /// Which field (0-3) is focused in the LSP edit form.
    lsp_edit_cursor: usize,

    // --- Keybindings tab ---
    kb_mode: KbMode,
    /// Which section (0-6) is selected.
    kb_section: usize,
    /// Cursor within the binding list of the current section.
    kb_cursor: usize,
    /// Scroll offset for binding list.
    kb_scroll: usize,

    // --- Buttons ---
    selected_button: usize,
    dirty: bool,

    // --- Area caches (for mouse hit-testing) ---
    last_modal_area: Option<Rect>,
    last_sidebar_area: Option<Rect>,
    last_content_area: Option<Rect>,
    last_buttons_area: Option<Rect>,
}

impl SettingsModal {
    pub fn new(config: Config) -> Self {
        let lsp_server_keys = Self::sorted_server_keys(&config);
        let mut m = Self {
            config,
            active_tab: SettingsTab::General,
            sidebar_cursor: 0,
            sidebar_scroll: 0,
            keybindings_expanded: false,
            focus: FocusArea::Sidebar,
            field_cursor: 0,
            content_scroll: 0,
            editing: false,
            edit_buffer: String::new(),
            lsp_mode: LspMode::Fields,
            lsp_edit_index: None,
            lsp_server_keys,
            lsp_edit_fields: Default::default(),
            lsp_edit_cursor: 0,
            kb_mode: KbMode::Bindings,
            kb_section: 0,
            kb_cursor: 0,
            kb_scroll: 0,
            selected_button: BUTTON_APPLY,
            dirty: false,
            last_modal_area: None,
            last_sidebar_area: None,
            last_content_area: None,
            last_buttons_area: None,
        };
        m.field_cursor = m.first_selectable_row();
        m
    }

    fn sorted_server_keys(config: &Config) -> Vec<String> {
        let mut keys: Vec<String> = config.lsp.servers.keys().cloned().collect();
        keys.sort();
        keys
    }

    fn refresh_server_keys(&mut self) {
        self.lsp_server_keys = Self::sorted_server_keys(&self.config);
    }

    // ---- Sizing ----

    fn calculate_size(screen: Rect) -> Rect {
        let w = ((screen.width as usize * 90) / 100).clamp(80, 140);
        let h = ((screen.height as usize * 85) / 100).clamp(20, 50);
        let w = w.min(screen.width as usize).max(60);
        let h = h.min(screen.height as usize).max(16);
        let x = (screen.width as usize).saturating_sub(w) / 2;
        let y = (screen.height as usize).saturating_sub(h) / 2;
        Rect::new(x as u16, y as u16, w as u16, h as u16)
    }

    // ---- Sidebar helpers ----

    /// Build the visible sidebar rows (respects `keybindings_expanded`).
    fn visible_sidebar_rows(&self) -> Vec<SidebarRow> {
        let mut rows: Vec<SidebarRow> = TOP_LEVEL_TABS
            .iter()
            .map(|&t| SidebarRow::Leaf(t))
            .collect();
        rows.push(SidebarRow::KbGroupHeader);
        if self.keybindings_expanded {
            for i in 0..KB_SECTIONS.len() {
                rows.push(SidebarRow::KbChild(i));
            }
        }
        rows
    }

    /// Find the sidebar cursor index matching the current `active_tab` / `kb_section`.
    fn sidebar_cursor_for_active(&self) -> usize {
        let rows = self.visible_sidebar_rows();
        for (i, row) in rows.iter().enumerate() {
            match *row {
                SidebarRow::Leaf(tab) if tab == self.active_tab => return i,
                SidebarRow::KbChild(idx)
                    if self.active_tab == SettingsTab::Keybindings && idx == self.kb_section =>
                {
                    return i;
                }
                SidebarRow::KbGroupHeader
                    if self.active_tab == SettingsTab::Keybindings
                        && !self.keybindings_expanded =>
                {
                    return i;
                }
                _ => {}
            }
        }
        0
    }

    /// Update `active_tab` etc to match the row under the cursor, WITHOUT toggling group
    /// expansion (used by arrow/tab navigation).
    fn preview_sidebar_row(&mut self, row: SidebarRow) {
        match row {
            SidebarRow::Leaf(tab) => {
                self.active_tab = tab;
                self.content_scroll = 0;
                self.editing = false;
                self.field_cursor = self.first_selectable_row();
            }
            SidebarRow::KbGroupHeader => {
                // No-op on navigation: keep whatever was active.
            }
            SidebarRow::KbChild(idx) => {
                self.active_tab = SettingsTab::Keybindings;
                self.kb_section = idx;
                self.kb_mode = KbMode::Bindings;
                self.kb_cursor = 0;
                self.kb_scroll = 0;
                self.editing = false;
            }
        }
    }

    /// Activate a row explicitly (Enter / mouse click). Toggles group header,
    /// otherwise behaves like `preview_sidebar_row`.
    fn activate_sidebar_row(&mut self, row: SidebarRow) {
        match row {
            SidebarRow::KbGroupHeader => {
                self.keybindings_expanded = !self.keybindings_expanded;
                if self.keybindings_expanded {
                    self.active_tab = SettingsTab::Keybindings;
                    self.kb_mode = KbMode::Bindings;
                    self.kb_cursor = 0;
                    self.kb_scroll = 0;
                }
            }
            other => self.preview_sidebar_row(other),
        }
    }

    /// Clamp sidebar scroll so `sidebar_cursor` is visible.
    fn clamp_sidebar_scroll(&mut self, visible: usize) {
        if self.sidebar_cursor < self.sidebar_scroll {
            self.sidebar_scroll = self.sidebar_cursor;
        }
        if visible > 0 && self.sidebar_cursor >= self.sidebar_scroll + visible {
            self.sidebar_scroll = self.sidebar_cursor - visible + 1;
        }
    }

    /// Localized label for the Keybindings group header (same as the tab label).
    fn kb_group_label() -> String {
        i18n::t().settings_tab_keybindings().to_string()
    }

    // ---- Content-row helpers ----

    /// Build the list of rows rendered in the content area for the active tab.
    /// Field indices reference `fields_for_tab(self.active_tab)`.
    fn content_rows(&self) -> Vec<ContentRow> {
        use ContentRow::*;
        match self.active_tab {
            SettingsTab::General => vec![
                Header("Appearance"),
                Field(1), // theme
                Field(2), // language
                Field(3), // icon_mode
                Spacer,
                Header("Input"),
                Field(0), // vim_mode
                Spacer,
                Header("Layout"),
                Field(4), // auto_stack_threshold
                Field(5), // min_panel_width
                Spacer,
                Header("Notifications"),
                Field(7), // bell
                Spacer,
                Header("Performance"),
                Field(6), // session_retention
                Field(8), // resource_monitor_interval
            ],
            SettingsTab::Editor => vec![
                Header("Typing"),
                Field(0), // tab_size
                Field(2), // auto_indent
                Field(3), // auto_close_brackets
                Spacer,
                Header("Display"),
                Field(1), // word_wrap
                Field(4), // show_git_diff
                Field(5), // show_blame
                Spacer,
                Header("Performance"),
                Field(6), // large_file_threshold
            ],
            SettingsTab::FileManager => vec![
                Header("Display"),
                Field(0),
                Spacer,
                Header("Search"),
                Field(1),
            ],
            SettingsTab::Terminal => vec![Field(0)],
            SettingsTab::Lsp => {
                let mut rows = vec![
                    Header("General"),
                    Field(0), // enabled
                    Field(1), // auto_completion
                    Spacer,
                    Header("Timing"),
                    Field(2), // completion_delay
                    Field(3), // hover_delay
                    Spacer,
                    Header("Servers"),
                    LspAddServer,
                ];
                for i in 0..self.lsp_server_keys.len() {
                    rows.push(LspServer(i));
                }
                rows
            }
            SettingsTab::Logging => vec![Field(0), Field(1)],
            SettingsTab::Vfs => vec![Field(0)],
            SettingsTab::Keybindings => Vec::new(),
        }
    }

    fn current_row(&self) -> Option<ContentRow> {
        self.content_rows().get(self.field_cursor).copied()
    }

    fn current_field_idx(&self) -> Option<usize> {
        match self.current_row()? {
            ContentRow::Field(i) => Some(i),
            _ => None,
        }
    }

    fn first_selectable_row(&self) -> usize {
        self.content_rows()
            .iter()
            .position(|r| r.is_selectable())
            .unwrap_or(0)
    }

    fn last_selectable_row(&self) -> usize {
        let rows = self.content_rows();
        rows.iter()
            .enumerate()
            .rev()
            .find_map(|(i, r)| if r.is_selectable() { Some(i) } else { None })
            .unwrap_or(0)
    }

    /// Move cursor to the next selectable row in the given direction.
    /// Returns false if no further selectable row exists.
    fn step_cursor(&mut self, forward: bool) -> bool {
        let rows = self.content_rows();
        if rows.is_empty() {
            return false;
        }
        let mut c = self.field_cursor.min(rows.len().saturating_sub(1));
        loop {
            if forward {
                if c + 1 >= rows.len() {
                    return false;
                }
                c += 1;
            } else if c == 0 {
                return false;
            } else {
                c -= 1;
            }
            if rows[c].is_selectable() {
                self.field_cursor = c;
                return true;
            }
        }
    }

    // ---- Scroll ----

    fn clamp_scroll(&mut self, visible: usize) {
        if self.field_cursor < self.content_scroll {
            self.content_scroll = self.field_cursor;
        }
        if visible > 0 && self.field_cursor >= self.content_scroll + visible {
            self.content_scroll = self.field_cursor - visible + 1;
        }
    }

    /// Commit the current edit buffer to the config.
    fn commit_edit(&mut self) {
        let tab = self.active_tab;
        let Some(field_idx) = self.current_field_idx() else {
            self.editing = false;
            return;
        };
        let fields = fields_for_tab(tab);
        let Some(desc) = fields.get(field_idx) else {
            self.editing = false;
            return;
        };

        match desc.field_type {
            FieldType::Number => {
                let val = self.edit_buffer.parse::<u64>().unwrap_or(0);
                self.apply_number(tab, field_idx, val);
                self.dirty = true;
            }
            FieldType::OptionalText => {
                let text = self.edit_buffer.clone();
                self.apply_text(tab, field_idx, &text);
                self.dirty = true;
            }
            _ => {}
        }
        self.editing = false;
    }

    /// Cancel the current inline edit.
    fn cancel_edit(&mut self) {
        self.editing = false;
    }

    /// Start editing the current field.
    fn start_edit(&mut self) {
        let Some(field_idx) = self.current_field_idx() else {
            return;
        };
        let fields = fields_for_tab(self.active_tab);
        let Some(desc) = fields.get(field_idx) else {
            return;
        };
        match desc.field_type {
            FieldType::Bool | FieldType::Enum => return,
            _ => {}
        }
        self.edit_buffer = get_field_value(&self.config, self.active_tab, field_idx);
        // Strip "(auto)" / "(none)" placeholders
        if self.edit_buffer.starts_with('(') {
            self.edit_buffer.clear();
        }
        self.editing = true;
    }

    fn apply_number(&mut self, tab: SettingsTab, index: usize, val: u64) {
        match tab {
            SettingsTab::General => match index {
                4 => self.config.general.auto_stack_threshold = val as u16,
                5 => self.config.general.min_panel_width = val as u16,
                6 => self.config.general.session_retention_days = val as u32,
                8 => self.config.general.resource_monitor_interval = val,
                _ => {}
            },
            SettingsTab::Editor => match index {
                0 => self.config.editor.tab_size = val as usize,
                6 => self.config.editor.large_file_threshold_mb = val,
                _ => {}
            },
            SettingsTab::FileManager => match index {
                0 => self.config.file_manager.extended_view_width = val as usize,
                1 => self.config.file_manager.content_search_max_file_size_mb = val,
                _ => {}
            },
            SettingsTab::Lsp => match index {
                2 => self.config.lsp.completion_delay_ms = val,
                3 => self.config.lsp.hover_delay_ms = val,
                _ => {}
            },
            SettingsTab::Vfs => {
                if index == 0 {
                    self.config.vfs.connection_timeout_secs = val;
                }
            }
            _ => {}
        }
    }

    fn apply_text(&mut self, tab: SettingsTab, index: usize, text: &str) {
        match tab {
            SettingsTab::Terminal => {
                if index == 0 {
                    if text.is_empty() {
                        self.config.terminal.default_shell = None;
                    } else {
                        self.config.terminal.default_shell = Some(text.to_string());
                    }
                }
            }
            SettingsTab::Logging => {
                if index == 0 {
                    if text.is_empty() {
                        self.config.logging.file_path = None;
                    } else {
                        self.config.logging.file_path = Some(text.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    // ---- Rendering helpers ----

    /// Render the left sidebar with section leaves and the expandable Keybindings group.
    fn render_sidebar(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width < 3 || area.height < 1 {
            return;
        }
        let width = area.width as usize;
        let visible = area.height as usize;
        self.clamp_sidebar_scroll(visible);

        let rows = self.visible_sidebar_rows();
        let focused = self.focus == FocusArea::Sidebar;
        // Only show the cursor highlight when the sidebar actually has focus.
        // When the user moves focus to Content/Buttons, the section title is
        // already shown in the content area header, so keeping a highlighted
        // row here would only confuse where the real focus lives.
        let active_cursor = if focused {
            Some(self.sidebar_cursor)
        } else {
            None
        };

        let kb_label = Self::kb_group_label();

        for row_i in 0..visible {
            let idx = self.sidebar_scroll + row_i;
            if idx >= rows.len() {
                break;
            }
            let y = area.y + row_i as u16;
            let is_selected = active_cursor == Some(idx);

            if is_selected {
                for x in area.x..area.x + area.width {
                    buf[(x, y)]
                        .set_style(Style::default().bg(theme.selected_bg).fg(theme.selected_fg));
                }
            }

            let (prefix, label): (String, String) = match rows[idx] {
                SidebarRow::Leaf(tab) => (" ".to_string(), tab.label()),
                SidebarRow::KbGroupHeader => (
                    if self.keybindings_expanded {
                        "▼ "
                    } else {
                        "▶ "
                    }
                    .to_string(),
                    kb_label.clone(),
                ),
                SidebarRow::KbChild(i) => ("   ".to_string(), KB_SECTIONS[i].to_string()),
            };

            let style = if is_selected {
                Style::default()
                    .fg(theme.selected_fg)
                    .bg(theme.selected_bg)
                    .add_modifier(Modifier::BOLD)
            } else if matches!(rows[idx], SidebarRow::KbGroupHeader) {
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg)
            };

            let full = format!("{}{}", prefix, label);
            let max_w = width.saturating_sub(1);
            let display = if full.chars().count() > max_w && max_w > 1 {
                let mut s: String = full.chars().take(max_w.saturating_sub(1)).collect();
                s.push('…');
                s
            } else {
                full
            };
            buf.set_string(area.x + 1, y, &display, style);
        }

        self.last_sidebar_area = Some(area);
    }

    /// Render the bottom button bar using standard button style.
    fn render_buttons(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width < 4 {
            return;
        }
        let y = area.y;

        // Separator line
        for x in area.x..area.x + area.width {
            buf[(x, y)]
                .set_char('─')
                .set_style(Style::default().fg(theme.disabled));
        }

        // Buttons on the next row
        let by = y + 1;
        if by >= area.y + area.height {
            self.last_buttons_area = Some(area);
            return;
        }

        let spacing = 4;
        let labels = button_labels();
        let total_label_len: usize = labels.iter().map(|l| l.len() + 4).sum::<usize>() // "[ label ]"
            + spacing * (labels.len().saturating_sub(1));
        let mut x = area.x as usize + (area.width as usize).saturating_sub(total_label_len) / 2;

        for (i, label) in labels.iter().enumerate() {
            let is_selected = self.focus == FocusArea::Buttons && self.selected_button == i;
            let style = if i == BUTTON_RESET && !self.dirty && !is_selected {
                Style::default().fg(theme.disabled)
            } else {
                button_style(is_selected, theme)
            };
            let btn = format!("[ {} ]", label);
            for ch in btn.chars() {
                if x < (area.x as usize) + area.width as usize {
                    buf[(x as u16, by)].set_char(ch).set_style(style);
                    x += 1;
                }
            }
            if i < labels.len() - 1 {
                for _ in 0..spacing {
                    if x < (area.x as usize) + area.width as usize {
                        buf[(x as u16, by)]
                            .set_char(' ')
                            .set_style(Style::default());
                        x += 1;
                    }
                }
            }
        }

        self.last_buttons_area = Some(Rect::new(area.x, by, area.width, 1));
    }

    /// Render a section title at the top of `area`. Returns the remaining area
    /// below the title (title row + blank row consumed).
    fn render_section_title(area: Rect, buf: &mut Buffer, theme: &Theme, title: &str) -> Rect {
        if area.height < 3 {
            return area;
        }
        buf.set_string(
            area.x + 2,
            area.y,
            title,
            Style::default()
                .fg(theme.accented_fg)
                .add_modifier(Modifier::BOLD),
        );
        Rect::new(area.x, area.y + 2, area.width, area.height - 2)
    }

    /// Render content area with field rows.
    fn render_content(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        // LSP server edit form — takes over the entire content area
        if self.active_tab == SettingsTab::Lsp && self.lsp_mode == LspMode::ServerEdit {
            let title = format!(
                "{} › {}",
                SettingsTab::Lsp.label(),
                i18n::t().settings_lsp_add_server()
            );
            let inner = Self::render_section_title(area, buf, theme, &title);
            self.render_lsp_edit_form(inner, buf, theme);
            self.last_content_area = Some(inner);
            return;
        }

        // Keybindings tab — dedicated renderer (renders its own title).
        if self.active_tab == SettingsTab::Keybindings {
            let section_name = KB_SECTIONS.get(self.kb_section).copied().unwrap_or("");
            let title = format!("{} › {}", Self::kb_group_label(), section_name);
            let inner = Self::render_section_title(area, buf, theme, &title);
            self.render_keybindings(inner, buf, theme);
            self.last_content_area = Some(inner);
            return;
        }

        // Regular tab: title + grouped fields.
        let title = self.active_tab.label();
        let area = Self::render_section_title(area, buf, theme, &title);

        let rows = self.content_rows();
        if rows.is_empty() {
            self.last_content_area = Some(area);
            return;
        }

        let visible_rows = area.height as usize;
        self.clamp_scroll(visible_rows);

        let fields = fields_for_tab(self.active_tab);
        let label_width = 32;
        let value_x = area.x as usize + 2 + label_width;
        let max_value_width = (area.x as usize + area.width as usize).saturating_sub(value_x);

        for row_off in 0..visible_rows {
            let row_idx = self.content_scroll + row_off;
            if row_idx >= rows.len() {
                break;
            }
            let y = area.y + row_off as u16;
            let row = rows[row_idx];
            let is_focused = self.focus == FocusArea::Content
                && row_idx == self.field_cursor
                && row.is_selectable();

            if is_focused {
                for x in area.x..area.x + area.width {
                    buf[(x, y)]
                        .set_style(Style::default().bg(theme.selected_bg).fg(theme.selected_fg));
                }
            }

            match row {
                ContentRow::Header(label) => {
                    let text = format!("── {} ──", label);
                    buf.set_string(
                        area.x + 2,
                        y,
                        &text,
                        Style::default()
                            .fg(theme.disabled)
                            .add_modifier(Modifier::BOLD),
                    );
                }
                ContentRow::Spacer => {
                    // Intentionally blank row between groups.
                }
                ContentRow::Field(field_idx) => {
                    let Some(desc) = fields.get(field_idx) else {
                        continue;
                    };
                    let label_style = if is_focused {
                        Style::default()
                            .fg(theme.selected_fg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.fg)
                    };

                    let label_text = truncate_str(desc.label, label_width);
                    buf.set_string(area.x + 2, y, label_text, label_style);

                    let value = if self.editing && is_focused {
                        format!("{}_", self.edit_buffer)
                    } else {
                        self.format_field_value(desc, field_idx)
                    };

                    let value_style = if is_focused {
                        Style::default().fg(theme.selected_fg)
                    } else {
                        match desc.field_type {
                            FieldType::Bool | FieldType::Enum => {
                                Style::default().fg(theme.accented_fg)
                            }
                            _ => Style::default().fg(theme.fg),
                        }
                    };

                    let display_value = if value.len() > max_value_width && max_value_width > 2 {
                        format!("{}…", &value[..max_value_width - 1])
                    } else {
                        value
                    };
                    buf.set_string(value_x as u16, y, &display_value, value_style);
                }
                ContentRow::LspAddServer => {
                    let label = i18n::t().settings_lsp_add_server();
                    let style = if is_focused {
                        Style::default()
                            .fg(theme.selected_fg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.accented_fg)
                    };
                    buf.set_string(area.x + 2, y, label, style);
                }
                ContentRow::LspServer(server_idx) => {
                    if server_idx >= self.lsp_server_keys.len() {
                        continue;
                    }
                    let lang = &self.lsp_server_keys[server_idx];
                    let label_style = if is_focused {
                        Style::default()
                            .fg(theme.selected_fg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.fg)
                    };
                    let label = format!("• {}", lang);
                    buf.set_string(area.x + 2, y, &label, label_style);

                    if let Some(srv) = self.config.lsp.servers.get(lang) {
                        let cmd_info = format!("{} {}", srv.command, srv.args.join(" "));
                        let cmd_style = if is_focused {
                            Style::default().fg(theme.selected_fg)
                        } else {
                            Style::default().fg(theme.disabled)
                        };
                        let max_cmd = max_value_width.saturating_sub(12);
                        let display_cmd = if cmd_info.len() > max_cmd && max_cmd > 2 {
                            format!("{}…", &cmd_info[..max_cmd - 1])
                        } else {
                            cmd_info
                        };
                        buf.set_string(value_x as u16, y, &display_cmd, cmd_style);

                        let del_label = if is_focused { "[Del]" } else { "" };
                        let del_x = (area.x as usize + area.width as usize).saturating_sub(6);
                        buf.set_string(
                            del_x as u16,
                            y,
                            del_label,
                            Style::default().fg(theme.accented_fg),
                        );
                    }
                }
            }
        }

        self.last_content_area = Some(area);
    }

    /// Render the LSP server edit form.
    fn render_lsp_edit_form(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let labels = [
            "Language:",
            "Command:",
            "Args (comma-sep):",
            "Root markers (comma-sep):",
        ];
        let x = area.x as usize + 4;
        let val_x = x + 26;
        let max_val = (area.x as usize + area.width as usize)
            .saturating_sub(val_x)
            .saturating_sub(2);

        for (i, label) in labels.iter().enumerate() {
            let y = area.y + 1 + i as u16;
            if y >= area.y + area.height {
                break;
            }
            let is_focused = self.lsp_edit_cursor == i;
            let label_style = if is_focused {
                Style::default()
                    .fg(theme.accented_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg)
            };
            buf.set_string(x as u16, y, label, label_style);

            let value = if is_focused {
                format!("{}_", self.lsp_edit_fields[i])
            } else {
                self.lsp_edit_fields[i].clone()
            };
            let display_val = if value.len() > max_val && max_val > 2 {
                format!("{}…", &value[..max_val - 1])
            } else {
                value
            };
            let val_style = if is_focused {
                Style::default().fg(theme.accented_fg)
            } else {
                Style::default().fg(theme.fg)
            };
            buf.set_string(val_x as u16, y, &display_val, val_style);
        }

        // Hint line
        let hint_y = area.y + 6;
        if hint_y < area.y + area.height {
            let hint = "Enter=save  Esc=cancel  Tab=next field";
            buf.set_string(x as u16, hint_y, hint, Style::default().fg(theme.disabled));
        }
    }

    /// Format a field value for display, with visual indicators.
    fn format_field_value(&self, desc: &FieldDescriptor, index: usize) -> String {
        let raw = get_field_value(&self.config, self.active_tab, index);
        match desc.field_type {
            FieldType::Bool => {
                if raw == "true" {
                    "[✓]".to_string()
                } else {
                    "[✗]".to_string()
                }
            }
            FieldType::Enum => {
                format!("< {} >", raw)
            }
            FieldType::OptionalText => raw,
            _ => raw,
        }
    }
}

// ---------------------------------------------------------------------------
// Modal trait implementation
// ---------------------------------------------------------------------------

impl Modal for SettingsModal {
    type Result = SettingsResult;

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let modal_rect = Self::calculate_size(area);
        self.last_modal_area = Some(modal_rect);

        // Clear and draw outer frame
        Clear.render(modal_rect, buf);
        let block = Block::default()
            .title(format!(
                " [X] Settings{} ",
                if self.dirty { " *" } else { "" }
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.accented_fg))
            .style(Style::default().bg(theme.bg));
        let inner = block.inner(modal_rect);
        block.render(modal_rect, buf);

        // Inner layout: body (flex, horizontal sidebar | separator | content) | buttons (2 rows)
        if inner.height < 5 {
            return;
        }
        let body_height = inner.height.saturating_sub(2);
        let body = Rect::new(inner.x, inner.y, inner.width, body_height);
        let buttons = Rect::new(inner.x, inner.y + body_height, inner.width, 2);

        // Horizontal split inside body
        let sidebar_w = MODAL_SIDEBAR_WIDTH.min(inner.width.saturating_sub(10));
        let sidebar = Rect::new(body.x, body.y, sidebar_w, body.height);
        let sep_x = body.x + sidebar_w;
        let content = Rect::new(
            sep_x + 1,
            body.y,
            body.width.saturating_sub(sidebar_w + 1),
            body.height,
        );

        self.render_sidebar(sidebar, buf, theme);

        // Vertical separator between sidebar and content
        for y in body.y..body.y + body.height {
            buf[(sep_x, y)]
                .set_char('│')
                .set_style(Style::default().fg(theme.disabled));
        }

        self.render_content(content, buf, theme);
        self.render_buttons(buttons, buf, theme);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<SettingsResult>>> {
        // If editing a text/number field, intercept all keys
        if self.editing {
            return self.handle_edit_key(key);
        }

        // Keybindings tab has its own key handling
        if self.active_tab == SettingsTab::Keybindings && self.focus == FocusArea::Content {
            return self.handle_keybindings_key(key);
        }

        match self.focus {
            FocusArea::Sidebar => self.handle_sidebar_key(key),
            FocusArea::Content => self.handle_content_key(key),
            FocusArea::Buttons => self.handle_buttons_key(key),
        }
    }

    fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        modal_area: Rect,
    ) -> Result<Option<ModalResult<SettingsResult>>> {
        if mouse.kind == MouseEventKind::ScrollUp {
            if self.focus == FocusArea::Content && self.content_scroll > 0 {
                self.content_scroll -= 1;
            } else if self.focus == FocusArea::Sidebar && self.sidebar_scroll > 0 {
                self.sidebar_scroll -= 1;
            }
            return Ok(None);
        }
        if mouse.kind == MouseEventKind::ScrollDown {
            if self.focus == FocusArea::Content {
                self.content_scroll += 1;
            } else if self.focus == FocusArea::Sidebar {
                self.sidebar_scroll += 1;
            }
            return Ok(None);
        }
        if !matches!(mouse.kind, MouseEventKind::Down(_)) {
            return Ok(None);
        }

        // Click outside modal → cancel
        let modal_rect = self.last_modal_area.unwrap_or(modal_area);
        if !modal_rect.contains((mouse.column, mouse.row).into()) {
            return Ok(Some(ModalResult::Cancelled));
        }

        // Click on sidebar
        if let Some(sidebar_area) = self.last_sidebar_area {
            if sidebar_area.contains((mouse.column, mouse.row).into()) {
                self.focus = FocusArea::Sidebar;
                let rel_y = mouse.row as usize - sidebar_area.y as usize;
                let idx = self.sidebar_scroll + rel_y;
                let rows = self.visible_sidebar_rows();
                if idx < rows.len() {
                    self.sidebar_cursor = idx;
                    self.activate_sidebar_row(rows[idx]);
                }
                return Ok(None);
            }
        }

        // Click on content area → focus and select row
        if let Some(content_area) = self.last_content_area {
            if content_area.contains((mouse.column, mouse.row).into()) {
                self.focus = FocusArea::Content;
                let rel_y = mouse.row as usize - content_area.y as usize;
                if self.active_tab == SettingsTab::Keybindings {
                    let idx = self.kb_scroll + rel_y;
                    let names = kb_binding_names(self.kb_section);
                    if idx < names.len() {
                        self.kb_cursor = idx;
                    }
                } else {
                    let idx = self.content_scroll + rel_y;
                    let rows = self.content_rows();
                    if idx < rows.len() && rows[idx].is_selectable() {
                        self.field_cursor = idx;
                    }
                }
                return Ok(None);
            }
        }

        // Click on buttons area — determine which button was clicked
        if let Some(btn_area) = self.last_buttons_area {
            if btn_area.contains((mouse.column, mouse.row).into()) {
                self.focus = FocusArea::Buttons;
                // Calculate button positions to determine which one was clicked
                let spacing = 4;
                let labels = button_labels();
                let total_label_len: usize = labels.iter().map(|l| l.len() + 4).sum::<usize>()
                    + spacing * (labels.len().saturating_sub(1));
                let mut x = btn_area.x as usize
                    + (btn_area.width as usize).saturating_sub(total_label_len) / 2;
                for (i, label) in labels.iter().enumerate() {
                    let btn_end = x + label.len() + 4; // "[ label ]"
                    if (mouse.column as usize) >= x && (mouse.column as usize) < btn_end {
                        self.selected_button = i;
                        return self.execute_selected_button();
                    }
                    x = btn_end + spacing;
                }
                return Ok(None);
            }
        }

        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Key handling per focus area
// ---------------------------------------------------------------------------

impl SettingsModal {
    fn handle_sidebar_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<SettingsResult>>> {
        let rows = self.visible_sidebar_rows();
        if rows.is_empty() {
            return Ok(None);
        }
        // Sync cursor with current active tab on first entry.
        if self.sidebar_cursor >= rows.len() {
            self.sidebar_cursor = self.sidebar_cursor_for_active();
        }

        match key.code {
            KeyCode::Up => {
                if self.sidebar_cursor > 0 {
                    self.sidebar_cursor -= 1;
                    self.preview_sidebar_row(rows[self.sidebar_cursor]);
                }
            }
            KeyCode::Down => {
                if self.sidebar_cursor + 1 < rows.len() {
                    self.sidebar_cursor += 1;
                    self.preview_sidebar_row(rows[self.sidebar_cursor]);
                }
            }
            KeyCode::Tab => {
                // Cycle focus zones: Sidebar → Content → Buttons → Sidebar.
                self.focus = FocusArea::Content;
                self.content_scroll = 0;
                self.field_cursor = self.first_selectable_row();
            }
            KeyCode::BackTab => {
                // Reverse cycle: Sidebar → Buttons → Content → Sidebar.
                self.selected_button = 0;
                self.focus = FocusArea::Buttons;
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                let row = rows[self.sidebar_cursor];
                if matches!(row, SidebarRow::KbGroupHeader) {
                    // Toggle expansion; after toggling, refresh rows and stay on header.
                    self.activate_sidebar_row(row);
                    let new_rows = self.visible_sidebar_rows();
                    if self.keybindings_expanded {
                        // Move cursor to first child for convenience.
                        if self.sidebar_cursor + 1 < new_rows.len() {
                            self.sidebar_cursor += 1;
                            self.activate_sidebar_row(new_rows[self.sidebar_cursor]);
                        }
                    }
                } else {
                    // Leaf or KbChild — move focus to content.
                    self.activate_sidebar_row(row);
                    self.focus = FocusArea::Content;
                    self.content_scroll = 0;
                    self.field_cursor = self.first_selectable_row();
                }
            }
            KeyCode::Left => {
                // Tree-style: collapse expanded group or move from child to its header.
                // Does not change focus area.
                match rows[self.sidebar_cursor] {
                    SidebarRow::KbGroupHeader if self.keybindings_expanded => {
                        self.keybindings_expanded = false;
                    }
                    SidebarRow::KbChild(_) => {
                        let new_rows = self.visible_sidebar_rows();
                        if let Some(pos) = new_rows
                            .iter()
                            .position(|r| matches!(r, SidebarRow::KbGroupHeader))
                        {
                            self.sidebar_cursor = pos;
                        }
                    }
                    _ => {}
                }
            }
            KeyCode::Esc => {
                return Ok(Some(ModalResult::Cancelled));
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(Some(ModalResult::Confirmed(SettingsResult::Apply(
                    Box::new(self.config.clone()),
                ))));
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_content_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<SettingsResult>>> {
        // LSP server edit form mode
        if self.active_tab == SettingsTab::Lsp && self.lsp_mode == LspMode::ServerEdit {
            return self.handle_lsp_edit_key(key);
        }

        let current = self.current_row();
        let field_desc = match current {
            Some(ContentRow::Field(i)) => fields_for_tab(self.active_tab).get(i).copied(),
            _ => None,
        };

        match key.code {
            KeyCode::Up => {
                if !self.step_cursor(false) {
                    self.focus = FocusArea::Sidebar;
                }
            }
            KeyCode::Down => {
                if !self.step_cursor(true) {
                    self.selected_button = 0;
                    self.focus = FocusArea::Buttons;
                }
            }
            KeyCode::Tab => {
                self.focus = FocusArea::Buttons;
            }
            KeyCode::BackTab | KeyCode::Esc => {
                self.focus = FocusArea::Sidebar;
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(Some(ModalResult::Confirmed(SettingsResult::Apply(
                    Box::new(self.config.clone()),
                ))));
            }
            KeyCode::Enter | KeyCode::Char(' ') => match current {
                Some(ContentRow::Field(field_idx)) => {
                    if let Some(d) = field_desc {
                        match d.field_type {
                            FieldType::Bool => {
                                toggle_field(&mut self.config, self.active_tab, field_idx);
                                self.dirty = true;
                            }
                            FieldType::Enum => {
                                cycle_enum_forward(&mut self.config, self.active_tab, field_idx);
                                self.dirty = true;
                            }
                            FieldType::Number | FieldType::OptionalText => {
                                self.start_edit();
                            }
                        }
                    }
                }
                Some(ContentRow::LspAddServer) => {
                    self.lsp_edit_fields = Default::default();
                    self.lsp_edit_index = None;
                    self.lsp_edit_cursor = 0;
                    self.lsp_mode = LspMode::ServerEdit;
                }
                Some(ContentRow::LspServer(idx)) => {
                    if idx < self.lsp_server_keys.len() {
                        let lang = self.lsp_server_keys[idx].clone();
                        if let Some(srv) = self.config.lsp.servers.get(&lang) {
                            self.lsp_edit_fields = [
                                lang,
                                srv.command.clone(),
                                srv.args.join(", "),
                                srv.root_markers.join(", "),
                            ];
                            self.lsp_edit_index = Some(idx);
                            self.lsp_edit_cursor = 0;
                            self.lsp_mode = LspMode::ServerEdit;
                        }
                    }
                }
                _ => {}
            },
            KeyCode::Delete => {
                if let Some(ContentRow::LspServer(idx)) = current {
                    if idx < self.lsp_server_keys.len() {
                        let lang = self.lsp_server_keys[idx].clone();
                        self.config.lsp.servers.remove(&lang);
                        self.refresh_server_keys();
                        self.dirty = true;
                    }
                }
            }
            KeyCode::Left => {
                if let (Some(ContentRow::Field(field_idx)), Some(d)) = (current, field_desc) {
                    if d.field_type == FieldType::Enum {
                        cycle_enum_backward(&mut self.config, self.active_tab, field_idx);
                        self.dirty = true;
                    }
                }
            }
            KeyCode::Right => {
                if let (Some(ContentRow::Field(field_idx)), Some(d)) = (current, field_desc) {
                    if d.field_type == FieldType::Enum {
                        cycle_enum_forward(&mut self.config, self.active_tab, field_idx);
                        self.dirty = true;
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }

    /// Handle keys in LSP server edit form.
    fn handle_lsp_edit_key(
        &mut self,
        key: KeyEvent,
    ) -> Result<Option<ModalResult<SettingsResult>>> {
        match key.code {
            KeyCode::Esc => {
                self.lsp_mode = LspMode::Fields;
            }
            KeyCode::Enter => {
                self.commit_lsp_edit();
                self.lsp_mode = LspMode::Fields;
            }
            KeyCode::Tab => {
                self.lsp_edit_cursor = (self.lsp_edit_cursor + 1) % 4;
            }
            KeyCode::BackTab => {
                self.lsp_edit_cursor = if self.lsp_edit_cursor == 0 {
                    3
                } else {
                    self.lsp_edit_cursor - 1
                };
            }
            KeyCode::Backspace => {
                self.lsp_edit_fields[self.lsp_edit_cursor].pop();
            }
            KeyCode::Char(c) => {
                self.lsp_edit_fields[self.lsp_edit_cursor].push(c);
            }
            _ => {}
        }
        Ok(None)
    }

    /// Commit the LSP server edit form.
    fn commit_lsp_edit(&mut self) {
        let lang = self.lsp_edit_fields[0].trim().to_string();
        if lang.is_empty() {
            return;
        }

        // If editing existing, remove old key if language changed
        if let Some(idx) = self.lsp_edit_index {
            if idx < self.lsp_server_keys.len() {
                let old_lang = self.lsp_server_keys[idx].clone();
                if old_lang != lang {
                    self.config.lsp.servers.remove(&old_lang);
                }
            }
        }

        let command = self.lsp_edit_fields[1].trim().to_string();
        let args: Vec<String> = if self.lsp_edit_fields[2].trim().is_empty() {
            vec![]
        } else {
            self.lsp_edit_fields[2]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        let root_markers: Vec<String> = if self.lsp_edit_fields[3].trim().is_empty() {
            vec![]
        } else {
            self.lsp_edit_fields[3]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };

        self.config.lsp.servers.insert(
            lang,
            LspServerSettings {
                command,
                args,
                root_markers,
            },
        );
        self.refresh_server_keys();
        self.dirty = true;
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<SettingsResult>>> {
        match key.code {
            KeyCode::Enter => {
                self.commit_edit();
            }
            KeyCode::Esc => {
                self.cancel_edit();
            }
            KeyCode::Backspace => {
                self.edit_buffer.pop();
            }
            KeyCode::Char(c) => {
                if let Some(field_idx) = self.current_field_idx() {
                    let fields = fields_for_tab(self.active_tab);
                    if let Some(d) = fields.get(field_idx) {
                        if d.field_type == FieldType::Number {
                            if c.is_ascii_digit() {
                                self.edit_buffer.push(c);
                            }
                        } else {
                            self.edit_buffer.push(c);
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_buttons_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<SettingsResult>>> {
        match key.code {
            KeyCode::Left => {
                if self.selected_button > 0 {
                    self.selected_button -= 1;
                }
            }
            KeyCode::Right => {
                if self.selected_button < button_labels().len() - 1 {
                    self.selected_button += 1;
                }
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.field_cursor = self.last_selectable_row();
                self.focus = FocusArea::Content;
            }
            KeyCode::Down | KeyCode::Tab => {
                self.focus = FocusArea::Sidebar;
            }
            KeyCode::Enter => {
                return self.execute_selected_button();
            }
            KeyCode::Esc => {
                return Ok(Some(ModalResult::Cancelled));
            }
            _ => {}
        }
        Ok(None)
    }

    fn execute_selected_button(&mut self) -> Result<Option<ModalResult<SettingsResult>>> {
        match self.selected_button {
            BUTTON_APPLY => Ok(Some(ModalResult::Confirmed(SettingsResult::Apply(
                Box::new(self.config.clone()),
            )))),
            BUTTON_RESET => {
                self.config = Config::default();
                self.dirty = true;
                self.field_cursor = 0;
                self.content_scroll = 0;
                self.editing = false;
                Ok(None)
            }
            _ => Ok(Some(ModalResult::Cancelled)),
        }
    }

    fn render_keybindings(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.height == 0 || area.width < 4 {
            return;
        }

        let list_y = area.y;
        let list_h = area.height.saturating_sub(1); // leave 1 row for hint

        let names = kb_binding_names(self.kb_section);
        let visible = list_h as usize;
        if self.kb_cursor < self.kb_scroll {
            self.kb_scroll = self.kb_cursor;
        }
        if visible > 0 && self.kb_cursor >= self.kb_scroll + visible {
            self.kb_scroll = self.kb_cursor - visible + 1;
        }

        let label_width = 28.min(area.width as usize / 2);

        for row in 0..visible {
            let idx = self.kb_scroll + row;
            if idx >= names.len() {
                break;
            }
            let y = list_y + row as u16;
            let is_focused_row = self.focus == FocusArea::Content && self.kb_cursor == idx;
            let is_capturing = self.kb_mode == KbMode::Capturing && self.kb_cursor == idx;

            if is_focused_row || is_capturing {
                for x in area.x..area.x + area.width {
                    buf[(x, y)]
                        .set_style(Style::default().bg(theme.selected_bg).fg(theme.selected_fg));
                }
            }

            let name = names[idx];
            let label_style = if is_focused_row || is_capturing {
                Style::default()
                    .fg(theme.selected_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg)
            };
            let display_name = if name.len() > label_width {
                let truncated = truncate_str(name, label_width - 1);
                format!("{truncated}…")
            } else {
                name.to_string()
            };
            buf.set_string(area.x + 2, y, &display_name, label_style);

            let val_x = area.x + 2 + label_width as u16;
            if is_capturing {
                buf.set_string(
                    val_x,
                    y,
                    i18n::t().settings_kb_press_key(),
                    Style::default()
                        .fg(theme.accented_fg)
                        .add_modifier(Modifier::BOLD),
                );
            } else {
                let val = get_kb_value(&self.config, self.kb_section, name);
                let val_style = if is_focused_row {
                    Style::default().fg(theme.selected_fg)
                } else {
                    Style::default().fg(theme.accented_fg)
                };
                let max_val = (area.x + area.width).saturating_sub(val_x) as usize;
                let display_val = if val.len() > max_val && max_val > 2 {
                    format!("{}…", &val[..max_val - 1])
                } else {
                    val
                };
                buf.set_string(val_x, y, &display_val, val_style);
            }
        }

        // Hint line at the bottom of the area.
        let hint_y = area.y + area.height - 1;
        let it = i18n::t();
        let hint = match self.kb_mode {
            KbMode::Bindings => it.settings_kb_hint_bindings(),
            KbMode::Capturing => it.settings_kb_hint_capturing(),
        };
        buf.set_string(
            area.x + 2,
            hint_y,
            hint,
            Style::default().fg(theme.disabled),
        );
    }

    fn handle_keybindings_key(
        &mut self,
        key: KeyEvent,
    ) -> Result<Option<ModalResult<SettingsResult>>> {
        match self.kb_mode {
            KbMode::Bindings => {
                let names = kb_binding_names(self.kb_section);
                match key.code {
                    KeyCode::Up => {
                        if self.kb_cursor > 0 {
                            self.kb_cursor -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if self.kb_cursor < names.len().saturating_sub(1) {
                            self.kb_cursor += 1;
                        }
                    }
                    KeyCode::Enter => {
                        self.kb_mode = KbMode::Capturing;
                    }
                    KeyCode::Delete | KeyCode::Backspace => {
                        if self.kb_cursor < names.len() {
                            set_kb_value(
                                &mut self.config,
                                self.kb_section,
                                names[self.kb_cursor],
                                KeyBinding::Single(String::new()),
                            );
                            self.dirty = true;
                        }
                    }
                    KeyCode::Esc | KeyCode::BackTab => {
                        self.focus = FocusArea::Sidebar;
                    }
                    KeyCode::Tab => {
                        self.focus = FocusArea::Buttons;
                    }
                    KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(Some(ModalResult::Confirmed(SettingsResult::Apply(
                            Box::new(self.config.clone()),
                        ))));
                    }
                    _ => {}
                }
            }
            KbMode::Capturing => {
                if key.code == KeyCode::Esc || key.code == KeyCode::Tab {
                    self.kb_mode = KbMode::Bindings;
                    return Ok(None);
                }
                let binding_str = format_key_event(&key);
                if !binding_str.is_empty() {
                    let names = kb_binding_names(self.kb_section);
                    if self.kb_cursor < names.len() {
                        set_kb_value(
                            &mut self.config,
                            self.kb_section,
                            names[self.kb_cursor],
                            KeyBinding::Single(binding_str),
                        );
                        self.dirty = true;
                    }
                }
                self.kb_mode = KbMode::Bindings;
            }
        }
        Ok(None)
    }
}
