use super::{loader, Translation};
use std::collections::HashMap;

/// Runtime translation implementation that loads from TOML files
pub struct RuntimeTranslation {
    strings: HashMap<String, String>,
    formats: HashMap<String, String>,
    plurals: HashMap<String, loader::PluralRules>,
}

impl RuntimeTranslation {
    pub fn new(lang: &str) -> anyhow::Result<Self> {
        let data = loader::load_language(lang)?;
        Ok(Self {
            strings: data.strings,
            formats: data.formats,
            plurals: data.plurals,
        })
    }

    fn get_string(&self, key: &str) -> &str {
        self.strings
            .get(key)
            .map(|s| s.as_str())
            .unwrap_or_else(|| {
                log::warn!("Missing translation key: {}", key);
                ""
            })
    }

    fn format(&self, key: &str, args: &[(&str, &str)]) -> String {
        let template = self
            .formats
            .get(key)
            .map(|s| s.as_str())
            .unwrap_or_else(|| {
                log::warn!("Missing format key: {}", key);
                ""
            });
        let mut result = template.to_string();
        for (placeholder, value) in args {
            let pattern = format!("{{{}}}", placeholder);
            result = result.replace(&pattern, value);
        }
        result
    }

    fn pluralize(&self, count: usize, key: &str) -> &str {
        if let Some(rules) = self.plurals.get(key) {
            match count {
                1 => &rules.one,
                2..=4 if rules.few.is_some() => rules.few.as_deref().unwrap_or(&rules.other),
                _ => &rules.other,
            }
        } else if count == 1 {
            ""
        } else {
            "s"
        }
    }
}

impl Translation for RuntimeTranslation {
    fn fm_paste_confirm(&self, count: usize, mode: &str, dest: &str) -> String {
        let plural = self.pluralize(count, "file");
        self.format(
            "fm_paste_confirm",
            &[
                ("count", &count.to_string()),
                ("mode", mode),
                ("dest", dest),
                ("plural", plural),
            ],
        )
    }

    fn fm_copy_prompt(&self, name: &str) -> String {
        self.format("fm_copy_prompt", &[("name", name)])
    }

    fn fm_move_prompt(&self, name: &str) -> String {
        self.format("fm_move_prompt", &[("name", name)])
    }

    fn git_operation_cancelled(&self) -> &str {
        self.get_string("git_operation_cancelled")
    }

    fn modal_yes(&self) -> &str {
        self.get_string("modal_yes")
    }

    fn modal_ok(&self) -> &str {
        self.get_string("modal_ok")
    }

    fn panel_help(&self) -> &str {
        self.get_string("panel_help")
    }

    fn panel_journal(&self) -> &str {
        self.get_string("panel_journal")
    }

    fn panel_operations(&self) -> &str {
        self.get_string("panel_operations")
    }

    fn no_active_operations(&self) -> &str {
        self.get_string("no_active_operations")
    }

    fn editor_close_unsaved(&self) -> &str {
        self.get_string("editor_close_unsaved")
    }

    fn editor_close_unsaved_question(&self) -> &str {
        self.get_string("editor_close_unsaved_question")
    }

    fn editor_save_and_close(&self) -> &str {
        self.get_string("editor_save_and_close")
    }

    fn editor_close_without_saving(&self) -> &str {
        self.get_string("editor_close_without_saving")
    }

    fn editor_cancel(&self) -> &str {
        self.get_string("editor_cancel")
    }

    fn editor_close_external(&self) -> &str {
        self.get_string("editor_close_external")
    }

    fn editor_close_external_question(&self) -> &str {
        self.get_string("editor_close_external_question")
    }

    fn editor_overwrite_disk(&self) -> &str {
        self.get_string("editor_overwrite_disk")
    }

    fn editor_keep_disk_close(&self) -> &str {
        self.get_string("editor_keep_disk_close")
    }

    fn editor_reload_into_editor(&self) -> &str {
        self.get_string("editor_reload_into_editor")
    }

    fn editor_close_conflict(&self) -> &str {
        self.get_string("editor_close_conflict")
    }

    fn editor_close_conflict_question(&self) -> &str {
        self.get_string("editor_close_conflict_question")
    }

    fn editor_reload_from_disk(&self) -> &str {
        self.get_string("editor_reload_from_disk")
    }

    fn editor_file_opened(&self, filename: &str) -> String {
        self.format("editor_file_opened", &[("filename", filename)])
    }

    fn editor_search_match_info(&self, current: usize, total: usize) -> String {
        self.format(
            "editor_search_match_info",
            &[
                ("current", &current.to_string()),
                ("total", &total.to_string()),
            ],
        )
    }

    fn editor_search_no_matches(&self) -> &str {
        self.get_string("editor_search_no_matches")
    }

    fn editor_deletion_marker(&self, count: usize) -> String {
        let plural = self.pluralize(count, "file");
        self.format(
            "editor_deletion_marker",
            &[("count", &count.to_string()), ("plural", plural)],
        )
    }

    fn file_search_title(&self) -> &str {
        self.get_string("file_search_title")
    }

    fn content_search_title(&self) -> &str {
        self.get_string("content_search_title")
    }

    fn fm_goto_title(&self) -> &str {
        self.get_string("fm_goto_title")
    }

    fn fm_goto_prompt(&self) -> &str {
        self.get_string("fm_goto_prompt")
    }

    fn connection_cancelled_title(&self) -> &str {
        self.get_string("connection_cancelled_title")
    }

    fn connection_error_title(&self) -> &str {
        self.get_string("connection_error_title")
    }

    fn connection_timeout_title(&self) -> &str {
        self.get_string("connection_timeout_title")
    }

    fn connection_timeout_message(&self) -> &str {
        self.get_string("connection_timeout_message")
    }

    fn app_quit_confirm(&self) -> &str {
        self.get_string("app_quit_confirm")
    }

    fn app_quit_title(&self) -> &str {
        self.get_string("app_quit_title")
    }

    fn help_global_keys(&self) -> &str {
        self.get_string("help_global_keys")
    }

