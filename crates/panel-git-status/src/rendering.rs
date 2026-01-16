//! Rendering functions for Git Status Panel.

use std::path::Path;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use unicode_width::UnicodeWidthStr;

use termide_core::ThemeColors;
use termide_git::{self as git, truncate_path_left};
use termide_ui::ScrollBar;
use termide_ui_render::InlineSelector;

use crate::types::Section;
use crate::GitStatusPanel;

impl GitStatusPanel {
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

        let theme = self.cached_theme.clone();
        let content_area = area;

        // Layout constants
        let selector_height: u16 = 1;
        let separator_height: u16 = 1;
        let buttons_height: u16 = 1;
        let fixed_height = selector_height + separator_height + buttons_height;
        let files_area_height = content_area.height.saturating_sub(fixed_height) as usize;

        // Cache viewport height for scroll calculations
        self.viewport_height = files_area_height;

        // Virtual content layout
        let unstaged_header_line = 0;
        let unstaged_files_start = 1;
        let unstaged_files_end = unstaged_files_start + self.unstaged_files.len();
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

        let repo_name = self
            .repo_manager
            .current()
            .map(git::get_repo_name)
            .unwrap_or_else(|| "No repo".to_string());
        let repo_focused = self.current_section == Section::RepoSelector && is_focused;
        let repo_selector =
            InlineSelector::new(&repo_name, self.repo_dropdown_open, repo_focused, &theme);
        let repo_width = repo_selector.render(content_area.x, y, content_area.width / 2, buf);

