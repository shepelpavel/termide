//! Panel rendering functions.
//!
//! Provides functions to render expanded and collapsed panels.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Widget},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Braille spinner characters used for loading indicators.
const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Smart title truncation that preserves spinner and status.
///
/// When truncating a title like "⠋ main.rs (indexing)", this function ensures:
/// - Spinner at the start is always preserved
/// - Status in parentheses at the end is always preserved
/// - Main text in the middle is truncated with "…" from the left
///
/// Returns the truncated title that fits within `max_width`.
fn smart_truncate_title(title: &str, max_width: usize) -> String {
    let title_width = title.width();
    if title_width <= max_width {
        return title.to_string();
    }

    // Parse title parts: [spinner] [main_text] [(status)]
    let chars: Vec<char> = title.chars().collect();
    if chars.is_empty() {
        return String::new();
    }

    // Detect spinner prefix (braille char + space)
    let (spinner, rest_start) = if SPINNER_CHARS.contains(&chars[0]) {
        let spinner_end = if chars.len() > 1 && chars[1] == ' ' {
            2
        } else {
            1
        };
        (chars[..spinner_end].iter().collect::<String>(), spinner_end)
    } else {
        (String::new(), 0)
    };

    let rest: String = chars[rest_start..].iter().collect();

    // Detect status suffix: " (something)" at the end
    let (main_text, status) = if let Some(paren_start) = rest.rfind(" (") {
        if rest.ends_with(')') {
            (
                rest[..paren_start].to_string(),
                rest[paren_start..].to_string(),
            )
        } else {
            (rest, String::new())
        }
    } else {
        (rest, String::new())
    };

    let spinner_width = spinner.width();
    let status_width = status.width();
    let fixed_width = spinner_width + status_width;

    // If even spinner + status don't fit, just truncate everything
    if fixed_width >= max_width {
        let mut result = String::new();
        let mut width = 0;
        for ch in title.chars() {
            let ch_width = ch.width().unwrap_or(0);
            if width + ch_width > max_width {
                break;
            }
            result.push(ch);
            width += ch_width;
        }
        return result;
    }

    // Available width for main text (with "…" if needed)
    let available_for_main = max_width - fixed_width;

    let main_width = main_text.width();
    let truncated_main = if main_width <= available_for_main {
        main_text
    } else if available_for_main > 1 {
        // Need to truncate main text, keep right part with "…"
        let target_width = available_for_main - 1; // Reserve 1 for "…"
        let main_chars: Vec<char> = main_text.chars().collect();
        let mut start_idx = 0;
        let mut current_width = main_width;

        // Remove chars from start until we fit
        while current_width > target_width && start_idx < main_chars.len() {
            current_width -= main_chars[start_idx].width().unwrap_or(0);
            start_idx += 1;
        }

        format!("…{}", main_chars[start_idx..].iter().collect::<String>())
    } else if available_for_main > 0 {
        // Very narrow, just take what we can from the end
        let main_chars: Vec<char> = main_text.chars().collect();
        let mut chars_rev = Vec::new();
        let mut width = 0;
        for ch in main_chars.iter().rev() {
            let ch_width = ch.width().unwrap_or(0);
            if width + ch_width > available_for_main {
                break;
            }
            chars_rev.push(*ch);
            width += ch_width;
        }
        chars_rev.iter().rev().collect()
    } else {
        String::new()
    };

    format!("{}{}{}", spinner, truncated_main, status)
}

use termide_config::Config;
use termide_core::{use_emoji_icons, Panel, PanelConfig, RenderContext, ThemeColors};

