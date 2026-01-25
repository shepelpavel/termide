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
        _panel_index: usize, // obsolete with LayoutManager
        _directory: PathBuf,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        if let Some(name) = value.downcast_ref::<String>() {
            let t = i18n::t();
            // Get active FileManager and create file
            let result = if let Some(panel) = self.layout_manager.active_panel_mut() {
                if let Some(fm) = panel.as_file_manager_mut() {
                    let result = fm.create_file(name.clone());
                    if result.is_ok() {
                        termide_logger::info(format!("File created: {}", name));
                        // Refresh directory contents
                        let _ = fm.load_directory();
                    }
                    Some(result)
                } else {
                    termide_logger::error("FileManager panel could not be accessed".to_string());
                    None
                }
            } else {
                termide_logger::error("FileManager not found".to_string());
                None
            };

            // Update status after FM borrow is dropped
            if let Some(result) = result {
                match result {
                    Ok(_) => {
                        self.state.set_info(t.status_file_created(name));
                    }
                    Err(e) => {
                        termide_logger::error(format!("File creation error '{}': {}", name, e));
                        // Show error in modal instead of status bar
                        use termide_modal::{ActiveModal, InfoModal};
                        let error_msg = format!("Failed to create file '{}': {}", name, e);
                        let lines = vec![(String::new(), error_msg)];
                        let modal = InfoModal::new("Error", lines);
                        self.state.active_modal = Some(ActiveModal::Info(Box::new(modal)));
                    }
                }
            }
        }
        Ok(())
    }

    /// Handle directory creation
    pub(in crate::app) fn handle_create_directory(
        &mut self,
        _panel_index: usize, // obsolete with LayoutManager
        _directory: PathBuf,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        if let Some(name) = value.downcast_ref::<String>() {
            let t = i18n::t();
            // Get active FileManager and create directory
            let result = if let Some(panel) = self.layout_manager.active_panel_mut() {
                if let Some(fm) = panel.as_file_manager_mut() {
                    let result = fm.create_directory(name.clone());
                    if result.is_ok() {
                        termide_logger::info(format!("Directory created: {}", name));
                        // Refresh directory contents
                        let _ = fm.load_directory();
                    }
                    Some(result)
                } else {
                    termide_logger::error("FileManager panel could not be accessed".to_string());
                    None
                }
            } else {
                termide_logger::error("FileManager not found".to_string());
                None
            };

            // Update status after FM borrow is dropped
            if let Some(result) = result {
                match result {
                    Ok(_) => {
                        self.state.set_info(t.status_dir_created(name));
                    }
                    Err(e) => {
                        termide_logger::error(format!(
                            "Directory creation error '{}': {}",
                            name, e
                        ));
                        // Show error in modal instead of status bar
                        use termide_modal::{ActiveModal, InfoModal};
                        let error_msg = format!("Failed to create directory '{}': {}", name, e);
                        let lines = vec![(String::new(), error_msg)];
                        let modal = InfoModal::new("Error", lines);
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
        _panel_index: usize, // obsolete with LayoutManager
        directory: PathBuf,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        // Store info needed for LSP notification (before mutable borrow)
        let mut lsp_info: Option<(String, PathBuf)> = None;
        let mut saved_path: Option<PathBuf> = None;
        let mut make_executable = false;

        if let Some(result) = value.downcast_ref::<SaveAsResult>() {
            let t = i18n::t();
            make_executable = result.executable;

            // Get active Editor panel and save file
            if let Some(panel) = self.layout_manager.active_panel_mut() {
                if let Some(editor) = panel.as_editor_mut() {
                    // Resolve path: absolute paths used as-is, relative joined with directory
                    let input_path = PathBuf::from(&result.path);
                    let file_path = if input_path.is_absolute() {
                        input_path
                    } else {
                        directory.join(&result.path)
                    };
                    let display_path = file_path.display().to_string();

                    match editor.save_file_as(file_path.clone()) {
                        Ok(_) => {
                            termide_logger::info(format!("File saved as: {}", display_path));
                            self.state.set_info(t.status_file_saved(&display_path));
                            saved_path = Some(file_path.clone());

                            // Collect LSP info for didSave notification
                            if let Some(lang) = editor.lsp_language() {
                                lsp_info = Some((lang.to_string(), file_path));
                            }
                        }
                        Err(e) => {
                            termide_logger::error(format!("Save error '{}': {}", display_path, e));
                            self.state.set_error(t.status_error_save(&e.to_string()));
                        }
                    }
                }
            }
        }

        // Set executable permission if requested
        if make_executable {
            if let Some(ref path) = saved_path {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(metadata) = std::fs::metadata(path) {
                        let mut perms = metadata.permissions();
                        let mode = perms.mode();
                        // Add execute bits for user, group, and others (where read is set)
                        let new_mode = mode | ((mode & 0o444) >> 2);
                        perms.set_mode(new_mode);
                        if let Err(e) = std::fs::set_permissions(path, perms) {
                            termide_logger::warn(format!(
                                "Failed to set executable permission: {}",
                                e
                            ));
                        } else {
                            termide_logger::info(format!(
                                "Set executable permission on: {}",
                                path.display()
                            ));
                        }
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
}