    fn help_file_manager_keys(&self) -> &str {
        self.get_string("help_file_manager_keys")
    }

    fn help_editor_keys(&self) -> &str {
        self.get_string("help_editor_keys")
    }

    fn help_terminal_keys(&self) -> &str {
        self.get_string("help_terminal_keys")
    }

    fn help_desc_menu(&self) -> &str {
        self.get_string("help_desc_menu")
    }

    fn help_desc_quit(&self) -> &str {
        self.get_string("help_desc_quit")
    }

    fn help_desc_help(&self) -> &str {
        self.get_string("help_desc_help")
    }

    fn help_desc_close_panel(&self) -> &str {
        self.get_string("help_desc_close_panel")
    }

    fn help_desc_escape_close(&self) -> &str {
        self.get_string("help_desc_escape_close")
    }

    fn help_desc_select(&self) -> &str {
        self.get_string("help_desc_select")
    }

    fn help_desc_new_terminal(&self) -> &str {
        self.get_string("help_desc_new_terminal")
    }

    fn help_desc_parent_dir(&self) -> &str {
        self.get_string("help_desc_parent_dir")
    }

    fn help_desc_home(&self) -> &str {
        self.get_string("help_desc_home")
    }

    fn help_desc_end(&self) -> &str {
        self.get_string("help_desc_end")
    }

    fn help_desc_page_scroll(&self) -> &str {
        self.get_string("help_desc_page_scroll")
    }

    fn help_desc_create_file(&self) -> &str {
        self.get_string("help_desc_create_file")
    }

    fn help_desc_create_dir(&self) -> &str {
        self.get_string("help_desc_create_dir")
    }

    fn help_desc_copy(&self) -> &str {
        self.get_string("help_desc_copy")
    }

    fn help_desc_move(&self) -> &str {
        self.get_string("help_desc_move")
    }

    fn help_desc_delete(&self) -> &str {
        self.get_string("help_desc_delete")
    }

    fn help_desc_rename(&self) -> &str {
        self.get_string("help_desc_rename")
    }

    fn help_desc_save(&self) -> &str {
        self.get_string("help_desc_save")
    }

    fn help_section_panels(&self) -> &str {
        self.get_string("help_section_panels")
    }

    fn help_section_git_status(&self) -> &str {
        self.get_string("help_section_git_status")
    }

    fn help_section_navigation(&self) -> &str {
        self.get_string("help_section_navigation")
    }

    fn help_section_git_diff(&self) -> &str {
        self.get_string("help_section_git_diff")
    }

    fn help_section_git_log(&self) -> &str {
        self.get_string("help_section_git_log")
    }

    fn help_desc_new_file_manager(&self) -> &str {
        self.get_string("help_desc_new_file_manager")
    }

    fn help_desc_new_editor(&self) -> &str {
        self.get_string("help_desc_new_editor")
    }

    fn help_desc_new_journal(&self) -> &str {
        self.get_string("help_desc_new_journal")
    }

    fn help_desc_open_preferences(&self) -> &str {
        self.get_string("help_desc_open_preferences")
    }

    fn help_desc_open_sessions(&self) -> &str {
        self.get_string("help_desc_open_sessions")
    }

    fn help_desc_open_git_status(&self) -> &str {
        self.get_string("help_desc_open_git_status")
    }

    fn help_desc_open_outline(&self) -> &str {
        self.get_string("help_desc_open_outline")
    }

    fn help_desc_open_diagnostics(&self) -> &str {
        self.get_string("help_desc_open_diagnostics")
    }

    fn help_desc_open_git_log(&self) -> &str {
        self.get_string("help_desc_open_git_log")
    }

    fn help_desc_toggle_stack(&self) -> &str {
        self.get_string("help_desc_toggle_stack")
    }

    fn help_desc_swap_left(&self) -> &str {
        self.get_string("help_desc_swap_left")
    }

    fn help_desc_swap_right(&self) -> &str {
        self.get_string("help_desc_swap_right")
    }

    fn help_desc_move_first(&self) -> &str {
        self.get_string("help_desc_move_first")
    }

    fn help_desc_move_last(&self) -> &str {
        self.get_string("help_desc_move_last")
    }

    fn help_desc_resize_smaller(&self) -> &str {
        self.get_string("help_desc_resize_smaller")
    }

    fn help_desc_resize_larger(&self) -> &str {
        self.get_string("help_desc_resize_larger")
    }

    fn help_desc_prev_group(&self) -> &str {
        self.get_string("help_desc_prev_group")
    }

    fn help_desc_next_group(&self) -> &str {
        self.get_string("help_desc_next_group")
    }

    fn help_desc_prev_panel(&self) -> &str {
        self.get_string("help_desc_prev_panel")
    }

    fn help_desc_next_panel(&self) -> &str {
        self.get_string("help_desc_next_panel")
    }

    fn help_desc_goto_panel(&self) -> &str {
        self.get_string("help_desc_goto_panel")
    }

    fn help_desc_save_as(&self) -> &str {
        self.get_string("help_desc_save_as")
    }

    fn help_desc_reload(&self) -> &str {
        self.get_string("help_desc_reload")
    }

    fn help_desc_duplicate_line(&self) -> &str {
        self.get_string("help_desc_duplicate_line")
    }

    fn help_desc_toggle_comment(&self) -> &str {
        self.get_string("help_desc_toggle_comment")
    }

    fn help_desc_search_next(&self) -> &str {
        self.get_string("help_desc_search_next")
    }

    fn help_desc_search_prev(&self) -> &str {
        self.get_string("help_desc_search_prev")
    }

    fn help_desc_replace(&self) -> &str {
        self.get_string("help_desc_replace")
    }

    fn help_desc_replace_current(&self) -> &str {
        self.get_string("help_desc_replace_current")
    }

    fn help_desc_replace_all(&self) -> &str {
        self.get_string("help_desc_replace_all")
    }

    // LSP help descriptions
    fn help_desc_trigger_completion(&self) -> &str {
        self.get_string("help_desc_trigger_completion")
    }

