//! Inline find / replace bar — a panel-embeddable search form.
//!
//! Unlike [`SearchModal`](crate::SearchModal), this widget renders *inside* a
//! host panel's own area (no modal frame) and is
//! meant to coexist with the panel's body: the panel keeps focus on its results
//! while the bar is open, and only routes keys to the bar when the user moves
//! focus into it (e.g. with `Tab`). The widget therefore manages focus *only
//! among its own controls* — deciding whether the bar or the panel body has
//! focus is the host's responsibility.
//!
//! The host drives it like this:
//! - call [`FindBar::height`] to reserve rows out of the panel's `Rect`;
//! - call [`FindBar::render`] with `active = true` when the bar (not the body)
//!   currently holds focus;
//! - forward keys to [`FindBar::handle_key`] / clicks to
//!   [`FindBar::handle_mouse`] and act on the returned [`FindBarAction`];
//! - read [`FindBar::find_text`] / [`FindBar::replace_text`] / [`FindBar::mask_text`]
//!   / [`FindBar::use_regex`] / [`FindBar::case_sensitive`] to run the search.

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

use termide_theme::Theme;

use crate::base::{render_labeled_input, screen_x_to_char_pos};
use crate::input_keys::{handle_input_key, InputKeyResult};
use crate::TextInputHandler;

/// An input field the bar can expose, in render order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindField {
    /// Glob mask (file-manager content search).
    Mask,
    /// The search query.
    Find,
    /// The replacement text.
    Replace,
}

impl FindField {
    fn label(self) -> &'static str {
        match self {
            FindField::Mask => "Mask: ",
            FindField::Find => "Find: ",
            FindField::Replace => "Repl: ",
        }
    }
}

/// A control on the buttons row. Action buttons confirm an operation; the two
/// trailing toggles flip regex / case. The host supplies the action buttons it
/// wants; the toggles are always appended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Replace,
    ReplaceAll,
    Prev,
    Next,
    Regex,
    Case,
}

impl Btn {
    fn is_toggle(self) -> bool {
        matches!(self, Btn::Regex | Btn::Case)
    }
}

/// What the host should do in response to a key / click.
///
/// Anything that mutates the query (typing, or flipping a toggle) collapses to
/// [`FindBarAction::QueryChanged`] so the host can re-run the search uniformly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindBarAction {
    /// A field was edited or a toggle flipped — re-run the search.
    QueryChanged,
    /// Go to the next match.
    Next,
    /// Go to the previous match.
    Previous,
    /// Replace the current match.
    Replace,
    /// Replace every match.
    ReplaceAll,
    /// `Enter` on a field — the host decides what to do based on
    /// [`FindBar::focused_field`].
    Submit,
    /// Close the bar.
    Close,
}

/// Configuration for a [`FindBar`].
pub struct FindBarConfig {
    /// Fields to show, top to bottom.
    pub fields: Vec<FindField>,
    /// Action buttons (Replace / ReplaceAll / Prev / Next), in order.
    pub action_buttons: Vec<Btn>,
    /// Append the regex / case toggles. Set false for searches where they have
    /// no effect (e.g. glob file-name search).
    pub toggles: bool,
}

/// A focusable control: either a field or a button-row entry (by index into
/// `buttons`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Field(usize),
    Button(usize),
}

/// Inline find / replace bar.
pub struct FindBar {
    fields: Vec<FindField>,
    inputs: Vec<TextInputHandler>,
    buttons: Vec<Btn>,
    use_regex: bool,
    case_sensitive: bool,
    /// Index into the focus ring (`ring()`).
    focus: usize,
    match_info: Option<(usize, usize)>,
    /// Per-field rendered areas, parallel to `fields` (for mouse).
    field_areas: Vec<Rect>,
    /// Rendered button areas: (area, index into `buttons`).
    button_areas: Vec<(Rect, usize)>,
}

