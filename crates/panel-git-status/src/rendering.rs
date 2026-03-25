//! Rendering functions for Git Status Panel.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use unicode_width::UnicodeWidthStr;

use termide_core::ThemeColors;
use termide_git::{self as git, truncate_left};
use termide_ui::ScrollBar;
use termide_ui_render::{render_simple_dropdown, InlineSelector};

use crate::types::{Section, ViewMode};
use crate::GitStatusPanel;

impl GitStatusPanel {
    /// Render the stash list view
    pub(crate) fn render_stash_view(&mut self, area: Rect, buf: &mut Buffer, is_focused: bool) {
        if area.height < 3 {
            return;
        }

        let theme = self.cached_theme;
        const MAX_VISIBLE: usize = 15;

        let header_style = Style::default().fg(theme.disabled);
        let selected_style = Style::default()
            .fg(theme.bg)
            .bg(theme.fg)
            .add_modifier(ratatui::style::Modifier::BOLD);
        let normal_style = Style::default().fg(theme.fg);
        let ref_style = Style::default().fg(theme.warning);
        let dim_style = Style::default()
            .fg(theme.disabled)
            .add_modifier(ratatui::style::Modifier::DIM);
        let hint_style = Style::default().fg(theme.info);

        let mut y = area.y;

        // Header line
        let header = format!(" Git Stash ({}) ", self.stash_entries.len());
        buf.set_string(area.x, y, &header, header_style);
        y += 1;

        // Separator
        self.render_horizontal_line(area.x, y, area.width, buf, &theme);
        y += 1;

        // Reserve last 2 rows for hint line + separator
        let hint_y = area.y + area.height - 1;
        let sep_y = hint_y - 1;
        let list_height = (sep_y.saturating_sub(y)) as usize;

        // List area
        if self.stash_entries.is_empty() {
            let msg = "  No stashes";
            buf.set_string(area.x, y, msg, dim_style);
        } else {
            let visible = list_height.min(MAX_VISIBLE);
            for row in 0..visible {
                let entry_idx = self.stash_scroll + row;
                let Some(entry) = self.stash_entries.get(entry_idx) else {
                    break;
                };
                let is_selected = entry_idx == self.stash_cursor && is_focused;

                let ref_part = format!(" {}  ", entry.ref_str);
                let msg_x = area.x + ref_part.width() as u16;
                let remaining = area.width.saturating_sub(ref_part.width() as u16) as usize;
                if is_selected {
                    // Fill entire row with selection background
                    for dx in 0..area.width {
                        buf[(area.x + dx, y)]
                            .set_symbol(" ")
                            .set_style(selected_style);
                    }
                    buf.set_string(area.x, y, &ref_part, selected_style);
                    if remaining > 0 {
                        let msg = git::truncate_right(&entry.message, remaining);
                        buf.set_string(msg_x, y, &msg, selected_style);
                    }
                } else {
                    buf.set_string(area.x, y, &ref_part, ref_style);
                    if remaining > 0 {
                        let msg = git::truncate_right(&entry.message, remaining);
                        buf.set_string(msg_x, y, &msg, normal_style);
                    }
                }
                y += 1;
            }
        }

        // Separator above hints
        self.render_horizontal_line(area.x, sep_y, area.width, buf, &theme);

        // Hints row
        buf.set_string(
            area.x,
            hint_y,
            " [N]ew [P]op [A]pply [D]rop [Enter]Diff [Esc]Back",
            hint_style,
        );
    }

