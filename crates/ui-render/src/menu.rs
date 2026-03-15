//! Menu bar rendering.
//!
//! Provides menu item definitions, color utilities, and menu rendering.

use chrono::Local;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use termide_i18n as i18n;
use termide_system_monitor::{format_net_speed, RamUnit};
use termide_theme::Theme;

/// Parameters for rendering the menu bar.
pub struct MenuRenderParams<'a> {
    pub theme: &'a Theme,
    pub selected_menu_item: Option<usize>,
    pub menu_open: bool,
    pub cpu_usage: u8,
    pub ram_percent: u8,
    pub ram_value: String,
    pub ram_unit: RamUnit,
    /// Network download rate in bytes per second
    pub net_down_rate: u64,
    /// Network upload rate in bytes per second
    pub net_up_rate: u64,
    /// Toggle menu keybinding display string (e.g., "Alt+M")
    pub toggle_menu_key: &'a str,
}

/// Get menu items with translations
pub fn get_menu_items() -> Vec<String> {
    let t = i18n::t();
    vec![
        t.menu_sessions().to_string(),
        t.menu_windows().to_string(),
        t.menu_scripts().to_string(),
        t.menu_bookmarks().to_string(),
        t.menu_options().to_string(),
    ]
}

/// Number of menu items
pub const MENU_ITEM_COUNT: usize = 5;

/// Index of Sessions menu item (no keyboard accelerator highlighting)
pub const SESSIONS_MENU_INDEX: usize = 0;

/// Index of Windows menu item (for submenu positioning)
pub const WINDOWS_MENU_INDEX: usize = 1;

/// Index of Scripts menu item (for submenu positioning)
pub const SCRIPTS_MENU_INDEX: usize = 2;

/// Index of Bookmarks menu item (for submenu positioning)
pub const BOOKMARKS_MENU_INDEX: usize = 3;

/// Index of Options menu item (for submenu positioning)
pub const OPTIONS_MENU_INDEX: usize = 4;

/// Pre-computed x positions and widths for all menu items.
/// Avoids repeated `get_menu_items()` allocations in hot paths.
pub struct MenuLayout {
    /// X position of each menu item
    pub x_positions: [u16; MENU_ITEM_COUNT],
    /// Width of each menu item
    pub widths: [u16; MENU_ITEM_COUNT],
    /// Total width used by all menu items (including separators)
    pub total_width: usize,
}

impl MenuLayout {
    pub fn compute() -> Self {
        let menu_items = get_menu_items();
        let mut x_positions = [0u16; MENU_ITEM_COUNT];
        let mut widths = [0u16; MENU_ITEM_COUNT];
        let mut x = 1u16; // initial " " padding

        for (i, item) in menu_items.iter().enumerate() {
            x_positions[i] = x;
            widths[i] = item.width() as u16;
            x += widths[i] + 2; // item + "  " separator
        }

        let total_width = x as usize - 1; // subtract trailing separator overshoot
        Self {
            x_positions,
            widths,
            total_width,
        }
    }
}

/// Calculate x position of a menu item by index.
/// Used for positioning submenus next to their parent menu item.
pub fn get_menu_item_x_position(menu_index: usize) -> u16 {
    MenuLayout::compute().x_positions[menu_index.min(MENU_ITEM_COUNT - 1)]
}

/// Get the width of a menu item by index
pub fn get_menu_item_width(menu_index: usize) -> u16 {
    MenuLayout::compute()
        .widths
        .get(menu_index)
        .copied()
        .unwrap_or(0)
}

/// Choose color indicator by load level
/// < 50% - green (success)
/// 50-75% - yellow (warning)
/// > 75% - red (error)
pub fn resource_color(usage: u8, theme: &Theme) -> Color {
    if usage > 75 {
        theme.error
    } else if usage >= 50 {
        theme.warning
    } else {
        theme.success
    }
}

