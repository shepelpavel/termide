//! Dynamic help text generator from keybindings configuration.
//!
//! Generates localized help text based on actual keybindings from config,
//! formatted as pseudo-graphic tables that span full panel width.

use ratatui::{
    style::Style,
    text::{Line, Span},
};
use termide_config::{
    Config, EditorKeybindings, FileManagerKeybindings, GitStatusKeybindings, GlobalKeybindings,
    KeyBinding, TerminalKeybindings,
};
use termide_i18n;
use termide_theme::Theme;

/// A single help entry with keybinding and description.
pub struct HelpEntry {
    /// Keybinding string, e.g., "Alt+M" or "C / F5"
    pub keys: String,
    /// Translated description of the action
    pub description: String,
}

/// A section of help entries with a header.
pub struct HelpSection {
    /// Section header, e.g., "GLOBAL KEYS"
    pub header: String,
    /// List of entries in this section
    pub entries: Vec<HelpEntry>,
}

/// Generator for dynamic help content.
pub struct HelpGenerator;

impl HelpGenerator {
    /// Generate help sections from configuration.
    pub fn generate(config: &Config) -> Vec<HelpSection> {
        let t = termide_i18n::t();

        vec![
            Self::generate_global_section(&config.general.keybindings, t),
            Self::generate_panel_section(&config.general.keybindings, t),
            Self::generate_navigation_section(t),
            Self::generate_file_manager_section(&config.file_manager.keybindings, t),
            Self::generate_editor_section(&config.editor.keybindings, t),
            Self::generate_git_status_section(&config.git_status.keybindings, t),
            Self::generate_git_diff_section(t),
            Self::generate_git_log_section(t),
            Self::generate_terminal_section(&config.terminal.keybindings, t),
        ]
    }

    /// Format keybinding to display string.
    fn format_keys(binding: &Option<KeyBinding>) -> String {
        match binding {
            Some(KeyBinding::Single(s)) => s.clone(),
            Some(KeyBinding::Multiple(v)) => v.join(" / "),
            None => String::new(),
        }
    }