    fn help_desc_accept_completion(&self) -> &str {
        self.get_string("help_desc_accept_completion")
    }

    fn help_desc_cancel_completion(&self) -> &str {
        self.get_string("help_desc_cancel_completion")
    }

    fn help_desc_navigate_completion(&self) -> &str {
        self.get_string("help_desc_navigate_completion")
    }

    fn help_desc_show_hover(&self) -> &str {
        self.get_string("help_desc_show_hover")
    }

    fn help_desc_goto_definition(&self) -> &str {
        self.get_string("help_desc_goto_definition")
    }

    fn help_desc_find_references(&self) -> &str {
        self.get_string("help_desc_find_references")
    }

    fn help_desc_rename_symbol(&self) -> &str {
        self.get_string("help_desc_rename_symbol")
    }

    fn help_desc_delete_generic(&self) -> &str {
        self.get_string("help_desc_delete_generic")
    }

    fn help_desc_open_bookmark_add(&self) -> &str {
        self.get_string("help_desc_open_bookmark_add")
    }

    fn help_desc_command_palette(&self) -> &str {
        self.get_string("help_desc_command_palette")
    }

    fn help_desc_word_nav(&self) -> &str {
        self.get_string("help_desc_word_nav")
    }

    fn help_desc_paragraph_nav(&self) -> &str {
        self.get_string("help_desc_paragraph_nav")
    }

    fn help_desc_view_file(&self) -> &str {
        self.get_string("help_desc_view_file")
    }

    fn help_desc_edit_file(&self) -> &str {
        self.get_string("help_desc_edit_file")
    }

    fn help_desc_search_content(&self) -> &str {
        self.get_string("help_desc_search_content")
    }

    fn help_desc_go_home(&self) -> &str {
        self.get_string("help_desc_go_home")
    }

    fn help_desc_toggle_hidden(&self) -> &str {
        self.get_string("help_desc_toggle_hidden")
    }

    fn help_desc_open_external(&self) -> &str {
        self.get_string("help_desc_open_external")
    }

    fn help_desc_stage_file(&self) -> &str {
        self.get_string("help_desc_stage_file")
    }

    fn help_desc_unstage_file(&self) -> &str {
        self.get_string("help_desc_unstage_file")
    }

    fn help_desc_next_section(&self) -> &str {
        self.get_string("help_desc_next_section")
    }

    fn help_desc_prev_section(&self) -> &str {
        self.get_string("help_desc_prev_section")
    }

    fn help_desc_terminal_copy(&self) -> &str {
        self.get_string("help_desc_terminal_copy")
    }

    fn help_desc_terminal_paste(&self) -> &str {
        self.get_string("help_desc_terminal_paste")
    }

    fn help_desc_scroll_up(&self) -> &str {
        self.get_string("help_desc_scroll_up")
    }

    fn help_desc_scroll_down(&self) -> &str {
        self.get_string("help_desc_scroll_down")
    }

    fn help_desc_scroll_top(&self) -> &str {
        self.get_string("help_desc_scroll_top")
    }

    fn help_desc_scroll_bottom(&self) -> &str {
        self.get_string("help_desc_scroll_bottom")
    }

    // Navigation help descriptions (static keys)
    fn help_desc_move_up(&self) -> &str {
        self.get_string("help_desc_move_up")
    }

    fn help_desc_move_down(&self) -> &str {
        self.get_string("help_desc_move_down")
    }

    fn help_desc_scroll_half_up(&self) -> &str {
        self.get_string("help_desc_scroll_half_up")
    }

    fn help_desc_scroll_half_down(&self) -> &str {
        self.get_string("help_desc_scroll_half_down")
    }

    // Git Diff help descriptions (static keys)
    fn help_desc_toggle_collapse(&self) -> &str {
        self.get_string("help_desc_toggle_collapse")
    }

    fn help_desc_open_file_editor(&self) -> &str {
        self.get_string("help_desc_open_file_editor")
    }

    // Git Log help descriptions (static keys)
    fn help_desc_view_commit_diff(&self) -> &str {
        self.get_string("help_desc_view_commit_diff")
    }

    fn status_file_created(&self, name: &str) -> String {
        self.format("status_file_created", &[("name", name)])
    }

    fn status_dir_created(&self, name: &str) -> String {
        self.format("status_dir_created", &[("name", name)])
    }

    fn status_file_saved(&self, name: &str) -> String {
        self.format("status_file_saved", &[("name", name)])
    }

    fn status_error_save(&self, error: &str) -> String {
        self.format("status_error_save", &[("error", error)])
    }

    fn status_file_reloaded(&self) -> &str {
        self.get_string("status_file_reloaded")
    }

    fn status_error_reload(&self, error: &str) -> String {
        self.format("status_error_reload", &[("error", error)])
    }

    fn status_error_open_file(&self, name: &str, error: &str) -> String {
        self.format(
            "status_error_open_file",
            &[("name", name), ("error", error)],
        )
    }

    fn status_opening_external(&self, name: &str) -> String {
        self.format("status_opening_external", &[("name", name)])
    }

    fn modal_copy_single_title(&self, name: &str) -> String {
        self.format("modal_copy_single_title", &[("name", name)])
    }

    fn modal_copy_multiple_title(&self, count: usize) -> String {
        let element = self.pluralize(count, "element");
        self.format(
            "modal_copy_multiple_title",
            &[("count", &count.to_string()), ("element", element)],
        )
    }

    fn modal_move_single_title(&self, name: &str) -> String {
        self.format("modal_move_single_title", &[("name", name)])
    }

    fn modal_move_multiple_title(&self, count: usize) -> String {
        let element = self.pluralize(count, "element");
        self.format(
            "modal_move_multiple_title",
            &[("count", &count.to_string()), ("element", element)],
        )
    }

    fn modal_create_file_title(&self) -> &str {
        self.get_string("modal_create_file_title")
    }

    fn modal_create_dir_title(&self) -> &str {
        self.get_string("modal_create_dir_title")
    }

    fn modal_delete_single_title(&self, name: &str) -> String {
        self.format("modal_delete_single_title", &[("name", name)])
    }

