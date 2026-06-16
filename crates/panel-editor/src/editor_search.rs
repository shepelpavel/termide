//! Search and replace methods for the Editor.
//!
//! This module contains search and replace functionality including:
//! - Search initiation and navigation
//! - Match highlighting and selection
//! - Replace current/all operations
//! - Navigation helpers that interact with search state

use anyhow::Result;
use termide_buffer::SearchState;

use termide_core::Searchable;

use crate::search;

use super::Editor;

impl Searchable for Editor {
    fn start_search(&mut self, query: String, case_sensitive: bool) {
        self.start_search(query, case_sensitive);
    }

    fn search_next(&mut self) {
        self.search_next();
    }

    fn search_prev(&mut self) {
        self.search_prev();
    }

    fn close_search(&mut self) {
        self.close_search();
    }

    fn get_search_match_info(&self) -> Option<(usize, usize)> {
        self.get_search_match_info()
    }
}

impl Editor {
    // =========================================================================
    // Search Operations
    // =========================================================================

    /// Start search
    pub fn start_search(&mut self, query: String, case_sensitive: bool) {
        let mut search_state = SearchState::new(query, case_sensitive);

        // Perform search throughout document
        self.perform_search(&mut search_state);

        // Find closest match to current cursor
        search_state.find_closest_match(&self.cursor);

        // Move cursor to end of match and create selection
        if let Some(idx) = search_state.current_match {
            if let Some(match_cursor) = search_state.matches.get(idx).cloned() {
                let match_len = search_state.match_len_at(idx);
                let (selection, end_cursor) = search::get_match_selection(&match_cursor, match_len);
                self.cursor = end_cursor;
                self.selection = Some(selection);
            }
        }

        self.search.state = Some(search_state);
    }

    /// Perform search in document
    fn perform_search(&self, search_state: &mut SearchState) {
        search::perform_search(&self.buffer, search_state);
    }

    /// Go to next match
    pub fn search_next(&mut self) {
        if let Some(ref mut search_state) = self.search.state {
            search_state.next_match();
            if let Some(idx) = search_state.current_match {
                if let Some(match_cursor) = search_state.matches.get(idx).cloned() {
                    let match_len = search_state.match_len_at(idx);
                    let (selection, end_cursor) =
                        search::get_match_selection(&match_cursor, match_len);
                    self.cursor = end_cursor;
                    self.selection = Some(selection);
                }
            }
        }
    }

    /// Go to previous match
    pub fn search_prev(&mut self) {
        if let Some(ref mut search_state) = self.search.state {
            search_state.prev_match();
            if let Some(idx) = search_state.current_match {
                if let Some(match_cursor) = search_state.matches.get(idx).cloned() {
                    let match_len = search_state.match_len_at(idx);
                    let (selection, end_cursor) =
                        search::get_match_selection(&match_cursor, match_len);
                    self.cursor = end_cursor;
                    self.selection = Some(selection);
                }
            }
        }
    }

    /// Close search
    pub fn close_search(&mut self) {
        // Preserve the last search/replace query before closing
        if let Some(ref search) = self.search.state {
            if let Some(ref replace_with) = search.replace_with {
                // This is a replace operation - save to replace history
                self.search.last_replace_find = Some(search.query.clone());
                self.search.last_replace_with = Some(replace_with.clone());
            } else {
                // This is a search operation - save to search history
                self.search.last_query = Some(search.query.clone());
            }
        }
        self.search.state = None;
    }

    /// Get search match information (current index, total count)
    pub fn get_search_match_info(&self) -> Option<(usize, usize)> {
        if let Some(ref search) = self.search.state {
            let current = search.current_match.unwrap_or(0);
            let total = search.matches.len();
            Some((current, total))
        } else {
            None
        }
    }

    // =========================================================================
    // Replace Operations
    // =========================================================================

    /// Start search with replace
    pub fn start_replace(&mut self, query: String, replace_with: String, case_sensitive: bool) {
        let mut search_state = SearchState::new_with_replace(query, replace_with, case_sensitive);

        // Perform search throughout document
        self.perform_search(&mut search_state);

        // Find closest match to current cursor
        search_state.find_closest_match(&self.cursor);

        // Move cursor to first match and create selection
        if let Some(idx) = search_state.current_match {
            if let Some(match_cursor) = search_state.matches.get(idx).cloned() {
                let match_len = search_state.match_len_at(idx);
                let (selection, end_cursor) = search::get_match_selection(&match_cursor, match_len);
                self.cursor = end_cursor;
                self.selection = Some(selection);
            }
        }

        self.search.state = Some(search_state);
    }