/// Get emoji icon for a panel type.
fn panel_icon(name: &str) -> &'static str {
    match name {
        "terminal" => "💻",
        "file_manager" => "📁",
        "editor" => "📝",
        "git_status" => "📊",
        "git_log" => "📜",
        "git_diff" => "🔀",
        "image" => "🖼️",
        "diagnostics" => "⚠️",
        "outline" => "🗂️",
        "operations" => "⚙️",
        _ => "📋",
    }
}
use termide_theme::Theme;

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // smart_truncate_title tests
    // =========================================================================

    #[test]
    fn test_truncate_short_title_unchanged() {
        assert_eq!(smart_truncate_title("main.rs", 20), "main.rs");
    }

    #[test]
    fn test_truncate_empty_title() {
        assert_eq!(smart_truncate_title("", 10), "");
    }

    #[test]
    fn test_truncate_exact_fit() {
        let title = "abcde";
        assert_eq!(smart_truncate_title(title, 5), "abcde");
    }

    #[test]
    fn test_truncate_with_spinner_prefix() {
        // Spinner char + space + title
        let title = "\u{280b} main.rs";
        let result = smart_truncate_title(title, 50);
        assert_eq!(result, title);
    }

    #[test]
    fn test_truncate_with_status_suffix() {
        let title = "main.rs (indexing)";
        let result = smart_truncate_title(title, 50);
        assert_eq!(result, title);
    }

    #[test]
    fn test_truncate_preserves_spinner_and_status() {
        // When title is too long, spinner and status should survive
        let title = "\u{280b} very_long_filename_that_needs_truncation.rs (indexing)";
        let result = smart_truncate_title(title, 30);
        // Spinner should be at start
        assert!(result.starts_with('\u{280b}'));
        // Status should be at end
        assert!(result.ends_with("(indexing)"));
    }

    #[test]
    fn test_truncate_long_title_gets_ellipsis() {
        let title = "a_very_long_filename_that_exceeds_width.rs";
        let result = smart_truncate_title(title, 15);
        assert!(result.contains('…'));
        assert!(result.len() <= title.len());
    }

    #[test]
    fn test_truncate_very_narrow_width() {
        let title = "main.rs";
        let result = smart_truncate_title(title, 3);
        // Should not panic, should produce something <= 3 chars wide
        assert!(result.width() <= 3);
    }

    #[test]
    fn test_truncate_width_1() {
        let title = "main.rs";
        let result = smart_truncate_title(title, 1);
        assert!(result.width() <= 1);
    }

    #[test]
    fn test_truncate_unicode_cjk() {
        // CJK chars are typically 2 cells wide
        let title = "\u{4f60}\u{597d}\u{4e16}\u{754c}"; // "你好世界"
        let result = smart_truncate_title(title, 4);
        // Should fit within 4 cells (2 CJK chars)
        assert!(result.width() <= 4);
    }

    #[test]
    fn test_truncate_only_status_no_main() {
        // Edge case: title that's mostly status
        let title = "x (very long status message here)";
        let result = smart_truncate_title(title, 10);
        // Should not panic
        assert!(result.width() <= 10);
    }
}

/// Render active divider during drag operation.
///
/// Only draws when a divider is being actively dragged.
/// Replaces both adjacent panel borders (right border of left panel
/// and left border of right panel) with double-line style `║`.
pub fn render_dividers(
    buf: &mut Buffer,
    divider_positions: &[(usize, u16)], // (group_idx, x_position)
    active_divider: Option<usize>,
    terminal_height: u16,
    theme: &Theme,
) {
    // Only draw when actively dragging
    let Some(active_idx) = active_divider else {
        return;
    };

    // Draw from below menu (y=1) to above status bar (y=height-2)
    let start_y = 1u16;
    let end_y = terminal_height.saturating_sub(1);
    let style = Style::default().fg(theme.accented_fg);

    // Find and draw only the active divider
    for &(group_idx, x) in divider_positions {
        if group_idx == active_idx {
            // Draw at both border positions:
            // x-1 = right border of left panel
            // x = left border of right panel
            let positions = [x.saturating_sub(1), x];
            for y in start_y..end_y {
                for &pos in &positions {
                    if let Some(cell) = buf.cell_mut((pos, y)) {
                        cell.set_symbol("║");
                        cell.set_style(style);
                    }
                }
            }
            break;
        }
    }
}

/// Parameters for rendering expanded panels.
#[derive(Clone, Copy)]
pub struct ExpandedPanelParams {
    pub tab_size: usize,
    pub word_wrap: bool,
    pub terminal_width: u16,
    pub terminal_height: u16,
}

