use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use super::{utils, FileManager};
use termide_config::FileManagerSettings;
use termide_git::GitStatus;
use termide_theme::Theme;

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
                    "▽"
                } else {
                    "▼"
                }
            }
            Some(false) | None => {
                if entry.is_symlink {
                    "▷"
                } else {
                    "▶"
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
        const PAD: &str = "                                                                                                                                                                                                        ";

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
                    format!("{:>10}", utils::format_size(size)).into()
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
}
