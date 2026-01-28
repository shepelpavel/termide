//! Session management operations.
//!
//! Contains save/load session functionality for persisting application state.

use anyhow::Result;
use termide_layout::LayoutManager;

use crate::LayoutManagerSession;
use crate::PanelExt;

use super::App;

impl App {
    /// Save current session to file
    pub(super) fn save_session(&mut self) -> Result<()> {
        // Get session directory for this project
        let session_dir = termide_session::Session::get_session_dir(&self.project_root)?;

        // Serialize layout to session (may save temporary buffers)
        let session = self.layout_manager.to_session(&session_dir);

        // Save session to file
        session.save(&self.project_root)?;
        log::info!("Session saved");
        Ok(())
    }

    /// Load session from file and restore layout
    pub fn load_session(&mut self) -> Result<()> {
        // Load session for this project
        let session = termide_session::Session::load(&self.project_root)?;

        // Get session directory for restoring temporary buffers
        let session_dir = termide_session::Session::get_session_dir(&self.project_root)?;

        // Get terminal dimensions for creating Terminal panels
        // Height: subtract menu (1) + status bar (1) + panel border (1) = 3
        let term_height = self.state.terminal.height.saturating_sub(3);
        // Width: full terminal width (vertical layout doesn't reduce width)
        let term_width = self.state.terminal.width;

        // Restore layout from session
        self.layout_manager = LayoutManager::from_session(
            session,
            &session_dir,
            term_height,
            term_width,
            self.state.editor_config(),
        )?;

        // Adapt panel widths to current terminal size
        self.layout_manager
            .redistribute_widths_proportionally(term_width);

        log::info!("Session loaded");

        // Initialize LSP for all restored editors
        if let Some(ref mut lsp_manager) = self.state.lsp_manager {
            for group in &mut self.layout_manager.panel_groups {
                for panel in group.panels_mut() {
                    if let Some(editor) = panel.as_editor_mut() {
                        editor.init_lsp(lsp_manager);
                    }
                }
            }
        }

        // Clean up orphaned buffer files (not referenced in session anymore)
        if let Err(e) = termide_session::cleanup_orphaned_buffers(&session_dir) {
            log::warn!("Failed to cleanup orphaned buffers: {}", e);
        }

        Ok(())
    }

    /// Auto-save session (ignores errors to not disrupt user experience)
    pub fn auto_save_session(&mut self) {
        if let Err(e) = self.save_session() {
            // Log error but don't interrupt user workflow
            log::error!("Failed to auto-save session: {}", e);
        }
    }
}