    /// Update replace_with value in active search state without rebuilding search
    pub fn update_replace_with(&mut self, replace_with: String) {
        if let Some(ref mut search) = self.search.state {
            search.replace_with = Some(replace_with);
        }
    }

    /// Replace current match
    pub fn replace_current(&mut self) -> Result<()> {
        // Collect what we need (regex-aware: per-match length + expanded
        // text) before mutating the buffer.
        let (match_cursor, match_len, replace_text) = {
            let Some(search_state) = self.search.state.as_ref() else {
                return Ok(());
            };
            let (Some(_), Some(idx)) = (&search_state.replace_with, search_state.current_match)
            else {
                return Ok(());
            };
            let Some(match_cursor) = search_state.matches.get(idx).cloned() else {
                return Ok(());
            };
            let match_len = search_state.match_len_at(idx);
            let replace_text =
                search::expand_replacement(&self.buffer, &match_cursor, match_len, search_state);
            (match_cursor, match_len, replace_text)
        };

        // Perform replacement
        let result =
            search::replace_at_position(&mut self.buffer, &match_cursor, match_len, &replace_text)?;
        self.cursor = result.new_cursor;

        // Invalidate caches (highlight + wrap) for changed lines
        let is_multiline = replace_text.contains('\n');
        self.invalidate_cache_after_edit(result.start_line, is_multiline);

        // Update search_state
        if let Some(ref mut search_state) = self.search.state {
            if let Some(idx) = search_state.current_match {
                // Remove this match (and its recorded length) from the lists
                search_state.matches.remove(idx);
                if idx < search_state.match_lengths.len() {
                    search_state.match_lengths.remove(idx);
                }

                // Shift remaining same-line matches by the length delta
                search::update_match_positions_after_replace(
                    &mut search_state.matches,
                    &match_cursor,
                    match_len,
                    replace_text.chars().count(),
                );

                // Update current match index
                if search_state.matches.is_empty() {
                    search_state.current_match = None;
                } else if idx >= search_state.matches.len() {
                    search_state.current_match = Some(search_state.matches.len() - 1);
                }

                // Move cursor to next match and create selection
                if let Some(cidx) = search_state.current_match {
                    if let Some(next_cursor) = search_state.matches.get(cidx).cloned() {
                        let next_len = search_state.match_len_at(cidx);
                        let (selection, end_cursor) =
                            search::get_match_selection(&next_cursor, next_len);
                        self.cursor = end_cursor;
                        self.selection = Some(selection);
                    }
                }
            }
        }

        Ok(())
    }

    /// Replace all matches
    pub fn replace_all(&mut self) -> Result<usize> {
        // Use take() instead of clone() to avoid allocation
        let Some(search_state) = self.search.state.take() else {
            return Ok(0);
        };

        if search_state.replace_with.is_none() {
            // Restore state if no replace_with
            self.search.state = Some(search_state);
            return Ok(0);
        }

        // Perform all replacements (regex-aware per-match length + expansion)
        let count = search::replace_all_matches(&mut self.buffer, &search_state)?;

        // Invalidate caches (highlight + wrap) for all affected lines
        if count > 0 {
            self.invalidate_cache_after_edit(0, true);
        }

        Ok(count)
    }

    // =========================================================================
    // Navigation Helpers
    // =========================================================================

    /// Prepare for navigation: close search, clear selection, and close popups.
    pub(crate) fn prepare_for_navigation(&mut self) {
        self.close_search();
        self.selection = None;
        // Close popups on cursor movement
        self.lsp.completion_popup = None;
        self.close_hover_popup();
    }

    /// Prepare for navigation with selection: close search, start/extend selection, and close popups.
    pub(crate) fn prepare_for_navigation_with_selection(&mut self) {
        self.close_search();
        self.start_or_extend_selection();
        // Close popups on cursor movement
        self.lsp.completion_popup = None;
        self.close_hover_popup();
    }
}
