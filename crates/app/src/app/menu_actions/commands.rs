//! Commands menu actions — commands dropdown, execution, and command utilities.

use anyhow::Result;
use std::path::PathBuf;
use termide_config::commands::{decode_command_menu_key, CommandMenuKeyKind};
use termide_config::{GlobalKeybindings, KeyBinding};
use termide_modal::ReservedHotkey;

use super::super::App;

fn push_reserved_hotkeys(reserved: &mut Vec<ReservedHotkey>, binding: &Option<KeyBinding>) {
    let Some(binding) = binding else {
        return;
    };
    match binding {
        KeyBinding::Single(key) => reserved.push(ReservedHotkey {
            binding: key.clone(),
        }),
        KeyBinding::Multiple(keys) => {
            for key in keys {
                reserved.push(ReservedHotkey {
                    binding: key.clone(),
                });
            }
        }
    }
}

fn collect_global_reserved_hotkeys(kb: &GlobalKeybindings) -> Vec<ReservedHotkey> {
    let mut reserved = Vec::new();
    push_reserved_hotkeys(&mut reserved, &kb.toggle_menu);
    push_reserved_hotkeys(&mut reserved, &kb.new_file_manager);
    push_reserved_hotkeys(&mut reserved, &kb.new_terminal);
    push_reserved_hotkeys(&mut reserved, &kb.new_editor);
    push_reserved_hotkeys(&mut reserved, &kb.new_journal);
    push_reserved_hotkeys(&mut reserved, &kb.open_help);
    push_reserved_hotkeys(&mut reserved, &kb.open_preferences);
    push_reserved_hotkeys(&mut reserved, &kb.open_sessions);
    push_reserved_hotkeys(&mut reserved, &kb.new_session);
    push_reserved_hotkeys(&mut reserved, &kb.open_git_status);
    push_reserved_hotkeys(&mut reserved, &kb.open_outline);
    push_reserved_hotkeys(&mut reserved, &kb.open_diagnostics);
    push_reserved_hotkeys(&mut reserved, &kb.open_git_log);
    push_reserved_hotkeys(&mut reserved, &kb.open_bookmark_add);
    push_reserved_hotkeys(&mut reserved, &kb.open_command_palette);
    push_reserved_hotkeys(&mut reserved, &kb.prev_group);
    push_reserved_hotkeys(&mut reserved, &kb.next_group);
    push_reserved_hotkeys(&mut reserved, &kb.prev_panel);
    push_reserved_hotkeys(&mut reserved, &kb.next_panel);
    push_reserved_hotkeys(&mut reserved, &kb.goto_panel_1);
    push_reserved_hotkeys(&mut reserved, &kb.goto_panel_2);
    push_reserved_hotkeys(&mut reserved, &kb.goto_panel_3);
    push_reserved_hotkeys(&mut reserved, &kb.goto_panel_4);
    push_reserved_hotkeys(&mut reserved, &kb.goto_panel_5);
    push_reserved_hotkeys(&mut reserved, &kb.goto_panel_6);
    push_reserved_hotkeys(&mut reserved, &kb.goto_panel_7);
    push_reserved_hotkeys(&mut reserved, &kb.goto_panel_8);
    push_reserved_hotkeys(&mut reserved, &kb.goto_panel_9);
    push_reserved_hotkeys(&mut reserved, &kb.close_panel);
    push_reserved_hotkeys(&mut reserved, &kb.toggle_stack);
    push_reserved_hotkeys(&mut reserved, &kb.swap_left);
    push_reserved_hotkeys(&mut reserved, &kb.swap_right);
    push_reserved_hotkeys(&mut reserved, &kb.move_first);
    push_reserved_hotkeys(&mut reserved, &kb.move_last);
    push_reserved_hotkeys(&mut reserved, &kb.resize_smaller);
    push_reserved_hotkeys(&mut reserved, &kb.resize_larger);
    push_reserved_hotkeys(&mut reserved, &kb.panel_action_menu);
    push_reserved_hotkeys(&mut reserved, &kb.quit);
    reserved
}