/// Compute x-ranges of the CPU and RAM indicators in the menu bar.
///
/// Returns `(cpu_range, ram_range)` as `Range<u16>` values relative to the area.
/// These ranges correspond to the positions computed in `render_menu()`.
pub fn get_resource_indicator_ranges(
    area_width: u16,
    params: &MenuRenderParams,
) -> (std::ops::Range<u16>, std::ops::Range<u16>) {
    let t = i18n::t();
    let layout = MenuLayout::compute();

    // Replicate the layout math from render_menu
    let used_width = 1 + layout.total_width;

    let ram_unit_str = match params.ram_unit {
        RamUnit::Gigabytes => t.size_gigabytes(),
        RamUnit::Megabytes => t.size_megabytes(),
    };

    let hint: std::borrow::Cow<str> = if params.menu_open {
        t.menu_navigate_hint().into()
    } else {
        format!("{} {}", params.toggle_menu_key, t.menu_open_hint_label()).into()
    };

    let net_down_text = format!("↓{} ", format_net_speed(params.net_down_rate));
    let net_up_text = format!("↑{} ", format_net_speed(params.net_up_rate));
    let cpu_text = format!("CPU {}% ", params.cpu_usage);
    let ram_text = format!("RAM {}{} ", params.ram_value, ram_unit_str);
    let current_time = chrono::Local::now().format("%H:%M").to_string();
    let clock_text = format!(" {} ", current_time);

    let hint_with_padding = format!(" {} ", hint);

    // Calculate positions from the right side
    // Layout order: ... [padding] [hint] [net_down] [net_up] [cpu] [ram] [clock]
    let remaining = (area_width as usize).saturating_sub(
        used_width
            + hint.width()
            + 2
            + net_down_text.width()
            + net_up_text.width()
            + cpu_text.width()
            + ram_text.width()
            + clock_text.width(),
    );

    let mut x = used_width + remaining;
    x += hint_with_padding.width(); // skip hint
    x += net_down_text.width(); // skip net down
    x += net_up_text.width(); // skip net up

    let cpu_start = x as u16;
    let cpu_end = cpu_start + cpu_text.width() as u16;

    let ram_start = cpu_end;
    let ram_end = ram_start + ram_text.width() as u16;

    (cpu_start..cpu_end, ram_start..ram_end)
}

/// Render top menu in Midnight Commander style
pub fn render_menu(frame: &mut Frame, area: Rect, params: &MenuRenderParams) {
    let mut spans = vec![Span::raw(" ")];
    let menu_items = get_menu_items();
    let t = i18n::t();

    for (i, item) in menu_items.iter().enumerate() {
        // Determine menu item style
        let is_selected = params.selected_menu_item == Some(i);
        let style = if is_selected && params.menu_open {
            Style::default()
                .fg(params.theme.selected_fg)
                .bg(params.theme.selected_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(params.theme.fg)
        };

        spans.push(Span::styled(item.as_str(), style));
        spans.push(Span::raw("  "));
    }

    // Add hint, resource indicators, and clock on the right
    let hint: std::borrow::Cow<str> = if params.menu_open {
        t.menu_navigate_hint().into()
    } else {
        format!("{} {}", params.toggle_menu_key, t.menu_open_hint_label()).into()
    };

    // System resource info
    let ram_unit_str = match params.ram_unit {
        RamUnit::Gigabytes => t.size_gigabytes(),
        RamUnit::Megabytes => t.size_megabytes(),
    };

    // Network indicators
    let net_down_text = format!("↓{} ", format_net_speed(params.net_down_rate));
    let net_up_text = format!("↑{} ", format_net_speed(params.net_up_rate));

    // CPU indicator
    let cpu_text = format!("CPU {}% ", params.cpu_usage);
    let cpu_color = resource_color(params.cpu_usage, params.theme);

    // RAM indicator
    let ram_text = format!("RAM {}{} ", params.ram_value, ram_unit_str);
    let ram_color = resource_color(params.ram_percent, params.theme);

    // Current time
    let current_time = Local::now().format("%H:%M").to_string();
    let clock_text = format!(" {} ", current_time);

    // Calculate spacing
    let used_width: usize = spans.iter().map(|s| s.width()).sum();
    let remaining = (area.width as usize).saturating_sub(
        used_width
            + hint.width()
            + 2
            + net_down_text.width()
            + net_up_text.width()
            + cpu_text.width()
            + ram_text.width()
            + clock_text.width(),
    );

    if remaining > 0 {
        spans.push(Span::raw(" ".repeat(remaining)));
    }

    // Pre-compute styles to avoid repeated Style::default() calls
    let hint_style = Style::default().fg(Color::DarkGray);
    let cpu_style = Style::default().fg(cpu_color);
    let ram_style = Style::default().fg(ram_color);
    let clock_style = Style::default()
        .fg(params.theme.fg)
        .add_modifier(Modifier::BOLD);

    // Add hint
    spans.push(Span::styled(format!(" {} ", hint), hint_style));

    // Add network indicators
    let net_down_style = Style::default().fg(params.theme.success);
    let net_up_style = Style::default().fg(Color::Cyan);
    spans.push(Span::styled(net_down_text, net_down_style));
    spans.push(Span::styled(net_up_text, net_up_style));

    // Add CPU indicator
    spans.push(Span::styled(cpu_text, cpu_style));

    // Add RAM indicator
    spans.push(Span::styled(ram_text, ram_style));

    // Add clock
    spans.push(Span::styled(clock_text, clock_style));

    let menu =
        Paragraph::new(Line::from(spans)).style(Style::default().bg(params.theme.accented_bg));

    frame.render_widget(menu, area);
}
