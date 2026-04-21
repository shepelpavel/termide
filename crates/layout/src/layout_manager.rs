//! Layout manager for panel arrangement.

use anyhow::{anyhow, Result};
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use termide_config::Config;
use termide_core::{Panel, WidthPreference};

use crate::PanelGroup;

/// Compute per-panel rectangles inside `main_area` using the same Layout
/// constraints as the renderer. Returns
/// `Vec<(group_idx, panel_idx, rect, is_expanded)>` — the authoritative
/// geometry used by mouse hit-testing and drag-overlay rendering.
pub fn calculate_panel_rects(
    panel_groups: &[PanelGroup],
    main_area: Rect,
) -> Vec<(usize, usize, Rect, bool)> {
    let mut result = Vec::new();
    if panel_groups.is_empty() {
        return result;
    }

    let group_constraints: Vec<Constraint> = panel_groups
        .iter()
        .map(|g| {
            let width = g.width.unwrap_or(main_area.width);
            Constraint::Length(width.max(20))
        })
        .collect();

    let group_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(group_constraints)
        .split(main_area);

    for (group_idx, group) in panel_groups.iter().enumerate() {
        if group.is_empty() || group_chunks[group_idx].height == 0 {
            continue;
        }
        let group_area = group_chunks[group_idx];
        let expanded_idx = group.expanded_index();

        let vertical_constraints: Vec<Constraint> = (0..group.len())
            .map(|i| {
                if i == expanded_idx {
                    Constraint::Min(0)
                } else {
                    Constraint::Length(1)
                }
            })
            .collect();

        let vertical_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vertical_constraints)
            .split(group_area);

        for panel_idx in 0..group.len() {
            let is_expanded = panel_idx == expanded_idx;
            result.push((
                group_idx,
                panel_idx,
                vertical_chunks[panel_idx],
                is_expanded,
            ));
        }
    }

    result
}

/// Determine the drop target under the cursor given pre-calculated panel
/// rects. Shared by the mouse handler and the drag overlay renderer.
///
/// Returns `None` if the cursor is outside the panel area (e.g. in the
/// menu/status bar or over an empty main area).
pub fn compute_drop_target(
    rects: &[(usize, usize, Rect, bool)],
    x: u16,
    y: u16,
) -> Option<PanelDropTarget> {
    if rects.is_empty() {
        return None;
    }

    // Collapse panel rects into group spans (left, right edges).
    let mut group_spans: Vec<(usize, u16, u16)> = Vec::new();
    for (gi, _, rect, _) in rects {
        if let Some(entry) = group_spans.iter_mut().find(|(g, _, _)| *g == *gi) {
            entry.1 = entry.1.min(rect.x);
            entry.2 = entry.2.max(rect.x + rect.width);
        } else {
            group_spans.push((*gi, rect.x, rect.x + rect.width));
        }
    }
    group_spans.sort_by_key(|(gi, _, _)| *gi);

    const GUTTER: u16 = 2;
    for i in 0..group_spans.len().saturating_sub(1) {
        let right_edge = group_spans[i].2;
        let next_left = group_spans[i + 1].1;
        let zone_start = right_edge.saturating_sub(GUTTER);
        let zone_end = next_left.saturating_add(GUTTER);
        if x >= zone_start && x < zone_end {
            return Some(PanelDropTarget::NewGroup {
                insert_at: group_spans[i + 1].0,
            });
        }
    }
    if let Some((last_gi, _, right_edge)) = group_spans.last() {
        if x >= *right_edge {
            return Some(PanelDropTarget::NewGroup {
                insert_at: *last_gi + 1,
            });
        }
    }

    for (gi, pi, rect, _) in rects {
        if x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height {
            let at_position = if y == rect.y { *pi } else { *pi + 1 };
            return Some(PanelDropTarget::IntoGroup {
                group_idx: *gi,
                at_position,
            });
        }
    }

    None
}

/// Where a dragged panel should be dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelDropTarget {
    /// Insert into an existing group at the given position (expanding to it).
    IntoGroup {
        group_idx: usize,
        at_position: usize,
    },
    /// Create a new group at the given index.
    NewGroup { insert_at: usize },
}

/// Panel layout manager with accordion support.
pub struct LayoutManager {
    /// Panel groups (horizontal columns with vertical accordion inside).
    pub panel_groups: Vec<PanelGroup>,
    /// Current focus (active group index).
    pub focus: usize,
}

impl LayoutManager {
    /// Create new empty manager.
    pub fn new() -> Self {
        Self {
            panel_groups: Vec::new(),
            focus: 0,
        }
    }

    /// Add panel with automatic stacking based on available width.
    pub fn add_panel(&mut self, panel: Box<dyn Panel>, config: &Config, terminal_width: u16) {
        let available_width = terminal_width;

        if self.panel_groups.is_empty() {
            let group = PanelGroup::new(panel);
            self.panel_groups.push(group);
            self.focus = 0;
            return;
        }

        let num_groups_after_split = self.panel_groups.len() + 1;
        let new_width_if_split = available_width / num_groups_after_split as u16;

        if new_width_if_split < config.general.auto_stack_threshold {
            // Auto-stacking: pick group by width preference
            let target_group_idx = self.find_preferred_group(&*panel);
            let group = &mut self.panel_groups[target_group_idx];
            let insert_pos = group.expanded_index() + 1;
            group.insert_panel(insert_pos, panel);
            group.set_expanded(insert_pos);
            self.focus = target_group_idx;
        } else {
            // Create new group horizontally
            let new_group = PanelGroup::new(panel);
            self.panel_groups.push(new_group);
            self.focus = self.panel_groups.len() - 1;
            self.redistribute_widths_proportionally(available_width);
        }
    }

