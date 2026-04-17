//! Script-creation modal result handler.

use anyhow::Result;

use crate::app::App;

impl App {
    pub(in crate::app) fn handle_create_script_result(
        &mut self,
        result: &termide_modal::ScriptCreateResult,
    ) -> Result<()> {
        use termide_modal::ScriptType;

        // Determine base directory
        let base_dir = if result.is_project {
            self.project_root.join(".termide").join("scripts")
        } else {
            termide_config::get_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join("scripts")
        };

        // Create group subdirectory if needed
        let dir = if let Some(ref group) = result.group {
            base_dir.join(group)
        } else {
            base_dir
        };
        std::fs::create_dir_all(&dir)?;

        // Build filename: name[.bg.|.report.].sh/.cmd
        #[cfg(unix)]
        let (template, ext) = ("#!/bin/sh\n\n", ".sh");
        #[cfg(not(unix))]
        let (template, ext) = ("@echo off\r\n\r\n", ".cmd");

        let type_suffix = match result.script_type {
            ScriptType::Terminal => "",
            ScriptType::Background => ".bg",
            ScriptType::Report => ".report",
        };
        let filename = format!("{}{}{}", result.name, type_suffix, ext);
        let path = dir.join(&filename);

        // Write template content
        std::fs::write(&path, template)?;

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }

        // Open in editor
        let _ = self.open_editor_for_file(path);

        Ok(())
    }
}