/// Render collapsed panel (header only, 1 line).
pub fn render_collapsed_panel(
    panel: &dyn Panel,
    area: Rect,
    buf: &mut Buffer,
    is_focused: bool,
    theme: &Theme,
    _group_size: usize,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let title = panel.title();
    let style = if is_focused {
        Style::default()
            .fg(theme.accented_fg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.disabled)
    };

    let y = area.y;

    // Left edge
    if area.width > 0 {
        buf[(area.x, y)].set_symbol("─").set_style(style);
    }

    // Buttons: [X][icon] with emoji, or [X] in unicode mode
    let buttons = if use_emoji_icons() {
        format!("[X][{}]", panel_icon(panel.name()))
    } else {
        "[X]".to_string()
    };
    let buttons_width = buttons.width() as u16;

    if area.width > 1 + buttons_width {
        buf.set_string(area.x + 1, y, buttons, style);
    }

    // Title (smart truncation preserving spinner and status)
    let title_start = area.x + 1 + buttons_width;
    let available_width = area.right().saturating_sub(title_start + 1) as usize;

    // Reserve 2 chars for padding spaces around title
    let content_width = available_width.saturating_sub(2);
    let truncated_title = smart_truncate_title(&title, content_width);
    let display_title = format!(" {} ", truncated_title);
    let title_width = display_title.width();

    if !display_title.is_empty() {
        buf.set_string(title_start, y, &display_title, style);
    }

    // Fill remaining with horizontal line
    let fill_start = title_start + title_width as u16;
    for x in fill_start..area.right() {
        buf[(x, y)].set_symbol("─").set_style(style);
    }
}

/// Render expanded panel (full border with content).
#[allow(clippy::too_many_arguments)]
pub fn render_expanded_panel(
    panel: &mut Box<dyn Panel>,
    area: Rect,
    buf: &mut Buffer,
    is_focused: bool,
    panel_index: usize,
    theme: &Theme,
    config: &Config,
    params: ExpandedPanelParams,
    _group_size: usize,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let title = panel.title();
    let style = if is_focused {
        Style::default()
            .fg(theme.accented_fg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.disabled)
    };

    // Create title: [X][icon] Title (with emoji) or [X] Title (unicode mode)
    // Smart truncate title to fit within panel width
    let buttons_text = if use_emoji_icons() {
        format!("[X][{}] ", panel_icon(panel.name()))
    } else {
        "[X] ".to_string()
    };
    let buttons_width = buttons_text.width();
    // Available width: panel width - 2 (borders) - buttons - 1 (trailing space)
    let available_for_title = (area.width as usize).saturating_sub(2 + buttons_width + 1);
    let truncated_title = smart_truncate_title(&title, available_for_title);
    let title_text = format!("{}{} ", buttons_text, truncated_title);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(style)
        .title(Span::styled(title_text, style));

    let inner = block.inner(area);
    block.render(area, buf);

    // Clear inner area before rendering content
    // Optimization: Single operation per cell instead of reset() + set_style()
    let clear_style = Style::default().bg(theme.bg);
    for y in inner.y..inner.y + inner.height {
        for x in inner.x..inner.x + inner.width {
            let cell = buf.cell_mut((x, y)).expect("cell in bounds");
            cell.set_char(' ');
            cell.set_style(clear_style);
        }
    }

    // Create RenderContext
    let colors = ThemeColors::from(theme);
    let panel_config = PanelConfig {
        tab_size: params.tab_size,
        word_wrap: params.word_wrap,
        show_line_numbers: true,
        show_hidden_files: false,
    };
    let ctx = RenderContext {
        theme: &colors,
        config: &panel_config,
        is_focused,
        panel_index,
        terminal_width: params.terminal_width,
        terminal_height: params.terminal_height,
        border_right_x: Some(area.x + area.width - 1),
    };

    // Prepare panel for rendering (update cached theme/config)
    panel.prepare_render(theme, config);

    // Render panel content
    panel.render(inner, buf, &ctx);
}