    /// Render the main content area
    pub(crate) fn render_content(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        is_focused: bool,
        border_right_x: Option<u16>,
    ) {
        if area.height < 5 {
            return;
        }

        // Dispatch to stash view when in stash mode
        if self.view_mode == ViewMode::Stash {
            self.render_stash_view(area, buf, is_focused);
            return;
        }

        let theme = self.cached_theme;
        let content_area = area;

        // Layout constants
        let selector_height: u16 = 1;
        let separator_height: u16 = 1;
        let buttons_height = self.calc_buttons_height(content_area.width);
        let fixed_height = selector_height + separator_height + buttons_height;
        let files_area_height = content_area.height.saturating_sub(fixed_height) as usize;

        // Cache viewport height for scroll calculations
        self.viewport_height = files_area_height;

        // Virtual content layout
        let unstaged_header_line = 0;
        let unstaged_files_start = 1;
        let unstaged_files_end = unstaged_files_start + self.unstaged_item_count();
        let staged_header_line = unstaged_files_end;
        let staged_files_start = staged_header_line + 1;
        let total_virtual_lines = self.total_virtual_lines();

        // Clamp scroll offset
        let max_scroll = total_virtual_lines.saturating_sub(files_area_height);
        if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }

        let mut y = content_area.y;

        // === TOP ZONE: Selectors ===
        self.selector_y = y;

        let t = termide_i18n::t();

        let repo_name = self
            .repo_manager
            .current()
            .map(git::get_repo_name)
            .unwrap_or_else(|| t.git_no_repo().to_string());
        let repo_focused = self.current_section == Section::RepoSelector && is_focused;
        let repo_selector =
            InlineSelector::new(&repo_name, self.repo_dropdown_open, repo_focused, &theme);
        let repo_width = repo_selector.render(content_area.x, y, content_area.width / 2, buf);

        let branch_name = self
            .branch
            .clone()
            .unwrap_or_else(|| t.git_branch_detached().to_string());
        let branch_focused = self.current_section == Section::BranchSelector && is_focused;
        let branch_x = content_area.x + repo_width + 2;
        self.branch_selector_x = branch_x;
        let branch_max_width = content_area.width.saturating_sub(repo_width + 2);
        let branch_selector = InlineSelector::new(
            &branch_name,
            self.branch_dropdown_open,
            branch_focused,
            &theme,
        );
        branch_selector.render(branch_x, y, branch_max_width, buf);

        y += selector_height;

        // === MIDDLE ZONE: Files area (unified scroll) ===
        let files_y = y;
        let files_width = content_area.width;

        // Store files area for mouse handling
        self.files_area = Rect {
            x: content_area.x,
            y: files_y,
            width: files_width,
            height: files_area_height as u16,
        };

        let files_active = self.current_section == Section::Files && is_focused;

        // Render visible virtual lines
        for screen_row in 0..files_area_height {
            let vline = self.scroll_offset + screen_row;
            if vline >= total_virtual_lines {
                break;
            }
            let line_y = files_y + screen_row as u16;

            if vline == unstaged_header_line {
                self.render_unstaged_header(
                    self.cursor == vline && files_active,
                    content_area.x,
                    line_y,
                    files_width,
                    buf,
                    &theme,
                );
            } else if vline >= unstaged_files_start && vline < unstaged_files_end {
                let item_idx = vline - unstaged_files_start;
                let is_selected = self.cursor == vline && files_active;
                self.render_tree_node_line(
                    true,
                    item_idx,
                    is_selected,
                    content_area.x,
                    line_y,
                    files_width,
                    buf,
                    &theme,
                    files_active,
                );
            } else if vline == staged_header_line {
                self.render_staged_header(
                    self.cursor == vline && files_active,
                    content_area.x,
                    line_y,
                    files_width,
                    buf,
                    &theme,
                );
            } else if vline >= staged_files_start {
                let item_idx = vline - staged_files_start;
                let is_selected = self.cursor == vline && files_active;
                self.render_tree_node_line(
                    false,
                    item_idx,
                    is_selected,
                    content_area.x,
                    line_y,
                    files_width,
                    buf,
                    &theme,
                    files_active,
                );
            }
        }

