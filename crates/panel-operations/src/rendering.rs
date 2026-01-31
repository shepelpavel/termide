//! Rendering utilities for operations panel.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};

use termide_ui::path_utils::truncate_left;

use crate::{OperationSnapshot, CARD_HEIGHT};

/// Format bytes to human-readable string
pub fn format_bytes(bytes: u64) -> String {
    let t = termide_i18n::t();
    if bytes >= 1_073_741_824 {
        format!(
            "{:.1}{}",
            bytes as f64 / 1_073_741_824.0,
            t.size_gigabytes()
        )
    } else if bytes >= 1_048_576 {
        format!("{:.1}{}", bytes as f64 / 1_048_576.0, t.size_megabytes())
    } else if bytes >= 1024 {
        format!("{:.1}{}", bytes as f64 / 1024.0, t.size_kilobytes())
    } else {
        format!("{}{}", bytes, t.size_bytes())
    }
}

/// Get localized label for operation type
fn op_type_label(op_type: &termide_state::OperationType) -> &str {
    let t = termide_i18n::t();
    match op_type {
        termide_state::OperationType::Copy => t.progress_copy_title(),
        termide_state::OperationType::Move => t.progress_move_title(),
        termide_state::OperationType::Rename => t.op_type_rename(),
        termide_state::OperationType::CopyUpload => t.op_type_copy_upload(),
        termide_state::OperationType::CopyDownload => t.op_type_copy_download(),
        termide_state::OperationType::MoveUpload => t.op_type_move_upload(),
        termide_state::OperationType::MoveDownload => t.op_type_move_download(),
        termide_state::OperationType::Delete => t.progress_delete_title(),
    }
}

