use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::{file_search::FileSearchMode, utils, FileManager};
use termide_config::FileManagerSettings;
use termide_git::GitStatus;
use termide_theme::Theme;
use termide_ui::grapheme_utils::{prepare_matched_line, truncate_from_start};

/// Pre-allocated spaces for padding (avoids `.repeat()` allocations in render loops).
const PAD: &str = "                                                                                                                                                                                                        ";

/// Get style for git status (extracted to avoid duplication)
fn git_status_style(status: GitStatus, theme: &Theme) -> Style {
    match status {
        GitStatus::Ignored => Style::default()
            .fg(theme.disabled)
            .add_modifier(Modifier::DIM),
        GitStatus::Modified => Style::default().fg(theme.warning),
        GitStatus::Added => Style::default().fg(theme.success),
        GitStatus::Deleted => Style::default().fg(theme.error),
        GitStatus::Unmodified => Style::default().fg(theme.fg),
    }
}

// On Windows, U+25B6 ▶ and U+25B7 ▷ are outside WGL4 and render as tofu squares.
// U+25BA ► is in WGL4 and displays correctly on all Windows console fonts.
const DIR_COLLAPSED: &str = if cfg!(windows) { "►" } else { "▶" };
const DIR_COLLAPSED_SYMLINK: &str = if cfg!(windows) { "►" } else { "▷" };
const DIR_EXPANDED_SYMLINK: &str = if cfg!(windows) { "▼" } else { "▽" };
const DIR_COLLAPSED_SPACE: &str = if cfg!(windows) { "► " } else { "▶ " };
const DIR_COLLAPSED_SLASH: &str = if cfg!(windows) { "► /" } else { "▶ /" };

/// Get icon for entry, accounting for expand/collapse state.
fn get_icon(entry: &super::FileEntry, expanded: Option<bool>) -> &'static str {
    if entry.git_status == GitStatus::Deleted {
        return "✗";
    }
    if entry.name == ".." {
        return "↑";
    }
    if entry.is_dir {
        return match expanded {
            Some(true) => {
                if entry.is_symlink {
                    DIR_EXPANDED_SYMLINK
                } else {
                    "▼"
                }
            }
            Some(false) | None => {
                if entry.is_symlink {
                    DIR_COLLAPSED_SYMLINK
                } else {
                    DIR_COLLAPSED
                }
            }
        };
    }
    " "
}

impl FileManager {
    /// Get list of lines for display
    pub(crate) fn get_items(
        &self,
        height: usize,
        available_width: usize,
        theme: &Theme,
        is_focused: bool,
        config: &FileManagerSettings,
    ) -> Vec<Line<'_>> {
        let mut lines = Vec::new();
        let visible_start = self.scroll_offset;
        let visible_end = visible_start + height;

        // Constants for extended mode
        const SIZE_COLUMN_WIDTH: usize = 10;
        const TIME_COLUMN_WIDTH: usize = 19;
        const SEPARATOR: &str = " │ ";
        const SEPARATOR_WIDTH: usize = 3;
        // Determine whether to show extended view with columns
        let show_extended = available_width >= config.extended_view_width;