impl FindBar {
    /// Create a bar from a config. The first field is focused by default.
    pub fn new(config: FindBarConfig) -> Self {
        let FindBarConfig {
            fields,
            action_buttons,
            toggles,
        } = config;
        let inputs = fields.iter().map(|_| TextInputHandler::new()).collect();
        let mut buttons = action_buttons;
        if toggles {
            buttons.push(Btn::Regex);
            buttons.push(Btn::Case);
        }
        Self {
            fields,
            inputs,
            buttons,
            use_regex: false,
            case_sensitive: false,
            focus: 0,
            match_info: None,
            field_areas: Vec::new(),
            button_areas: Vec::new(),
        }
    }

    /// The focus ring: every field followed by every button.
    fn ring(&self) -> Vec<Control> {
        let mut ring: Vec<Control> = (0..self.fields.len()).map(Control::Field).collect();
        ring.extend((0..self.buttons.len()).map(Control::Button));
        ring
    }

    fn current(&self) -> Control {
        let ring = self.ring();
        ring[self.focus.min(ring.len().saturating_sub(1))]
    }

    fn field_index(&self, field: FindField) -> Option<usize> {
        self.fields.iter().position(|&f| f == field)
    }

    // === Host-facing accessors ===

    /// Number of terminal rows the bar needs: one per field plus the buttons
    /// row.
    pub fn height(&self) -> u16 {
        self.fields.len() as u16 + 1
    }

    /// Move focus to the first field (host calls this when entering the bar).
    pub fn focus_first(&mut self) {
        self.focus = 0;
    }

    /// Focus a specific field, if the bar exposes it. Fields occupy the leading
    /// slots of the focus ring, so the field index is the ring index.
    pub fn focus_field(&mut self, field: FindField) {
        if let Some(i) = self.field_index(field) {
            self.focus = i;
        }
    }

    /// Whether a click at `(col, row)` lands on any of the bar's controls
    /// (after a [`FindBar::render`] recorded their areas). Lets the host decide
    /// whether a click belongs to the bar or to its own body.
    pub fn click_hits_bar(&self, col: u16, row: u16) -> bool {
        self.field_areas.iter().any(|a| hit(*a, col, row))
            || self.button_areas.iter().any(|(a, _)| hit(*a, col, row))
    }

    /// Whether the bar exposes `field`.
    pub fn has_field(&self, field: FindField) -> bool {
        self.fields.contains(&field)
    }

    /// The field that currently has focus, if any (vs a button).
    pub fn focused_field(&self) -> Option<FindField> {
        match self.current() {
            Control::Field(i) => self.fields.get(i).copied(),
            Control::Button(_) => None,
        }
    }

    fn text_of(&self, field: FindField) -> Option<&str> {
        self.field_index(field).map(|i| self.inputs[i].text())
    }

    /// Current query text.
    pub fn find_text(&self) -> &str {
        self.text_of(FindField::Find).unwrap_or("")
    }

    /// Current replacement text (empty if there is no replace field).
    pub fn replace_text(&self) -> &str {
        self.text_of(FindField::Replace).unwrap_or("")
    }

    /// Current glob mask (empty if there is no mask field).
    pub fn mask_text(&self) -> &str {
        self.text_of(FindField::Mask).unwrap_or("")
    }

    /// Whether regex matching is enabled.
    pub fn use_regex(&self) -> bool {
        self.use_regex
    }

    /// Whether matching is case-sensitive.
    pub fn case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    /// Seed a field's text (e.g. restoring the previous query).
    pub fn set_text(&mut self, field: FindField, text: String) {
        if let Some(i) = self.field_index(field) {
            self.inputs[i] = TextInputHandler::with_default(text);
        }
    }

    /// Update the "N of M" counter.
    pub fn set_match_info(&mut self, current: usize, total: usize) {
        self.match_info = Some((current, total));
    }

    /// Clear the match counter.
    pub fn clear_match_info(&mut self) {
        self.match_info = None;
    }

    // === Input ===

    /// Handle a key while the bar holds focus. Returns the host action, if any.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<FindBarAction> {
        if key.code == KeyCode::Esc {
            return Some(FindBarAction::Close);
        }

