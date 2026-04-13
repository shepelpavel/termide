//! Internationalization support for termide.
//!
//! Provides translation loading and language detection.

// I18n trait methods are prepared for future use
#![allow(dead_code)]

use std::sync::RwLock;

mod detect;
pub mod loader;
pub mod runtime;

pub use detect::{detect_language, normalize_lang};
pub use loader::{Metadata, PluralRules, TranslationData};

/// Supported languages with their native names (code, native_name).
/// Sorted alphabetically by language code.
pub const SUPPORTED_LANGUAGES: &[(&str, &str)] = &[
    ("bn", "বাংলা"),
    ("de", "Deutsch"),
    ("en", "English"),
    ("es", "Español"),
    ("fr", "Français"),
    ("hi", "हिन्दी"),
    ("id", "Bahasa Indonesia"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("pt", "Português"),
    ("ru", "Русский"),
    ("th", "ไทย"),
    ("tr", "Türkçe"),
    ("vi", "Tiếng Việt"),
    ("zh", "中文"),
];

/// Global translation instance (RwLock for runtime switching).
///
/// Stores a leaked `&'static` reference so that `t()` can return `&'static dyn Translation`
/// without unsafe pointer casts. Old translations are intentionally leaked on language switch
/// (a few KB each, happens rarely).
static TRANSLATION: RwLock<Option<&'static dyn Translation>> = RwLock::new(None);

/// Current language code (RwLock for runtime switching).
static CURRENT_LANGUAGE: RwLock<String> = RwLock::new(String::new());

/// Translation trait for all user-facing strings.
pub trait Translation: Send + Sync {
    // File Manager operations
    fn fm_paste_confirm(&self, count: usize, mode: &str, dest: &str) -> String;
    fn fm_copy_prompt(&self, name: &str) -> String;
    fn fm_move_prompt(&self, name: &str) -> String;
    fn git_operation_cancelled(&self) -> &str;

    // Modal buttons
    fn modal_yes(&self) -> &str;
    fn modal_ok(&self) -> &str;
    // Panel titles
    fn panel_help(&self) -> &str;
    fn panel_journal(&self) -> &str;
    fn panel_operations(&self) -> &str;

    // Operations panel
    fn no_active_operations(&self) -> &str;

    // Editor
    fn editor_close_unsaved(&self) -> &str;
    fn editor_close_unsaved_question(&self) -> &str;
    fn editor_save_and_close(&self) -> &str;
    fn editor_close_without_saving(&self) -> &str;
    fn editor_cancel(&self) -> &str;
    fn editor_close_external(&self) -> &str;
    fn editor_close_external_question(&self) -> &str;
    fn editor_overwrite_disk(&self) -> &str;
    fn editor_keep_disk_close(&self) -> &str;
    fn editor_reload_into_editor(&self) -> &str;
    fn editor_close_conflict(&self) -> &str;
    fn editor_close_conflict_question(&self) -> &str;
    fn editor_reload_from_disk(&self) -> &str;
    fn editor_file_opened(&self, filename: &str) -> String;
    fn editor_search_match_info(&self, current: usize, total: usize) -> String;
    fn editor_search_no_matches(&self) -> &str;
    fn editor_deletion_marker(&self, count: usize) -> String;

    // File manager
    fn file_search_title(&self) -> &str;
    fn content_search_title(&self) -> &str;
    fn fm_goto_title(&self) -> &str;
    fn fm_goto_prompt(&self) -> &str;
    fn connection_cancelled_title(&self) -> &str;
    fn connection_error_title(&self) -> &str;
    fn connection_timeout_title(&self) -> &str;
    fn connection_timeout_message(&self) -> &str;

    // Terminal
    // Git status
    // Application quit
    fn app_quit_confirm(&self) -> &str;
    fn app_quit_title(&self) -> &str;

    // Errors
    // Help modal
    fn help_global_keys(&self) -> &str;
    fn help_file_manager_keys(&self) -> &str;
    fn help_editor_keys(&self) -> &str;
    fn help_terminal_keys(&self) -> &str;
    // Help key descriptions
    fn help_desc_menu(&self) -> &str;
    fn help_desc_quit(&self) -> &str;
    fn help_desc_help(&self) -> &str;
    fn help_desc_close_panel(&self) -> &str;
    fn help_desc_escape_close(&self) -> &str;
    fn help_desc_select(&self) -> &str;
    fn help_desc_new_terminal(&self) -> &str;
    fn help_desc_parent_dir(&self) -> &str;
    fn help_desc_home(&self) -> &str;
    fn help_desc_end(&self) -> &str;
    fn help_desc_page_scroll(&self) -> &str;
    fn help_desc_create_file(&self) -> &str;
    fn help_desc_create_dir(&self) -> &str;
    fn help_desc_copy(&self) -> &str;
    fn help_desc_move(&self) -> &str;
    fn help_desc_delete(&self) -> &str;
    fn help_desc_rename(&self) -> &str;
    fn help_desc_save(&self) -> &str;
    // Help section headers
    fn help_section_panels(&self) -> &str;
    fn help_section_git_status(&self) -> &str;
    fn help_section_navigation(&self) -> &str;
    fn help_section_git_diff(&self) -> &str;
    fn help_section_git_log(&self) -> &str;

    // Additional help key descriptions
    fn help_desc_new_file_manager(&self) -> &str;
    fn help_desc_new_editor(&self) -> &str;
    fn help_desc_new_journal(&self) -> &str;
    fn help_desc_open_preferences(&self) -> &str;
    fn help_desc_open_sessions(&self) -> &str;
    fn help_desc_open_git_status(&self) -> &str;
    fn help_desc_open_outline(&self) -> &str;
    fn help_desc_open_diagnostics(&self) -> &str;
    fn help_desc_open_git_log(&self) -> &str;
    fn help_desc_toggle_stack(&self) -> &str;
    fn help_desc_swap_left(&self) -> &str;
    fn help_desc_swap_right(&self) -> &str;
    fn help_desc_move_first(&self) -> &str;
    fn help_desc_move_last(&self) -> &str;
    fn help_desc_resize_smaller(&self) -> &str;
    fn help_desc_resize_larger(&self) -> &str;
    fn help_desc_prev_group(&self) -> &str;
    fn help_desc_next_group(&self) -> &str;
    fn help_desc_prev_panel(&self) -> &str;
    fn help_desc_next_panel(&self) -> &str;
    fn help_desc_goto_panel(&self) -> &str;
    fn help_desc_save_as(&self) -> &str;
    fn help_desc_reload(&self) -> &str;
    fn help_desc_duplicate_line(&self) -> &str;
    fn help_desc_toggle_comment(&self) -> &str;
    fn help_desc_search_next(&self) -> &str;
    fn help_desc_search_prev(&self) -> &str;
    fn help_desc_replace(&self) -> &str;
    fn help_desc_replace_current(&self) -> &str;
    fn help_desc_replace_all(&self) -> &str;
    // LSP help descriptions
    fn help_desc_trigger_completion(&self) -> &str;
    fn help_desc_accept_completion(&self) -> &str;
    fn help_desc_cancel_completion(&self) -> &str;
    fn help_desc_navigate_completion(&self) -> &str;
    fn help_desc_show_hover(&self) -> &str;
    fn help_desc_goto_definition(&self) -> &str;
    fn help_desc_find_references(&self) -> &str;
    fn help_desc_rename_symbol(&self) -> &str;
    fn help_desc_word_nav(&self) -> &str;
    fn help_desc_paragraph_nav(&self) -> &str;
    fn help_desc_view_file(&self) -> &str;
    fn help_desc_edit_file(&self) -> &str;
    fn help_desc_search_content(&self) -> &str;
    fn help_desc_go_home(&self) -> &str;
    fn help_desc_toggle_hidden(&self) -> &str;
    fn help_desc_open_external(&self) -> &str;
    fn help_desc_delete_generic(&self) -> &str;
    fn help_desc_open_bookmark_add(&self) -> &str;
    fn help_desc_command_palette(&self) -> &str;
    fn help_desc_stage_file(&self) -> &str;
    fn help_desc_unstage_file(&self) -> &str;
    fn help_desc_next_section(&self) -> &str;
    fn help_desc_prev_section(&self) -> &str;
    fn help_desc_terminal_copy(&self) -> &str;
    fn help_desc_terminal_paste(&self) -> &str;
    fn help_desc_scroll_up(&self) -> &str;
    fn help_desc_scroll_down(&self) -> &str;
    fn help_desc_scroll_top(&self) -> &str;
    fn help_desc_scroll_bottom(&self) -> &str;

    // Navigation help descriptions (static keys)
    fn help_desc_move_up(&self) -> &str;
    fn help_desc_move_down(&self) -> &str;
    fn help_desc_scroll_half_up(&self) -> &str;
    fn help_desc_scroll_half_down(&self) -> &str;

    // Git Diff help descriptions (static keys)
    fn help_desc_toggle_collapse(&self) -> &str;
    fn help_desc_open_file_editor(&self) -> &str;

    // Git Log help descriptions (static keys)
    fn help_desc_view_commit_diff(&self) -> &str;

    // File operation status
    fn status_file_created(&self, name: &str) -> String;
    fn status_dir_created(&self, name: &str) -> String;
    fn status_file_saved(&self, name: &str) -> String;
    fn status_error_save(&self, error: &str) -> String;
    fn status_file_reloaded(&self) -> &str;
    fn status_error_reload(&self, error: &str) -> String;
    fn status_error_open_file(&self, name: &str, error: &str) -> String;
    fn status_opening_external(&self, name: &str) -> String;

    // Action words
    // Modal titles
    fn modal_copy_single_title(&self, name: &str) -> String;
    fn modal_copy_multiple_title(&self, count: usize) -> String;
    fn modal_move_single_title(&self, name: &str) -> String;
    fn modal_move_multiple_title(&self, count: usize) -> String;
    fn modal_create_file_title(&self) -> &str;
    fn modal_create_dir_title(&self) -> &str;
    fn modal_delete_single_title(&self, name: &str) -> String;
    fn modal_delete_multiple_title(&self, count: usize) -> String;
    fn modal_save_as_title(&self) -> &str;
    fn modal_copy_single_prompt(&self, name: &str) -> String;
    fn modal_copy_multiple_prompt(&self, count: usize) -> String;
    fn modal_move_single_prompt(&self, name: &str) -> String;
    fn modal_move_multiple_prompt(&self, count: usize) -> String;

    // Batch results
    fn batch_result_file_copied(&self) -> &str;
    fn batch_result_file_moved(&self) -> &str;
    fn batch_result_error_copy(&self) -> &str;
    fn batch_result_error_move(&self) -> &str;
    fn batch_result_copied(&self) -> &str;
    fn batch_result_moved(&self) -> &str;
    fn batch_result_skipped_fmt(&self, count: usize) -> String;
    fn batch_result_errors_fmt(&self, count: usize) -> String;

    // Menu
    fn menu_sessions(&self) -> &str;
    fn menu_windows(&self) -> &str;
    fn menu_scripts(&self) -> &str;
    fn menu_scripts_add(&self) -> &str;
    fn menu_options(&self) -> &str;
    fn menu_quit(&self) -> &str;
    fn menu_navigate_hint(&self) -> &str;
    fn menu_open_hint_label(&self) -> &str;
    fn menu_bookmarks(&self) -> &str;

    // Bookmarks submenu
    fn bookmarks_add_bookmark(&self) -> &str;
    fn bookmarks_manage(&self) -> &str;
    fn bookmarks_no_bookmarks(&self) -> &str;
    fn bookmarks_add_title(&self) -> &str;
    fn bookmarks_add_path(&self) -> &str;
    fn bookmarks_add_description(&self) -> &str;
    fn bookmarks_add_group(&self) -> &str;
    fn bookmarks_add_project(&self) -> &str;
    // Tools submenu
    fn tools_files(&self) -> &str;
    fn tools_terminal(&self) -> &str;
    fn tools_editor(&self) -> &str;
    fn tools_git_status(&self) -> &str;
    fn tools_git_log(&self) -> &str;
    fn tools_git_stash(&self) -> &str;
    fn tools_journal(&self) -> &str;
    fn tools_diagnostics(&self) -> &str;
    fn tools_operations(&self) -> &str;
    fn tools_outline(&self) -> &str;

    // Options submenu
    fn options_help(&self) -> &str;
    fn options_manage_scripts(&self) -> &str;

    // Git action buttons
    fn git_action_diff(&self) -> &str;
    fn git_action_revert(&self) -> &str;
    fn git_action_close(&self) -> &str;
    fn git_action_git_status(&self) -> &str;
    fn git_action_init(&self) -> &str;
    fn git_init_success(&self, path: &str) -> String;
    fn git_action_commit(&self) -> &str;
    fn git_action_push(&self) -> &str;
    fn git_action_pull(&self) -> &str;
    fn git_revert_confirm(&self) -> &str;

    // Git commit modal
    fn git_commit_title(&self, count: usize, repo: &str, branch: &str) -> String;

    // Git file properties modal
    fn git_file_properties_title(&self) -> &str;
    fn git_props_path(&self) -> &str;
    fn git_props_status(&self) -> &str;
    fn git_props_size(&self) -> &str;
    fn git_props_diff(&self) -> &str;
    fn git_props_deleted(&self) -> &str;
    fn git_action_edit(&self) -> &str;
    fn git_status_added(&self) -> String;
    fn git_status_deleted(&self) -> String;
    fn git_status_modified(&self) -> String;
    fn git_status_renamed(&self) -> String;
    fn git_status_untracked(&self) -> String;

    // Git operation progress messages
    fn git_push_in_progress(&self) -> String;
    fn git_pull_in_progress(&self) -> String;
    fn git_fetch_in_progress(&self) -> String;

    // Git operation result messages
    fn git_push_success(&self) -> String;
    fn git_push_failed(&self) -> String;
    fn git_pull_success(&self) -> String;
    fn git_pull_failed(&self) -> String;
    fn git_completed(&self) -> String;
    fn git_operation_timed_out(&self) -> &str;

    // Preferences submenu
    fn preferences_themes(&self) -> &str;
    fn preferences_language(&self) -> &str;
    fn preferences_edit(&self) -> &str;
    fn theme_changed(&self, name: &str) -> String;
    fn language_changed(&self, name: &str) -> String;

    // Sessions
    fn sessions_title(&self) -> &str;
    fn sessions_current(&self) -> &str;
    fn sessions_new(&self) -> &str;
    fn sessions_switch(&self) -> &str;
    fn sessions_change_root(&self) -> &str;
    fn session_created(&self) -> &str;
    fn session_moved(&self) -> &str;

    // Directory picker
    fn directory_picker_create(&self) -> &str;
    fn directory_picker_move(&self) -> &str;
    fn directory_picker_cancel(&self) -> &str;

    // Directory switcher
    fn directory_switcher_title(&self) -> &str;
    fn directory_switcher_no_paths(&self) -> &str;
    fn directory_switcher_unsupported(&self) -> &str;
    fn directory_switcher_process_running(&self) -> &str;

    // Relative time
    fn time_just_now(&self) -> &str;
    fn time_minutes_ago(&self, count: usize) -> String;
    fn time_hours_ago(&self, count: usize) -> String;
    fn time_days_ago(&self, count: usize) -> String;
    fn time_weeks_ago(&self, count: usize) -> String;
    fn time_months_ago(&self, count: usize) -> String;

    // Status bar
    fn status_dir(&self) -> &str;
    fn status_file(&self) -> &str;
    fn status_mod(&self) -> &str;
    fn status_owner(&self) -> &str;
    fn status_selected(&self) -> &str;
    fn status_pos(&self) -> &str;
    fn status_tab(&self) -> &str;
    fn status_plain_text(&self) -> &str;
    fn status_readonly(&self) -> &str;
    fn status_terminal(&self) -> &str;
    fn status_layout(&self) -> &str;
    // UI elements
    fn ui_yes(&self) -> &str;
    fn ui_no(&self) -> &str;
    fn ui_ok(&self) -> &str;
    fn ui_cancel(&self) -> &str;
    fn ui_continue(&self) -> &str;
    fn ui_close(&self) -> &str;
    fn ui_hint_separator(&self) -> &str;

    // Checkboxes
    fn checkbox_executable(&self) -> &str;
    fn checkbox_create_symlink(&self) -> &str;

    // File size units
    fn size_bytes(&self) -> &str;
    fn size_kilobytes(&self) -> &str;
    fn size_megabytes(&self) -> &str;
    fn size_gigabytes(&self) -> &str;
    // File info modal
    fn file_info_title_file(&self, name: &str) -> String;
    fn file_info_title_directory(&self, name: &str) -> String;
    fn file_info_title_symlink(&self, name: &str) -> String;
    fn file_info_path(&self) -> &str;
    fn file_info_size(&self) -> &str;
    fn file_info_owner(&self) -> &str;
    fn file_info_group(&self) -> &str;
    fn file_info_created(&self) -> &str;
    fn file_info_modified(&self) -> &str;
    fn file_info_calculating(&self) -> &str;
    fn file_info_git(&self) -> &str;
    fn file_info_git_uncommitted(&self, count: usize) -> String;
    fn file_info_git_ahead(&self, count: usize) -> String;
    fn file_info_git_behind(&self, count: usize) -> String;
    fn file_info_git_ignored(&self) -> &str;
    fn file_info_follow_symlink(&self) -> &str;
    fn perm_permissions(&self) -> &str;
    fn perm_owner(&self) -> &str;
    fn perm_group(&self) -> &str;
    fn perm_others(&self) -> &str;

    // File types
    fn file_type_directory(&self) -> &str;
    fn file_type_file(&self) -> &str;
    // Progress modal
    fn progress_scanning(&self) -> &str;
    fn progress_delete_title(&self) -> &str;
    fn progress_copy_title(&self) -> &str;
    fn progress_move_title(&self) -> &str;
    fn progress_resume(&self) -> &str;
    fn progress_suspend(&self) -> &str;
    fn progress_pause(&self) -> &str;
    fn progress_abort(&self) -> &str;
    fn progress_counting_files(&self) -> &str;
    fn progress_files_count(&self, current: usize, total: usize) -> String;
    fn progress_files_size(&self, count: &str, size: &str) -> String;
    fn progress_data_count(&self, current: &str, total: &str) -> String;
    fn progress_speed_eta(&self, speed: &str, eta: &str) -> String;
    fn progress_speed(&self, speed: &str) -> String;

    // Conflict modal
    fn conflict_directory_title(&self) -> &str;
    fn conflict_file_title(&self) -> &str;
    fn conflict_overwrite(&self) -> &str;
    fn conflict_skip(&self) -> &str;
    fn conflict_rename(&self) -> &str;
    fn conflict_overwrite_all(&self) -> &str;
    fn conflict_skip_all(&self) -> &str;
    fn conflict_rename_all(&self) -> &str;
    fn conflict_already_exists(&self, item_type: &str, name: &str) -> String;

    // Operation cancellation
    // Status messages
    fn status_config_saved(&self) -> &str;
    fn status_delete_failed(&self, error: &str) -> String;

    // Operation type labels (for operation cards)
    fn op_type_copy_upload(&self) -> &str;
    fn op_type_copy_download(&self) -> &str;
    fn op_type_move_upload(&self) -> &str;
    fn op_type_move_download(&self) -> &str;
    fn op_type_rename(&self) -> &str;
    fn op_type_script(&self) -> &str;
    fn op_type_scanning(&self) -> &str;
    fn op_found_count(&self, count: usize) -> String;
    fn op_files_progress(&self, current: usize, total: usize) -> String;
    fn op_data_progress(&self, current: &str, total: &str) -> String;
    fn op_speed_rate(&self, speed: &str) -> String;
    fn op_elapsed(&self, time: &str) -> String;

    // Modal titles
    fn modal_confirm_title(&self) -> &str;
    fn modal_error_title(&self) -> &str;

    // Git panel strings
    fn git_no_repo(&self) -> &str;
    fn git_branch_detached(&self) -> &str;
    fn git_refreshed(&self) -> &str;
    fn git_status_loading(&self) -> &str;
    fn git_staged_header(&self) -> &str;
    fn git_unstaged_header(&self) -> &str;
    fn git_stage_all_btn(&self) -> &str;
    fn git_unstage_all_btn(&self) -> &str;
    fn git_checkout_not_impl(&self) -> &str;
    fn git_no_remote_url(&self) -> &str;
    fn git_diff_staged_marker(&self) -> &str;
    fn git_pushing(&self) -> &str;
    fn git_pulling(&self) -> &str;
    fn git_action_files_fmt(&self, action: &str, count: usize) -> String;
    fn git_action_error_fmt(&self, action: &str, error: &str) -> String;
    fn git_switched_to_fmt(&self, branch: &str) -> String;
    fn git_checkout_error_fmt(&self, error: &str) -> String;
    fn git_init_failed_fmt(&self, error: &str) -> String;
    fn git_log_title_fmt(&self, repo: &str, branch: &str) -> String;
    fn git_diff_title_commit_fmt(
        &self,
        repo: &str,
        branch: &str,
        hash: &str,
        files: &str,
    ) -> String;
    fn git_diff_title_fmt(&self, repo: &str, branch: &str, files: &str) -> String;

    // Git commit info modal
    fn git_commit_info_title(&self, hash: &str) -> String;
    fn git_commit_author(&self) -> &str;
    fn git_commit_date(&self) -> &str;
    fn git_commit_message(&self) -> &str;
    fn git_commit_files(&self) -> &str;
    fn git_commit_files_modified(&self) -> &str;
    fn git_commit_files_added(&self) -> &str;
    fn git_commit_files_deleted(&self) -> &str;
    fn git_commit_lines(&self) -> &str;

    // Outline panel strings
    fn outline_title(&self) -> &str;
    fn outline_no_symbols(&self) -> &str;
    fn outline_title_count_fmt(&self, count: usize) -> String;

    // Diagnostics panel strings
    fn diagnostics_title(&self) -> &str;
    fn diagnostics_no_items(&self) -> &str;
    fn diagnostics_filter_all(&self) -> &str;
    fn diagnostics_filter_errors(&self) -> &str;
    fn diagnostics_filter_ew(&self) -> &str;
    fn diagnostics_title_fmt(&self, errors: usize, warnings: usize) -> String;
    fn diagnostics_filter_fmt(&self, filter: &str, count: usize) -> String;

    // Terminal
    fn terminal_kill_confirm(&self) -> &str;

    // Image panel
    fn panel_image(&self) -> &str;
    fn image_error_fmt(&self, error: &str) -> String;

    // Resource modals
    fn resource_cpu_top_title(&self) -> &str;
    fn resource_ram_top_title(&self) -> &str;
    fn resource_disk_title(&self) -> &str;
    fn resource_disk_free(&self) -> &str;
    fn resource_disk_used(&self) -> &str;
    fn resource_disk_total(&self) -> &str;
    fn resource_processes(&self) -> &str;
    fn resource_count(&self) -> &str;
    fn resource_net_title(&self) -> &str;

    // Calendar
    fn calendar_mon(&self) -> &str;
    fn calendar_tue(&self) -> &str;
    fn calendar_wed(&self) -> &str;
    fn calendar_thu(&self) -> &str;
    fn calendar_fri(&self) -> &str;
    fn calendar_sat(&self) -> &str;
    fn calendar_sun(&self) -> &str;
    fn calendar_january(&self) -> &str;
    fn calendar_february(&self) -> &str;
    fn calendar_march(&self) -> &str;
    fn calendar_april(&self) -> &str;
    fn calendar_may(&self) -> &str;
    fn calendar_june(&self) -> &str;
    fn calendar_july(&self) -> &str;
    fn calendar_august(&self) -> &str;
    fn calendar_september(&self) -> &str;
    fn calendar_october(&self) -> &str;
    fn calendar_november(&self) -> &str;
    fn calendar_december(&self) -> &str;

    // VFS remote connections
    fn vfs_connecting(&self) -> &str;
    fn vfs_connection_failed(&self) -> &str;
    fn vfs_ftp_connected(&self) -> &str;
    fn vfs_password_prompt(&self) -> &str;
    fn vfs_smb_connected(&self) -> &str;
    fn vfs_username_prompt(&self) -> &str;
}

/// Initialize translation system.
///
/// Returns Ok(()) on success, Err if translation loading fails completely
/// (including fallback to English).
pub fn init() -> anyhow::Result<()> {
    init_with_language("auto")
}

/// Initialize translation system with specified language.
///
/// Returns Ok(()) on success, Err if translation loading fails completely
/// (including fallback to English).
pub fn init_with_language(lang: &str) -> anyhow::Result<()> {
    let detected = if lang == "auto" || lang.is_empty() {
        detect_language()
    } else {
        lang.to_string()
    };

    let translation = runtime::RuntimeTranslation::new(&detected).or_else(|e| {
        log::warn!(
            "Failed to load translations for '{}': {}. Falling back to English",
            detected,
            e
        );
        runtime::RuntimeTranslation::new("en")
    })?;

    let leaked: &'static dyn Translation = Box::leak(Box::new(translation));
    if let Ok(mut guard) = TRANSLATION.write() {
        *guard = Some(leaked);
    }
    if let Ok(mut guard) = CURRENT_LANGUAGE.write() {
        *guard = detected;
    }
    Ok(())
}

/// Set language at runtime (for live preview and language switching).
///
/// Returns Ok(()) on success, Err if language loading fails.
pub fn set_language(lang: &str) -> anyhow::Result<()> {
    let translation = runtime::RuntimeTranslation::new(lang)?;
    let leaked: &'static dyn Translation = Box::leak(Box::new(translation));

    if let Ok(mut guard) = TRANSLATION.write() {
        *guard = Some(leaked);
    }
    if let Ok(mut guard) = CURRENT_LANGUAGE.write() {
        *guard = lang.to_string();
    }
    Ok(())
}

/// Get the current translation.
///
/// # Panics
/// Panics if the translation system is not initialized.
pub fn t() -> &'static dyn Translation {
    let guard = TRANSLATION.read().expect("Translation lock poisoned");
    guard.expect("Translation system not initialized. Call i18n::init() first.")
}

/// Get the current language code.
pub fn current_language() -> String {
    CURRENT_LANGUAGE
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_else(|_| "en".to_string())
}

/// Get list of all supported languages with their native names.
/// Returns Vec of (code, native_name) tuples.
pub fn get_language_list() -> Vec<(&'static str, &'static str)> {
    SUPPORTED_LANGUAGES.to_vec()
}

/// Get the native name of a language by its code.
pub fn get_language_name(code: &str) -> Option<&'static str> {
    SUPPORTED_LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, name)| *name)
}

/// Check if a language is supported.
pub fn is_supported(lang: &str) -> bool {
    SUPPORTED_LANGUAGES.iter().any(|(code, _)| *code == lang)
}