        for (vis_i, &tree_idx) in self.visible_indices.iter().enumerate() {
            if vis_i < visible_start || vis_i >= visible_end {
                continue;
            }

            let tree_entry = &self.tree_entries[tree_idx];
            let entry = &tree_entry.file_entry;
            let tree_prefix = &self.tree_prefixes[vis_i];
            let tree_prefix_width = tree_prefix.width();

            let is_selected = self.selection.is_selected(vis_i);
            let is_cursor = vis_i == self.selected;

            let attr = utils::get_attribute(entry, is_selected);
            let icon = get_icon(entry, tree_entry.expanded);
            let attr_width = 1; // always 1 character
            let icon_width = 1; // always 1 character
            let dir_prefix = if entry.is_dir && entry.name != ".." {
                "/"
            } else {
                ""
            };
            let prefix_width = dir_prefix.width();

            // Calculate maximum visual width of name WITHOUT prefix, considering display mode
            let max_name_len = if show_extended {
                available_width.saturating_sub(
                    tree_prefix_width
                        + attr_width
                        + icon_width
                        + 1
                        + prefix_width
                        + SEPARATOR_WIDTH
                        + SIZE_COLUMN_WIDTH
                        + SEPARATOR_WIDTH
                        + TIME_COLUMN_WIDTH,
                )
            } else {
                available_width
                    .saturating_sub(tree_prefix_width + attr_width + icon_width + 1 + prefix_width)
            };

            let name = utils::truncate_name(&entry.name, max_name_len);
            let name_width = name.width();
            let full_name = format!("{}{}", dir_prefix, name);

            let (bg_style, fg_style) = if is_cursor && is_focused {
                let normal_fg_style = git_status_style(entry.git_status, theme);
                let fg_color = normal_fg_style.fg.unwrap_or(theme.fg);
                let cursor_style = Style::default()
                    .bg(fg_color)
                    .fg(theme.bg)
                    .add_modifier(Modifier::BOLD);
                (cursor_style, cursor_style)
            } else {
                let fg_style = git_status_style(entry.git_status, theme);
                (Style::default(), fg_style)
            };

            let attr_style = if is_selected {
                Style::default()
                    .fg(theme.accented_fg)
                    .add_modifier(Modifier::BOLD)
            } else if attr == "R" {
                Style::default().fg(theme.disabled)
            } else {
                fg_style
            };

            let fg_style = if is_selected && !(is_cursor && is_focused) {
                Style::default()
                    .fg(theme.accented_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                fg_style
            };

            let icon_style = if entry.git_status == GitStatus::Deleted {
                Style::default().fg(theme.error)
            } else {
                fg_style
            };

            let name_style = {
                let mut style = fg_style;
                if entry.git_status == GitStatus::Deleted && !(is_cursor && is_focused) {
                    style = style.add_modifier(Modifier::CROSSED_OUT);
                }
                if !entry.is_dir && entry.is_symlink {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                if !entry.is_dir && entry.is_executable {
                    style = style.add_modifier(Modifier::BOLD);
                }
                style
            };

            // Tree prefix style: dimmed connectors
            let prefix_style = if is_cursor && is_focused {
                bg_style
            } else {
                Style::default().fg(theme.disabled)
            };

            if show_extended {
                let padding_len = max_name_len.saturating_sub(name_width);
                let padding = &PAD[..padding_len.min(PAD.len())];

                let size_str: std::borrow::Cow<'static, str> = if let Some(size) = entry.size {
                    format!("{:>10}", utils::format_size_compact(size)).into()
                } else if entry.is_dir
                    && entry.name != ".."
                    && !self.is_remote()
                    && config.dir_size_in_wide_view
                    && config.dir_size_budget_ms > 0
                {
                    match self.dir_size_cache.get(&tree_entry.full_path) {
                        Some(outcome) if outcome.overflowed => {
                            std::borrow::Cow::Borrowed("         -")
                        }
                        Some(outcome) => {
                            format!("{:>10}", utils::format_size_compact(outcome.size)).into()
                        }
                        None => std::borrow::Cow::Borrowed("          "),
                    }
                } else {
                    std::borrow::Cow::Borrowed("          ")
                };

                let time_str = utils::format_modified_time(entry.modified);

                let mut spans = Vec::with_capacity(10);
                if !tree_prefix.is_empty() {
                    spans.push(Span::styled(tree_prefix.as_str(), prefix_style));
                }
                spans.extend([
                    Span::styled(attr, attr_style),
                    Span::styled(icon, icon_style),
                    Span::styled(" ", bg_style),
                    Span::styled(full_name, name_style),
                    Span::styled(padding, bg_style),
                    Span::styled(SEPARATOR, bg_style.fg(theme.disabled)),
                    Span::styled(size_str, fg_style),
                    Span::styled(SEPARATOR, bg_style.fg(theme.disabled)),
                    Span::styled(time_str, fg_style),
                ]);
                lines.push(Line::from(spans));
            } else {
                let content_width =
                    tree_prefix_width + attr_width + icon_width + 1 + prefix_width + name_width;
                let padding_len = available_width.saturating_sub(content_width);
                let padding = &PAD[..padding_len.min(PAD.len())];

                let mut spans = Vec::with_capacity(6);
                if !tree_prefix.is_empty() {
                    spans.push(Span::styled(tree_prefix.as_str(), prefix_style));
                }
                spans.extend([
                    Span::styled(attr, attr_style),
                    Span::styled(icon, icon_style),
                    Span::styled(" ", bg_style),
                    Span::styled(full_name, name_style),
                    Span::styled(padding, bg_style),
                ]);
                lines.push(Line::from(spans));
            }
        }

        // Fill remaining space with empty lines (with separators in extended mode)
        if show_extended && lines.len() < height {
            let name_column_width = available_width.saturating_sub(
                SEPARATOR_WIDTH + SIZE_COLUMN_WIDTH + SEPARATOR_WIDTH + TIME_COLUMN_WIDTH,
            );
            let empty_name = &PAD[..name_column_width.min(PAD.len())];
            let empty_size = &PAD[..SIZE_COLUMN_WIDTH.min(PAD.len())];
            let empty_time = &PAD[..TIME_COLUMN_WIDTH.min(PAD.len())];
            let separator_style = Style::default().fg(theme.disabled);

            for _ in lines.len()..height {
                lines.push(Line::from(vec![
                    Span::raw(empty_name),
                    Span::styled(SEPARATOR, separator_style),
                    Span::raw(empty_size),
                    Span::styled(SEPARATOR, separator_style),
                    Span::raw(empty_time),
                ]));
            }
        }

        lines
    }

