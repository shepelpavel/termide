//! Command palette modal result handler.

use anyhow::Result;

use super::super::App;

impl App {
    /// Handle result from the command palette modal.
    ///
    /// `value` is a type-erased `usize` — the index into `command_palette_actions`.
    pub(in crate::app) fn handle_command_palette_result(
        &mut self,
        value: Box<dyn std::any::Any>,
    ) -> Result<()> {
        let index = match value.downcast_ref::<usize>() {
            Some(&i) => i,
            None => return Ok(()),
        };

        if let Some(actions) = self.command_palette_actions.take() {
            if let Some(action) = actions.into_iter().nth(index) {
                self.execute_hotkey_action(action)?;
            }
        }

        Ok(())
    }
}
