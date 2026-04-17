//! Progress-modal pause/cancel/resume handling for batch operations.

use anyhow::Result;

use crate::app::App;
use crate::state::ActiveModal;
use termide_modal::ModalResult;

impl App {
    /// Handle progress modal pause/cancel/resume actions.
    /// Returns `Some(Ok(()))` if handled (caller should return), `None` if not a progress action.
    pub(in crate::app) fn handle_progress_modal_action(
        &mut self,
        result: &ModalResult<Box<dyn std::any::Any>>,
    ) -> Option<Result<()>> {
        if let ModalResult::Confirmed(value) = result {
            if let Some(paused) = value.downcast_ref::<bool>() {
                if *paused {
                    // User toggled pause - update BatchOperation pause state
                    if let Some(termide_state::PendingAction::ContinueBatchOperation {
                        ref mut operation,
                    }) = self.state.pending_action
                    {
                        // Get modal pause state to sync
                        if let Some(ActiveModal::Progress(m)) = &self.state.active_modal {
                            operation.pause_state = if m.is_paused() {
                                termide_state::PauseState::Paused
                            } else {
                                termide_state::PauseState::Running
                            };

                            // If resumed, continue processing
                            if operation.pause_state == termide_state::PauseState::Running {
                                let op =
                                    self.state.pending_action.take().expect(
                                        "pending_action confirmed Some by enclosing if-let",
                                    );
                                if let termide_state::PendingAction::ContinueBatchOperation {
                                    operation: batch_op,
                                } = op
                                {
                                    self.process_batch_operation(batch_op);
                                }
                            }
                        }
                    }
                    return Some(Ok(())); // Don't close modal
                } else {
                    // User cancelled - cancel all running operations via OperationManager
                    self.state.cancel_all_operations();

                    // Close progress modal - poll_operation_manager will handle cleanup
                    self.state.close_modal();
                    return Some(Ok(()));
                }
            }
        }
        None
    }
}