    /// Add panel without changing focus.
    /// Used for preview panels where focus should stay on the source panel.
    pub fn add_panel_without_focus(
        &mut self,
        panel: Box<dyn Panel>,
        config: &Config,
        terminal_width: u16,
    ) {
        let saved_focus = self.focus;
        self.add_panel(panel, config, terminal_width);
        self.focus = saved_focus;
    }

    /// Find panel by name, expand it in its group, return mutable reference.
    /// Does NOT change focus. Used for reusing existing panels.
    pub fn find_and_expand_panel_by_name(&mut self, name: &str) -> Option<&mut Box<dyn Panel>> {
        // First pass: find the group and panel index
        let mut found: Option<(usize, usize)> = None;
        for (group_idx, group) in self.panel_groups.iter().enumerate() {
            for (panel_idx, panel) in group.panels().iter().enumerate() {
                if panel.name() == name {
                    found = Some((group_idx, panel_idx));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }

        // Second pass: expand and return mutable reference
        if let Some((group_idx, panel_idx)) = found {
            let group = &mut self.panel_groups[group_idx];
            group.set_expanded(panel_idx);
            return group.panels_mut().get_mut(panel_idx);
        }
        None
    }

    /// Find the best group for a panel based on its width preference.
    fn find_preferred_group(&self, panel: &dyn Panel) -> usize {
        match panel.width_preference() {
            WidthPreference::NoPreference => self.focus,
            WidthPreference::PreferNarrow => self
                .panel_groups
                .iter()
                .enumerate()
                .min_by_key(|(_, g)| g.width.unwrap_or(u16::MAX))
                .map(|(idx, _)| idx)
                .unwrap_or(self.focus),
            WidthPreference::PreferWide => self
                .panel_groups
                .iter()
                .enumerate()
                .max_by_key(|(_, g)| g.width.unwrap_or(0))
                .map(|(idx, _)| idx)
                .unwrap_or(self.focus),
        }
    }

    /// Toggle panel stacking/unstacking with smart direction choice.
    pub fn toggle_panel_stacking(&mut self, available_width: u16) -> Result<()> {
        let active_group_idx = self.focus;

        let group = self
            .panel_groups
            .get(active_group_idx)
            .ok_or_else(|| anyhow!("No active group"))?;

        let group_len = group.len();

        if group_len == 1 {
            if self.panel_groups.len() == 1 {
                return Err(anyhow!("Only one group exists, nothing to merge with"));
            }

            // Priority: left
            if active_group_idx > 0 {
                self.merge_into_left(active_group_idx, available_width)
            } else if active_group_idx + 1 < self.panel_groups.len() {
                self.merge_into_right(active_group_idx, available_width)
            } else {
                Err(anyhow!("No adjacent group found"))
            }
        } else {
            self.unstack_current_panel(active_group_idx, available_width)
        }
    }

    fn merge_into_left(&mut self, active_group_idx: usize, available_width: u16) -> Result<()> {
        if active_group_idx == 0 {
            return Err(anyhow!("No left group to merge into"));
        }

        let current_group = self.panel_groups.remove(active_group_idx);
        let mut panels = current_group.take_panels();
        let panel = panels.pop().ok_or_else(|| anyhow!("No panel to merge"))?;

        let left_group_idx = active_group_idx - 1;
        if let Some(left_group) = self.panel_groups.get_mut(left_group_idx) {
            left_group.add_panel(panel);
            left_group.set_expanded(left_group.len() - 1);
        }

        self.focus = left_group_idx;
        self.redistribute_widths_proportionally(available_width);
        Ok(())
    }

    fn merge_into_right(&mut self, active_group_idx: usize, available_width: u16) -> Result<()> {
        if active_group_idx >= self.panel_groups.len().saturating_sub(1) {
            return Err(anyhow!("No right group to merge into"));
        }

        let current_group = self.panel_groups.remove(active_group_idx);
        let mut panels = current_group.take_panels();
        let panel = panels.pop().ok_or_else(|| anyhow!("No panel to merge"))?;

        if let Some(right_group) = self.panel_groups.get_mut(active_group_idx) {
            right_group.add_panel(panel);
            right_group.set_expanded(right_group.len() - 1);
        }

        self.focus = active_group_idx;
        self.redistribute_widths_proportionally(available_width);
        Ok(())
    }

    fn unstack_current_panel(
        &mut self,
        active_group_idx: usize,
        available_width: u16,
    ) -> Result<()> {
        let group = self
            .panel_groups
            .get_mut(active_group_idx)
            .ok_or_else(|| anyhow!("No active group"))?;

        if group.len() <= 1 {
            return Err(anyhow!("Panel is already alone in group"));
        }

        let expanded_idx = group.expanded_index();
        let panel_to_extract = group
            .remove_panel(expanded_idx)
            .ok_or_else(|| anyhow!("No panel to unstack"))?;

        let new_group = PanelGroup::new(panel_to_extract);
        self.panel_groups.insert(active_group_idx + 1, new_group);
        self.focus = active_group_idx + 1;
        self.redistribute_widths_proportionally(available_width);
        Ok(())
    }

    /// Move panel to previous group.
    pub fn move_panel_to_prev_group(&mut self, available_width: u16) -> Result<()> {
        let group_idx = self.focus;

        if group_idx == 0 {
            return Ok(());
        }

        if self.panel_groups.get(group_idx).map(|g| g.len()) == Some(1) {
            self.panel_groups.swap(group_idx - 1, group_idx);
            self.focus = group_idx - 1;
        } else {
            let group = self
                .panel_groups
                .get_mut(group_idx)
                .expect("group_idx validated at function start");
            let expanded_idx = group.expanded_index();
            let panel = group
                .remove_panel(expanded_idx)
                .expect("expanded panel must exist in non-empty group");

            let prev_group = self
                .panel_groups
                .get_mut(group_idx - 1)
                .expect("prev group exists since group_idx > 0");
            prev_group.add_panel(panel);
            prev_group.set_expanded(prev_group.len() - 1);
            self.focus = group_idx - 1;

            if self
                .panel_groups
                .get(group_idx)
                .map(|g| g.is_empty())
                .unwrap_or(false)
            {
                self.panel_groups.remove(group_idx);
                self.redistribute_widths_proportionally(available_width);
            }
        }
        Ok(())
    }

    /// Move panel to next group.
    pub fn move_panel_to_next_group(&mut self, available_width: u16) -> Result<()> {
        let group_idx = self.focus;

        if group_idx >= self.panel_groups.len().saturating_sub(1) {
            return Ok(());
        }

        if self.panel_groups.get(group_idx).map(|g| g.len()) == Some(1) {
            self.panel_groups.swap(group_idx, group_idx + 1);
            self.focus = group_idx + 1;
        } else {
            let group = self
                .panel_groups
                .get_mut(group_idx)
                .expect("group_idx validated at function start");
            let expanded_idx = group.expanded_index();
            let panel = group
                .remove_panel(expanded_idx)
                .expect("expanded panel must exist in non-empty group");

            let next_group = self
                .panel_groups
                .get_mut(group_idx + 1)
                .expect("next group exists since group_idx < len-1");
            next_group.add_panel(panel);
            next_group.set_expanded(next_group.len() - 1);
            self.focus = group_idx + 1;

            if self
                .panel_groups
                .get(group_idx)
                .map(|g| g.is_empty())
                .unwrap_or(false)
            {
                self.panel_groups.remove(group_idx);
                self.focus = group_idx;
                self.redistribute_widths_proportionally(available_width);
            }
        }
        Ok(())
    }

    /// Move panel to first group.
    pub fn move_panel_to_first_group(&mut self, available_width: u16) -> Result<()> {
        let group_idx = self.focus;

        if group_idx == 0 {
            return Ok(());
        }

        let is_alone = self.panel_groups.get(group_idx).map(|g| g.len()) == Some(1);
        let group = self
            .panel_groups
            .get_mut(group_idx)
            .expect("group_idx validated at function start");
        let expanded_idx = group.expanded_index();
        let panel = group
            .remove_panel(expanded_idx)
            .expect("expanded panel must exist in non-empty group");

        let first_group = self
            .panel_groups
            .get_mut(0)
            .expect("at least one group must exist");
        first_group.add_panel(panel);
        let target_len = first_group.len();
        first_group.set_expanded(target_len - 1);
        self.focus = 0;

        if is_alone {
            self.panel_groups.remove(group_idx);
            self.redistribute_widths_proportionally(available_width);
        }
        Ok(())
    }

    /// Move panel to last group.
    pub fn move_panel_to_last_group(&mut self, available_width: u16) -> Result<()> {
        let group_idx = self.focus;
        let last_idx = self.panel_groups.len().saturating_sub(1);

        if group_idx == last_idx {
            return Ok(());
        }

        let is_alone = self.panel_groups.get(group_idx).map(|g| g.len()) == Some(1);
        let group = self
            .panel_groups
            .get_mut(group_idx)
            .expect("group_idx validated at function start");
        let expanded_idx = group.expanded_index();
        let panel = group
            .remove_panel(expanded_idx)
            .expect("expanded panel must exist in non-empty group");

        let last_group = self
            .panel_groups
            .get_mut(last_idx)
            .expect("last_idx is valid since group_idx != last_idx");
        last_group.add_panel(panel);
        let target_len = last_group.len();
        last_group.set_expanded(target_len - 1);

        if is_alone {
            self.panel_groups.remove(group_idx);
            self.redistribute_widths_proportionally(available_width);
        }

        self.focus = self.panel_groups.len().saturating_sub(1);
        Ok(())
    }

    /// Switch to next group (horizontal).
    pub fn next_group(&mut self) {
        if !self.panel_groups.is_empty() {
            self.focus = (self.focus + 1) % self.panel_groups.len();
        }
    }

    /// Switch to previous group (horizontal).
    pub fn prev_group(&mut self) {
        if !self.panel_groups.is_empty() {
            self.focus = if self.focus == 0 {
                self.panel_groups.len() - 1
            } else {
                self.focus - 1
            };
        }
    }

    /// Switch to next panel in current group (vertical).
    pub fn next_panel_in_group(&mut self) {
        if let Some(group) = self.panel_groups.get_mut(self.focus) {
            group.next_panel();
        }
    }

    /// Switch to previous panel in current group (vertical).
    pub fn prev_panel_in_group(&mut self) {
        if let Some(group) = self.panel_groups.get_mut(self.focus) {
            group.prev_panel();
        }
    }

    /// Move active panel up in current group.
    pub fn move_panel_up_in_group(&mut self) -> Result<()> {
        let group = self
            .panel_groups
            .get_mut(self.focus)
            .ok_or_else(|| anyhow!("No active group"))?;
        let expanded_idx = group.expanded_index();
        group.move_panel_up(expanded_idx)
    }

    /// Move active panel down in current group.
    pub fn move_panel_down_in_group(&mut self) -> Result<()> {
        let group = self
            .panel_groups
            .get_mut(self.focus)
            .ok_or_else(|| anyhow!("No active group"))?;
        let expanded_idx = group.expanded_index();
        group.move_panel_down(expanded_idx)
    }

    /// Move an arbitrary panel from `(from_gi, from_pi)` to the given drop
    /// target. Handles source-group cleanup, target index shifting and
    /// width redistribution.
    ///
    /// Returns `(final_group_idx, final_panel_idx)` where the panel ended
    /// up, so the caller can update focus / expanded state.
    pub fn move_panel_to(
        &mut self,
        from_gi: usize,
        from_pi: usize,
        target: PanelDropTarget,
        available_width: u16,
    ) -> Result<(usize, usize)> {
        let source_group = self
            .panel_groups
            .get(from_gi)
            .ok_or_else(|| anyhow!("Invalid source group index"))?;
        if from_pi >= source_group.len() {
            return Err(anyhow!("Invalid source panel index"));
        }

        // No-op: dropping a panel exactly where it already lives.
        if let PanelDropTarget::IntoGroup {
            group_idx,
            at_position,
        } = target
        {
            if group_idx == from_gi && source_group.len() == 1 {
                return Ok((from_gi, from_pi));
            }
            if group_idx == from_gi && (at_position == from_pi || at_position == from_pi + 1) {
                return Ok((from_gi, from_pi));
            }
        }
        if let PanelDropTarget::NewGroup { insert_at } = target {
            if source_group.len() == 1 && (insert_at == from_gi || insert_at == from_gi + 1) {
                return Ok((from_gi, from_pi));
            }
        }

        // Extract the panel from the source group.
        let panel = self
            .panel_groups
            .get_mut(from_gi)
            .and_then(|g| g.remove_panel(from_pi))
            .ok_or_else(|| anyhow!("Failed to remove source panel"))?;

        // If the source group is now empty, drop it and shift downstream
        // indices so the target still points at the right slot.
        let source_was_removed = self
            .panel_groups
            .get(from_gi)
            .map(|g| g.is_empty())
            .unwrap_or(false);

        if source_was_removed {
            self.panel_groups.remove(from_gi);
        }

        // Adjust the target indices for a removed source group.
        let adjusted_target = if source_was_removed {
            match target {
                PanelDropTarget::IntoGroup {
                    group_idx,
                    at_position,
                } => {
                    let gi = if group_idx > from_gi {
                        group_idx - 1
                    } else {
                        group_idx
                    };
                    PanelDropTarget::IntoGroup {
                        group_idx: gi,
                        at_position,
                    }
                }
                PanelDropTarget::NewGroup { insert_at } => {
                    let at = if insert_at > from_gi {
                        insert_at - 1
                    } else {
                        insert_at
                    };
                    PanelDropTarget::NewGroup { insert_at: at }
                }
            }
        } else {
            target
        };

        // Perform the insertion.
        let (final_gi, final_pi) = match adjusted_target {
            PanelDropTarget::IntoGroup {
                group_idx,
                at_position,
            } => {
                let group = self
                    .panel_groups
                    .get_mut(group_idx)
                    .ok_or_else(|| anyhow!("Invalid target group index"))?;
                let pos = at_position.min(group.len());
                group.insert_panel(pos, panel);
                group.set_expanded(pos);
                if source_was_removed {
                    self.redistribute_widths_proportionally(available_width);
                }
                (group_idx, pos)
            }
            PanelDropTarget::NewGroup { insert_at } => {
                let pos = insert_at.min(self.panel_groups.len());
                self.panel_groups.insert(pos, PanelGroup::new(panel));
                self.redistribute_widths_proportionally(available_width);
                (pos, 0)
            }
        };

        self.focus = final_gi;
        Ok((final_gi, final_pi))
    }

    /// Get mutable reference to active panel.
    pub fn active_panel_mut(&mut self) -> Option<&mut Box<dyn Panel>> {
        self.panel_groups
            .get_mut(self.focus)
            .and_then(|group| group.expanded_panel_mut())
    }

    /// Get reference to active panel.
    #[allow(clippy::borrowed_box)]
    pub fn active_panel(&self) -> Option<&Box<dyn Panel>> {
        self.panel_groups
            .get(self.focus)
            .and_then(|group| group.expanded_panel())
    }

    /// Get active group index.
    pub fn active_group_index(&self) -> Option<usize> {
        Some(self.focus)
    }

    /// Iterator over all panels (mutable).
    pub fn iter_all_panels_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Panel>> {
        self.panel_groups
            .iter_mut()
            .flat_map(|g| g.panels_mut().iter_mut())
    }

    /// Iterator over all panels with their expanded state (mutable).
    /// Returns `(panel, is_expanded)` for each panel.
    pub fn iter_all_panels_with_expanded_state_mut(
        &mut self,
    ) -> impl Iterator<Item = (&mut Box<dyn Panel>, bool)> {
        self.panel_groups.iter_mut().flat_map(|g| {
            let expanded = g.expanded_index();
            g.panels_mut()
                .iter_mut()
                .enumerate()
                .map(move |(idx, panel)| (panel, idx == expanded))
        })
    }

    /// Iterator over only expanded (visible) panels (mutable).
    pub fn iter_expanded_panels_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Panel>> {
        self.panel_groups
            .iter_mut()
            .filter_map(|g| g.expanded_panel_mut())
    }

    /// Close active panel.
    pub fn close_active_panel(&mut self, available_width: u16) -> Result<()> {
        let active_group_idx = self.focus;

        let group = self
            .panel_groups
            .get_mut(active_group_idx)
            .ok_or_else(|| anyhow!("No active group"))?;

        if group.len() <= 1 {
            self.panel_groups.remove(active_group_idx);

            if !self.panel_groups.is_empty() {
                self.focus = active_group_idx.min(self.panel_groups.len() - 1);
            } else {
                self.focus = 0;
            }
            self.redistribute_widths_proportionally(available_width);
        } else {
            let expanded_idx = group.expanded_index();
            group.remove_panel(expanded_idx);
        }
        Ok(())
    }

    /// Check if active panel can be closed.
    pub fn can_close_active(&self) -> bool {
        !self.panel_groups.is_empty()
    }

    /// Check if there are any panels.
    pub fn has_panels(&self) -> bool {
        !self.panel_groups.is_empty()
    }

    /// Get total panel count.
    pub fn panel_count(&self) -> usize {
        self.panel_groups.iter().map(|g| g.len()).sum()
    }

    /// Calculate actual widths of all groups.
    pub fn calculate_actual_widths(&self, available_width: u16) -> Vec<u16> {
        if self.panel_groups.is_empty() {
            return Vec::new();
        }

        let total_fixed_width: u16 = self.panel_groups.iter().filter_map(|g| g.width).sum();
        let auto_count = self
            .panel_groups
            .iter()
            .filter(|g| g.width.is_none())
            .count();
        let remaining_width = available_width.saturating_sub(total_fixed_width);
        let auto_width = if auto_count > 0 {
            remaining_width / auto_count as u16
        } else {
            0
        };

        self.panel_groups
            .iter()
            .map(|g| g.width.unwrap_or(auto_width))
            .collect()
    }

    /// Proportionally redistribute group widths.
    pub fn redistribute_widths_proportionally(&mut self, available_width: u16) {
        if self.panel_groups.is_empty() {
            return;
        }

        if self.panel_groups.len() == 1 {
            self.panel_groups[0].width = Some(available_width.max(20));
            return;
        }

        // Freeze auto-width groups
        let has_auto_groups = self.panel_groups.iter().any(|g| g.width.is_none());
        if has_auto_groups {
            let auto_count = self
                .panel_groups
                .iter()
                .filter(|g| g.width.is_none())
                .count();
            let fixed_groups: Vec<u16> = self.panel_groups.iter().filter_map(|g| g.width).collect();

            if !fixed_groups.is_empty() && auto_count > 0 {
                let fixed_total: u16 = fixed_groups.iter().sum();
                let remaining = available_width.saturating_sub(fixed_total);
                let per_auto = (remaining / auto_count as u16).max(20);
                for group in self.panel_groups.iter_mut() {
                    if group.width.is_none() {
                        group.width = Some(per_auto);
                    }
                }
            } else {
                let actual_widths_before_freeze = self.calculate_actual_widths(available_width);
                for (idx, &width) in actual_widths_before_freeze.iter().enumerate() {
                    if self.panel_groups[idx].width.is_none() {
                        self.panel_groups[idx].width = Some(width.max(20));
                    }
                }
            }
        }

        let actual_widths = self.calculate_actual_widths(available_width);
        let total_actual: u16 = actual_widths.iter().sum();

        if total_actual == 0 {
            return;
        }

        let min_width: u16 = 20;
        let n = actual_widths.len();
        let min_total = min_width * n as u16;

        // If all groups at minimum already exceed budget, just assign minimums.
        if min_total >= available_width {
            let mut new_widths = vec![min_width; n];
            // Give any leftover to the last group (may be 0)
            let last = n - 1;
            new_widths[last] = available_width
                .saturating_sub(min_width * (n - 1) as u16)
                .max(min_width);
            for (idx, &width) in new_widths.iter().enumerate() {
                self.panel_groups[idx].width = Some(width);
            }
            return;
        }

        // Compute proportional widths using floor, enforcing minimum.
        // Track fractional remainders for largest-remainder distribution.
        let mut new_widths = Vec::with_capacity(n);
        let mut remainders = Vec::with_capacity(n);
        let mut allocated_width: u16 = 0;

        for (idx, &actual_width) in actual_widths.iter().enumerate() {
            let proportion = actual_width as f64 / total_actual as f64;
            let exact = available_width as f64 * proportion;
            let floored = (exact.floor() as u16).max(min_width);
            new_widths.push(floored);
            // Only groups above minimum can receive remainder pixels
            let remainder = if floored > min_width {
                exact - floored as f64
            } else {
                // Was clamped to min_width, fractional part is not meaningful
                -1.0
            };
            remainders.push((idx, remainder));
            allocated_width += floored;
        }

        // Distribute leftover pixels to groups with the largest fractional remainders
        let mut leftover = available_width.saturating_sub(allocated_width);
        if leftover > 0 {
            remainders.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            for &(idx, _) in &remainders {
                if leftover == 0 {
                    break;
                }
                new_widths[idx] += 1;
                leftover -= 1;
            }
        }

        // If over-allocated (due to min_width clamps pushing total up), trim largest groups
        let mut total: u16 = new_widths.iter().sum();
        while total > available_width {
            // Find largest group above minimum
            if let Some(idx) = new_widths
                .iter()
                .enumerate()
                .filter(|(_, &w)| w > min_width)
                .max_by_key(|(_, &w)| w)
                .map(|(i, _)| i)
            {
                new_widths[idx] -= 1;
                total -= 1;
            } else {
                break; // all at minimum, can't reduce further
            }
        }

        for (idx, &width) in new_widths.iter().enumerate() {
            self.panel_groups[idx].width = Some(width);
        }
    }

    /// Set focus to specific group index.
    pub fn set_focus(&mut self, index: usize) {
        if index < self.panel_groups.len() {
            self.focus = index;
        }
    }

    /// Get mutable reference to group by index.
    pub fn get_group_mut(&mut self, index: usize) -> Option<&mut PanelGroup> {
        self.panel_groups.get_mut(index)
    }

    /// Get reference to group by index.
    pub fn get_group(&self, index: usize) -> Option<&PanelGroup> {
        self.panel_groups.get(index)
    }

    /// Get number of groups.
    pub fn group_count(&self) -> usize {
        self.panel_groups.len()
    }

    /// Find divider at given position (for drag resize).
    ///
    /// Returns divider index if position is within grab zone (±1 from divider).
    /// Divider N is between groups N and N+1.
    pub fn find_divider_at_position(&self, x: u16, y: u16, terminal_height: u16) -> Option<usize> {
        // Skip menu row (y == 0) and status bar (y == terminal_height - 1)
        if y == 0 || y >= terminal_height.saturating_sub(1) {
            return None;
        }

        // Need at least 2 groups for a divider
        if self.panel_groups.len() < 2 {
            return None;
        }

        let mut current_x: u16 = 0;
        for (idx, group) in self.panel_groups.iter().enumerate() {
            current_x += group.width.unwrap_or(0);

            // Check if this is not the last group (divider exists after it)
            if idx < self.panel_groups.len() - 1 {
                // Grab zone: [current_x - 1, current_x]
                if x >= current_x.saturating_sub(1) && x <= current_x {
                    return Some(idx);
                }
            }
        }
        None
    }

    /// Get X positions of all dividers.
    ///
    /// Returns Vec of (divider_index, x_position).
    pub fn get_divider_positions(&self) -> Vec<(usize, u16)> {
        let mut positions = Vec::new();
        let mut current_x: u16 = 0;

        for (idx, group) in self.panel_groups.iter().enumerate() {
            current_x += group.width.unwrap_or(0);

            // Divider exists after each group except the last
            if idx < self.panel_groups.len() - 1 {
                positions.push((idx, current_x));
            }
        }
        positions
    }

    /// Resize two adjacent groups.
    ///
    /// `left_idx` is the index of the left group (divider is between left_idx and left_idx+1).
    pub fn resize_groups(&mut self, left_idx: usize, new_left_width: u16, new_right_width: u16) {
        if left_idx + 1 >= self.panel_groups.len() {
            return;
        }

        self.panel_groups[left_idx].width = Some(new_left_width);
        self.panel_groups[left_idx + 1].width = Some(new_right_width);
    }
}

impl Default for LayoutManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;
    use ratatui::{buffer::Buffer, layout::Rect};
    use std::any::Any;
    use termide_core::{PanelEvent, RenderContext, WidthPreference};

    /// Minimal mock panel for layout tests.
    struct MockPanel {
        name: &'static str,
        width_pref: WidthPreference,
    }

    impl MockPanel {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                width_pref: WidthPreference::NoPreference,
            }
        }

