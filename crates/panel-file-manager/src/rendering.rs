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
        // Static padding buffer to avoid per-item allocation
        const PAD: &str = "                                                                                                                                                                                                        ";

        // Determine whether to show extended view with columns
        let show_extended = available_width >= config.extended_view_width;

        for (i, entry) in self.entries.iter().enumerate() {
            if i < visible_start || i >= visible_end {
                continue;
            }

            let is_selected = self.selection.is_selected(i);
            let is_cursor = i == self.selected;

            let attr = utils::get_attribute(entry, is_selected);
            let icon = utils::get_icon(entry);
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
                // For wide mode: attr + icon + space + prefix + two columns and two separators
                available_width.saturating_sub(
                    attr_width
                        + icon_width
                        + 1
                        + prefix_width
                        + SEPARATOR_WIDTH
                        + SIZE_COLUMN_WIDTH
                        + SEPARATOR_WIDTH
                        + TIME_COLUMN_WIDTH,
                )
            } else {
                // For normal mode: attr + icon + space + prefix
                available_width.saturating_sub(attr_width + icon_width + 1 + prefix_width)
            };

            let name = utils::truncate_name(&entry.name, max_name_len);
            let name_width = name.width();
            let full_name = format!("{}{}", dir_prefix, name);

            let (bg_style, fg_style) = if is_cursor && is_focused {
                // Get the normal foreground style for this entry
                let normal_fg_style = git_status_style(entry.git_status, theme);

                // Extract fg color and create inverted cursor style
                let fg_color = normal_fg_style.fg.unwrap_or(theme.fg);
                let cursor_style = Style::default()
                    .bg(fg_color) // Swap: entry fg becomes cursor bg
                    .fg(theme.bg) // Swap: theme bg becomes cursor fg
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

            // Переопределить fg_style для выделенных файлов (если не курсор)
            let fg_style = if is_selected && !(is_cursor && is_focused) {
                Style::default()
                    .fg(theme.accented_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                fg_style
            };

            // Icon style: same as fg_style but without CROSSED_OUT for deleted files
            let icon_style = if entry.git_status == GitStatus::Deleted {
                Style::default().fg(theme.error)
            } else {
                fg_style
            };

            // Name style: add CROSSED_OUT only for deleted files (strikethrough only on name)
            let name_style = if entry.git_status == GitStatus::Deleted && !(is_cursor && is_focused)
            {
                fg_style.add_modifier(Modifier::CROSSED_OUT)
            } else {
                fg_style
            };

            if show_extended {
                // Extended mode with columns
                // Use name_width without prefix, since max_name_len already accounted for prefix_width when subtracting
                let padding_len = max_name_len.saturating_sub(name_width);
                let padding = &PAD[..padding_len.min(PAD.len())];

                // Format size (or spaces for directories and ".."), right-aligned
                let size_str: std::borrow::Cow<'static, str> = if let Some(size) = entry.size {
                    format!("{:>10}", utils::format_size(size)).into()
                } else {
                    std::borrow::Cow::Borrowed("          ")
                };

                // Format time
                let time_str = utils::format_modified_time(entry.modified);

                lines.push(Line::from(vec![
                    Span::styled(attr, attr_style),
                    Span::styled(icon, icon_style),
                    Span::styled(" ", bg_style),
                    Span::styled(full_name, name_style),
                    Span::styled(padding, bg_style),
                    Span::styled(SEPARATOR, bg_style.fg(theme.disabled)),
                    Span::styled(size_str, fg_style),
                    Span::styled(SEPARATOR, bg_style.fg(theme.disabled)),
                    Span::styled(time_str, fg_style),
                ]));
            } else {
                // Normal mode without columns
                let content_width = attr_width + icon_width + 1 + prefix_width + name_width;
                let padding_len = available_width.saturating_sub(content_width);
                let padding = &PAD[..padding_len.min(PAD.len())];

                lines.push(Line::from(vec![
                    Span::styled(attr, attr_style),
                    Span::styled(icon, icon_style),
                    Span::styled(" ", bg_style),
                    Span::styled(full_name, name_style),
                    Span::styled(padding, bg_style),
                ]));
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