    fn modal_delete_multiple_title(&self, count: usize) -> String {
        let element = self.pluralize(count, "element");
        self.format(
            "modal_delete_multiple_title",
            &[("count", &count.to_string()), ("element", element)],
        )
    }

    fn modal_save_as_title(&self) -> &str {
        self.get_string("modal_save_as_title")
    }

    fn modal_copy_single_prompt(&self, name: &str) -> String {
        self.format("modal_copy_single_prompt", &[("name", name)])
    }

    fn modal_copy_multiple_prompt(&self, count: usize) -> String {
        let element = self.pluralize(count, "element");
        self.format(
            "modal_copy_multiple_prompt",
            &[("count", &count.to_string()), ("element", element)],
        )
    }

    fn modal_move_single_prompt(&self, name: &str) -> String {
        self.format("modal_move_single_prompt", &[("name", name)])
    }

    fn modal_move_multiple_prompt(&self, count: usize) -> String {
        let element = self.pluralize(count, "element");
        self.format(
            "modal_move_multiple_prompt",
            &[("count", &count.to_string()), ("element", element)],
        )
    }

    fn batch_result_file_copied(&self) -> &str {
        self.get_string("batch_result_file_copied")
    }

    fn batch_result_file_moved(&self) -> &str {
        self.get_string("batch_result_file_moved")
    }

    fn batch_result_error_copy(&self) -> &str {
        self.get_string("batch_result_error_copy")
    }

    fn batch_result_error_move(&self) -> &str {
        self.get_string("batch_result_error_move")
    }

    fn batch_result_copied(&self) -> &str {
        self.get_string("batch_result_copied")
    }

    fn batch_result_moved(&self) -> &str {
        self.get_string("batch_result_moved")
    }

    fn batch_result_skipped_fmt(&self, count: usize) -> String {
        self.format("batch_result_skipped_fmt", &[("count", &count.to_string())])
    }

    fn batch_result_errors_fmt(&self, count: usize) -> String {
        self.format("batch_result_errors_fmt", &[("count", &count.to_string())])
    }

    fn menu_sessions(&self) -> &str {
        self.get_string("menu_sessions")
    }

    fn menu_windows(&self) -> &str {
        self.get_string("menu_windows")
    }

    fn menu_scripts(&self) -> &str {
        self.get_string("menu_scripts")
    }

    fn menu_scripts_add(&self) -> &str {
        self.get_string("menu_scripts_add")
    }

    fn menu_options(&self) -> &str {
        self.get_string("menu_options")
    }

    fn menu_quit(&self) -> &str {
        self.get_string("menu_quit")
    }

    fn menu_navigate_hint(&self) -> &str {
        self.get_string("menu_navigate_hint")
    }

    fn menu_open_hint_label(&self) -> &str {
        self.get_string("menu_open_hint_label")
    }

    fn menu_bookmarks(&self) -> &str {
        self.get_string("menu_bookmarks")
    }

    fn bookmarks_add_bookmark(&self) -> &str {
        self.get_string("bookmarks_add_bookmark")
    }

    fn bookmarks_manage(&self) -> &str {
        self.get_string("bookmarks_manage")
    }

    fn bookmarks_no_bookmarks(&self) -> &str {
        self.get_string("bookmarks_no_bookmarks")
    }

    fn bookmarks_add_title(&self) -> &str {
        self.get_string("bookmarks_add_title")
    }

    fn bookmarks_add_path(&self) -> &str {
        self.get_string("bookmarks_add_path")
    }

    fn bookmarks_add_description(&self) -> &str {
        self.get_string("bookmarks_add_description")
    }

    fn bookmarks_add_group(&self) -> &str {
        self.get_string("bookmarks_add_group")
    }

    fn bookmarks_add_project(&self) -> &str {
        self.get_string("bookmarks_add_project")
    }

    fn tools_files(&self) -> &str {
        self.get_string("tools_files")
    }

    fn tools_terminal(&self) -> &str {
        self.get_string("tools_terminal")
    }

    fn tools_editor(&self) -> &str {
        self.get_string("tools_editor")
    }

    fn tools_git_status(&self) -> &str {
        self.get_string("tools_git_status")
    }

    fn tools_git_log(&self) -> &str {
        self.get_string("tools_git_log")
    }

    fn stash_new(&self) -> &str {
        self.get_string("stash_new")
    }

    fn stash_include_untracked(&self) -> &str {
        self.get_string("stash_include_untracked")
    }

    fn stash_created(&self) -> &str {
        self.get_string("stash_created")
    }

    fn stash_changes(&self) -> &str {
        self.get_string("stash_changes")
    }

    fn stash_files(&self) -> &str {
        self.get_string("stash_files")
    }

    fn stash_more(&self) -> &str {
        self.get_string("stash_more")
    }

    fn stash_pop(&self) -> &str {
        self.get_string("stash_pop")
    }

    fn stash_apply(&self) -> &str {
        self.get_string("stash_apply")
    }

    fn stash_drop(&self) -> &str {
        self.get_string("stash_drop")
    }

    fn stash_diff(&self) -> &str {
        self.get_string("stash_diff")
    }

    fn git_stash_button(&self) -> &str {
        self.get_string("git_stash_button")
    }

    fn tools_journal(&self) -> &str {
        self.get_string("tools_journal")
    }

    fn tools_diagnostics(&self) -> &str {
        self.get_string("tools_diagnostics")
    }

    fn tools_operations(&self) -> &str {
        self.get_string("tools_operations")
    }

    fn tools_outline(&self) -> &str {
        self.get_string("tools_outline")
    }

    fn options_help(&self) -> &str {
        self.get_string("options_help")
    }

    fn options_manage_scripts(&self) -> &str {
        self.get_string("options_manage_scripts")
    }

    fn git_action_diff(&self) -> &str {
        self.get_string("git_action_diff")
    }

    fn git_action_revert(&self) -> &str {
        self.get_string("git_action_revert")
    }

