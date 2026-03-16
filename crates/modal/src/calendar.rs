//! Calendar modal dialog.
//!
//! Displays a monthly calendar with day navigation.
//! Opened by clicking the clock in the menu bar.

use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::base::render_modal_block;
use crate::{centered_rect_with_size, Modal, ModalResult};
use termide_i18n as i18n;
use termide_theme::Theme;

/// Width of the calendar modal (borders + 7 columns × 3 chars + padding).
const CALENDAR_WIDTH: u16 = 24;
/// Height: border(1) + weekday header(1) + 6 week rows + border(1) = 10
const CALENDAR_HEIGHT: u16 = 10;

/// Calendar modal — shows a monthly calendar grid.
#[derive(Debug)]
pub struct CalendarModal {
    year: i32,
    month: u32,
    selected_day: u32,
    today: NaiveDate,
    /// Cached area for mouse hit-testing
    last_area: Option<Rect>,
}

impl Default for CalendarModal {
    fn default() -> Self {
        Self::new()
    }
}

impl CalendarModal {
    /// Create a new calendar modal showing the current month.
    pub fn new() -> Self {
        let today = Local::now().date_naive();
        Self {
            year: today.year(),
            month: today.month(),
            selected_day: today.day(),
            today,
            last_area: None,
        }
    }

    /// First day of the displayed month.
    fn first_of_month(&self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.year, self.month, 1).unwrap()
    }

    /// Number of days in the displayed month.
    fn days_in_month(&self) -> u32 {
        let first = self.first_of_month();
        // Go to first of next month and subtract one day
        if self.month == 12 {
            NaiveDate::from_ymd_opt(self.year + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(self.year, self.month + 1, 1)
        }
        .unwrap()
        .signed_duration_since(first)
        .num_days() as u32
    }

    /// Move to the previous month.
    fn prev_month(&mut self) {
        if self.month == 1 {
            self.month = 12;
            self.year -= 1;
        } else {
            self.month -= 1;
        }
        self.clamp_selected_day();
    }

    /// Move to the next month.
    fn next_month(&mut self) {
        if self.month == 12 {
            self.month = 1;
            self.year += 1;
        } else {
            self.month += 1;
        }
        self.clamp_selected_day();
    }

    /// Ensure selected_day doesn't exceed the days in the current month.
    fn clamp_selected_day(&mut self) {
        let max = self.days_in_month();
        if self.selected_day > max {
            self.selected_day = max;
        }
    }

    /// Move selection by a number of days (positive = forward, negative = back).
    fn move_selection(&mut self, delta: i32) {
        let current = NaiveDate::from_ymd_opt(self.year, self.month, self.selected_day).unwrap();
        let new_date = current + chrono::Duration::days(delta as i64);
        self.year = new_date.year();
        self.month = new_date.month();
        self.selected_day = new_date.day();
    }

    /// Jump to today.
    fn go_today(&mut self) {
        self.year = self.today.year();
        self.month = self.today.month();
        self.selected_day = self.today.day();
    }

    /// Check if a given day is today.
    fn is_today(&self, day: u32) -> bool {
        self.today.year() == self.year
            && self.today.month() == self.month
            && self.today.day() == day
    }
}

impl Modal for CalendarModal {
    type Result = ();

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let modal_area = centered_rect_with_size(CALENDAR_WIDTH, CALENDAR_HEIGHT, area);
        self.last_area = Some(modal_area);

        let t = i18n::t();
        let month_names = [
            t.calendar_january(),
            t.calendar_february(),
            t.calendar_march(),
            t.calendar_april(),
            t.calendar_may(),
            t.calendar_june(),
            t.calendar_july(),
            t.calendar_august(),
            t.calendar_september(),
            t.calendar_october(),
            t.calendar_november(),
            t.calendar_december(),
        ];
        let title = format!("{} {}", month_names[(self.month - 1) as usize], self.year);

        let inner = render_modal_block(modal_area, buf, &title, theme);

        if inner.width < 2 || inner.height < 2 {
            return;
        }

