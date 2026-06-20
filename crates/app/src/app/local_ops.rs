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
        // Don't start new operation if OperationManager already has active operations
        if self.state.has_pending_operations() {
            return;
        }

        // Proceed once Operations-panel batch tracking has been set up.
        if self.state.batch.tracking_id.is_some() {
            // Don't consume the pending batch while a user-interactive modal is
            // open (e.g. Conflict, RenamePattern).
            if self.state.has_modal() {
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