    fn git_action_close(&self) -> &str {
        self.get_string("git_action_close")
    }

    fn git_action_git_status(&self) -> &str {
        self.get_string("git_action_git_status")
    }

    fn git_action_init(&self) -> &str {
        self.get_string("git_action_init")
    }

    fn git_init_success(&self, path: &str) -> String {
        self.get_string("git_init_success").replace("{path}", path)
    }

    fn git_action_commit(&self) -> &str {
        self.get_string("git_action_commit")
    }

    fn git_action_push(&self) -> &str {
        self.get_string("git_action_push")
    }

    fn git_action_pull(&self) -> &str {
        self.get_string("git_action_pull")
    }

    fn git_revert_confirm(&self) -> &str {
        self.get_string("git_revert_confirm")
    }

    fn git_commit_title(&self, count: usize, repo: &str, branch: &str) -> String {
        self.get_string("git_commit_title")
            .replace("{count}", &count.to_string())
            .replace("{repo}", repo)
            .replace("{branch}", branch)
    }

    fn git_file_properties_title(&self) -> &str {
        self.get_string("git_file_properties_title")
    }

    fn git_props_path(&self) -> &str {
        self.get_string("git_props_path")
    }

    fn git_props_status(&self) -> &str {
        self.get_string("git_props_status")
    }

    fn git_props_size(&self) -> &str {
        self.get_string("git_props_size")
    }

    fn git_props_diff(&self) -> &str {
        self.get_string("git_props_diff")
    }

    fn git_props_deleted(&self) -> &str {
        self.get_string("git_props_deleted")
    }

    fn git_action_edit(&self) -> &str {
        self.get_string("git_action_edit")
    }

    fn git_status_added(&self) -> String {
        self.get_string("git_status_added").to_string()
    }

    fn git_status_deleted(&self) -> String {
        self.get_string("git_status_deleted").to_string()
    }

    fn git_status_modified(&self) -> String {
        self.get_string("git_status_modified").to_string()
    }

    fn git_status_renamed(&self) -> String {
        self.get_string("git_status_renamed").to_string()
    }

    fn git_status_untracked(&self) -> String {
        self.get_string("git_status_untracked").to_string()
    }

    fn git_push_in_progress(&self) -> String {
        self.get_string("git_push_in_progress").to_string()
    }

    fn git_pull_in_progress(&self) -> String {
        self.get_string("git_pull_in_progress").to_string()
    }

    fn git_fetch_in_progress(&self) -> String {
        self.get_string("git_fetch_in_progress").to_string()
    }

    fn git_push_success(&self) -> String {
        self.get_string("git_push_success").to_string()
    }

    fn git_push_failed(&self) -> String {
        self.get_string("git_push_failed").to_string()
    }

    fn git_pull_success(&self) -> String {
        self.get_string("git_pull_success").to_string()
    }

    fn git_pull_failed(&self) -> String {
        self.get_string("git_pull_failed").to_string()
    }

    fn git_completed(&self) -> String {
        self.get_string("git_completed").to_string()
    }

    fn git_operation_timed_out(&self) -> &str {
        self.get_string("git_operation_timed_out")
    }

    fn preferences_themes(&self) -> &str {
        self.get_string("preferences_themes")
    }

    fn preferences_language(&self) -> &str {
        self.get_string("preferences_language")
    }

    fn preferences_edit(&self) -> &str {
        self.get_string("preferences_edit")
    }

    fn theme_changed(&self, name: &str) -> String {
        self.format("theme_changed", &[("name", name)])
    }

    fn language_changed(&self, name: &str) -> String {
        self.format("language_changed", &[("name", name)])
    }

    fn sessions_title(&self) -> &str {
        self.get_string("sessions_title")
    }

    fn sessions_current(&self) -> &str {
        self.get_string("sessions_current")
    }

    fn sessions_new(&self) -> &str {
        self.get_string("sessions_new")
    }

    fn sessions_switch(&self) -> &str {
        self.get_string("sessions_switch")
    }

    fn sessions_change_root(&self) -> &str {
        self.get_string("sessions_change_root")
    }

    fn session_created(&self) -> &str {
        self.get_string("session_created")
    }

    fn session_moved(&self) -> &str {
        self.get_string("session_moved")
    }

    fn directory_picker_create(&self) -> &str {
        self.get_string("directory_picker_create")
    }

    fn directory_picker_move(&self) -> &str {
        self.get_string("directory_picker_move")
    }

    fn directory_picker_cancel(&self) -> &str {
        self.get_string("directory_picker_cancel")
    }

    fn directory_switcher_title(&self) -> &str {
        self.get_string("directory_switcher_title")
    }

    fn directory_switcher_no_paths(&self) -> &str {
        self.get_string("directory_switcher_no_paths")
    }

    fn directory_switcher_unsupported(&self) -> &str {
        self.get_string("directory_switcher_unsupported")
    }

    fn directory_switcher_process_running(&self) -> &str {
        self.get_string("directory_switcher_process_running")
    }

    fn time_just_now(&self) -> &str {
        self.get_string("time_just_now")
    }

    fn time_minutes_ago(&self, count: usize) -> String {
        let plural = self.pluralize(count, "minute");
        self.format(
            "time_minutes_ago",
            &[("count", &count.to_string()), ("plural", plural)],
        )
    }

    fn time_hours_ago(&self, count: usize) -> String {
        let plural = self.pluralize(count, "hour");
        self.format(
            "time_hours_ago",
            &[("count", &count.to_string()), ("plural", plural)],
        )
    }

    fn time_days_ago(&self, count: usize) -> String {
        let plural = self.pluralize(count, "day");
        self.format(
            "time_days_ago",
            &[("count", &count.to_string()), ("plural", plural)],
        )
    }

    fn time_weeks_ago(&self, count: usize) -> String {
        let plural = self.pluralize(count, "week");
        self.format(
            "time_weeks_ago",
            &[("count", &count.to_string()), ("plural", plural)],
        )
    }

    fn time_months_ago(&self, count: usize) -> String {
        let plural = self.pluralize(count, "month");
        self.format(
            "time_months_ago",
            &[("count", &count.to_string()), ("plural", plural)],
        )
    }