        #[allow(dead_code)]
        fn with_width_pref(name: &'static str, pref: WidthPreference) -> Self {
            Self {
                name,
                width_pref: pref,
            }
        }
    }

    impl Panel for MockPanel {
        fn name(&self) -> &'static str {
            self.name
        }
        fn title(&self) -> String {
            self.name.to_string()
        }
        fn render(&mut self, _area: Rect, _buf: &mut Buffer, _ctx: &RenderContext) {}
        fn handle_key(&mut self, _key: KeyEvent) -> Vec<PanelEvent> {
            vec![]
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn width_preference(&self) -> WidthPreference {
            self.width_pref
        }
    }

    fn make_config(threshold: u16) -> Config {
        let mut config = Config::default();
        config.general.auto_stack_threshold = threshold;
        config
    }

    fn panel(name: &'static str) -> Box<dyn Panel> {
        Box::new(MockPanel::new(name))
    }

    // =========================================================================
    // Panel stacking / unstacking
    // =========================================================================

    #[test]
    fn test_add_panel_to_empty_layout() {
        let mut lm = LayoutManager::new();
        let config = make_config(80);
        lm.add_panel(panel("a"), &config, 200);
        assert_eq!(lm.group_count(), 1);
        assert_eq!(lm.panel_count(), 1);
        assert_eq!(lm.focus, 0);
    }

    #[test]
    fn test_add_panel_creates_new_group_when_wide() {
        let mut lm = LayoutManager::new();
        let config = make_config(40); // threshold 40
        lm.add_panel(panel("a"), &config, 200);
        lm.add_panel(panel("b"), &config, 200);
        // 200 / 2 = 100 >= 40, so new group
        assert_eq!(lm.group_count(), 2);
        assert_eq!(lm.panel_count(), 2);
        assert_eq!(lm.focus, 1); // focus moves to new panel
    }

    #[test]
    fn test_add_panel_stacks_when_narrow() {
        let mut lm = LayoutManager::new();
        let config = make_config(80); // threshold 80
        lm.add_panel(panel("a"), &config, 100);
        lm.add_panel(panel("b"), &config, 100);
        // 100 / 2 = 50 < 80, so auto-stack
        assert_eq!(lm.group_count(), 1);
        assert_eq!(lm.panel_count(), 2);
    }

    #[test]
    fn test_unstack_panel_from_group() {
        let mut lm = LayoutManager::new();
        let config = make_config(80);
        // Create a single group with 2 panels (force stack)
        lm.add_panel(panel("a"), &config, 100);
        lm.add_panel(panel("b"), &config, 100);
        assert_eq!(lm.group_count(), 1);
        assert_eq!(lm.panel_count(), 2);

        // Unstack should create a new group
        lm.toggle_panel_stacking(200).unwrap();
        assert_eq!(lm.group_count(), 2);
        assert_eq!(lm.panel_count(), 2);
    }

    #[test]
    fn test_stack_panel_merges_into_left() {
        let mut lm = LayoutManager::new();
        let config = make_config(40);
        lm.add_panel(panel("a"), &config, 200);
        lm.add_panel(panel("b"), &config, 200);
        assert_eq!(lm.group_count(), 2);

        // Focus on group 1 (single panel), stacking merges into left
        lm.focus = 1;
        lm.toggle_panel_stacking(200).unwrap();
        assert_eq!(lm.group_count(), 1);
        assert_eq!(lm.panel_count(), 2);
        assert_eq!(lm.focus, 0);
    }

    // =========================================================================
    // Focus tracking after layout changes
    // =========================================================================

    #[test]
    fn test_focus_updates_on_add_panel() {
        let mut lm = LayoutManager::new();
        let config = make_config(40);
        lm.add_panel(panel("a"), &config, 400);
        assert_eq!(lm.focus, 0);
        lm.add_panel(panel("b"), &config, 400);
        assert_eq!(lm.focus, 1);
        lm.add_panel(panel("c"), &config, 400);
        assert_eq!(lm.focus, 2);
    }

    #[test]
    fn test_add_panel_without_focus_preserves_focus() {
        let mut lm = LayoutManager::new();
        let config = make_config(40);
        lm.add_panel(panel("a"), &config, 400);
        lm.add_panel_without_focus(panel("b"), &config, 400);
        assert_eq!(lm.focus, 0); // focus stays on first panel
        assert_eq!(lm.group_count(), 2);
    }

    #[test]
    fn test_focus_after_close_last_group() {
        let mut lm = LayoutManager::new();
        let config = make_config(40);
        lm.add_panel(panel("a"), &config, 400);
        lm.add_panel(panel("b"), &config, 400);
        lm.focus = 1;
        lm.close_active_panel(400).unwrap();
        assert_eq!(lm.group_count(), 1);
        assert_eq!(lm.focus, 0);
    }

    // =========================================================================
    // Width redistribution
    // =========================================================================

    #[test]
    fn test_single_group_gets_full_width() {
        let mut lm = LayoutManager::new();
        let config = make_config(40);
        lm.add_panel(panel("a"), &config, 200);
        lm.redistribute_widths_proportionally(200);
        assert_eq!(lm.panel_groups[0].width, Some(200));
    }

    #[test]
    fn test_widths_assigned_after_multiple_groups() {
        let mut lm = LayoutManager::new();
        let config = make_config(20);
        lm.add_panel(panel("a"), &config, 200);
        lm.add_panel(panel("b"), &config, 200);
        lm.add_panel(panel("c"), &config, 200);

        let widths = lm.calculate_actual_widths(200);
        let total: u16 = widths.iter().sum();
        // Total widths should equal available width
        assert_eq!(total, 200);
    }

    #[test]
    fn test_redistribute_widths_empty() {
        let mut lm = LayoutManager::new();
        // Should not panic
        lm.redistribute_widths_proportionally(200);
        assert!(lm.calculate_actual_widths(200).is_empty());
    }

    // =========================================================================
    // Panel navigation
    // =========================================================================

    #[test]
    fn test_next_prev_group_wrapping() {
        let mut lm = LayoutManager::new();
        let config = make_config(20);
        lm.add_panel(panel("a"), &config, 400);
        lm.add_panel(panel("b"), &config, 400);
        lm.add_panel(panel("c"), &config, 400);

        lm.focus = 0;
        lm.next_group();
        assert_eq!(lm.focus, 1);
        lm.next_group();
        assert_eq!(lm.focus, 2);
        // Wrap around
        lm.next_group();
        assert_eq!(lm.focus, 0);

        // prev wraps back
        lm.prev_group();
        assert_eq!(lm.focus, 2);
    }

    #[test]
    fn test_next_prev_panel_in_group() {
        let mut lm = LayoutManager::new();
        let config = make_config(80);
        // Force stacking
        lm.add_panel(panel("a"), &config, 100);
        lm.add_panel(panel("b"), &config, 100);
        lm.add_panel(panel("c"), &config, 100);
        assert_eq!(lm.group_count(), 1);

        let group = &lm.panel_groups[0];
        let initial_expanded = group.expanded_index();

        lm.next_panel_in_group();
        let after = lm.panel_groups[0].expanded_index();
        assert_eq!(after, (initial_expanded + 1) % 3);
    }

    // =========================================================================
    // Panel move operations
    // =========================================================================

    #[test]
    fn test_move_panel_to_next_group_swaps_single() {
        let mut lm = LayoutManager::new();
        let config = make_config(20);
        lm.add_panel(panel("a"), &config, 400);
        lm.add_panel(panel("b"), &config, 400);
        assert_eq!(lm.group_count(), 2);

        // Move group 0 (single panel) to next — this swaps the groups
        lm.focus = 0;
        lm.move_panel_to_next_group(400).unwrap();
        assert_eq!(lm.group_count(), 2); // swap, not merge
        assert_eq!(lm.focus, 1);
    }

    #[test]
    fn test_move_panel_to_prev_group_swaps_single() {
        let mut lm = LayoutManager::new();
        let config = make_config(20);
        lm.add_panel(panel("a"), &config, 400);
        lm.add_panel(panel("b"), &config, 400);
        assert_eq!(lm.group_count(), 2);

        lm.focus = 1;
        lm.move_panel_to_prev_group(400).unwrap();
        assert_eq!(lm.group_count(), 2); // swap, not merge
        assert_eq!(lm.focus, 0);
    }

    #[test]
    fn test_move_panel_from_stacked_group_merges() {
        let mut lm = LayoutManager::new();
        let config = make_config(80);
        // Create group with 2 stacked panels
        lm.add_panel(panel("a"), &config, 100);
        lm.add_panel(panel("b"), &config, 100);
        assert_eq!(lm.group_count(), 1);
        assert_eq!(lm.panel_count(), 2);

        // Now add a separate group (wide enough)
        let config2 = make_config(20);
        lm.add_panel(panel("c"), &config2, 400);
        assert_eq!(lm.group_count(), 2);

        // Focus on first group (has 2 panels), move expanded panel to next group
        lm.focus = 0;
        lm.panel_groups[0].set_expanded(1); // expand "b"
        lm.move_panel_to_next_group(400).unwrap();
        // "b" moved to group 1, group 0 still has "a"
        assert_eq!(lm.group_count(), 2);
        assert_eq!(lm.panel_groups[1].len(), 2); // c + b
    }

    #[test]
    fn test_move_panel_up_down_in_group() {
        let mut lm = LayoutManager::new();
        let config = make_config(80);
        lm.add_panel(panel("a"), &config, 100);
        lm.add_panel(panel("b"), &config, 100);
        lm.add_panel(panel("c"), &config, 100);
        assert_eq!(lm.group_count(), 1);

        // expanded is the last added panel ("c" at index 2)
        let group = &lm.panel_groups[0];
        assert_eq!(group.expanded_index(), 2);

        // Move down should be no-op (already at bottom)
        lm.move_panel_down_in_group().unwrap();
        assert_eq!(lm.panel_groups[0].expanded_index(), 2);

        // Move up should swap with index 1
        lm.move_panel_up_in_group().unwrap();
        assert_eq!(lm.panel_groups[0].expanded_index(), 1);
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn test_close_last_panel_removes_group() {
        let mut lm = LayoutManager::new();
        let config = make_config(40);
        lm.add_panel(panel("a"), &config, 200);
        lm.add_panel(panel("b"), &config, 200);
        assert_eq!(lm.group_count(), 2);

        // Close panel in second group
        lm.focus = 1;
        lm.close_active_panel(200).unwrap();
        assert_eq!(lm.group_count(), 1);
    }

    #[test]
    fn test_close_all_panels() {
        let mut lm = LayoutManager::new();
        let config = make_config(40);
        lm.add_panel(panel("a"), &config, 200);
        lm.close_active_panel(200).unwrap();
        assert_eq!(lm.group_count(), 0);
        assert!(!lm.has_panels());
        assert_eq!(lm.panel_count(), 0);
    }

    #[test]
    fn test_active_panel_with_no_panels() {
        let lm = LayoutManager::new();
        assert!(lm.active_panel().is_none());
    }

    #[test]
    fn test_next_group_with_no_panels() {
        let mut lm = LayoutManager::new();
        // Should not panic
        lm.next_group();
        lm.prev_group();
        assert_eq!(lm.focus, 0);
    }

    #[test]
    fn test_set_focus_out_of_bounds() {
        let mut lm = LayoutManager::new();
        let config = make_config(40);
        lm.add_panel(panel("a"), &config, 200);
        lm.set_focus(100); // out of bounds
        assert_eq!(lm.focus, 0); // unchanged
    }
}
