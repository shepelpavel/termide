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
use termide_system_monitor::RamUnit;
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
        t.menu_options().to_string(),
    ]
}

/// Number of menu items
pub const MENU_ITEM_COUNT: usize = 4;

/// Index of Sessions menu item (no keyboard accelerator highlighting)
pub const SESSIONS_MENU_INDEX: usize = 0;

/// Index of Windows menu item (for submenu positioning)
pub const WINDOWS_MENU_INDEX: usize = 1;

/// Index of Scripts menu item (for submenu positioning)
pub const SCRIPTS_MENU_INDEX: usize = 2;

/// Index of Options menu item (for submenu positioning)
pub const OPTIONS_MENU_INDEX: usize = 3;

/// Calculate x position of a menu item by index.
/// Used for positioning submenus next to their parent menu item.
pub fn get_menu_item_x_position(menu_index: usize) -> u16 {
    let menu_items = get_menu_items();
    let mut x = 1_u16; // Start with initial padding (1 space)

    for (i, item) in menu_items.iter().enumerate() {
        if i == menu_index {
            return x;
        }
        // Each item takes: item width + 2 spaces separator
        x += item.width() as u16 + 2;
    }

    x
}

/// Get the width of a menu item by index
pub fn get_menu_item_width(menu_index: usize) -> u16 {
    let menu_items = get_menu_items();
    menu_items
        .get(menu_index)
        .map(|item| item.width() as u16)
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
        used_width + hint.width() + 2 + cpu_text.width() + ram_text.width() + clock_text.width(),
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