    fn status_dir(&self) -> &str {
        self.get_string("status_dir")
    }

    fn status_file(&self) -> &str {
        self.get_string("status_file")
    }

    fn status_mod(&self) -> &str {
        self.get_string("status_mod")
    }

    fn status_owner(&self) -> &str {
        self.get_string("status_owner")
    }

    fn status_selected(&self) -> &str {
        self.get_string("status_selected")
    }

    fn status_pos(&self) -> &str {
        self.get_string("status_pos")
    }

    fn status_tab(&self) -> &str {
        self.get_string("status_tab")
    }

    fn status_plain_text(&self) -> &str {
        self.get_string("status_plain_text")
    }

    fn status_readonly(&self) -> &str {
        self.get_string("status_readonly")
    }

    fn status_terminal(&self) -> &str {
        self.get_string("status_terminal")
    }

    fn status_layout(&self) -> &str {
        self.get_string("status_layout")
    }

    fn ui_yes(&self) -> &str {
        self.get_string("ui_yes")
    }

    fn ui_no(&self) -> &str {
        self.get_string("ui_no")
    }

    fn ui_ok(&self) -> &str {
        self.get_string("ui_ok")
    }

    fn ui_cancel(&self) -> &str {
        self.get_string("ui_cancel")
    }

    fn ui_continue(&self) -> &str {
        self.get_string("ui_continue")
    }

    fn ui_close(&self) -> &str {
        self.get_string("ui_close")
    }

    fn ui_hint_separator(&self) -> &str {
        self.get_string("ui_hint_separator")
    }

    fn checkbox_executable(&self) -> &str {
        self.get_string("checkbox_executable")
    }

    fn checkbox_create_symlink(&self) -> &str {
        self.get_string("checkbox_create_symlink")
    }

    fn size_bytes(&self) -> &str {
        self.get_string("size_bytes")
    }

    fn size_kilobytes(&self) -> &str {
        self.get_string("size_kilobytes")
    }

    fn size_megabytes(&self) -> &str {
        self.get_string("size_megabytes")
    }

    fn size_gigabytes(&self) -> &str {
        self.get_string("size_gigabytes")
    }

    fn file_info_title_file(&self, name: &str) -> String {
        self.format("file_info_title_file", &[("name", name)])
    }

    fn file_info_title_directory(&self, name: &str) -> String {
        self.format("file_info_title_directory", &[("name", name)])
    }

    fn file_info_title_symlink(&self, name: &str) -> String {
        self.format("file_info_title_symlink", &[("name", name)])
    }

    fn file_info_path(&self) -> &str {
        self.get_string("file_info_path")
    }

    fn file_info_size(&self) -> &str {
        self.get_string("file_info_size")
    }

    fn file_info_owner(&self) -> &str {
        self.get_string("file_info_owner")
    }

    fn file_info_group(&self) -> &str {
        self.get_string("file_info_group")
    }

    fn file_info_created(&self) -> &str {
        self.get_string("file_info_created")
    }

    fn file_info_modified(&self) -> &str {
        self.get_string("file_info_modified")
    }

    fn file_info_calculating(&self) -> &str {
        self.get_string("file_info_calculating")
    }

    fn file_info_git(&self) -> &str {
        self.get_string("file_info_git")
    }

    fn file_info_git_uncommitted(&self, count: usize) -> String {
        self.format(
            "file_info_git_uncommitted",
            &[("count", &count.to_string())],
        )
    }

    fn file_info_git_ahead(&self, count: usize) -> String {
        self.format("file_info_git_ahead", &[("count", &count.to_string())])
    }

    fn file_info_git_behind(&self, count: usize) -> String {
        self.format("file_info_git_behind", &[("count", &count.to_string())])
    }

    fn file_info_git_ignored(&self) -> &str {
        self.get_string("file_info_git_ignored")
    }

    fn file_info_follow_symlink(&self) -> &str {
        self.get_string("file_info_follow_symlink")
    }

    fn perm_permissions(&self) -> &str {
        self.get_string("perm_permissions")
    }

    fn perm_owner(&self) -> &str {
        self.get_string("perm_owner")
    }

    fn perm_group(&self) -> &str {
        self.get_string("perm_group")
    }

    fn perm_others(&self) -> &str {
        self.get_string("perm_others")
    }

    fn file_type_directory(&self) -> &str {
        self.get_string("file_type_directory")
    }

    fn file_type_file(&self) -> &str {
        self.get_string("file_type_file")
    }

    fn progress_scanning(&self) -> &str {
        self.get_string("progress_scanning")
    }

    fn progress_delete_title(&self) -> &str {
        self.get_string("progress_delete_title")
    }

    fn progress_copy_title(&self) -> &str {
        self.get_string("progress_copy_title")
    }

    fn progress_move_title(&self) -> &str {
        self.get_string("progress_move_title")
    }

    fn progress_resume(&self) -> &str {
        self.get_string("progress_resume")
    }

    fn progress_suspend(&self) -> &str {
        self.get_string("progress_suspend")
    }

    fn progress_pause(&self) -> &str {
        self.get_string("progress_pause")
    }

    fn progress_abort(&self) -> &str {
        self.get_string("progress_abort")
    }

    fn progress_counting_files(&self) -> &str {
        self.get_string("progress_counting_files")
    }

    fn progress_files_count(&self, current: usize, total: usize) -> String {
        self.format(
            "progress_files_count",
            &[
                ("current", &current.to_string()),
                ("total", &total.to_string()),
            ],
        )
    }

    fn progress_files_size(&self, count: &str, size: &str) -> String {
        self.format("progress_files_size", &[("count", count), ("size", size)])
    }

    fn progress_data_count(&self, current: &str, total: &str) -> String {
        self.format(
            "progress_data_count",
            &[("current", current), ("total", total)],
        )
    }

    fn progress_speed_eta(&self, speed: &str, eta: &str) -> String {
        self.format("progress_speed_eta", &[("speed", speed), ("eta", eta)])
    }