    /// Render search results instead of normal file tree
    pub(crate) fn render_search_results(
        &self,
        area: Rect,
        buf: &mut Buffer,
        search: &super::file_search::FileSearchState,
        theme: &Theme,
    ) {
        if search.is_searching {
            let style = Style::default()
                .fg(theme.accented_bg)
                .add_modifier(Modifier::DIM);
            buf.set_string(area.x, area.y, "Searching…", style);
            return;
        }

        if search.tree_nodes.is_empty() {
            let style = Style::default()
                .fg(theme.accented_bg)
                .add_modifier(Modifier::DIM);
            buf.set_string(area.x, area.y, "No matches found", style);
            return;
        }

        match search.mode {
            FileSearchMode::FileGlob => {
                self.render_file_search_results(area, buf, search, theme);
            }
            FileSearchMode::Content => {
                self.render_content_search_results(area, buf, search, theme);
            }
        }
    }

    fn render_file_search_results(
        &self,
        area: Rect,
        buf: &mut Buffer,
        search: &super::file_search::FileSearchState,
        theme: &Theme,
    ) {
        let max_lines = area.height as usize;
        let content_width = area.width as usize;

        for (vis_idx, (idx, node)) in search
            .tree_nodes
            .iter()
            .enumerate()
            .skip(search.scroll_offset)
            .enumerate()
        {
            if vis_idx >= max_lines {
                break;
            }

            let is_selected = idx == search.cursor;
            let prefix = &search.tree_prefixes[idx];

            let style = if is_selected {
                Style::default()
                    .fg(theme.bg)
                    .bg(theme.accented_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                git_status_style(node.git_status, theme)
            };

            let prefix_style = if is_selected {
                style
            } else {
                Style::default().fg(theme.disabled)
            };

            let icon_text = if node.is_dir { DIR_COLLAPSED_SPACE } else { "" };

            let mut spans = Vec::new();
            if !prefix.is_empty() {
                spans.push(Span::styled(prefix.clone(), prefix_style));
            }
            spans.push(Span::styled(icon_text, style));
            if node.is_dir {
                spans.push(Span::styled("/", style));
            }
            spans.push(Span::styled(node.name.as_str(), style));

            let text_width: usize = spans.iter().map(|s| s.content.width()).sum();
            let padding_len = content_width.saturating_sub(text_width);
            if padding_len > 0 {
                spans.push(Span::styled(&PAD[..padding_len.min(PAD.len())], style));
            }

            let y = area.y + vis_idx as u16;
            buf.set_line(area.x, y, &Line::from(spans), area.width);
        }
    }

    fn render_content_search_results(
        &self,
        area: Rect,
        buf: &mut Buffer,
        search: &super::file_search::FileSearchState,
        theme: &Theme,
    ) {
        let max_lines = area.height as usize;
        let content_width = area.width as usize;
        let mut y = area.y;
        let mut lines_rendered = 0;

        let editor_bg = theme.bg;
        let line_num_style = Style::default().fg(theme.disabled).bg(editor_bg);
        let context_text_style = Style::default().fg(theme.fg).bg(editor_bg);
        let matched_text_style = Style::default().fg(theme.fg).bg(editor_bg);
        let highlight_style = Style::default().bg(theme.selected_bg).fg(theme.selected_fg);
        let separator_style = Style::default().fg(theme.disabled).bg(editor_bg);

        let line_num_width = 4usize;
        let separator = " │ ";
        let separator_len = 3;
        let max_text_width = content_width.saturating_sub(line_num_width + separator_len);

        for (idx, node) in search
            .tree_nodes
            .iter()
            .enumerate()
            .skip(search.scroll_offset)
        {
            if lines_rendered >= max_lines || y >= area.y + area.height {
                break;
            }

            let is_selected = idx == search.cursor;

            if node.is_dir {
                let prefix = &search.tree_prefixes[idx];
                let style = if is_selected {
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.accented_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg)
                };
                let prefix_style = if is_selected {
                    style
                } else {
                    Style::default().fg(theme.disabled)
                };

                let mut x = area.x;
                if !prefix.is_empty() {
                    buf.set_string(x, y, prefix, prefix_style);
                    x += prefix.width() as u16;
                }
                let dir_prefix = DIR_COLLAPSED_SLASH;
                buf.set_string(x, y, dir_prefix, style);
                x += dir_prefix.width() as u16;
                buf.set_string(x, y, &node.name, style);
                x += node.name.width() as u16;
                let pad = content_width.saturating_sub((x - area.x) as usize);
                if pad > 0 {
                    buf.set_string(x, y, &PAD[..pad.min(PAD.len())], style);
                }

                y += 1;
                lines_rendered += 1;
            } else if let Some(ref cm) = node.content_match {
                if y + 4 > area.y + area.height {
                    break;
                }

                // Line 1: path:line_number
                let prefix = &search.tree_prefixes[idx];
                let path_text = format!("{}{}:{}", prefix, node.name, cm.line_number);
                let path_style = if is_selected {
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.accented_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg)
                };
                let padding = content_width.saturating_sub(path_text.width());
                buf.set_string(area.x, y, &path_text, path_style);
                if padding > 0 {
                    buf.set_string(
                        area.x + path_text.width() as u16,
                        y,
                        &PAD[..padding.min(PAD.len())],
                        path_style,
                    );
                }
                y += 1;

                let fill_bg = |buf: &mut Buffer, row: u16| {
                    for col in 0..content_width {
                        buf.set_string(
                            area.x + col as u16,
                            row,
                            " ",
                            Style::default().bg(editor_bg),
                        );
                    }
                };

                // Line 2: previous line
                fill_bg(buf, y);
                if let Some(ref line_before) = cm.line_before {
                    let line_num = format!("{:>4}", cm.line_number - 1);
                    let content = truncate_from_start(line_before, max_text_width);
                    buf.set_string(area.x, y, &line_num, line_num_style);
                    buf.set_string(
                        area.x + line_num_width as u16,
                        y,
                        separator,
                        separator_style,
                    );
                    buf.set_string(
                        area.x + (line_num_width + separator_len) as u16,
                        y,
                        &content,
                        context_text_style,
                    );
                }
                y += 1;

                // Line 3: matched line with highlight
                fill_bg(buf, y);
                let line_num = format!("{:>4}", cm.line_number);
                buf.set_string(area.x, y, &line_num, line_num_style);
                buf.set_string(
                    area.x + line_num_width as u16,
                    y,
                    separator,
                    separator_style,
                );

                let content_start_x = area.x + (line_num_width + separator_len) as u16;
                let (display_line, match_start_g, match_end_g) = prepare_matched_line(
                    &cm.matched_line,
                    cm.match_start,
                    cm.match_end,
                    max_text_width,
                );

                let mut x = content_start_x;
                for (grapheme_idx, grapheme) in display_line.graphemes(true).enumerate() {
                    let style = if grapheme_idx >= match_start_g && grapheme_idx < match_end_g {
                        highlight_style
                    } else {
                        matched_text_style
                    };
                    buf.set_string(x, y, grapheme, style);
                    x += grapheme.width() as u16;
                }
                y += 1;

                // Line 4: next line
                fill_bg(buf, y);
                if let Some(ref line_after) = cm.line_after {
                    let line_num = format!("{:>4}", cm.line_number + 1);
                    let content = truncate_from_start(line_after, max_text_width);
                    buf.set_string(area.x, y, &line_num, line_num_style);
                    buf.set_string(
                        area.x + line_num_width as u16,
                        y,
                        separator,
                        separator_style,
                    );
                    buf.set_string(
                        area.x + (line_num_width + separator_len) as u16,
                        y,
                        &content,
                        context_text_style,
                    );
                }
                y += 1;

                lines_rendered += 4;
            } else {
                // File without content match
                let prefix = &search.tree_prefixes[idx];
                let style = if is_selected {
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.accented_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    git_status_style(node.git_status, theme)
                };
                let text = format!("{}  {}", prefix, node.name);
                let padding = content_width.saturating_sub(text.width());
                buf.set_string(area.x, y, &text, style);
                if padding > 0 {
                    buf.set_string(
                        area.x + text.width() as u16,
                        y,
                        &PAD[..padding.min(PAD.len())],
                        style,
                    );
                }
                y += 1;
                lines_rendered += 1;
            }
        }
    }
}
