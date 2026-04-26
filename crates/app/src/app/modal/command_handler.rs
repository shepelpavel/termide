//! Command config modal result handler (create + edit).

use anyhow::Result;

use termide_config::commands::{CommandMetadata, CommandsMetadata};

use crate::app::App;

impl App {
    /// Handle result from CommandConfigModal (both Create and Edit modes).
    pub(in crate::app) fn handle_command_config_result(
        &mut self,
        result: &termide_modal::CommandConfigResult,
    ) -> Result<()> {
        let config_dir = if result.is_project {
            self.project_root.join(".termide")
        } else {
            termide_config::get_data_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        };

        if result.is_edit {
            // Edit mode: update existing entry
            self.update_command_entry(result, &config_dir)?;
        } else {
            // Create mode: insert new entry
            self.create_command_entry(result, &config_dir)?;
        }

        self.state.cache.commands_registry = None;
        self.state.cache.hotkey_table = None;
        Ok(())
    }

    /// Create a new command entry in commands.toml.
    fn create_command_entry(
        &mut self,
        result: &termide_modal::CommandConfigResult,
        config_dir: &std::path::Path,
    ) -> Result<()> {
        let mut metadata = CommandsMetadata::load(config_dir);
        let key = result.name.clone();
        let mut entry = metadata.entries.remove(&key).unwrap_or_default();
        apply_result_to_entry(&mut entry, result);
        metadata.entries.insert(key, entry);
        metadata.save(config_dir)?;
        Ok(())
    }

    /// Update an existing command entry in commands.toml (edit mode).
    fn update_command_entry(
        &mut self,
        result: &termide_modal::CommandConfigResult,
        config_dir: &std::path::Path,
    ) -> Result<()> {
        let mut metadata = CommandsMetadata::load(config_dir);
        // The command name from the modal is the original TOML key
        let old_key = result.name.clone();
        let old_entry = metadata.entries.remove(&old_key);
        let mut entry = old_entry.unwrap_or_default();
        apply_result_to_entry(&mut entry, result);
        metadata.entries.insert(old_key, entry);
        metadata.save(config_dir)?;
        Ok(())
    }
}

/// Apply CommandConfigResult fields to a CommandMetadata entry.
fn apply_result_to_entry(entry: &mut CommandMetadata, result: &termide_modal::CommandConfigResult) {
    if result.display_name.is_some() {
        entry.display_name = result.display_name.clone();
    }
    entry.command = result.command.clone();
    entry.mode = Some(result.mode);
    entry.key = result.hotkey.clone();
    entry.group = result.group.clone();
}
