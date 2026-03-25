//! Input modal result handling.

// Note: PanelExt is used for FileManager file operations (create file/dir).
#![allow(deprecated)]

use anyhow::Result;
use std::path::PathBuf;

use super::super::App;
use crate::PanelExt;
use termide_i18n as i18n;
use termide_modal::SaveAsResult;

impl App {
    /// Handle file creation
    pub(in crate::app) fn handle_create_file(
        &mut self,
        _directory: PathBuf,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        self.create_entry_in_file_manager(value, false)
    }

    /// Handle directory creation
    pub(in crate::app) fn handle_create_directory(
        &mut self,
        _directory: PathBuf,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        self.create_entry_in_file_manager(value, true)
    }

    /// Shared helper for creating a file or directory in the active FileManager.
    fn create_entry_in_file_manager(
        &mut self,
        value: Box<dyn std::any::Any>,
        is_directory: bool,
    ) -> Result<()> {
        if let Some(name) = value.downcast_ref::<String>() {
            let t = i18n::t();
            let result = if let Some(panel) = self.layout_manager.active_panel_mut() {
                if let Some(fm) = panel.as_file_manager_mut() {
                    let result = if is_directory {
                        fm.create_directory(name.clone())
                    } else {
                        fm.create_file(name.clone())
                    };
                    if result.is_ok() {
                        log::info!(
                            "{} created: {}",
                            if is_directory { "Directory" } else { "File" },
                            name
                        );
                        let _ = fm.load_directory();
                    }
                    Some(result)
                } else {
                    log::error!("FileManager panel could not be accessed");
                    None
                }
            } else {
                log::error!("FileManager not found");
                None
            };

            if let Some(result) = result {
                match result {
                    Ok(_) => {
                        let msg = if is_directory {
                            t.status_dir_created(name)
                        } else {
                            t.status_file_created(name)
                        };
                        self.state.set_info(msg);
                    }
                    Err(e) => {
                        let kind = if is_directory { "directory" } else { "file" };
                        log::error!("{} creation error '{}': {}", kind, name, e);
                        use termide_modal::{ActiveModal, InfoModal};
                        let error_msg = format!("Failed to create {} '{}': {}", kind, name, e);
                        let lines = vec![(String::new(), error_msg)];
                        let modal = InfoModal::new(termide_i18n::t().modal_error_title(), lines);
                        self.state.active_modal = Some(ActiveModal::Info(Box::new(modal)));
                    }
                }
            }
        }
        Ok(())
    }

    /// Handle saving file with new name
    pub(in crate::app) fn handle_save_file_as(
        &mut self,
        directory: PathBuf,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        // Store info needed for LSP notification (before mutable borrow)
        let mut lsp_info: Option<(String, PathBuf)> = None;
        #[cfg(unix)]
        let mut saved_path: Option<PathBuf> = None;
        #[cfg(unix)]
        let mut make_executable = false;

        if let Some(result) = value.downcast_ref::<SaveAsResult>() {
            let t = i18n::t();
            #[cfg(unix)]
            {
                make_executable = result.executable;
            }

            // Get active Editor panel and save file
            if let Some(panel) = self.layout_manager.active_panel_mut() {
                if let Some(editor) = panel.as_editor_mut() {
                    // Resolve path: expand ~ and handle absolute/relative paths
                    let input_path = termide_ui::expand_tilde(&result.path);
                    let file_path = if input_path.is_absolute() {
                        input_path
                    } else {
                        directory.join(&result.path)
                    };
                    let display_path = file_path.display().to_string();

                    match editor.save_file_as(file_path.clone()) {
                        Ok(_) => {
                            log::info!("File saved as: {}", display_path);
                            self.state.set_info(t.status_file_saved(&display_path));
                            #[cfg(unix)]
                            {
                                saved_path = Some(file_path.clone());
                            }

                            // Collect LSP info for didSave notification
                            if let Some(lang) = editor.lsp_language() {
                                lsp_info = Some((lang.to_string(), file_path));
                            }
                        }
                        Err(e) => {
                            log::error!("Save error '{}': {}", display_path, e);
                            self.state.set_error(t.status_error_save(&e.to_string()));
                        }
                    }
                }
            }
        }

        // Set executable permission if requested (Unix only — Windows has no executable bit)
        #[cfg(unix)]
        if make_executable {
            if let Some(ref path) = saved_path {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(metadata) = std::fs::metadata(path) {
                    let mut perms = metadata.permissions();
                    let mode = perms.mode();
                    // Add execute bits for user, group, and others (where read is set)
                    let new_mode = mode | ((mode & 0o444) >> 2);
                    perms.set_mode(new_mode);
                    if let Err(e) = std::fs::set_permissions(path, perms) {
                        log::warn!("Failed to set executable permission: {}", e);
                    } else {
                        log::info!("Set executable permission on: {}", path.display());
                    }
                }
            }
        }

        // Send LSP didSave notification (triggers full analysis for semantic errors)
        if let Some((lang, file_path)) = lsp_info {
            if let Some(ref lsp_manager) = self.state.lsp_manager {
                lsp_manager.did_save(&lang, &file_path, None);
            }
        }

        Ok(())
    }

    /// Handle LSP rename symbol: user confirmed new name, send request to LSP.
    pub(in crate::app) fn handle_lsp_rename_symbol(
        &mut self,
        _file_path: std::path::PathBuf,
        line: usize,
        column: usize,
        value: Box<dyn std::any::Any>,
    ) -> anyhow::Result<()> {
        let new_name = match value.downcast_ref::<String>() {
            Some(s) => s.trim().to_string(),
            None => return Ok(()),
        };
        if new_name.is_empty() {
            return Ok(());
        }

        if let Some(panel) = self.layout_manager.active_panel_mut() {
            if let Some(editor) = panel.as_editor_mut() {
                if let Some(ref lsp_manager) = self.state.lsp_manager {
                    editor.request_rename(line, column, new_name, lsp_manager);
                }
            }
        }
        Ok(())
    }
}