        let default_style = Style::default().fg(theme.fg).bg(theme.bg);
        let header_style = Style::default()
            .fg(theme.accented_fg)
            .bg(theme.bg)
            .add_modifier(Modifier::BOLD);
        let today_style = Style::default()
            .fg(theme.accented_fg)
            .bg(theme.bg)
            .add_modifier(Modifier::BOLD);
        let selected_style = Style::default().fg(theme.selected_fg).bg(theme.selected_bg);

        // Row 0: Weekday headers (Monday-first)
        let weekday_header = Line::from(vec![Span::styled(
            format!(
                " {:>2} {:>2} {:>2} {:>2} {:>2} {:>2} {:>2}",
                t.calendar_mon(),
                t.calendar_tue(),
                t.calendar_wed(),
                t.calendar_thu(),
                t.calendar_fri(),
                t.calendar_sat(),
                t.calendar_sun(),
            ),
            header_style,
        )]);
        let header_area = Rect::new(inner.x, inner.y, inner.width, 1);
        Paragraph::new(weekday_header).render(header_area, buf);

        // Rows 1-6: Day grid
        let first = self.first_of_month();
        // Monday = 0 offset
        let start_weekday = first.weekday().num_days_from_monday();
        let total_days = self.days_in_month();

        let mut day = 1u32;
        for week in 0..6u16 {
            let row_y = inner.y + 1 + week;
            if row_y >= inner.y + inner.height {
                break;
            }

            let mut spans = Vec::with_capacity(8);
            spans.push(Span::styled(" ", default_style));

            for weekday in 0..7u32 {
                let cell_idx = week as u32 * 7 + weekday;
                if cell_idx < start_weekday || day > total_days {
                    spans.push(Span::styled("   ", default_style));
                } else {
                    let text = format!("{:>2} ", day);
                    let style = if day == self.selected_day && self.is_today(day) {
                        // Both selected and today
                        selected_style.add_modifier(Modifier::BOLD)
                    } else if day == self.selected_day {
                        selected_style
                    } else if self.is_today(day) {
                        today_style
                    } else {
                        default_style
                    };
                    spans.push(Span::styled(text, style));
                    day += 1;
                }
            }

            let line = Line::from(spans);
            let row_area = Rect::new(inner.x, row_y, inner.width, 1);
            Paragraph::new(line).render(row_area, buf);
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>> {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                return Ok(Some(ModalResult::Cancelled));
            }
            KeyCode::Left => self.move_selection(-1),
            KeyCode::Right => self.move_selection(1),
            KeyCode::Up => self.move_selection(-7),
            KeyCode::Down => self.move_selection(7),
            KeyCode::PageUp | KeyCode::Char('h') => self.prev_month(),
            KeyCode::PageDown | KeyCode::Char('l') => self.next_month(),
            KeyCode::Home | KeyCode::Char('t') => self.go_today(),
            _ => {}
        }
        Ok(None)
    }

    fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        modal_area: Rect,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return Ok(None);
        }

        let x = mouse.column;
        let y = mouse.row;

        // Check if click is outside modal
        if x < modal_area.x
            || x >= modal_area.x + modal_area.width
            || y < modal_area.y
            || y >= modal_area.y + modal_area.height
        {
            return Ok(Some(ModalResult::Cancelled));
        }

        // Inner area (1 pixel border)
        let inner_x = modal_area.x + 1;
        let inner_y = modal_area.y + 1;

        // Day grid rows (inner_y + 1 through inner_y + 6)
        if y > inner_y && y < inner_y + 7 {
            let week = (y - inner_y - 1) as u32;
            // Each cell is 3 chars wide, with 1 char left padding
            let col_offset = x.saturating_sub(inner_x + 1);
            let weekday = (col_offset / 3) as u32;
            if weekday < 7 {
                let first = self.first_of_month();
                let start_weekday = first.weekday().num_days_from_monday();
                let cell_idx = week * 7 + weekday;
                if cell_idx >= start_weekday {
                    let day = cell_idx - start_weekday + 1;
                    if day >= 1 && day <= self.days_in_month() {
                        self.selected_day = day;
                    }
                }
            }
        }

        Ok(None)
    }
}