        // Single scrollbar for entire files area
        if let Some(border_x) = border_right_x {
            ScrollBar::render(
                buf,
                border_x,
                files_y,
                files_area_height as u16,
                self.scroll_offset,
                files_area_height,
                total_virtual_lines,
                &theme,
                files_active,
            );
        }

        // === STICKY HEADERS ===
        // When a section header scrolls out of view, render it at the top of files area
        // so user always knows which section they're viewing

        // Staged header is sticky if we've scrolled past it (into staged files only)
        let staged_sticky =
            self.scroll_offset > staged_header_line && !self.staged_files.is_empty();

        // Unstaged header is sticky if scrolled past line 0, but NOT if staged is sticky
        let unstaged_sticky = self.scroll_offset > unstaged_header_line
            && !self.unstaged_files.is_empty()
            && !staged_sticky;

        if unstaged_sticky {
            self.render_unstaged_header(
                self.cursor == unstaged_header_line && files_active,
                content_area.x,
                files_y,
                files_width,
                buf,
                &theme,
            );
        }

        if staged_sticky {
            self.render_staged_header(
                self.cursor == staged_header_line && files_active,
                content_area.x,
                files_y,
                files_width,
                buf,
                &theme,
            );
        }

        y += files_area_height as u16;

        // Separator before buttons
        self.render_horizontal_line(content_area.x, y, content_area.width, buf, &theme);
        y += separator_height;

        // === BOTTOM ZONE: Buttons ===
        self.buttons_y = y;
        self.cached_buttons_height = buttons_height;
        self.render_buttons(
            content_area.x,
            y,
            content_area.width,
            buf,
            &theme,
            is_focused,
        );

