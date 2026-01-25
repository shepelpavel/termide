//! Panel extension traits for downcasting.
//!
//! # Deprecation Notice
//!
//! This trait is **deprecated** in favor of the `handle_command()` method on `Panel`.
//!
//! Instead of downcasting to concrete panel types, use `Panel::handle_command()`
//! with appropriate `PanelCommand` variants:
//!
//! ```rust,ignore
//! // Old approach (deprecated):
//! if let Some(editor) = panel.as_editor_mut() {
//!     editor.update_git_diff();
//! }
//!
//! // New approach (preferred):
//! panel.handle_command(PanelCommand::OnGitUpdate { repo_paths: &paths });
//! ```
//!
//! # When PanelExt is still used
//!
//! Some operations intentionally remain using PanelExt because they don't fit
//! the command pattern well:
//!
//! - **Resource extraction**: `take_config_update()`, `dir_size_receiver.take()`
//! - **Complex type-specific methods**: `go_to_line()`, `save_as()`, batch operations
//! - **Modal requests**: `take_modal_request()` (returns concrete types)
//!
//! These will be reviewed for potential migration in future versions.

use std::any::Any;

use termide_core::Panel;
use termide_modal::ActiveModal;
use termide_panel_diagnostics::DiagnosticsPanel;
use termide_panel_editor::Editor;
use termide_panel_file_manager::FileManager;
use termide_panel_git_status::GitStatusPanel;
use termide_panel_misc::JournalPanel;
use termide_panel_terminal::Terminal;
use termide_state::PendingAction;

/// Extension trait for convenient downcasting of Panel trait objects.
///
/// # Deprecated
///
/// This trait is deprecated. Use `Panel::handle_command()` with `PanelCommand` instead.
/// See module documentation for migration examples.
// Allow deprecated use within this module for internal implementation
#[allow(deprecated)]
#[deprecated(
    since = "0.5.0",
    note = "Use Panel::handle_command() with PanelCommand variants instead"
)]
pub trait PanelExt {
    /// Downcast to Editor (immutable)
    fn as_editor(&self) -> Option<&Editor>;
    /// Downcast to Editor (mutable)
    fn as_editor_mut(&mut self) -> Option<&mut Editor>;
    /// Downcast to FileManager (mutable)
    fn as_file_manager_mut(&mut self) -> Option<&mut FileManager>;
    /// Downcast to Terminal (mutable)
    fn as_terminal_mut(&mut self) -> Option<&mut Terminal>;
    /// Downcast to GitStatusPanel (mutable)
    fn as_git_status_mut(&mut self) -> Option<&mut GitStatusPanel>;
    /// Downcast to DiagnosticsPanel (mutable)
    fn as_diagnostics_panel_mut(&mut self) -> Option<&mut DiagnosticsPanel>;
    /// Check if panel is a Journal panel
    fn is_journal(&self) -> bool;
    /// Take modal request from FileManager, Editor, or GitStatusPanel.
    fn take_modal_request(&mut self) -> Option<(PendingAction, ActiveModal)>;

    /// Take pending upload operation from Editor.
    fn take_pending_upload(
        &mut self,
    ) -> Option<(
        termide_vfs::VfsOperation<()>,
        termide_vfs::VfsPath,
        std::path::PathBuf,
    )>;
}

#[allow(deprecated)]
impl PanelExt for dyn Panel {
    fn as_editor(&self) -> Option<&Editor> {
        (self as &dyn Any).downcast_ref::<Editor>()
    }

    fn as_editor_mut(&mut self) -> Option<&mut Editor> {
        (self as &mut dyn Any).downcast_mut::<Editor>()
    }

    fn as_file_manager_mut(&mut self) -> Option<&mut FileManager> {
        (self as &mut dyn Any).downcast_mut::<FileManager>()
    }

    fn as_terminal_mut(&mut self) -> Option<&mut Terminal> {
        (self as &mut dyn Any).downcast_mut::<Terminal>()
    }

    fn as_git_status_mut(&mut self) -> Option<&mut GitStatusPanel> {
        (self as &mut dyn Any).downcast_mut::<GitStatusPanel>()
    }

    fn as_diagnostics_panel_mut(&mut self) -> Option<&mut DiagnosticsPanel> {
        (self as &mut dyn Any).downcast_mut::<DiagnosticsPanel>()
    }

    fn is_journal(&self) -> bool {
        (self as &dyn Any).is::<JournalPanel>()
    }

    fn take_modal_request(&mut self) -> Option<(PendingAction, ActiveModal)> {
        if let Some(fm) = self.as_file_manager_mut() {
            return fm.take_modal_request();
        }
        if let Some(editor) = self.as_editor_mut() {
            return editor.take_modal_request();
        }
        if let Some(git_status) = self.as_git_status_mut() {
            return git_status.take_modal_request();
        }
        None
    }

    fn take_pending_upload(
        &mut self,
    ) -> Option<(
        termide_vfs::VfsOperation<()>,
        termide_vfs::VfsPath,
        std::path::PathBuf,
    )> {
        if let Some(editor) = self.as_editor_mut() {
            return editor.take_pending_upload();
        }
        None
    }
}

#[allow(deprecated)]
impl PanelExt for Box<dyn Panel> {
    fn as_editor(&self) -> Option<&Editor> {
        (**self).as_editor()
    }

    fn as_editor_mut(&mut self) -> Option<&mut Editor> {
        (**self).as_editor_mut()
    }

    fn as_file_manager_mut(&mut self) -> Option<&mut FileManager> {
        (**self).as_file_manager_mut()
    }

    fn as_terminal_mut(&mut self) -> Option<&mut Terminal> {
        (**self).as_terminal_mut()
    }

    fn as_git_status_mut(&mut self) -> Option<&mut GitStatusPanel> {
        (**self).as_git_status_mut()
    }

    fn as_diagnostics_panel_mut(&mut self) -> Option<&mut DiagnosticsPanel> {
        (**self).as_diagnostics_panel_mut()
    }

    fn is_journal(&self) -> bool {
        (**self).is_journal()
    }

    fn take_modal_request(&mut self) -> Option<(PendingAction, ActiveModal)> {
        (**self).take_modal_request()
    }

    fn take_pending_upload(
        &mut self,
    ) -> Option<(
        termide_vfs::VfsOperation<()>,
        termide_vfs::VfsPath,
        std::path::PathBuf,
    )> {
        (**self).take_pending_upload()
    }
}