impl App {
    fn collect_reserved_command_hotkeys(
        &self,
        registry: &termide_config::commands::CommandsRegistry,
        exclude: Option<(&str, bool)>,
    ) -> Vec<ReservedHotkey> {
        let mut reserved = collect_global_reserved_hotkeys(&self.state.config.general.keybindings);
        for (command, key_str) in registry.commands_with_hotkeys() {
            if exclude.is_some_and(|(name, is_project)| {
                command.name == name && command.is_project == is_project
            }) {
                continue;
            }
            reserved.push(ReservedHotkey {
                binding: key_str.to_string(),
            });
        }
        reserved
    }

    // =========================================================================
    // Commands submenu handling
    // =========================================================================

    /// Handle keyboard event in Commands submenu
    pub(in crate::app) fn handle_commands_submenu_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use super::{navigate_submenu, SubmenuNavAction};

        // If nested submenu is open, delegate to nested handler
        if self.state.ui.commands_nested.open {
            return self.handle_commands_nested_submenu_key(key);
        }

        let registry = self.commands_registry();
        let commands_items = registry
            .as_ref()
            .map(|r| {
                use termide_ui_render::get_commands_items;
                get_commands_items(r)
            })
            .unwrap_or_default();
        let item_count = commands_items.len();
        let separators: Vec<usize> = commands_items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.is_separator)
            .map(|(idx, _)| idx)
            .collect();

        match navigate_submenu(
            &key,
            &mut self.state.ui.commands_submenu,
            item_count,
            &separators,
        ) {
            SubmenuNavAction::Close => self.state.close_menu(),
            SubmenuNavAction::Execute => self.execute_commands_submenu_action()?,
            SubmenuNavAction::Right => {
                let sel = self.state.ui.commands_submenu.selected;
                let has_sub = registry
                    .as_ref()
                    .map(|r| {
                        use termide_ui_render::get_commands_items;
                        get_commands_items(r)
                    })
                    .and_then(|items| items.get(sel).map(|i| i.has_submenu))
                    .unwrap_or(false);
                if has_sub {
                    self.execute_commands_submenu_action()?;
                } else {
                    self.switch_to_next_menu()?;
                }
            }
            SubmenuNavAction::Left => self.switch_to_prev_menu()?,
            SubmenuNavAction::Rename => self.rename_selected_command()?,
            SubmenuNavAction::Edit => self.edit_selected_command()?,
            SubmenuNavAction::Delete => self.delete_selected_command()?,
            SubmenuNavAction::None => {}
        }
        Ok(())
    }

    /// Execute action for selected Commands submenu item
    pub(in crate::app) fn execute_commands_submenu_action(&mut self) -> Result<()> {
        let selected = self.state.ui.commands_submenu.selected;

        let registry = match self.commands_registry() {
            Some(r) => r,
            None => return Ok(()),
        };

        // Look up the selected item by index in the rendered menu items
        let items = termide_ui_render::get_commands_items(&registry);
        let item = match items.get(selected) {
            Some(i) if !i.is_separator => i,
            _ => return Ok(()),
        };

        let key = &item.key;

        // Special keys: "Manage commands" or "Add command..."
        if key == termide_ui_render::COMMAND_MANAGE || key == termide_ui_render::COMMAND_ADD_NEW {
            self.state.close_menu();
            self.handle_add_command()?;
            return Ok(());
        }

        let Some(decoded) = decode_command_menu_key(key) else {
            return Ok(());
        };

        match decoded.kind {
            CommandMenuKeyKind::Command => {
                if let Some(command) = registry.find_root_command(&decoded.name, decoded.is_project)
                {
                    self.state.close_menu();
                    self.run_command(command)?;
                }
            }
            CommandMenuKeyKind::Group => {
                if registry
                    .find_group(&decoded.name, decoded.is_project)
                    .is_some()
                {
                    if self.state.ui.commands_nested.open
                        && self.state.ui.current_commands_group.as_deref() == Some(key.as_str())
                    {
                        self.state.close_commands_nested_submenu();
                    } else {
                        self.state.open_commands_nested_submenu(key.clone());
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle keyboard event in Commands nested submenu (group items)
    fn handle_commands_nested_submenu_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use super::{navigate_submenu, SubmenuNavAction};

        let registry = self.commands_registry();
        let group_name = self.state.ui.current_commands_group.clone();

        let item_count = registry
            .as_ref()
            .and_then(|r| {
                group_name
                    .as_ref()
                    .and_then(|name| {
                        let decoded = decode_command_menu_key(name)?;
                        r.find_group(&decoded.name, decoded.is_project)
                    })
                    .map(|g| g.items.len())
            })
            .unwrap_or(0);

        match navigate_submenu(&key, &mut self.state.ui.commands_nested, item_count, &[]) {
            SubmenuNavAction::Close | SubmenuNavAction::Left => {
                self.state.close_commands_nested_submenu();
            }
            SubmenuNavAction::Execute => self.execute_commands_nested_action()?,
            SubmenuNavAction::Right => self.switch_to_next_menu()?,
            SubmenuNavAction::Rename => self.rename_selected_nested_command()?,
            SubmenuNavAction::Edit => self.edit_selected_nested_command()?,
            SubmenuNavAction::Delete => self.delete_selected_nested_command()?,
            SubmenuNavAction::None => {}
        }
        Ok(())
    }

    /// Execute action for selected item in Commands nested submenu
    pub(in crate::app) fn execute_commands_nested_action(&mut self) -> Result<()> {
        let registry = match self.commands_registry() {
            Some(r) => r,
            None => return Ok(()),
        };

        let group_name = match &self.state.ui.current_commands_group {
            Some(name) => name.clone(),
            None => return Ok(()),
        };

        let decoded = match decode_command_menu_key(&group_name) {
            Some(decoded) if decoded.kind == CommandMenuKeyKind::Group => decoded,
            _ => return Ok(()),
        };

        let group = match registry.find_group(&decoded.name, decoded.is_project) {
            Some(g) => g,
            None => return Ok(()),
        };

        if let Some(command) = group.items.get(self.state.ui.commands_nested.selected) {
            self.state.close_menu();
            self.run_command(command)?;
        }

        Ok(())
    }

    /// Open selected command config modal (F4 from commands submenu)
    fn edit_selected_command(&mut self) -> Result<()> {
        let selected = self.state.ui.commands_submenu.selected;

        // Index 0: "Add command..." — nothing to edit
        if selected == 0 {
            return Ok(());
        }

        if let Some(registry) = self.commands_registry() {
            let items = termide_ui_render::get_commands_items(&registry);
            if let Some(item) = items.get(selected) {
                if item.is_separator || item.has_submenu {
                    return Ok(());
                }
                let Some(decoded) = decode_command_menu_key(&item.key) else {
                    return Ok(());
                };
                if decoded.kind != CommandMenuKeyKind::Command {
                    return Ok(());
                }
                if let Some(command) = registry.find_root_command(&decoded.name, decoded.is_project)
                {
                    self.state.close_menu();
                    let reserved = self.collect_reserved_command_hotkeys(
                        &registry,
                        Some((&command.name, command.is_project)),
                    );
                    let groups: Vec<String> =
                        registry.groups.iter().map(|g| g.name.clone()).collect();
                    let title = format!("Edit command: {}", command.name);
                    let modal = termide_modal::CommandConfigModal::new_edit(
                        title,
                        command.name.clone(),
                        command.metadata.as_ref().and_then(|m| m.group.clone()),
                        command.is_project,
                        None,
                        groups,
                        command.metadata.clone(),
                    )
                    .with_reserved_hotkeys(reserved);
                    self.state.set_pending_action(
                        termide_state::PendingAction::EditCommand {
                            command_name: command.name.clone(),
                            is_project: command.is_project,
                            group: None,
                            selected,
                        },
                        crate::state::ActiveModal::CommandConfig(Box::new(modal)),
                    );
                }
            }
        }
        Ok(())
    }

    /// Open selected nested command config modal (F4 from commands nested submenu)
    fn edit_selected_nested_command(&mut self) -> Result<()> {
        let registry = match self.commands_registry() {
            Some(r) => r,
            None => return Ok(()),
        };
        let group_name = match &self.state.ui.current_commands_group {
            Some(name) => name.clone(),
            None => return Ok(()),
        };
        let decoded = match decode_command_menu_key(&group_name) {
            Some(decoded) if decoded.kind == CommandMenuKeyKind::Group => decoded,
            _ => return Ok(()),
        };
        if let Some(group) = registry.find_group(&decoded.name, decoded.is_project) {
            let selected = self.state.ui.commands_nested.selected;
            if let Some(command) = group.items.get(selected) {
                self.state.close_menu();
                let reserved = self.collect_reserved_command_hotkeys(
                    &registry,
                    Some((&command.name, command.is_project)),
                );
                let groups: Vec<String> = registry.groups.iter().map(|g| g.name.clone()).collect();
                let title = format!("Edit command: {}", command.name);
                let modal = termide_modal::CommandConfigModal::new_edit(
                    title,
                    command.name.clone(),
                    command.metadata.as_ref().and_then(|m| m.group.clone()),
                    command.is_project,
                    None,
                    groups,
                    command.metadata.clone(),
                )
                .with_reserved_hotkeys(reserved);
                self.state.set_pending_action(
                    termide_state::PendingAction::EditCommand {
                        command_name: command.name.clone(),
                        is_project: command.is_project,
                        group: Some(group_name),
                        selected,
                    },
                    crate::state::ActiveModal::CommandConfig(Box::new(modal)),
                );
            }
        }
        Ok(())
    }

    /// Open the "Add command" modal form
    fn handle_add_command(&mut self) -> Result<()> {
        let registry = self.commands_registry();
        let groups: Vec<String> = registry
            .as_ref()
            .map(|r| r.groups.iter().map(|g| g.name.clone()).collect())
            .unwrap_or_default();
        let reserved = registry
            .as_ref()
            .map(|r| self.collect_reserved_command_hotkeys(r, None))
            .unwrap_or_else(|| {
                collect_global_reserved_hotkeys(&self.state.config.general.keybindings)
            });

        let t = termide_i18n::t();
        let modal = termide_modal::CommandConfigModal::new_create(t.menu_commands_add(), groups)
            .with_reserved_hotkeys(reserved);
        self.state.set_pending_action(
            termide_state::PendingAction::CreateCommand,
            crate::state::ActiveModal::CommandConfig(Box::new(modal)),
        );
        Ok(())
    }

    /// Delete selected command with confirmation
    fn delete_selected_command(&mut self) -> Result<()> {
        let selected = self.state.ui.commands_submenu.selected;
        if let Some(registry) = self.commands_registry() {
            let items = termide_ui_render::get_commands_items(&registry);
            if let Some(item) = items.get(selected) {
                if item.is_separator || item.has_submenu {
                    return Ok(());
                }
                let Some(decoded) = decode_command_menu_key(&item.key) else {
                    return Ok(());
                };
                if decoded.kind != CommandMenuKeyKind::Command {
                    return Ok(());
                }
                if let Some(command) = registry.find_root_command(&decoded.name, decoded.is_project)
                {
                    self.state.close_menu();
                    let t = termide_i18n::t();
                    let message = format!("{} \"{}\"?", t.help_desc_delete_generic(), command.name);
                    let modal = termide_modal::ConfirmModal::new(t.modal_confirm_title(), &message);
                    self.state.set_pending_action(
                        termide_state::PendingAction::DeleteCommand {
                            command_name: command.name.clone(),
                            is_project: command.is_project,
                            selected,
                        },
                        crate::state::ActiveModal::Confirm(Box::new(modal)),
                    );
                }
            }
        }
        Ok(())
    }

    /// Delete selected command in nested submenu with confirmation
    fn delete_selected_nested_command(&mut self) -> Result<()> {
        let registry = match self.commands_registry() {
            Some(r) => r,
            None => return Ok(()),
        };
        let group_name = match &self.state.ui.current_commands_group {
            Some(name) => name.clone(),
            None => return Ok(()),
        };
        let decoded = match decode_command_menu_key(&group_name) {
            Some(decoded) if decoded.kind == CommandMenuKeyKind::Group => decoded,
            _ => return Ok(()),
        };
        if let Some(group) = registry.find_group(&decoded.name, decoded.is_project) {
            let selected = self.state.ui.commands_nested.selected;
            if let Some(command) = group.items.get(selected) {
                self.state.close_menu();
                let t = termide_i18n::t();
                let message = format!("{} \"{}\"?", t.help_desc_delete_generic(), command.name);
                let modal = termide_modal::ConfirmModal::new(t.modal_confirm_title(), &message);
                self.state.set_pending_action(
                    termide_state::PendingAction::DeleteCommand {
                        command_name: command.name.clone(),
                        is_project: command.is_project,
                        selected,
                    },
                    crate::state::ActiveModal::Confirm(Box::new(modal)),
                );
            }
        }
        Ok(())
    }

    /// Rename selected command (F2) — shows InputModal with current filename
    fn rename_selected_command(&mut self) -> Result<()> {
        let selected = self.state.ui.commands_submenu.selected;
        if let Some(registry) = self.commands_registry() {
            let items = termide_ui_render::get_commands_items(&registry);
            if let Some(item) = items.get(selected) {
                if item.is_separator || item.has_submenu {
                    return Ok(());
                }
                let Some(decoded) = decode_command_menu_key(&item.key) else {
                    return Ok(());
                };
                if decoded.kind != CommandMenuKeyKind::Command {
                    return Ok(());
                }
                if let Some(command) = registry.find_root_command(&decoded.name, decoded.is_project)
                {
                    self.state.close_menu();
                    let t = termide_i18n::t();
                    let modal = termide_modal::InputModal::with_default(
                        t.help_desc_rename(),
                        t.help_desc_rename(),
                        &command.name,
                    );
                    self.state.set_pending_action(
                        termide_state::PendingAction::RenameCommand {
                            command_name: command.name.clone(),
                            is_project: command.is_project,
                            group: None,
                            selected,
                        },
                        crate::state::ActiveModal::Input(Box::new(modal)),
                    );
                }
            }
        }
        Ok(())
    }

    /// Rename selected command in nested submenu (F2)
    fn rename_selected_nested_command(&mut self) -> Result<()> {
        let registry = match self.commands_registry() {
            Some(r) => r,
            None => return Ok(()),
        };
        let group_name = match &self.state.ui.current_commands_group {
            Some(name) => name.clone(),
            None => return Ok(()),
        };
        let decoded = match decode_command_menu_key(&group_name) {
            Some(decoded) if decoded.kind == CommandMenuKeyKind::Group => decoded,
            _ => return Ok(()),
        };
        if let Some(group) = registry.find_group(&decoded.name, decoded.is_project) {
            let selected = self.state.ui.commands_nested.selected;
            if let Some(command) = group.items.get(selected) {
                self.state.close_menu();
                let t = termide_i18n::t();
                let modal = termide_modal::InputModal::with_default(
                    t.help_desc_rename(),
                    t.help_desc_rename(),
                    &command.name,
                );
                self.state.set_pending_action(
                    termide_state::PendingAction::RenameCommand {
                        command_name: command.name.clone(),
                        is_project: command.is_project,
                        group: Some(group_name),
                        selected,
                    },
                    crate::state::ActiveModal::Input(Box::new(modal)),
                );
            }
        }
        Ok(())
    }

    pub(in crate::app) fn run_command_by_menu_key(&mut self, key: &str) -> Result<()> {
        let registry = match self.commands_registry() {
            Some(r) => r,
            None => return Ok(()),
        };

        let Some(decoded) = decode_command_menu_key(key) else {
            return Ok(());
        };
        if decoded.kind != CommandMenuKeyKind::Command {
            return Ok(());
        }

        let command = match registry.find_command_anywhere_scoped(&decoded.name, decoded.is_project)
        {
            Some(command) => command.clone(),
            None => return Ok(()),
        };

        if let Some(ref meta) = command.metadata {
            if !meta.params.is_empty() {
                let modal = termide_modal::CommandParamsModal::new(
                    command.name.clone(),
                    meta.params.clone(),
                );
                self.state.set_pending_action(
                    termide_state::PendingAction::RunCommandWithParams { command },
                    crate::state::ActiveModal::CommandParams(Box::new(modal)),
                );
                return Ok(());
            }
        }

        self.run_command(&command)
    }

    /// Run a command with user-provided parameters (from CommandParamsModal).
    pub(in crate::app) fn run_command_with_params(
        &mut self,
        command: &termide_config::commands::CommandItem,
        params: &std::collections::HashMap<String, String>,
    ) -> Result<()> {
        use termide_config::commands::CommandMode;
        use termide_panel_terminal::Terminal;

        let cwd = self.get_focused_panel_cwd();
        let mut cmd = build_command_command(command, &cwd);

        // Pass parameters as TERMIDE_PARAM_<NAME> env vars
        for (name, value) in params {
            let env_key = format!("TERMIDE_PARAM_{}", name.to_uppercase().replace('-', "_"));
            cmd.env(&env_key, value);
        }

        if command.mode == CommandMode::Report {
            self.run_report_command_with_cmd(command, cmd)?;
        } else if command.mode == CommandMode::Background {
            log::info!(
                "Running background command '{}' with {} params",
                command.name,
                params.len()
            );
            match cmd
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .stdin(std::process::Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    let pid = child.id();
                    let op_id = self.state.next_synthetic_operation_id();
                    self.state.track_operation(
                        op_id,
                        termide_state::OperationType::CommandBackground,
                        command.name.clone(),
                        String::new(),
                        0,
                        0,
                    );
                    let (tx, rx) = std::sync::mpsc::channel::<()>();
                    std::thread::spawn(move || {
                        let _ = child.wait();
                        let _ = tx.send(());
                    });
                    self.state.bg_command_handles.push((op_id, rx, pid));
                    let _ = self.open_operations_panel();
                }
                Err(e) => {
                    log::error!("Failed to run background command '{}': {}", command.name, e);
                    self.show_error_modal(format!("Failed to run command: {}", e));
                }
            }
        } else {
            log::info!(
                "Running command '{}' with {} params",
                command.name,
                params.len()
            );
            self.close_help_panels();
            let width = self.state.terminal.width;
            let height = self.state.terminal.height;
            let term_height = height.saturating_sub(3);
            let term_width = width.saturating_sub(2);
            // Can't pass env to Terminal::new_with_cwd, so just run without params for terminal mode
            let command_str = command_terminal_command(command);
            match Terminal::new_with_cwd(term_height, term_width, Some(cwd)) {
                Ok(mut terminal) => {
                    let _ = terminal.send_command(&command_str);
                    self.add_panel(Box::new(terminal));
                    self.auto_save_session();
                }
                Err(e) => {
                    log::error!(
                        "Failed to create terminal for command '{}': {}",
                        command.name,
                        e
                    );
                    self.show_error_modal(format!("Failed to run command: {}", e));
                }
            }
        }

        Ok(())
    }

    /// Run a command
    fn run_command(&mut self, command: &termide_config::commands::CommandItem) -> Result<()> {
        use termide_config::commands::CommandMode;
        use termide_panel_terminal::Terminal;

        let cwd = self.get_focused_panel_cwd();

        if command.mode == CommandMode::Report {
            // Run in background with output capture, show result in modal
            self.run_report_command(command, &cwd)?;
        } else if command.mode == CommandMode::Background {
            // Background spawn — tracked in Operations panel
            log::info!("Running background command '{}' in {:?}", command.name, cwd);
            match build_command_command(command, &cwd)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .stdin(std::process::Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    let pid = child.id();
                    let op_id = self.state.next_synthetic_operation_id();
                    self.state.track_operation(
                        op_id,
                        termide_state::OperationType::CommandBackground,
                        command.name.clone(),
                        String::new(),
                        0,
                        0,
                    );
                    // Track completion in background thread
                    let (tx, rx) = std::sync::mpsc::channel::<()>();
                    std::thread::spawn(move || {
                        let _ = child.wait();
                        let _ = tx.send(());
                    });
                    // Store handle to poll for completion
                    self.state.bg_command_handles.push((op_id, rx, pid));
                    // Open operations panel to show progress
                    let _ = self.open_operations_panel();
                }
                Err(e) => {
                    log::error!("Failed to run background command '{}': {}", command.name, e);
                    self.show_error_modal(format!("Failed to run command: {}", e));
                }
            }
        } else {
            // Run in new terminal panel
            log::info!("Running command '{}' in {:?}", command.name, cwd);

            self.close_help_panels();

            let width = self.state.terminal.width;
            let height = self.state.terminal.height;
            let term_height = height.saturating_sub(3);
            let term_width = width.saturating_sub(2);

            let command_str = command_terminal_command(command);

            match Terminal::new_with_cwd(term_height, term_width, Some(cwd)) {
                Ok(mut terminal) => {
                    let _ = terminal.send_command(&command_str);
                    self.add_panel(Box::new(terminal));
                    self.auto_save_session();
                }
                Err(e) => {
                    log::error!(
                        "Failed to create terminal for command '{}': {}",
                        command.name,
                        e
                    );
                    self.show_error_modal(format!("Failed to run command: {}", e));
                }
            }
        }

        Ok(())
    }

    /// Run a report command in background, capturing output for modal display
    fn run_report_command(
        &mut self,
        command: &termide_config::commands::CommandItem,
        cwd: &std::path::Path,
    ) -> Result<()> {
        log::info!("Running report command '{}' in {:?}", command.name, cwd);

        let cmd = build_command_command(command, cwd);
        self.run_report_command_with_cmd(command, cmd)
    }

    /// Run a report command with a pre-built Command (e.g. with env vars from params).
    fn run_report_command_with_cmd(
        &mut self,
        command: &termide_config::commands::CommandItem,
        mut cmd: std::process::Command,
    ) -> Result<()> {
        use crate::state::{CommandOperationHandle, CommandOperationResult};

        let child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        match child {
            Ok(child) => {
                let pid = child.id();
                let command_name = command.name.clone();
                let (tx, rx) = std::sync::mpsc::channel();

                std::thread::spawn(move || {
                    let output = child.wait_with_output();
                    let result = match output {
                        Ok(out) => CommandOperationResult {
                            command_name: command_name.clone(),
                            success: out.status.success(),
                            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                        },
                        Err(e) => CommandOperationResult {
                            command_name: command_name.clone(),
                            success: false,
                            stdout: String::new(),
                            stderr: e.to_string(),
                        },
                    };
                    let _ = tx.send(result);
                });

                let op_id = self.state.next_synthetic_operation_id();
                self.state.track_operation(
                    op_id,
                    termide_state::OperationType::CommandReport,
                    command.name.clone(),
                    String::new(),
                    0,
                    0,
                );

                self.state
                    .command_operation_handles
                    .push(CommandOperationHandle {
                        receiver: rx,
                        command_name: command.name.clone(),
                        operation_id: Some(op_id),
                        pid: Some(pid),
                    });

                self.open_operations_panel()?;
            }
            Err(e) => {
                log::error!("Failed to run report command '{}': {}", command.name, e);
                self.show_error_modal(format!("Failed to run command: {}", e));
            }
        }

        Ok(())
    }

    /// Get the working directory from the focused panel
    fn get_focused_panel_cwd(&self) -> PathBuf {
        // Use the Panel::get_working_directory() method
        if let Some(panel) = self.layout_manager.active_panel() {
            if let Some(cwd) = panel.get_working_directory() {
                return cwd;
            }
        }

        // Fallback to project root
        self.project_root.clone()
    }

    /// Reopen commands menu after modal (rename/delete).
    /// If `group` is Some, also opens the nested submenu for that group.
    pub(in crate::app) fn reopen_commands_menu(
        &mut self,
        group: Option<String>,
        fallback_selected: usize,
    ) {
        use termide_ui_render::menu::COMMANDS_MENU_INDEX;
        self.state.ui.menu_open = true;
        self.state.ui.selected_menu_item = Some(COMMANDS_MENU_INDEX);
        self.state.open_commands_submenu();

        if let Some(group_name) = group {
            if let Some(registry) = self.commands_registry() {
                let items = termide_ui_render::get_commands_items(&registry);
                let group_idx = items
                    .iter()
                    .position(|i| i.has_submenu && i.key == group_name)
                    .unwrap_or(fallback_selected);
                self.state.ui.commands_submenu.selected = group_idx;
                self.state.open_commands_nested_submenu(group_name);
            }
        } else {
            self.state.ui.commands_submenu.selected = fallback_selected;
        }
    }
}

// =========================================================================
// Command execution utilities (private module-level functions)
// =========================================================================

/// Get the command string to send to a terminal panel.
fn command_terminal_command(command: &termide_config::commands::CommandItem) -> String {
    command.command.clone().unwrap_or_default()
}

/// Build a Command for executing a command via `sh -c`.
fn build_command_command(
    command: &termide_config::commands::CommandItem,
    cwd: &std::path::Path,
) -> std::process::Command {
    let command_str = match &command.command {
        Some(cmd) => cmd.clone(),
        None => String::new(),
    };

    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(&command_str);
    cmd.current_dir(cwd);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }

    if let Some(env) = get_direnv_json(cwd) {
        for (key, value) in &env {
            match value {
                Some(v) => {
                    cmd.env(key, v);
                }
                None => {
                    cmd.env_remove(key);
                }
            }
        }
    }

    cmd
}

