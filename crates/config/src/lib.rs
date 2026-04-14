//! Configuration management for termide.
//!
//! This crate provides configuration loading, saving, and validation
//! with support for TOML format and XDG directory conventions.

pub mod bookmarks;
pub mod constants;
pub mod keybindings;
pub mod scripts;
mod settings;
mod xdg;

pub use bookmarks::{Bookmark, BookmarkType, BookmarksConfig};
pub use keybindings::{
    cyrillic_to_latin, is_go_end, is_go_home, is_move_down, is_move_up, latin_to_cyrillic,
    parse_keybinding, EditorKeybindings, FileManagerKeybindings, GitDiffKeybindings,
    GitLogKeybindings, GlobalKeybindings, KeyBinding, ParsedKeyBinding, TerminalKeybindings,
};
pub use settings::{
    Config, EditorSettings, FileManagerSettings, GeneralSettings, GitDiffSettings, GitLogSettings,
    GitStatusSettings, IconMode, LegacyConfig, LoggingSettings, LspServerSettings, LspSettings,
    TerminalSettings, VfsSettings,
};
pub use xdg::{get_cache_dir, get_config_dir, get_data_dir};

use anyhow::Result;
use std::path::PathBuf;

/// Default values as constants
pub mod defaults {
    pub const THEME_NAME: &str = "default";
    pub const LANGUAGE: &str = "auto";
    pub const AUTO_STACK_THRESHOLD: u16 = 80;
    pub const MIN_PANEL_WIDTH: u16 = 20;
    pub const SESSION_RETENTION_DAYS: u32 = 30;
    pub const TAB_SIZE: usize = 4;
    pub const SHOW_GIT_DIFF: bool = true;
    pub const WORD_WRAP: bool = true;
    pub const VIM_MODE: bool = false;
    pub const LARGE_FILE_THRESHOLD_MB: u64 = 5;
    pub const CONTENT_SEARCH_MAX_FILE_SIZE_MB: u64 = 1;
    pub const EXTENDED_VIEW_WIDTH: usize = 50;
    pub const MIN_LOG_LEVEL: &str = "info";
    pub const RESOURCE_MONITOR_INTERVAL: u64 = 2000;
    pub const BELL_ON_OPERATION_COMPLETE: bool = true;
    /// Minimum panel width (columns) to enable tree/wide view in list panels
    pub const TREE_VIEW_MIN_WIDTH: u16 = 35;
    // LSP defaults
    pub const LSP_ENABLED: bool = true;
    pub const LSP_AUTO_COMPLETION: bool = true;
    pub const LSP_COMPLETION_DELAY_MS: u64 = 150;
    pub const LSP_HOVER_DELAY_MS: u64 = 1000;
}

impl Config {
    /// Load configuration from file.
    ///
    /// On first run, creates config file with default values.
    /// Auto-completes missing keys with default values.
    /// Supports migration from legacy flat format.
    pub fn load() -> Result<Self> {
        let config_path = Self::config_file_path()?;

        if config_path.exists() {
            let original_content = std::fs::read_to_string(&config_path)?;

            // Try parsing as new structured format first
            let mut config: Self = match toml::from_str(&original_content) {
                Ok(config) => config,
                Err(_) => {
                    // Try parsing as legacy flat format
                    let legacy: LegacyConfig = toml::from_str(&original_content)?;
                    let mut config: Config = legacy.into();
                    // Normalize keybindings before saving
                    config.normalize();
                    config.save()?;
                    return Ok(config);
                }
            };

            // Fill None keybinding values with defaults
            config.normalize();

            // Serialize back to get normalized content
            let normalized_content = toml::to_string_pretty(&config)?;

            // If content changed, save the updated config
            if original_content != normalized_content {
                config.save()?;
            }

            Ok(config)
        } else {
            // First run - create config file with default values
            let mut config = Self::default();
            // Fill all keybindings with defaults
            config.normalize();
            config.save()?;

            // Create themes directory
            Self::ensure_themes_dir()?;

            Ok(config)
        }
    }

    /// Load configuration from a specific file path.
    ///
    /// Unlike `load()`, this does not create the file if it doesn't exist
    /// and does not auto-save normalized content back.
    pub fn load_from(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config: Self = toml::from_str(&content)?;
        config.normalize();
        Ok(config)
    }

    /// Save configuration to file.
    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_file_path()?;

        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        std::fs::write(config_path, content)?;
        Ok(())
    }

    /// Get path to config file.
    pub fn config_file_path() -> Result<PathBuf> {
        Ok(get_config_dir()?.join("config.toml"))
    }

    /// Get path to themes directory.
    pub fn get_themes_dir() -> Result<PathBuf> {
        Ok(get_config_dir()?.join("themes"))
    }

    /// Get path to scripts directory for user scripts.
    pub fn get_scripts_dir() -> Result<PathBuf> {
        Ok(get_data_dir()?.join("scripts"))
    }

    /// Check if path is the config file.
    pub fn is_config_file(path: &std::path::Path) -> bool {
        Self::config_file_path().map(|p| p == path).unwrap_or(false)
    }

    /// Validate config content.
    pub fn validate_content(content: &str) -> Result<Config> {
        toml::from_str(content).map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Ensure themes directory exists.
    fn ensure_themes_dir() -> Result<()> {
        let themes_dir = Self::get_themes_dir()?;
        if !themes_dir.exists() {
            std::fs::create_dir_all(themes_dir)?;
        }
        Ok(())
    }
}