/// Render a single operation card from a snapshot.
fn render_snapshot_card(
    op: &OperationSnapshot,
    area: Rect,
    buf: &mut Buffer,
    is_selected: bool,
    fg_color: Color,
    accent_color: Color,
    disabled_color: Color,
) {
    let border_color = if is_selected {
        accent_color
    } else {
        disabled_color
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height < 7 || inner.width < 10 {
        return; // Not enough space to render
    }

    let content_width = inner.width as usize;
    let is_scanning = op.is_scanning;
    let t = termide_i18n::t();

    // Line 1: [⏸] Type                    45%
    let pause_icon = if op.is_paused { "\u{23F8} " } else { "" }; // ⏸
    let type_label = if is_scanning {
        t.op_type_scanning()
    } else {
        op_type_label(&op.op_type)
    };
    let percent = format!("{}%", op.progress.percent());
    let header_left = format!("{}{}", pause_icon, type_label);
    let padding = content_width.saturating_sub(header_left.chars().count() + percent.len());

    // Static buffers for padding and progress bar (avoids per-frame allocation)
    const SPACES: &str = "                                                                                                                                                                                                        ";
    const FILLED: &str = "████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████";
    const EMPTY: &str = "░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░";

    let header_line = Line::from(vec![
        Span::styled(&header_left, Style::default().fg(fg_color)),
        Span::raw(&SPACES[..padding.min(SPACES.len())]),
        Span::styled(&percent, Style::default().fg(accent_color)),
    ]);
    buf.set_line(inner.x, inner.y, &header_line, inner.width);

    // Line 2: Progress bar
    let bar_width = content_width;
    let percent_val = op.progress.percent() as usize;
    let filled = (bar_width * percent_val) / 100;
    let empty = bar_width.saturating_sub(filled);
    let filled_part = &FILLED[..filled.min(FILLED.len() / 3) * 3]; // █ is 3 bytes
    let empty_part = &EMPTY[..empty.min(EMPTY.len() / 3) * 3]; // ░ is 3 bytes
    let bar_color = if op.is_paused {
        accent_color
    } else {
        Color::Green
    };
    let bar_line = Line::from(vec![
        Span::styled(filled_part, Style::default().fg(bar_color)),
        Span::styled(empty_part, Style::default().fg(bar_color)),
    ]);
    buf.set_line(inner.x, inner.y + 1, &bar_line, inner.width);

    // Line 3: Source path (truncate left)
    let source = truncate_left(&op.source, content_width);
    buf.set_line(
        inner.x,
        inner.y + 2,
        &Line::from(Span::styled(source, Style::default().fg(disabled_color))),
        inner.width,
    );

    // Line 4: Destination path (truncate left) - skip for Delete operations
    if !op.dest.is_empty() {
        let dest = truncate_left(&op.dest, content_width);
        buf.set_line(
            inner.x,
            inner.y + 3,
            &Line::from(Span::styled(dest, Style::default().fg(disabled_color))),
            inner.width,
        );
    }

    // Line 5: Files count (during scanning show "Found: N")
    let files = if is_scanning {
        t.op_found_count(op.progress.files_completed)
    } else {
        t.op_files_progress(op.progress.files_completed, op.progress.total_files)
    };
    buf.set_line(
        inner.x,
        inner.y + 4,
        &Line::from(Span::styled(files, Style::default().fg(fg_color))),
        inner.width,
    );

    // Line 6: Data (hide for delete operations and scanning phase)
    if !is_scanning && op.op_type.has_data_progress() {
        let data = t.op_data_progress(
            &format_bytes(op.progress.bytes_transferred),
            &format_bytes(op.progress.total_bytes),
        );
        buf.set_line(
            inner.x,
            inner.y + 5,
            &Line::from(Span::styled(data, Style::default().fg(fg_color))),
            inner.width,
        );
    }

    // Line 7: Speed (hide for delete operations and scanning phase)
    if !is_scanning && op.op_type.has_data_progress() {
        let speed = t.op_speed_rate(&format_bytes(op.speed as u64));
        buf.set_line(
            inner.x,
            inner.y + 6,
            &Line::from(Span::styled(speed, Style::default().fg(fg_color))),
            inner.width,
        );
    }
}

/// Render the full operations panel using operation snapshots.
/// This version uses OperationSnapshot instead of ActiveOperation references.
pub fn render_operations_panel_snapshots(
    operations: &[OperationSnapshot],
    selected_index: usize,
    scroll_offset: usize,
    area: Rect,
    buf: &mut Buffer,
    is_focused: bool,
    fg_color: Color,
    accent_color: Color,
    disabled_color: Color,
) -> Vec<(usize, Rect)> {
    let mut card_areas = Vec::new();

    if operations.is_empty() {
        // Render "No active operations" message
        let t = termide_i18n::t();
        let text = ratatui::widgets::Paragraph::new(Line::from(t.no_active_operations()))
            .style(Style::default().fg(disabled_color))
            .alignment(ratatui::layout::Alignment::Center);
        text.render(area, buf);
        return card_areas;
    }

    let visible_cards = (area.height / CARD_HEIGHT) as usize;

    for (i, op) in operations.iter().skip(scroll_offset).enumerate() {
        if (i as u16) * CARD_HEIGHT >= area.height {
            break;
        }

        let card_area = Rect {
            x: area.x,
            y: area.y + (i as u16) * CARD_HEIGHT,
            width: area.width,
            height: CARD_HEIGHT.min(area.height - (i as u16) * CARD_HEIGHT),
        };

        let op_index = scroll_offset + i;
        let is_selected = is_focused && op_index == selected_index;

        render_snapshot_card(
            op,
            card_area,
            buf,
            is_selected,
            fg_color,
            accent_color,
            disabled_color,
        );

        card_areas.push((op_index, card_area));
    }

    // Render scroll indicators if needed
    if scroll_offset > 0 {
        // Show "↑" indicator at top
        let indicator = Span::styled("\u{25B2}", Style::default().fg(accent_color)); // ▲
        buf.set_span(area.x + area.width - 3, area.y, &indicator, 1);
    }

    if scroll_offset + visible_cards < operations.len() {
        // Show "↓" indicator at bottom
        let indicator = Span::styled("\u{25BC}", Style::default().fg(accent_color)); // ▼
        buf.set_span(
            area.x + area.width - 3,
            area.y + area.height - 1,
            &indicator,
            1,
        );
    }

    card_areas
}