/// Get project environment via `direnv export json`.
///
/// Returns a map of KEY → Some(value) for set vars, KEY → None for unset vars.
/// Uses caching with 60s TTL to avoid repeated subprocess calls.
#[cfg(unix)]
fn get_direnv_json(
    cwd: &std::path::Path,
) -> Option<std::collections::HashMap<String, Option<String>>> {
    use std::sync::Mutex;

    // Check if direnv is available
    static DIRENV_AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let available = *DIRENV_AVAILABLE.get_or_init(|| {
        std::process::Command::new("direnv")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    });
    if !available {
        return None;
    }

    // Cache with TTL
    type Cache = std::collections::HashMap<
        std::path::PathBuf,
        (
            std::collections::HashMap<String, Option<String>>,
            std::time::Instant,
        ),
    >;
    static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
    const TTL: std::time::Duration = std::time::Duration::from_secs(60);

    let mut cache = CACHE.lock().unwrap();
    let cache = cache.get_or_insert_with(std::collections::HashMap::new);

    if let Some((env, ts)) = cache.get(cwd) {
        if ts.elapsed() < TTL {
            return Some(env.clone());
        }
    }

    // Run direnv export json
    let output = std::process::Command::new("direnv")
        .args(["export", "json"])
        .current_dir(cwd)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return None;
    }

    // Parse JSON: { "KEY": "value", "KEY2": null }
    // Minimal JSON parser — no serde dependency needed
    let mut env = std::collections::HashMap::new();
    // Simple line-by-line parse of direnv JSON output
    for line in stdout.lines() {
        let line = line.trim().trim_end_matches(',');
        if line.starts_with('{') || line.starts_with('}') {
            continue;
        }
        // "KEY": "VALUE" or "KEY": null
        if let Some((key_part, val_part)) = line.split_once(':') {
            let key = key_part.trim().trim_matches('"').to_string();
            let val = val_part.trim();
            if val == "null" {
                env.insert(key, None);
            } else {
                // Remove surrounding quotes, handle escaped chars
                let v = val.trim_matches('"');
                // Unescape JSON string basics
                let v = v
                    .replace("\\\"", "\"")
                    .replace("\\\\", "\\")
                    .replace("\\n", "\n")
                    .replace("\\t", "\t");
                env.insert(key, Some(v));
            }
        }
    }

    cache.insert(cwd.to_path_buf(), (env.clone(), std::time::Instant::now()));
    Some(env)
}

#[cfg(not(unix))]
fn get_direnv_json(
    _cwd: &std::path::Path,
) -> Option<std::collections::HashMap<String, Option<String>>> {
    None
}