        let branch_name = self
            .branch
            .clone()
            .unwrap_or_else(|| "(detached)".to_string());
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
                // Unstaged header
                let title = format!("Unstaged ({})", self.unstaged_files.len());
                let btn = if !self.unstaged_files.is_empty() {
                    Some("[Stage all]")
                } else {
                    None
                };
                let is_selected = self.cursor == vline && files_active;
                self.stage_all_btn_area = self.render_section_header_simple(
                    &title,
                    btn,
                    is_selected,
                    content_area.x,
                    line_y,
                    files_width,
                    buf,
                    &theme,
                );
            } else if vline >= unstaged_files_start && vline < unstaged_files_end {
                // Unstaged file
                let file_idx = vline - unstaged_files_start;
                let is_selected = self.cursor == vline && files_active;
                self.render_unstaged_file_line(
                    file_idx,
                    is_selected,
                    content_area.x,
                    line_y,
                    files_width,
                    buf,
                    &theme,
                    files_active,
                );
            } else if vline == staged_header_line {
                // Staged header
                let title = format!("Staged ({})", self.staged_files.len());
                let btn = if !self.staged_files.is_empty() {
                    Some("[Unstage all]")
                } else {
                    None
                };
                let is_selected = self.cursor == vline && files_active;
                self.unstage_all_btn_area = self.render_section_header_simple(
                    &title,
                    btn,
                    is_selected,
                    content_area.x,
                    line_y,
                    files_width,
                    buf,
                    &theme,
                );
            } else if vline >= staged_files_start {
                // Staged file
                let file_idx = vline - staged_files_start;
                let is_selected = self.cursor == vline && files_active;
                self.render_staged_file_line(
                    file_idx,
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
            let title = format!("Unstaged ({})", self.unstaged_files.len());
            let btn = if !self.unstaged_files.is_empty() {
                Some("[Stage all]")
            } else {
                None
            };
            let is_selected = self.cursor == unstaged_header_line && files_active;
            self.stage_all_btn_area = self.render_section_header_simple(
                &title,
                btn,
                is_selected,
                content_area.x,
                files_y,
                files_width,
                buf,
                &theme,
            );
        }

        if staged_sticky {
            let title = format!("Staged ({})", self.staged_files.len());
            let btn = if !self.staged_files.is_empty() {
                Some("[Unstage all]")
            } else {
                None
            };
            let is_selected = self.cursor == staged_header_line && files_active;
            self.unstage_all_btn_area = self.render_section_header_simple(
                &title,
                btn,
                is_selected,
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
            self.render_dropdown_list(
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
            self.render_dropdown_list(
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

    /// Render a single file line (works for both staged and unstaged files)
    pub(crate) fn render_file_line(
        path: &Path,
        status: char,
        untracked: bool,
        is_selected: bool,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
        is_focused: bool,
    ) {
        let (fg_color, extra_modifier) = Self::get_file_style(status, untracked, theme);

        let style = if is_selected && is_focused {
            Style::default()
                .fg(theme.bg)
                .bg(fg_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(fg_color).add_modifier(extra_modifier)
        };

        if is_selected && is_focused {
            for dx in 0..width {
                buf[(x + dx, y)].set_symbol(" ").set_style(style);
            }
        }

        let path_str = path.to_string_lossy();
        let line = format!(" {}", path_str);
        let truncated = truncate_path_left(&line, width as usize);
        buf.set_string(x, y, &truncated, style);
    }

    /// Render a single unstaged file line
    pub(crate) fn render_unstaged_file_line(
        &self,
        file_idx: usize,
        is_selected: bool,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
        is_focused: bool,
    ) {
        if let Some(file) = self.unstaged_files.get(file_idx) {
            Self::render_file_line(
                &file.path,
                file.status,
                file.untracked,
                is_selected,
                x,
                y,
                width,
                buf,
                theme,
                is_focused,
            );
        }
    }

    /// Render a single staged file line
    pub(crate) fn render_staged_file_line(
        &self,
        file_idx: usize,
        is_selected: bool,
        x: u16,
        y: u16,
        width: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
        is_focused: bool,
    ) {
        if let Some(file) = self.staged_files.get(file_idx) {
            Self::render_file_line(
                &file.path,
                file.status,
                false, // staged files are never untracked
                is_selected,
                x,
                y,
                width,
                buf,
                theme,
                is_focused,
            );
        }
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

            if current_x + label.width() as u16 > x + width {
                // Wrap to next line (not implemented for simplicity)
                break;
            }

            buf.set_string(current_x, y, &label, style);
            current_x += label.width() as u16 + 1;
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

    /// Render dropdown list overlay
    pub(crate) fn render_dropdown_list(
        &self,
        items: &[String],
        selected: usize,
        cursor: usize,
        x: u16,
        y: u16,
        max_width: u16,
        max_height: u16,
        buf: &mut Buffer,
        theme: &ThemeColors,
    ) {
        if items.is_empty() {
            return;
        }

        let visible_count = items.len().min(max_height as usize);
        let scroll_offset = if cursor >= visible_count {
            cursor - visible_count + 1
        } else {
            0
        };

        // Calculate dropdown width
        let item_max_width = items.iter().map(|s| s.width()).max().unwrap_or(10);
        let width = (item_max_width + 4).min(max_width as usize) as u16;

        // Border style
        let border_style = Style::default().fg(theme.border_focused);
        let bg_style = Style::default().bg(theme.bg);

        // Clear area and draw border
        let dropdown_height = visible_count as u16 + 2; // +2 for borders
        for dy in 0..dropdown_height {
            for dx in 0..width {
                let cell = &mut buf[(x + dx, y + dy)];
                cell.set_style(bg_style);
                if dy == 0 || dy == dropdown_height - 1 {
                    if dx == 0 {
                        cell.set_symbol(if dy == 0 { "┌" } else { "└" });
                    } else if dx == width - 1 {
                        cell.set_symbol(if dy == 0 { "┐" } else { "┘" });
                    } else {
                        cell.set_symbol("─");
                    }
                    cell.set_style(border_style);
                } else if dx == 0 || dx == width - 1 {
                    cell.set_symbol("│").set_style(border_style);
                } else {
                    cell.set_symbol(" ");
                }
            }
        }

        // Draw items
        for (i, item) in items
            .iter()
            .skip(scroll_offset)
            .take(visible_count)
            .enumerate()
        {
            let item_y = y + 1 + i as u16;
            let is_cursor = scroll_offset + i == cursor;
            let is_selected = scroll_offset + i == selected;

            let style = if is_cursor {
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.selection_bg)
            } else if is_selected {
                Style::default().fg(theme.cursor)
            } else {
                Style::default().fg(theme.fg)
            };

            // Truncate item
            let max_item_width = (width - 2) as usize;
            let display_item = if item.width() > max_item_width {
                &item[..max_item_width]
            } else {
                item
            };

            // Clear line and draw item
            for dx in 1..width - 1 {
                buf[(x + dx, item_y)].set_symbol(" ").set_style(style);
            }
            buf.set_string(x + 1, item_y, display_item, style);
        }
    }

    /// Check if coordinates are within a rect
    pub(crate) fn is_in_rect(&self, col: u16, row: u16, rect: Rect) -> bool {
        col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
    }
}