        match self.current() {
            Control::Field(i) => self.handle_field_key(i, key),
            Control::Button(i) => self.handle_button_key(i, key),
        }
    }

    fn handle_field_key(&mut self, field_idx: usize, key: KeyEvent) -> Option<FindBarAction> {
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                self.focus_next();
                None
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.focus_prev();
                None
            }
            KeyCode::Enter => Some(FindBarAction::Submit),
            _ => match handle_input_key(&mut self.inputs[field_idx], key) {
                InputKeyResult::TextModified => Some(FindBarAction::QueryChanged),
                InputKeyResult::Handled => None,
                InputKeyResult::NotHandled => None,
            },
        }
    }

    fn handle_button_key(&mut self, btn_idx: usize, key: KeyEvent) -> Option<FindBarAction> {
        match key.code {
            KeyCode::Left | KeyCode::BackTab => {
                self.focus_prev();
                None
            }
            KeyCode::Right | KeyCode::Tab => {
                self.focus_next();
                None
            }
            KeyCode::Up => {
                // Jump back to the last field, if there is one.
                if !self.fields.is_empty() {
                    self.focus = self.fields.len() - 1;
                }
                None
            }
            KeyCode::Enter | KeyCode::Char(' ') => self.activate_button(btn_idx),
            _ => None,
        }
    }

    fn focus_next(&mut self) {
        let len = self.ring().len();
        if len > 0 {
            self.focus = (self.focus + 1) % len;
        }
    }

    fn focus_prev(&mut self) {
        let len = self.ring().len();
        if len > 0 {
            self.focus = (self.focus + len - 1) % len;
        }
    }

    fn activate_button(&mut self, btn_idx: usize) -> Option<FindBarAction> {
        match self.buttons.get(btn_idx).copied() {
            Some(Btn::Replace) => Some(FindBarAction::Replace),
            Some(Btn::ReplaceAll) => Some(FindBarAction::ReplaceAll),
            Some(Btn::Prev) => Some(FindBarAction::Previous),
            Some(Btn::Next) => Some(FindBarAction::Next),
            Some(Btn::Regex) => {
                self.use_regex = !self.use_regex;
                Some(FindBarAction::QueryChanged)
            }
            Some(Btn::Case) => {
                self.case_sensitive = !self.case_sensitive;
                Some(FindBarAction::QueryChanged)
            }
            None => None,
        }
    }

    /// Handle a mouse click. Clicking a field focuses it and positions the
    /// cursor; clicking a control activates it.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> Option<FindBarAction> {
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return None;
        }
        let (col, row) = (mouse.column, mouse.row);

        // Fields first.
        for (i, area) in self.field_areas.clone().into_iter().enumerate() {
            if hit(area, col, row) {
                // Fields occupy the leading slots of the focus ring.
                self.focus = i;
                let label_w = self.fields[i].label().len() as u16;
                let start_x = area.x + label_w;
                if col >= start_x {
                    let click_x = (col - start_x) as usize;
                    let pos = screen_x_to_char_pos(self.inputs[i].text(), click_x);
                    self.inputs[i].set_cursor_with_selection_start(pos);
                }
                return None;
            }
        }

        // Then buttons.
        let clicked = self
            .button_areas
            .iter()
            .find_map(|(area, idx)| hit(*area, col, row).then_some(*idx));
        if let Some(idx) = clicked {
            // Focus the clicked control too, so keyboard picks up from there.
            self.focus = self.fields.len() + idx;
            return self.activate_button(idx);
        }
        None
    }

    // === Rendering ===

    /// Render the bar into `area`. `active` is whether the bar (rather than the
    /// panel body) currently holds focus — it controls cursor/highlight display.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme, active: bool) {
        self.field_areas.clear();
        let focused_control = self.current();

        // One row per field.
        for (i, &field) in self.fields.iter().enumerate() {
            let row = Rect {
                x: area.x,
                y: area.y + i as u16,
                width: area.width,
                height: 1,
            };
            self.field_areas.push(row);
            let is_focused = active && matches!(focused_control, Control::Field(f) if f == i);
            render_labeled_input(
                buf,
                row,
                field.label(),
                self.inputs[i].text(),
                self.inputs[i].cursor_pos(),
                self.inputs[i].selection_range(),
                is_focused,
                theme,
            );
        }

        // Buttons + counter on the last reserved row.
        let buttons_row = Rect {
            x: area.x,
            y: area.y + self.fields.len() as u16,
            width: area.width,
            height: 1,
        };
        self.render_buttons(buttons_row, buf, theme, active, focused_control);
    }

    fn render_buttons(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        active: bool,
        focused_control: Control,
    ) {
        self.button_areas.clear();

        // Counter ("3 of 12") right-aligned.
        let counter = self
            .match_info
            .map(|(cur, total)| format!("{} of {}", cur, total))
            .unwrap_or_default();
        let counter_w = counter.chars().count() as u16;
        let counter_left = area.x + area.width.saturating_sub(counter_w);
        if !counter.is_empty() {
            buf.set_string(
                counter_left,
                area.y,
                &counter,
                Style::default().fg(theme.disabled),
            );
        }

        let mut x = area.x;
        for (idx, btn) in self.buttons.clone().into_iter().enumerate() {
            let focused = active && matches!(focused_control, Control::Button(b) if b == idx);
            let (text, style) = self.button_render(btn, focused, theme);
            let w = text.chars().count() as u16;
            if x + w >= counter_left {
                break;
            }
            self.button_areas.push((
                Rect {
                    x,
                    y: area.y,
                    width: w,
                    height: 1,
                },
                idx,
            ));
            buf.set_string(x, area.y, &text, style);
            x += w + 1;
        }
    }

    fn button_render(&self, btn: Btn, focused: bool, theme: &Theme) -> (String, Style) {
        if btn.is_toggle() {
            let (label, on) = match btn {
                Btn::Regex => (".*", self.use_regex),
                Btn::Case => ("Aa", self.case_sensitive),
                _ => unreachable!(),
            };
            let text = format!("[{}]", label);
            let mut style = if on {
                Style::default()
                    .fg(theme.accented_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.disabled)
            };
            if focused {
                style = style.add_modifier(Modifier::REVERSED);
            }
            (text, style)
        } else {
            let label = match btn {
                Btn::Replace => "Replace",
                Btn::ReplaceAll => "All",
                Btn::Prev => "◄ Prev",
                Btn::Next => "Next ►",
                _ => unreachable!(),
            };
            let text = if focused {
                format!("[ {} ]", label)
            } else {
                format!("  {}  ", label)
            };
            let mut style = Style::default().fg(theme.fg);
            if focused {
                style = style.add_modifier(Modifier::BOLD | Modifier::REVERSED);
            }
            (text, style)
        }
    }
}