        // === DROPDOWNS (rendered last to overlay) ===
        if self.repo_dropdown_open {
            let dropdown_y = content_area.y + 1;
            let max_dropdown_height = content_area.height.saturating_sub(3) as usize;
            let repo_names: Vec<String> = self
                .repo_manager
                .repos()
                .iter()
                .map(|p| git::get_repo_name(p))
                .collect();
            let visible_count = repo_names.len().min(max_dropdown_height);
            let scroll_offset = if self.dropdown_cursor >= visible_count {
                self.dropdown_cursor - visible_count + 1
            } else {
                0
            };
            self.dropdown_scroll = scroll_offset;
            self.repo_dropdown_area = Some(Rect {
                x: content_area.x,
                y: dropdown_y,
                width: content_area.width / 2,
                height: visible_count as u16 + 2,
            });
            render_simple_dropdown(
                &repo_names,
                self.repo_manager.selected_index(),
                self.dropdown_cursor,
                content_area.x,
                dropdown_y,
                content_area.width / 2,
                max_dropdown_height as u16,
                buf,
                &theme,
            );
        } else {
            self.repo_dropdown_area = None;
        }
        if self.branch_dropdown_open {
            let dropdown_y = content_area.y + 1;
            let max_dropdown_height = content_area.height.saturating_sub(3) as usize;
            let current_branch_idx = self
                .branches
                .iter()
                .position(|b| Some(b.as_str()) == self.branch.as_deref())
                .unwrap_or(0);
            let visible_count = self.branches.len().min(max_dropdown_height);
            let scroll_offset = if self.dropdown_cursor >= visible_count {
                self.dropdown_cursor - visible_count + 1
            } else {
                0
            };
            self.dropdown_scroll = scroll_offset;
            self.branch_dropdown_area = Some(Rect {
                x: branch_x,
                y: dropdown_y,
                width: branch_max_width,
                height: visible_count as u16 + 2,
            });
            render_simple_dropdown(
                &self.branches,
                current_branch_idx,
                self.dropdown_cursor,
                branch_x,
                dropdown_y,
                branch_max_width,
                max_dropdown_height as u16,
                buf,
                &theme,
            );
        } else {
            self.branch_dropdown_area = None;
        }
    }

    /// Render section header with optional button selection highlighting
    /// Render the unstaged files section header, updating `stage_all_btn_area`.
    fn render_unstaged_header(
        &mut self,
        is_selected: bool,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
    ) {
        let t = termide_i18n::t();
        let title = format!(
            "{} ({})",
            t.git_unstaged_header(),
            self.unstaged_files.len()
        );
        let btn_str = format!("[{}]", t.git_stage_all_btn());
        let btn = if !self.unstaged_files.is_empty() {
            Some(btn_str.as_str())
        } else {
            None
        };
        self.stage_all_btn_area =
            self.render_section_header_simple(&title, btn, is_selected, x, y, width, buf, theme);
    }

    /// Render the staged files section header, updating `unstage_all_btn_area`.
    fn render_staged_header(
        &mut self,
        is_selected: bool,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
    ) {
        let t = termide_i18n::t();
        let title = format!("{} ({})", t.git_staged_header(), self.staged_files.len());
        let btn_str = format!("[{}]", t.git_unstage_all_btn());
        let btn = if !self.staged_files.is_empty() {
            Some(btn_str.as_str())
        } else {
            None
        };
        self.unstage_all_btn_area =
            self.render_section_header_simple(&title, btn, is_selected, x, y, width, buf, theme);
    }

    pub(crate) fn render_section_header_simple(
        &self,
        title: &str,
        action_btn: Option<&str>,
        is_selected: bool,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
    ) -> Option<Rect> {
        let header_style = Style::default().fg(theme.disabled);

        // Draw line with embedded title
        let title_with_space = format!(" {} ", title);
        let title_width = title_with_space.width();

        // Left part of line
        buf.set_string(x, y, "─", header_style);

        // Title
        buf.set_string(x + 1, y, &title_with_space, header_style);

        // Rest of line (or action button)
        let after_title = x + 1 + title_width as u16;
        let remaining = width.saturating_sub(1 + title_width as u16);

        if let Some(btn_text) = action_btn {
            let btn_width = btn_text.width() as u16;
            if remaining > btn_width + 2 {
                // Line before button
                let line_width = remaining - btn_width - 1;
                for dx in 0..line_width {
                    buf.set_string(after_title + dx, y, "─", header_style);
                }
                // Button - inverted style when selected
                let btn_x = after_title + line_width;
                let btn_style = if is_selected {
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg)
                };
                buf.set_string(btn_x, y, btn_text, btn_style);
                Some(Rect {
                    x: btn_x,
                    y,
                    width: btn_width,
                    height: 1,
                })
            } else {
                for dx in 0..remaining {
                    buf.set_string(after_title + dx, y, "─", header_style);
                }
                None
            }
        } else {
            for dx in 0..remaining {
                buf.set_string(after_title + dx, y, "─", header_style);
            }
            None
        }
    }

    /// Get color and modifier for file status
    pub(crate) fn get_file_style(
        status: char,
        untracked: bool,
        theme: &ThemeColors,
    ) -> (Color, Modifier) {
        if untracked {
            (theme.success, Modifier::empty())
        } else {
            match status {
                'M' => (theme.warning, Modifier::empty()),
                'D' => (theme.error, Modifier::CROSSED_OUT),
                'A' | 'R' => (theme.success, Modifier::empty()),
                _ => (theme.fg, Modifier::empty()),
            }
        }
    }

    /// Render a tree node line (directory or file) in tree view mode
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_tree_node_line(
        &self,
        is_unstaged: bool,
        visible_idx: usize,
        is_selected: bool,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
        is_focused: bool,
    ) {
        let ft = if is_unstaged {
            &self.unstaged
        } else {
            &self.staged
        };
        let (tree_nodes, visible, prefixes) = (&ft.tree, &ft.visible, &ft.prefixes);

        let Some(&tree_idx) = visible.get(visible_idx) else {
            return;
        };
        let node = &tree_nodes[tree_idx];
        let prefix = prefixes.get(visible_idx).map(|s| s.as_str()).unwrap_or("");

        // Determine style based on node kind
        let (fg_color, extra_modifier, label) = match node.kind {
            crate::tree::TreeNodeKind::Directory { expanded } => {
                let (status, untracked) = crate::tree::aggregate_dir_status(tree_nodes, tree_idx);
                let (color, _modifier) = Self::get_file_style(status, untracked, theme);
                const DIR_COLLAPSED: &str = if cfg!(windows) { "►" } else { "▶" };
                let arrow = if expanded { "▼" } else { DIR_COLLAPSED };
                (
                    color,
                    Modifier::empty(),
                    format!("{} /{}", arrow, node.label),
                )
            }
            crate::tree::TreeNodeKind::File {
                status, untracked, ..
            } => {
                let (color, modifier) = Self::get_file_style(status, untracked, theme);
                (color, modifier, node.label.clone())
            }
        };

        // Style for the file label (with CROSSED_OUT for deleted files)
        let label_style = if is_selected && is_focused {
            Style::default()
                .fg(theme.bg)
                .bg(fg_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(fg_color).add_modifier(extra_modifier)
        };

        // Style for the tree prefix (no CROSSED_OUT, just color)
        let prefix_style = if is_selected && is_focused {
            label_style // selection style has no CROSSED_OUT
        } else {
            Style::default().fg(fg_color)
        };

        // Fill background when selected
        if is_selected && is_focused {
            for dx in 0..width {
                buf[(x + dx, y)].set_symbol(" ").set_style(label_style);
            }
        }

        // Render prefix and label separately so CROSSED_OUT only applies to file name
        let prefix_part = format!(" {}", prefix);
        let prefix_len = prefix_part.width() as u16;
        buf.set_string(x, y, &prefix_part, prefix_style);

        // File name — with strikethrough if deleted
        let remaining = width.saturating_sub(prefix_len) as usize;
        if remaining > 0 {
            let truncated_label = truncate_left(&label, remaining);
            buf.set_string(x + prefix_len, y, &truncated_label, label_style);
        }
    }

    /// Calculate how many rows the buttons need at the given width.
    pub(crate) fn calc_buttons_height(&self, width: u16) -> u16 {
        let buttons = self.get_visible_buttons();
        if buttons.is_empty() {
            return 1;
        }
        let mut current_x: u16 = 0;
        let mut rows: u16 = 1;
        for button in &buttons {
            let label = format!("[{}]", button.label(self.spinner_frame));
            let w = label.width() as u16;
            if current_x > 0 && current_x + w > width {
                rows += 1;
                current_x = w + 1;
            } else {
                current_x += w + 1;
            }
        }
        rows
    }

    /// Render action buttons
    pub(crate) fn render_buttons(
        &self,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
        is_focused: bool,
    ) {
        let buttons = self.get_visible_buttons();
        let mut current_x = x;
        let mut current_y = y;

        for (i, button) in buttons.iter().enumerate() {
            let is_selected = self.current_section == Section::Buttons && i == self.selected_button;
            let label = format!("[{}]", button.label(self.spinner_frame));

            let style = if is_selected && is_focused {
                // Inverted cursor style - only when focused
                Style::default()
                    .fg(theme.bg)
                    .bg(theme.fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg)
            };

            let lw = label.width() as u16;
            if current_x > x && current_x + lw > x + width {
                current_y += 1;
                current_x = x;
            }

            buf.set_string(current_x, current_y, &label, style);
            current_x += lw + 1;
        }
    }

    /// Render a horizontal line separator
    pub(crate) fn render_horizontal_line(
        &self,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
    ) {
        let style = Style::default().fg(theme.border);
        for i in 0..width {
            buf[(x + i, y)].set_symbol("─").set_style(style);
        }
    }

    /// Check if coordinates are within a rect
    pub(crate) fn is_in_rect(&self, col: u16, row: u16, rect: Rect) -> bool {
        col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
    }
}