    fn progress_speed(&self, speed: &str) -> String {
        self.format("progress_speed", &[("speed", speed)])
    }

    fn conflict_directory_title(&self) -> &str {
        self.get_string("conflict_directory_title")
    }

    fn conflict_file_title(&self) -> &str {
        self.get_string("conflict_file_title")
    }

    fn conflict_overwrite(&self) -> &str {
        self.get_string("conflict_overwrite")
    }

    fn conflict_skip(&self) -> &str {
        self.get_string("conflict_skip")
    }

    fn conflict_rename(&self) -> &str {
        self.get_string("conflict_rename")
    }

    fn conflict_overwrite_all(&self) -> &str {
        self.get_string("conflict_overwrite_all")
    }

    fn conflict_skip_all(&self) -> &str {
        self.get_string("conflict_skip_all")
    }

    fn conflict_rename_all(&self) -> &str {
        self.get_string("conflict_rename_all")
    }

    fn conflict_already_exists(&self, item_type: &str, name: &str) -> String {
        self.format(
            "conflict_already_exists",
            &[("type", item_type), ("name", name)],
        )
    }

    fn status_config_saved(&self) -> &str {
        self.get_string("status_config_saved")
    }

    fn status_delete_failed(&self, error: &str) -> String {
        self.format("status_delete_failed", &[("error", error)])
    }

    fn op_type_copy_upload(&self) -> &str {
        self.get_string("op_type_copy_upload")
    }

    fn op_type_copy_download(&self) -> &str {
        self.get_string("op_type_copy_download")
    }

    fn op_type_move_upload(&self) -> &str {
        self.get_string("op_type_move_upload")
    }

    fn op_type_move_download(&self) -> &str {
        self.get_string("op_type_move_download")
    }

    fn op_type_rename(&self) -> &str {
        self.get_string("op_type_rename")
    }

    fn op_type_script(&self) -> &str {
        self.get_string("op_type_script")
    }

    fn op_type_scanning(&self) -> &str {
        self.get_string("op_type_scanning")
    }

    fn op_found_count(&self, count: usize) -> String {
        self.format("op_found_count", &[("count", &count.to_string())])
    }

    fn op_files_progress(&self, current: usize, total: usize) -> String {
        self.format(
            "op_files_progress",
            &[
                ("current", &current.to_string()),
                ("total", &total.to_string()),
            ],
        )
    }

    fn op_data_progress(&self, current: &str, total: &str) -> String {
        self.format(
            "op_data_progress",
            &[("current", current), ("total", total)],
        )
    }

    fn op_speed_rate(&self, speed: &str) -> String {
        self.format("op_speed_rate", &[("speed", speed)])
    }

    fn op_elapsed(&self, time: &str) -> String {
        self.format("op_elapsed", &[("time", time)])
    }

    fn modal_confirm_title(&self) -> &str {
        self.get_string("modal_confirm_title")
    }

    fn modal_error_title(&self) -> &str {
        self.get_string("modal_error_title")
    }

    fn git_no_repo(&self) -> &str {
        self.get_string("git_no_repo")
    }

    fn git_branch_detached(&self) -> &str {
        self.get_string("git_branch_detached")
    }

    fn git_refreshed(&self) -> &str {
        self.get_string("git_refreshed")
    }

    fn git_status_loading(&self) -> &str {
        self.get_string("git_status_loading")
    }

    fn git_staged_header(&self) -> &str {
        self.get_string("git_staged_header")
    }

    fn git_unstaged_header(&self) -> &str {
        self.get_string("git_unstaged_header")
    }

    fn git_stage_all_btn(&self) -> &str {
        self.get_string("git_stage_all_btn")
    }

    fn git_unstage_all_btn(&self) -> &str {
        self.get_string("git_unstage_all_btn")
    }

    fn git_revert_all_btn(&self) -> &str {
        self.get_string("git_revert_all_btn")
    }

    fn git_log_btn(&self) -> &str {
        self.get_string("git_log_btn")
    }

    fn git_revert_all_confirm(&self) -> &str {
        self.get_string("git_revert_all_confirm")
    }

    fn git_checkout_not_impl(&self) -> &str {
        self.get_string("git_checkout_not_impl")
    }

    fn git_no_remote_url(&self) -> &str {
        self.get_string("git_no_remote_url")
    }

    fn git_diff_staged_marker(&self) -> &str {
        self.get_string("git_diff_staged_marker")
    }

    fn git_pushing(&self) -> &str {
        self.get_string("git_pushing")
    }

    fn git_pulling(&self) -> &str {
        self.get_string("git_pulling")
    }

    fn git_action_files_fmt(&self, action: &str, count: usize) -> String {
        let plural = self.pluralize(count, "file");
        self.format(
            "git_action_files_fmt",
            &[
                ("action", action),
                ("count", &count.to_string()),
                ("plural", plural),
            ],
        )
    }

    fn git_action_error_fmt(&self, action: &str, error: &str) -> String {
        self.format(
            "git_action_error_fmt",
            &[("action", action), ("error", error)],
        )
    }

    fn git_switched_to_fmt(&self, branch: &str) -> String {
        self.format("git_switched_to_fmt", &[("branch", branch)])
    }

    fn git_checkout_error_fmt(&self, error: &str) -> String {
        self.format("git_checkout_error_fmt", &[("error", error)])
    }

    fn git_init_failed_fmt(&self, error: &str) -> String {
        self.format("git_init_failed_fmt", &[("error", error)])
    }

    fn git_log_title_fmt(&self, repo: &str, branch: &str) -> String {
        self.format("git_log_title_fmt", &[("repo", repo), ("branch", branch)])
    }

    fn git_diff_title_commit_fmt(
        &self,
        repo: &str,
        branch: &str,
        hash: &str,
        files: &str,
    ) -> String {
        self.format(
            "git_diff_title_commit_fmt",
            &[
                ("repo", repo),
                ("branch", branch),
                ("hash", hash),
                ("files", files),
            ],
        )
    }