fn hit(area: Rect, col: u16, row: u16) -> bool {
    col >= area.x && col < area.x + area.width && row == area.y
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn content_bar() -> FindBar {
        FindBar::new(FindBarConfig {
            fields: vec![FindField::Mask, FindField::Find, FindField::Replace],
            action_buttons: vec![Btn::Prev, Btn::Next, Btn::ReplaceAll],
            toggles: true,
        })
    }

    #[test]
    fn height_is_fields_plus_button_row() {
        assert_eq!(content_bar().height(), 4);
        let find_only = FindBar::new(FindBarConfig {
            fields: vec![FindField::Find],
            action_buttons: vec![Btn::Prev, Btn::Next],
            toggles: true,
        });
        assert_eq!(find_only.height(), 2);
    }

    #[test]
    fn toggles_are_appended_after_action_buttons() {
        let bar = content_bar();
        assert_eq!(
            bar.buttons,
            vec![Btn::Prev, Btn::Next, Btn::ReplaceAll, Btn::Regex, Btn::Case]
        );
    }

    #[test]
    fn typing_in_first_field_edits_it_and_reports_change() {
        let mut bar = content_bar();
        assert_eq!(bar.focused_field(), Some(FindField::Mask));
        assert_eq!(
            bar.handle_key(key(KeyCode::Char('*'))),
            Some(FindBarAction::QueryChanged)
        );
        assert_eq!(bar.mask_text(), "*");
        assert_eq!(bar.find_text(), "");
    }

    #[test]
    fn tab_walks_fields_then_buttons_and_wraps() {
        let mut bar = content_bar();
        // Mask -> Find -> Replace
        bar.handle_key(key(KeyCode::Tab));
        assert_eq!(bar.focused_field(), Some(FindField::Find));
        bar.handle_key(key(KeyCode::Tab));
        assert_eq!(bar.focused_field(), Some(FindField::Replace));
        // Replace -> first button (no field focus anymore)
        bar.handle_key(key(KeyCode::Tab));
        assert_eq!(bar.focused_field(), None);
        // Walk all 5 buttons -> wrap back to Mask
        for _ in 0..5 {
            bar.handle_key(key(KeyCode::Tab));
        }
        assert_eq!(bar.focused_field(), Some(FindField::Mask));
    }

    #[test]
    fn left_right_cycle_buttons_when_focused_on_them() {
        let mut bar = content_bar();
        // Move onto the first button.
        for _ in 0..3 {
            bar.handle_key(key(KeyCode::Tab));
        }
        assert_eq!(bar.current(), Control::Button(0)); // Prev
                                                       // Right cycles forward through buttons.
        bar.handle_key(key(KeyCode::Right));
        assert_eq!(bar.current(), Control::Button(1)); // Next
                                                       // Left cycles back.
        bar.handle_key(key(KeyCode::Left));
        assert_eq!(bar.current(), Control::Button(0));
    }

    #[test]
    fn activating_buttons_yields_actions() {
        let mut bar = content_bar();
        // Focus the Prev button (index 0 among buttons -> ring index 3).
        bar.focus = 3;
        assert_eq!(
            bar.handle_key(key(KeyCode::Enter)),
            Some(FindBarAction::Previous)
        );
        bar.focus = 4; // Next
        assert_eq!(
            bar.handle_key(key(KeyCode::Char(' '))),
            Some(FindBarAction::Next)
        );
        bar.focus = 5; // ReplaceAll
        assert_eq!(
            bar.handle_key(key(KeyCode::Enter)),
            Some(FindBarAction::ReplaceAll)
        );
    }

    #[test]
    fn toggles_flip_state_and_report_query_change() {
        let mut bar = content_bar();
        bar.focus = 6; // Regex toggle
        assert!(!bar.use_regex());
        assert_eq!(
            bar.handle_key(key(KeyCode::Enter)),
            Some(FindBarAction::QueryChanged)
        );
        assert!(bar.use_regex());
        bar.focus = 7; // Case toggle
        assert!(!bar.case_sensitive());
        assert_eq!(
            bar.handle_key(key(KeyCode::Char(' '))),
            Some(FindBarAction::QueryChanged)
        );
        assert!(bar.case_sensitive());
    }

    #[test]
    fn enter_on_a_field_submits() {
        let mut bar = content_bar();
        bar.handle_key(key(KeyCode::Tab)); // -> Find
        assert_eq!(
            bar.handle_key(key(KeyCode::Enter)),
            Some(FindBarAction::Submit)
        );
        assert_eq!(bar.focused_field(), Some(FindField::Find));
    }

    #[test]
    fn esc_closes_from_anywhere() {
        let mut bar = content_bar();
        assert_eq!(
            bar.handle_key(key(KeyCode::Esc)),
            Some(FindBarAction::Close)
        );
        bar.focus = 5;
        assert_eq!(
            bar.handle_key(key(KeyCode::Esc)),
            Some(FindBarAction::Close)
        );
    }

    #[test]
    fn seed_and_read_back_text() {
        let mut bar = content_bar();
        bar.set_text(FindField::Find, "needle".into());
        bar.set_text(FindField::Replace, "thread".into());
        assert_eq!(bar.find_text(), "needle");
        assert_eq!(bar.replace_text(), "thread");
    }

    #[test]
    fn up_from_buttons_returns_to_last_field() {
        let mut bar = content_bar();
        bar.focus = 4; // a button
        bar.handle_key(key(KeyCode::Up));
        assert_eq!(bar.focused_field(), Some(FindField::Replace));
    }
}
