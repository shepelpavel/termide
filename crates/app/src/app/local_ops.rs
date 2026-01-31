//! Local file operation handlers.
//!
//! Contains handlers for local file system operations:
//! - Batch operation scheduling

use crate::state::PendingAction;

use super::App;

impl App {
    /// Check if there's a pending local batch operation that needs to start
    /// (after progress modal has been rendered)
    pub(super) fn check_pending_batch_operation(&mut self) {
        use crate::state::ActiveModal;

        // Don't start new operation if OperationManager already has active operations
        if self.state.has_pending_operations() {
            return;
        }

        // Check if we have a pending batch operation ready to start.
        // This can happen after:
        // 1. Progress modal has been rendered (legacy path)
        // 2. Operations panel batch tracking has been set up (new path)
        let has_progress_modal = matches!(&self.state.active_modal, Some(ActiveModal::Progress(_)));
        let has_batch_tracking = self.state.batch_tracking_id.is_some();

        if has_progress_modal || has_batch_tracking {
            // Don't consume the pending batch if a user-interactive modal is open
            // (e.g. Conflict, RenamePattern). Only proceed when there's no modal
            // or it's a Progress modal (which is non-blocking for batch flow).
            let has_blocking_modal = self.state.active_modal.is_some() && !has_progress_modal;
            if has_blocking_modal {
                return;
            }

            if let Some(PendingAction::ContinueBatchOperation { operation }) =
                self.state.pending_action.take()
            {
                // UI has been rendered, now start the actual batch operation
                self.process_batch_operation(operation);
            }
        }
    }
}