    fn git_diff_title_fmt(&self, repo: &str, branch: &str, files: &str) -> String {
        self.format(
            "git_diff_title_fmt",
            &[("repo", repo), ("branch", branch), ("files", files)],
        )
    }

    fn git_commit_info_title(&self, hash: &str) -> String {
        self.format("git_commit_info_title", &[("hash", hash)])
    }

    fn git_commit_author(&self) -> &str {
        self.get_string("git_commit_author")
    }

    fn git_commit_date(&self) -> &str {
        self.get_string("git_commit_date")
    }

    fn git_commit_message(&self) -> &str {
        self.get_string("git_commit_message")
    }

    fn git_commit_files(&self) -> &str {
        self.get_string("git_commit_files")
    }

    fn git_commit_files_modified(&self) -> &str {
        self.get_string("git_commit_files_modified")
    }

    fn git_commit_files_added(&self) -> &str {
        self.get_string("git_commit_files_added")
    }

    fn git_commit_files_deleted(&self) -> &str {
        self.get_string("git_commit_files_deleted")
    }

    fn git_commit_lines(&self) -> &str {
        self.get_string("git_commit_lines")
    }

    fn outline_title(&self) -> &str {
        self.get_string("outline_title")
    }

    fn outline_no_symbols(&self) -> &str {
        self.get_string("outline_no_symbols")
    }

    fn outline_title_count_fmt(&self, count: usize) -> String {
        self.format("outline_title_count_fmt", &[("count", &count.to_string())])
    }

    fn diagnostics_title(&self) -> &str {
        self.get_string("diagnostics_title")
    }

    fn diagnostics_no_items(&self) -> &str {
        self.get_string("diagnostics_no_items")
    }

    fn diagnostics_filter_all(&self) -> &str {
        self.get_string("diagnostics_filter_all")
    }

    fn diagnostics_filter_errors(&self) -> &str {
        self.get_string("diagnostics_filter_errors")
    }

    fn diagnostics_filter_ew(&self) -> &str {
        self.get_string("diagnostics_filter_ew")
    }

    fn diagnostics_title_fmt(&self, errors: usize, warnings: usize) -> String {
        self.format(
            "diagnostics_title_fmt",
            &[
                ("errors", &errors.to_string()),
                ("warnings", &warnings.to_string()),
            ],
        )
    }

    fn diagnostics_filter_fmt(&self, filter: &str, count: usize) -> String {
        self.format(
            "diagnostics_filter_fmt",
            &[("filter", filter), ("count", &count.to_string())],
        )
    }

    fn terminal_kill_confirm(&self) -> &str {
        self.get_string("terminal_kill_confirm")
    }

    fn panel_image(&self) -> &str {
        self.get_string("panel_image")
    }

    fn image_error_fmt(&self, error: &str) -> String {
        self.format("image_error_fmt", &[("error", error)])
    }

    fn resource_cpu_top_title(&self) -> &str {
        self.get_string("resource_cpu_top_title")
    }

    fn resource_ram_top_title(&self) -> &str {
        self.get_string("resource_ram_top_title")
    }

    fn resource_disk_title(&self) -> &str {
        self.get_string("resource_disk_title")
    }

    fn resource_disk_free(&self) -> &str {
        self.get_string("resource_disk_free")
    }

    fn resource_disk_used(&self) -> &str {
        self.get_string("resource_disk_used")
    }

    fn resource_disk_total(&self) -> &str {
        self.get_string("resource_disk_total")
    }

    fn resource_processes(&self) -> &str {
        self.get_string("resource_processes")
    }

    fn resource_count(&self) -> &str {
        self.get_string("resource_count")
    }

    fn resource_net_title(&self) -> &str {
        self.get_string("resource_net_title")
    }

    fn vfs_connecting(&self) -> &str {
        self.get_string("vfs_connecting")
    }

    fn vfs_connection_failed(&self) -> &str {
        self.get_string("vfs_connection_failed")
    }

    fn vfs_ftp_connected(&self) -> &str {
        self.get_string("vfs_ftp_connected")
    }

    fn vfs_password_prompt(&self) -> &str {
        self.get_string("vfs_password_prompt")
    }

    fn vfs_smb_connected(&self) -> &str {
        self.get_string("vfs_smb_connected")
    }

    fn vfs_username_prompt(&self) -> &str {
        self.get_string("vfs_username_prompt")
    }

    // Calendar
    fn calendar_mon(&self) -> &str {
        self.get_string("calendar_mon")
    }
    fn calendar_tue(&self) -> &str {
        self.get_string("calendar_tue")
    }
    fn calendar_wed(&self) -> &str {
        self.get_string("calendar_wed")
    }
    fn calendar_thu(&self) -> &str {
        self.get_string("calendar_thu")
    }
    fn calendar_fri(&self) -> &str {
        self.get_string("calendar_fri")
    }
    fn calendar_sat(&self) -> &str {
        self.get_string("calendar_sat")
    }
    fn calendar_sun(&self) -> &str {
        self.get_string("calendar_sun")
    }
    fn calendar_january(&self) -> &str {
        self.get_string("calendar_january")
    }
    fn calendar_february(&self) -> &str {
        self.get_string("calendar_february")
    }
    fn calendar_march(&self) -> &str {
        self.get_string("calendar_march")
    }
    fn calendar_april(&self) -> &str {
        self.get_string("calendar_april")
    }
    fn calendar_may(&self) -> &str {
        self.get_string("calendar_may")
    }
    fn calendar_june(&self) -> &str {
        self.get_string("calendar_june")
    }
    fn calendar_july(&self) -> &str {
        self.get_string("calendar_july")
    }
    fn calendar_august(&self) -> &str {
        self.get_string("calendar_august")
    }
    fn calendar_september(&self) -> &str {
        self.get_string("calendar_september")
    }
    fn calendar_october(&self) -> &str {
        self.get_string("calendar_october")
    }
    fn calendar_november(&self) -> &str {
        self.get_string("calendar_november")
    }
    fn calendar_december(&self) -> &str {
        self.get_string("calendar_december")
    }
}