    /// Generate global keybindings section.
    fn generate_global_section(
        kb: &GlobalKeybindings,
        t: &dyn termide_i18n::Translation,
    ) -> HelpSection {
        let entries = vec![
            HelpEntry {
                keys: Self::format_keys(&kb.toggle_menu),
                description: t.help_desc_menu().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.open_help),
                description: t.help_desc_help().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.quit),
                description: t.help_desc_quit().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.new_file_manager),
                description: t.help_desc_new_file_manager().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.new_terminal),
                description: t.help_desc_new_terminal().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.new_editor),
                description: t.help_desc_new_editor().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.new_journal),
                description: t.help_desc_new_journal().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.open_preferences),
                description: t.help_desc_open_preferences().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.open_sessions),
                description: t.help_desc_open_sessions().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.open_git_status),
                description: t.help_desc_open_git_status().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.open_outline),
                description: t.help_desc_open_outline().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.open_diagnostics),
                description: t.help_desc_open_diagnostics().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.open_git_log),
                description: t.help_desc_open_git_log().to_string(),
            },
        ];

        HelpSection {
            header: t.help_global_keys().to_string(),
            entries,
        }
    }

    /// Generate panel management section.
    fn generate_panel_section(
        kb: &GlobalKeybindings,
        t: &dyn termide_i18n::Translation,
    ) -> HelpSection {
        let entries = vec![
            HelpEntry {
                // Add static Escape to configurable close_panel binding
                keys: format!("{} / Escape", Self::format_keys(&kb.close_panel)),
                description: t.help_desc_close_panel().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.toggle_stack),
                description: t.help_desc_toggle_stack().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.swap_left),
                description: t.help_desc_swap_left().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.swap_right),
                description: t.help_desc_swap_right().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.move_first),
                description: t.help_desc_move_first().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.move_last),
                description: t.help_desc_move_last().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.resize_smaller),
                description: t.help_desc_resize_smaller().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.resize_larger),
                description: t.help_desc_resize_larger().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.prev_group),
                description: t.help_desc_prev_group().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.next_group),
                description: t.help_desc_next_group().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.prev_panel),
                description: t.help_desc_prev_panel().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.next_panel),
                description: t.help_desc_next_panel().to_string(),
            },
            HelpEntry {
                keys: "Alt+1..9".to_string(),
                description: t.help_desc_goto_panel().to_string(),
            },
        ];

        HelpSection {
            header: t.help_section_panels().to_string(),
            entries,
        }
    }

    /// Generate file manager keybindings section.
    fn generate_file_manager_section(
        kb: &FileManagerKeybindings,
        t: &dyn termide_i18n::Translation,
    ) -> HelpSection {
        let entries = vec![
            HelpEntry {
                keys: Self::format_keys(&kb.copy_files),
                description: t.help_desc_copy().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.move_files),
                description: t.help_desc_move().to_string(),
            },
            HelpEntry {
                keys: "Delete / F8".to_string(),
                description: t.help_desc_delete().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.rename_file),
                description: t.help_desc_rename().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.view_file),
                description: t.help_desc_view_file().to_string(),
            },
            HelpEntry {
                keys: "E / F4".to_string(),
                description: t.help_desc_edit_file().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.new_file),
                description: t.help_desc_create_file().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.new_directory),
                description: t.help_desc_create_dir().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.search),
                description: t.help_desc_search().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.search_content),
                description: t.help_desc_search_content().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.go_home),
                description: t.help_desc_go_home().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.go_parent),
                description: t.help_desc_parent_dir().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.refresh),
                description: t.help_desc_refresh().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.toggle_hidden),
                description: t.help_desc_toggle_hidden().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.toggle_selection),
                description: t.help_desc_select().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.select_all),
                description: t.help_desc_select_all().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.open_external),
                description: t.help_desc_open_external().to_string(),
            },
        ];

        HelpSection {
            header: t.help_file_manager_keys().to_string(),
            entries,
        }
    }

    /// Generate editor keybindings section.
    fn generate_editor_section(
        kb: &EditorKeybindings,
        t: &dyn termide_i18n::Translation,
    ) -> HelpSection {
        let entries = vec![
            HelpEntry {
                keys: Self::format_keys(&kb.save),
                description: t.help_desc_save().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.save_as),
                description: t.help_desc_save_as().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.reload),
                description: t.help_desc_reload().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.undo),
                description: t.help_desc_undo().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.redo),
                description: t.help_desc_redo().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.copy),
                description: t.help_desc_copy_system().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.cut),
                description: t.help_desc_cut_system().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.paste),
                description: t.help_desc_paste_system().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.select_all),
                description: t.help_desc_select_all().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.duplicate_line),
                description: t.help_desc_duplicate_line().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.toggle_comment),
                description: t.help_desc_toggle_comment().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.search),
                description: t.help_desc_search().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.search_next),
                description: t.help_desc_search_next().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.search_prev),
                description: t.help_desc_search_prev().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.replace),
                description: t.help_desc_replace().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.replace_current),
                description: t.help_desc_replace_current().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.replace_all),
                description: t.help_desc_replace_all().to_string(),
            },
            // LSP
            HelpEntry {
                keys: Self::format_keys(&kb.trigger_completion),
                description: t.help_desc_trigger_completion().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.show_hover),
                description: t.help_desc_show_hover().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.goto_definition),
                description: t.help_desc_goto_definition().to_string(),
            },
            // Word / paragraph navigation (non-configurable)
            HelpEntry {
                keys: "Ctrl+←/→".to_string(),
                description: t.help_desc_word_nav().to_string(),
            },
            HelpEntry {
                keys: "Ctrl+↑/↓".to_string(),
                description: t.help_desc_paragraph_nav().to_string(),
            },
            HelpEntry {
                keys: "Enter".to_string(),
                description: t.help_desc_accept_completion().to_string(),
            },
            HelpEntry {
                keys: "Escape".to_string(),
                description: t.help_desc_cancel_completion().to_string(),
            },
            HelpEntry {
                keys: "↑ / ↓".to_string(),
                description: t.help_desc_navigate_completion().to_string(),
            },
        ];

        HelpSection {
            header: t.help_editor_keys().to_string(),
            entries,
        }
    }

    /// Generate git status keybindings section.
    fn generate_git_status_section(
        kb: &GitStatusKeybindings,
        t: &dyn termide_i18n::Translation,
    ) -> HelpSection {
        let entries = vec![
            HelpEntry {
                keys: Self::format_keys(&kb.stage_file),
                description: t.help_desc_stage_file().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.unstage_file),
                description: t.help_desc_unstage_file().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.refresh),
                description: t.help_desc_refresh().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.next_section),
                description: t.help_desc_next_section().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.prev_section),
                description: t.help_desc_prev_section().to_string(),
            },
        ];

        HelpSection {
            header: t.help_section_git_status().to_string(),
            entries,
        }
    }

    /// Generate terminal keybindings section.
    fn generate_terminal_section(
        kb: &TerminalKeybindings,
        t: &dyn termide_i18n::Translation,
    ) -> HelpSection {
        let entries = vec![
            HelpEntry {
                keys: Self::format_keys(&kb.copy),
                description: t.help_desc_terminal_copy().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.paste),
                description: t.help_desc_terminal_paste().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.scroll_up),
                description: t.help_desc_scroll_up().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.scroll_down),
                description: t.help_desc_scroll_down().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.scroll_top),
                description: t.help_desc_scroll_top().to_string(),
            },
            HelpEntry {
                keys: Self::format_keys(&kb.scroll_bottom),
                description: t.help_desc_scroll_bottom().to_string(),
            },
        ];

        HelpSection {
            header: t.help_terminal_keys().to_string(),
            entries,
        }
    }

    /// Generate navigation section (static keys for all panels).
    fn generate_navigation_section(t: &dyn termide_i18n::Translation) -> HelpSection {
        let entries = vec![
            HelpEntry {
                keys: "↑ / k".to_string(),
                description: t.help_desc_move_up().to_string(),
            },
            HelpEntry {
                keys: "↓ / j".to_string(),
                description: t.help_desc_move_down().to_string(),
            },
            HelpEntry {
                keys: "PgUp".to_string(),
                description: t.help_desc_page_scroll().to_string(),
            },
            HelpEntry {
                keys: "PgDn".to_string(),
                description: t.help_desc_page_scroll().to_string(),
            },
            HelpEntry {
                keys: "Home / g".to_string(),
                description: t.help_desc_home().to_string(),
            },
            HelpEntry {
                keys: "End / G".to_string(),
                description: t.help_desc_end().to_string(),
            },
            HelpEntry {
                keys: "Ctrl+U".to_string(),
                description: t.help_desc_scroll_half_up().to_string(),
            },
            HelpEntry {
                keys: "Ctrl+D".to_string(),
                description: t.help_desc_scroll_half_down().to_string(),
            },
            HelpEntry {
                keys: "Tab".to_string(),
                description: t.help_desc_next_section().to_string(),
            },
            HelpEntry {
                keys: "Shift+Tab".to_string(),
                description: t.help_desc_prev_section().to_string(),
            },
        ];

        HelpSection {
            header: t.help_section_navigation().to_string(),
            entries,
        }
    }

    /// Generate Git Diff section (static keys).
    fn generate_git_diff_section(t: &dyn termide_i18n::Translation) -> HelpSection {
        let entries = vec![
            HelpEntry {
                keys: "Enter / Space".to_string(),
                description: t.help_desc_toggle_collapse().to_string(),
            },
            HelpEntry {
                keys: "e".to_string(),
                description: t.help_desc_open_file_editor().to_string(),
            },
            HelpEntry {
                keys: "Ctrl+R".to_string(),
                description: t.help_desc_refresh().to_string(),
            },
        ];

        HelpSection {
            header: t.help_section_git_diff().to_string(),
            entries,
        }
    }

    /// Generate Git Log section (static keys).
    fn generate_git_log_section(t: &dyn termide_i18n::Translation) -> HelpSection {
        let entries = vec![
            HelpEntry {
                keys: "Enter / d".to_string(),
                description: t.help_desc_view_commit_diff().to_string(),
            },
            HelpEntry {
                keys: "o / Shift+Enter".to_string(),
                description: t.help_desc_open_external().to_string(),
            },
        ];

        HelpSection {
            header: t.help_section_git_log().to_string(),
            entries,
        }
    }

    /// Format sections as ratatui Lines for direct rendering.
    /// Sections use double-line headers spanning full width.
    pub fn format_lines(
        sections: &[HelpSection],
        width: usize,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let header_style = Style::default().fg(theme.accented_fg);
        let key_style = Style::default().fg(theme.fg);
        let desc_style = Style::default().fg(theme.disabled);

        // Version header: ═══════ Termide x.y.z ═══════
        let version = env!("CARGO_PKG_VERSION");
        let version_text = format!("Termide {}", version);
        let version_len = version_text.chars().count();
        let side_padding = width.saturating_sub(version_len + 2) / 2; // +2 for spaces around text
        let left_pad = "═".repeat(side_padding);
        let right_pad = "═".repeat(width.saturating_sub(side_padding + version_len + 2));
        lines.push(Line::styled(
            format!("{} {} {}", left_pad, version_text, right_pad),
            header_style,
        ));
        lines.push(Line::from(""));

        for section in sections {
            if section.entries.is_empty() {
                continue;
            }

            // Calculate key column width based on content
            let max_keys_len = section
                .entries
                .iter()
                .map(|e| e.keys.chars().count())
                .max()
                .unwrap_or(0)
                .max(12);

            // Header: ═══ HEADER ═══════════════════════
            let header_prefix = format!("═══ {} ", section.header);
            let prefix_len = header_prefix.chars().count();
            let padding = width.saturating_sub(prefix_len);
            lines.push(Line::styled(
                format!("{}{}", header_prefix, "═".repeat(padding)),
                header_style,
            ));

            // Empty line after header
            lines.push(Line::from(""));

            // Entry lines:   key              description
            for entry in &section.entries {
                let keys_padded = Self::pad_string(&entry.keys, max_keys_len + 2);
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(keys_padded, key_style),
                    Span::styled(entry.description.clone(), desc_style),
                ]));
            }

            // Empty line between sections
            lines.push(Line::from(""));
        }

        lines
    }

    /// Pad string to specified width (character-aware).
    fn pad_string(s: &str, width: usize) -> String {
        let char_count = s.chars().count();
        if char_count >= width {
            s.to_string()
        } else {
            let padding = width - char_count;
            let mut result = s.to_string();
            for _ in 0..padding {
                result.push(' ');
            }
            result
        }
    }
}
