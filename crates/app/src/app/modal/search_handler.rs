//! Search / replace modal result handlers and the shared result-routing enum.

use anyhow::Result;

use crate::app::App;
use crate::state::ActiveModal;
use termide_modal::{ModalResult, SearchAction, SearchModalResult};

/// Result of processing search/replace modal.
///
/// Kept module-local: only `handle_search_replace_modal` cares about this,
/// and the extracted handlers collaborate via their own private helpers.
pub(super) enum SearchReplaceResult {
    /// Keep modal open (navigation action).
    KeepOpen,
    /// Close modal.
    Close,
    /// Modal cancelled — close and clear search.
    Cancelled,
    /// Content-mode Replace All: close the search modal and ask the user to
    /// confirm replacing across the matched files.
    ReplaceConfirm {
        message: String,
        replace_with: String,
    },
    /// Not a search/replace modal.
    NotApplicable,
}

impl App {
    /// Handle search result
    pub(in crate::app) fn handle_search(&mut self, value: Box<dyn std::any::Any>) -> Result<()> {
        if let Some(query) = value.downcast_ref::<String>() {
            // Start search in active panel (case insensitive, literal by default)
            if let Some(searchable) = self.active_searchable_mut() {
                searchable.start_search(query.clone(), false, false);
            }
        }
        Ok(())
    }

    /// Handle search action from SearchModal
    pub(in crate::app) fn handle_search_action(
        &mut self,
        search_result: &SearchModalResult,
    ) -> Result<()> {
        use termide_core::SearchMode;

        match search_result.mode {
            SearchMode::Text => {
                // Get active searchable panel (Editor, Journal, or Terminal)
                if let Some(searchable) = self.active_searchable_mut() {
                    match search_result.action {
                        SearchAction::Search => {
                            searchable.start_search(
                                search_result.query.clone(),
                                search_result.case_sensitive,
                                search_result.use_regex,
                            );
                        }
                        SearchAction::Next => {
                            searchable.search_next();
                        }
                        SearchAction::Previous => {
                            searchable.search_prev();
                        }
                        SearchAction::CloseWithSelection => {
                            // Selection is already set by editor methods
                        }
                        SearchAction::ReplaceAll => {}
                    }
                }
            }
            SearchMode::FileGlob => {
                if let Some(fm) = self.active_file_manager_mut() {
                    match search_result.action {
                        SearchAction::Search => {
                            fm.start_file_search(&search_result.query, false, false);
                        }
                        SearchAction::Next => {
                            fm.search_next();
                        }
                        SearchAction::Previous => {
                            fm.search_prev();
                        }
                        SearchAction::CloseWithSelection => {
                            fm.close_search_with_selection();
                        }
                        SearchAction::ReplaceAll => {}
                    }
                }
            }
            SearchMode::Content => {
                let mut open_event = None;
                if let Some(fm) = self.active_file_manager_mut() {
                    let content_query = search_result.content_query.as_deref().unwrap_or("");
                    match search_result.action {
                        SearchAction::Search => {
                            fm.start_content_search(
                                &search_result.query,
                                content_query,
                                search_result.use_regex,
                                search_result.case_sensitive,
                            );
                        }
                        SearchAction::Next => {
                            fm.search_next();
                        }
                        SearchAction::Previous => {
                            fm.search_prev();
                        }
                        SearchAction::CloseWithSelection => {
                            open_event = fm.close_search_with_selection();
                        }
                        // ReplaceAll is handled (with confirmation) in
                        // process_search_modal_result.
                        SearchAction::ReplaceAll => {}
                    }
                    // Keep the replacement preview in sync with the modal.
                    fm.set_content_replace(search_result.replace_query.clone());
                }
                if let Some(event) = open_event {
                    self.process_single_event(event)?;
                }
            }
        }
        Ok(())
    }

    /// Process search modal result and determine what to do
    pub(super) fn process_search_modal_result(
        &mut self,
        result: &ModalResult<Box<dyn std::any::Any>>,
    ) -> SearchReplaceResult {
        use termide_core::SearchMode;

        if let ModalResult::Confirmed(value) = result {
            if let Some(search_result) = value.downcast_ref::<SearchModalResult>() {
                // Content-mode "Replace all": close the search modal and ask
                // for confirmation before writing files.
                if search_result.action == SearchAction::ReplaceAll {
                    let summary = self
                        .active_file_manager_mut()
                        .and_then(|fm| fm.content_search_summary());
                    if let Some((files, matches)) = summary {
                        if matches > 0 {
                            return SearchReplaceResult::ReplaceConfirm {
                                message: termide_i18n::t().replace_confirm_fmt(matches, files),
                                replace_with: search_result
                                    .replace_query
                                    .clone()
                                    .unwrap_or_default(),
                            };
                        }
                    }
                    return SearchReplaceResult::KeepOpen;
                }

                // Handle search action based on mode
                if self.handle_search_action(search_result).is_err() {
                    return SearchReplaceResult::Close;
                }

                // Check if we should close modal
                if matches!(search_result.action, SearchAction::CloseWithSelection) {
                    return SearchReplaceResult::Close;
                }

                // Get match info based on mode
                let match_info = match search_result.mode {
                    SearchMode::Text => self
                        .active_searchable_mut()
                        .and_then(|s| s.get_search_match_info()),
                    SearchMode::FileGlob | SearchMode::Content => self
                        .active_file_manager_mut()
                        .and_then(|fm| fm.get_file_search_match_info()),
                };

                // Update match info in modal
                if let Some(ActiveModal::Search(search_modal)) = &mut self.state.active_modal {
                    if let Some((current, total)) = match_info {
                        search_modal.set_match_info(current, total);
                    } else {
                        search_modal.clear_match_info();
                    }
                }

                return SearchReplaceResult::KeepOpen;
            }
        } else if matches!(result, ModalResult::Cancelled) {
            return SearchReplaceResult::Cancelled;
        }
        SearchReplaceResult::NotApplicable
    }

    /// Handle search modal result and return whether to continue processing.
    /// (Editor replace is now an inline bar, not a modal.)
    pub(in crate::app) fn handle_search_replace_modal(
        &mut self,
        is_search: bool,
        result: &ModalResult<Box<dyn std::any::Any>>,
    ) -> Option<()> {
        if is_search {
            match self.process_search_modal_result(result) {
                SearchReplaceResult::KeepOpen => return Some(()),
                SearchReplaceResult::Close => {
                    self.state.close_modal();
                    return Some(());
                }
                SearchReplaceResult::Cancelled => {
                    // Determine mode before closing modal
                    let mode = if let Some(ActiveModal::Search(ref m)) = self.state.active_modal {
                        Some(m.mode())
                    } else {
                        None
                    };
                    self.state.close_modal();
                    match mode {
                        Some(termide_core::SearchMode::Text) => {
                            if let Some(searchable) = self.active_searchable_mut() {
                                searchable.close_search();
                            }
                        }
                        Some(
                            termide_core::SearchMode::FileGlob | termide_core::SearchMode::Content,
                        ) => {
                            if let Some(fm) = self.active_file_manager_mut() {
                                fm.close_file_search();
                            }
                        }
                        None => {}
                    }
                    return Some(());
                }
                SearchReplaceResult::ReplaceConfirm {
                    message,
                    replace_with,
                } => {
                    // Close the search modal, then ask to confirm the writes.
                    self.state.close_modal();
                    self.event_show_confirm(
                        message,
                        termide_core::ConfirmAction::ReplaceInContent(replace_with),
                    );
                    return Some(());
                }
                SearchReplaceResult::NotApplicable => {}
            }
        }

        None // Continue with normal modal handling
    }
}
