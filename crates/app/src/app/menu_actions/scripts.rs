//! Scripts menu actions — scripts dropdown, execution, and script utilities.

use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::Mutex;
#[cfg(unix)]
use std::time::{Duration, Instant};

use super::super::App;

impl App {
    // =========================================================================
    // Scripts submenu handling
    // =========================================================================

    /// Handle keyboard event in Scripts submenu
    pub(in crate::app) fn handle_scripts_submenu_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use super::{navigate_submenu, SubmenuNavAction};

        // If nested submenu is open, delegate to nested handler
        if self.state.ui.scripts_nested.open {
            return self.handle_scripts_nested_submenu_key(key);
        }

        let registry = self.scripts_registry();
        let scripts_items = registry
            .as_ref()
            .map(|r| {
                use termide_ui_render::get_scripts_items;
                get_scripts_items(r)
            })
            .unwrap_or_default();
        let item_count = scripts_items.len();
        let separators: Vec<usize> = scripts_items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.is_separator)
            .map(|(idx, _)| idx)
            .collect();

        match navigate_submenu(
            &key,
            &mut self.state.ui.scripts_submenu,
            item_count,
            &separators,
        ) {
            SubmenuNavAction::Close => self.state.close_menu(),
            SubmenuNavAction::Execute => self.execute_scripts_submenu_action()?,
            SubmenuNavAction::Right => {
                let sel = self.state.ui.scripts_submenu.selected;
                let has_sub = registry
                    .as_ref()
                    .map(|r| {
                        use termide_ui_render::get_scripts_items;
                        get_scripts_items(r)
                    })
                    .and_then(|items| items.get(sel).map(|i| i.has_submenu))
                    .unwrap_or(false);
                if has_sub {
                    self.execute_scripts_submenu_action()?;
                } else {
                    self.switch_to_next_menu()?;
                }
            }
            SubmenuNavAction::Left => self.switch_to_prev_menu()?,
            SubmenuNavAction::Rename => self.rename_selected_script()?,
            SubmenuNavAction::Edit => self.edit_selected_script()?,
            SubmenuNavAction::Delete => self.delete_selected_script()?,
            SubmenuNavAction::None => {}
        }
        Ok(())
    }

    /// Execute action for selected Scripts submenu item
    pub(in crate::app) fn execute_scripts_submenu_action(&mut self) -> Result<()> {
        let selected = self.state.ui.scripts_submenu.selected;

        let registry = match self.scripts_registry() {
            Some(r) => r,
            None => return Ok(()),
        };

        // Look up the selected item by index in the rendered menu items
        let items = termide_ui_render::get_scripts_items(&registry);
        let item = match items.get(selected) {
            Some(i) if !i.is_separator => i,
            _ => return Ok(()),
        };

        let key = &item.key;

        // Special keys: "Manage scripts" or "Add script..."
        if key == termide_ui_render::SCRIPT_MANAGE || key == termide_ui_render::SCRIPT_ADD_NEW {
            self.state.close_menu();
            self.handle_add_script()?;
            return Ok(());
        }

        // Match by name — root scripts
        if let Some(script) = registry.root_items.iter().find(|s| s.name == *key) {
            self.state.close_menu();
            self.run_script(script)?;
            return Ok(());
        }

        // Match by name — groups (open nested submenu)
        if registry.groups.iter().any(|g| g.name == *key) {
            self.state.open_scripts_nested_submenu(key.clone());
        }

        Ok(())
    }

    /// Handle keyboard event in Scripts nested submenu (group items)
    fn handle_scripts_nested_submenu_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        use super::{navigate_submenu, SubmenuNavAction};

        let registry = self.scripts_registry();
        let group_name = self.state.ui.current_scripts_group.clone();

        let item_count = registry
            .as_ref()
            .and_then(|r| {
                group_name
                    .as_ref()
                    .and_then(|name| r.groups.iter().find(|g| &g.name == name))
                    .map(|g| g.items.len())
            })
            .unwrap_or(0);

        match navigate_submenu(&key, &mut self.state.ui.scripts_nested, item_count, &[]) {
            SubmenuNavAction::Close | SubmenuNavAction::Left => {
                self.state.close_scripts_nested_submenu();
            }
            SubmenuNavAction::Execute => self.execute_scripts_nested_action()?,
            SubmenuNavAction::Right => self.switch_to_next_menu()?,
            SubmenuNavAction::Rename => self.rename_selected_nested_script()?,
            SubmenuNavAction::Edit => self.edit_selected_nested_script()?,
            SubmenuNavAction::Delete => self.delete_selected_nested_script()?,
            SubmenuNavAction::None => {}
        }
        Ok(())
    }

    /// Execute action for selected item in Scripts nested submenu
    pub(in crate::app) fn execute_scripts_nested_action(&mut self) -> Result<()> {
        let registry = match self.scripts_registry() {
            Some(r) => r,
            None => return Ok(()),
        };

        let group_name = match &self.state.ui.current_scripts_group {
            Some(name) => name.clone(),
            None => return Ok(()),
        };

        let group = match registry.groups.iter().find(|g| g.name == group_name) {
            Some(g) => g,
            None => return Ok(()),
        };

        if let Some(script) = group.items.get(self.state.ui.scripts_nested.selected) {
            self.state.close_menu();
            self.run_script(script)?;
        }

        Ok(())
    }

    /// Open selected script in editor (F4 from scripts submenu)
    fn edit_selected_script(&mut self) -> Result<()> {
        let selected = self.state.ui.scripts_submenu.selected;

        // Index 0: "Add script..." — nothing to edit
        if selected == 0 {
            return Ok(());
        }

        // Index 1: separator, Index 2+: scripts
        if let Some(registry) = self.scripts_registry() {
            let items = termide_ui_render::get_scripts_items(&registry);
            if let Some(item) = items.get(selected) {
                if item.is_separator || item.has_submenu {
                    return Ok(());
                }
                // Find the script by name
                if let Some(script) = registry.find_script_by_name(&item.key) {
                    self.state.close_menu();
                    let _ = self.open_editor_for_file(script.path.clone());
                }
            }
        }
        Ok(())
    }

    /// Open selected nested script in editor (F4 from scripts nested submenu)
    fn edit_selected_nested_script(&mut self) -> Result<()> {
        let registry = match self.scripts_registry() {
            Some(r) => r,
            None => return Ok(()),
        };
        let group_name = match &self.state.ui.current_scripts_group {
            Some(name) => name.clone(),
            None => return Ok(()),
        };
        if let Some(group) = registry.groups.iter().find(|g| g.name == group_name) {
            if let Some(script) = group.items.get(self.state.ui.scripts_nested.selected) {
                self.state.close_menu();
                let _ = self.open_editor_for_file(script.path.clone());
            }
        }
        Ok(())
    }

    /// Open the "Add script" modal form
    fn handle_add_script(&mut self) -> Result<()> {
        let registry = self.scripts_registry();
        let groups: Vec<String> = registry
            .map(|r| r.groups.iter().map(|g| g.name.clone()).collect())
            .unwrap_or_default();

        let t = termide_i18n::t();
        let modal = termide_modal::ScriptCreateModal::new(t.menu_scripts_add(), groups);
        self.state.set_pending_action(
            termide_state::PendingAction::CreateScript,
            crate::state::ActiveModal::ScriptCreate(Box::new(modal)),
        );
        Ok(())
    }

    /// Delete selected script with confirmation
    fn delete_selected_script(&mut self) -> Result<()> {
        let selected = self.state.ui.scripts_submenu.selected;
        if let Some(registry) = self.scripts_registry() {
            let items = termide_ui_render::get_scripts_items(&registry);
            if let Some(item) = items.get(selected) {
                if item.is_separator || item.has_submenu {
                    return Ok(());
                }
                if let Some(script) = registry.find_script_by_name(&item.key) {
                    self.state.close_menu();
                    let t = termide_i18n::t();
                    let message = format!("{} \"{}\"?", t.help_desc_delete_generic(), script.name);
                    let modal = termide_modal::ConfirmModal::new(t.modal_confirm_title(), &message);
                    self.state.set_pending_action(
                        termide_state::PendingAction::DeleteScript {
                            path: script.path.clone(),
                            selected,
                        },
                        crate::state::ActiveModal::Confirm(Box::new(modal)),
                    );
                }
            }
        }
        Ok(())
    }

    /// Delete selected script in nested submenu with confirmation
    fn delete_selected_nested_script(&mut self) -> Result<()> {
        let registry = match self.scripts_registry() {
            Some(r) => r,
            None => return Ok(()),
        };
        let group_name = match &self.state.ui.current_scripts_group {
            Some(name) => name.clone(),
            None => return Ok(()),
        };
        if let Some(group) = registry.groups.iter().find(|g| g.name == group_name) {
            let selected = self.state.ui.scripts_nested.selected;
            if let Some(script) = group.items.get(selected) {
                self.state.close_menu();
                let t = termide_i18n::t();
                let message = format!("{} \"{}\"?", t.help_desc_delete_generic(), script.name);
                let modal = termide_modal::ConfirmModal::new(t.modal_confirm_title(), &message);
                self.state.set_pending_action(
                    termide_state::PendingAction::DeleteScript {
                        path: script.path.clone(),
                        selected,
                    },
                    crate::state::ActiveModal::Confirm(Box::new(modal)),
                );
            }
        }
        Ok(())
    }

    /// Rename selected script (F2) — shows InputModal with current filename
    fn rename_selected_script(&mut self) -> Result<()> {
        let selected = self.state.ui.scripts_submenu.selected;
        if let Some(registry) = self.scripts_registry() {
            let items = termide_ui_render::get_scripts_items(&registry);
            if let Some(item) = items.get(selected) {
                if item.is_separator || item.has_submenu {
                    return Ok(());
                }
                if let Some(script) = registry.find_script_by_name(&item.key) {
                    let filename = script
                        .path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&script.name);
                    self.state.close_menu();
                    let t = termide_i18n::t();
                    let modal = termide_modal::InputModal::with_default(
                        t.help_desc_rename(),
                        t.help_desc_rename(),
                        filename,
                    );
                    self.state.set_pending_action(
                        termide_state::PendingAction::RenameScript {
                            old_path: script.path.clone(),
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

    /// Rename selected script in nested submenu (F2)
    fn rename_selected_nested_script(&mut self) -> Result<()> {
        let registry = match self.scripts_registry() {
            Some(r) => r,
            None => return Ok(()),
        };
        let group_name = match &self.state.ui.current_scripts_group {
            Some(name) => name.clone(),
            None => return Ok(()),
        };
        if let Some(group) = registry.groups.iter().find(|g| g.name == group_name) {
            let selected = self.state.ui.scripts_nested.selected;
            if let Some(script) = group.items.get(selected) {
                let filename = script
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&script.name);
                self.state.close_menu();
                let t = termide_i18n::t();
                let modal = termide_modal::InputModal::with_default(
                    t.help_desc_rename(),
                    t.help_desc_rename(),
                    filename,
                );
                self.state.set_pending_action(
                    termide_state::PendingAction::RenameScript {
                        old_path: script.path.clone(),
                        group: Some(group_name),
                        selected,
                    },
                    crate::state::ActiveModal::Input(Box::new(modal)),
                );
            }
        }
        Ok(())
    }

    /// Run a script
    fn run_script(&mut self, script: &termide_config::scripts::ScriptItem) -> Result<()> {
        use termide_panel_terminal::Terminal;

        let cwd = self.get_focused_panel_cwd();

        if script.is_report {
            // Run in background with output capture, show result in modal
            self.run_report_script(script, &cwd)?;
        } else if script.is_background {
            // Background spawn — tracked in Operations panel
            log::info!("Running background script '{}' in {:?}", script.name, cwd);
            match shell_command(&script.path, &cwd)
                .current_dir(&cwd)
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
                        termide_state::OperationType::ScriptBackground,
                        script.name.clone(),
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
                    self.state.bg_script_handles.push((op_id, rx, pid));
                    // Open operations panel to show progress
                    let _ = self.open_operations_panel();
                }
                Err(e) => {
                    log::error!("Failed to run background script '{}': {}", script.name, e);
                    self.show_error_modal(format!("Failed to run script: {}", e));
                }
            }
        } else {
            // Run in new terminal panel
            log::info!("Running script '{}' in {:?}", script.name, cwd);

            self.close_help_panels();

            let width = self.state.terminal.width;
            let height = self.state.terminal.height;
            let term_height = height.saturating_sub(3);
            let term_width = width.saturating_sub(2);

            let command = shell_quote(&script.path);

            match Terminal::new_with_cwd(term_height, term_width, Some(cwd)) {
                Ok(mut terminal) => {
                    let _ = terminal.send_command(&command);
                    self.add_panel(Box::new(terminal));
                    self.auto_save_session();
                }
                Err(e) => {
                    log::error!(
                        "Failed to create terminal for script '{}': {}",
                        script.name,
                        e
                    );
                    self.show_error_modal(format!("Failed to run script: {}", e));
                }
            }
        }

        Ok(())
    }

    /// Run a report script in background, capturing output for modal display
    fn run_report_script(
        &mut self,
        script: &termide_config::scripts::ScriptItem,
        cwd: &std::path::Path,
    ) -> Result<()> {
        use crate::state::{ScriptOperationHandle, ScriptOperationResult};

        log::info!("Running report script '{}' in {:?}", script.name, cwd);

        let child = shell_command(&script.path, cwd)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        match child {
            Ok(child) => {
                let pid = child.id();
                let script_name = script.name.clone();
                let (tx, rx) = std::sync::mpsc::channel();

                std::thread::spawn(move || {
                    let output = child.wait_with_output();
                    let result = match output {
                        Ok(out) => ScriptOperationResult {
                            script_name: script_name.clone(),
                            success: out.status.success(),
                            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                        },
                        Err(e) => ScriptOperationResult {
                            script_name: script_name.clone(),
                            success: false,
                            stdout: String::new(),
                            stderr: e.to_string(),
                        },
                    };
                    let _ = tx.send(result);
                });

                // Track in Operations panel
                let op_id = self.state.next_synthetic_operation_id();
                self.state.track_operation(
                    op_id,
                    termide_state::OperationType::ScriptReport,
                    script.name.clone(),
                    String::new(),
                    0,
                    0,
                );

                self.state
                    .script_operation_handles
                    .push(ScriptOperationHandle {
                        receiver: rx,
                        script_name: script.name.clone(),
                        operation_id: Some(op_id),
                        pid: Some(pid),
                    });

                // Open operations panel to show progress
                self.open_operations_panel()?;
            }
            Err(e) => {
                log::error!("Failed to run report script '{}': {}", script.name, e);
                self.show_error_modal(format!("Failed to run script: {}", e));
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

    /// Reopen scripts menu after modal (rename/delete).
    /// If `group` is Some, also opens the nested submenu for that group.
    pub(in crate::app) fn reopen_scripts_menu(
        &mut self,
        group: Option<String>,
        fallback_selected: usize,
    ) {
        use termide_ui_render::menu::SCRIPTS_MENU_INDEX;
        self.state.ui.menu_open = true;
        self.state.ui.selected_menu_item = Some(SCRIPTS_MENU_INDEX);
        self.state.open_scripts_submenu();

        if let Some(group_name) = group {
            if let Some(registry) = self.scripts_registry() {
                let items = termide_ui_render::get_scripts_items(&registry);
                let group_idx = items
                    .iter()
                    .position(|i| i.has_submenu && i.key == group_name)
                    .unwrap_or(fallback_selected);
                self.state.ui.scripts_submenu.selected = group_idx;
                self.state.open_scripts_nested_submenu(group_name);
            }
        } else {
            self.state.ui.scripts_submenu.selected = fallback_selected;
        }
    }
}

// =========================================================================
// Script execution utilities (private module-level functions)
// =========================================================================

#[cfg(unix)]
fn shell_quote(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(not(unix))]
fn shell_quote(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// Per-directory environment cache (for direnv integration).
#[cfg(unix)]
type EnvCache = HashMap<PathBuf, (HashMap<String, String>, Instant)>;

#[cfg(unix)]
static DIR_ENV_CACHE: Mutex<Option<EnvCache>> = Mutex::new(None);

#[cfg(unix)]
const ENV_CACHE_TTL: Duration = Duration::from_secs(60);

#[cfg(unix)]
fn has_direnv() -> bool {
    use std::sync::OnceLock;
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        std::process::Command::new("direnv")
            .arg("--version")
            .output()
            .is_ok()
    })
}

/// Get project-specific environment variables via direnv.
#[cfg(unix)]
fn get_project_env(cwd: &std::path::Path) -> Option<HashMap<String, String>> {
    if !has_direnv() {
        return None;
    }

    let mut cache = DIR_ENV_CACHE.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);

    // Check cache
    if let Some((env, ts)) = cache.get(cwd) {
        if ts.elapsed() < ENV_CACHE_TTL {
            return Some(env.clone());
        }
    }

    let output = std::process::Command::new("direnv")
        .args(["export", "bash"])
        .current_dir(cwd)
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || stdout.trim().is_empty() {
        return None;
    }

    let mut env_map = HashMap::new();
    // Parse `export KEY="VALUE"` lines
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("export ") {
            if let Some((key, value)) = rest.split_once('=') {
                let (key, value) = (key.trim(), value.trim());
                let value = value.trim_matches('"');
                env_map.insert(key.to_string(), value.to_string());
            }
        }
    }

    let cache_mut = cache.get_mut(cwd);
    if let Some((_, ts)) = cache_mut {
        *ts = Instant::now();
    } else {
        cache.insert(cwd.to_path_buf(), (env_map.clone(), Instant::now()));
    }

    Some(env_map)
}

/// Build a shell Command for executing a script, with direnv support.
#[cfg(unix)]
fn shell_command(script_path: &std::path::Path, cwd: &std::path::Path) -> std::process::Command {
    if let Some(env) = get_project_env(cwd) {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = std::process::Command::new(&shell);
        cmd.arg("-c");
        cmd.arg(shell_quote(script_path));
        cmd.current_dir(cwd);
        for (k, v) in env {
            cmd.env(&k, &v);
        }
        cmd
    } else {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = std::process::Command::new(&shell);
        cmd.arg("-c");
        cmd.arg(shell_quote(script_path));
        cmd.current_dir(cwd);
        cmd
    }
}

#[cfg(not(unix))]
fn shell_command(script_path: &std::path::Path, cwd: &std::path::Path) -> std::process::Command {
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/C", &script_path.to_string_lossy()]);
    cmd.current_dir(cwd);
    cmd
}
