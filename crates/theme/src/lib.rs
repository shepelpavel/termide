//! Theme system for termide.
//!
//! Provides color theme management with support for custom TOML themes.

mod colors;
mod loader;

pub use colors::Theme;
pub use loader::load_theme;

use ratatui::style::Color;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

// Embed theme files at compile time
const THEME_ATOM_ONE_LIGHT_TOML: &str = include_str!("../themes/atom-one-light.toml");
const THEME_AYU_LIGHT_TOML: &str = include_str!("../themes/ayu-light.toml");
const THEME_DOS_NAVIGATOR_TOML: &str = include_str!("../themes/dos-navigator.toml");
const THEME_DRACULA_TOML: &str = include_str!("../themes/dracula.toml");
const THEME_FAR_MANAGER_TOML: &str = include_str!("../themes/far-manager.toml");
const THEME_GITHUB_LIGHT_TOML: &str = include_str!("../themes/github-light.toml");
const THEME_MACOS_DARK_TOML: &str = include_str!("../themes/macos-dark.toml");
const THEME_MACOS_LIGHT_TOML: &str = include_str!("../themes/macos-light.toml");
const THEME_MATERIAL_LIGHTER_TOML: &str = include_str!("../themes/material-lighter.toml");
const THEME_MIDNIGHT_TOML: &str = include_str!("../themes/midnight.toml");
const THEME_MONOKAI_TOML: &str = include_str!("../themes/monokai.toml");
const THEME_NORD_TOML: &str = include_str!("../themes/nord.toml");
const THEME_NORTON_COMMANDER_TOML: &str = include_str!("../themes/norton-commander.toml");
const THEME_ONEDARK_TOML: &str = include_str!("../themes/onedark.toml");
const THEME_SOLARIZED_DARK_TOML: &str = include_str!("../themes/solarized-dark.toml");
const THEME_SOLARIZED_LIGHT_TOML: &str = include_str!("../themes/solarized-light.toml");
const THEME_VOLKOV_COMMANDER_TOML: &str = include_str!("../themes/volkov-commander.toml");
const THEME_WINDOWS_95_TOML: &str = include_str!("../themes/windows-95.toml");
const THEME_WINDOWS_98_TOML: &str = include_str!("../themes/windows-98.toml");
const THEME_WINDOWS_XP_TOML: &str = include_str!("../themes/windows-xp.toml");

// Static theme instances
static THEME_ATOM_ONE_LIGHT: OnceLock<Theme> = OnceLock::new();
static THEME_AYU_LIGHT: OnceLock<Theme> = OnceLock::new();
static THEME_DOS_NAVIGATOR: OnceLock<Theme> = OnceLock::new();
static THEME_DRACULA: OnceLock<Theme> = OnceLock::new();
static THEME_FAR_MANAGER: OnceLock<Theme> = OnceLock::new();
static THEME_GITHUB_LIGHT: OnceLock<Theme> = OnceLock::new();
static THEME_MACOS_DARK: OnceLock<Theme> = OnceLock::new();
static THEME_MACOS_LIGHT: OnceLock<Theme> = OnceLock::new();
static THEME_MATERIAL_LIGHTER: OnceLock<Theme> = OnceLock::new();
static THEME_MIDNIGHT: OnceLock<Theme> = OnceLock::new();
static THEME_MONOKAI: OnceLock<Theme> = OnceLock::new();
static THEME_NORD: OnceLock<Theme> = OnceLock::new();
static THEME_NORTON_COMMANDER: OnceLock<Theme> = OnceLock::new();
static THEME_ONEDARK: OnceLock<Theme> = OnceLock::new();
static THEME_SOLARIZED_DARK: OnceLock<Theme> = OnceLock::new();
static THEME_SOLARIZED_LIGHT: OnceLock<Theme> = OnceLock::new();
static THEME_VOLKOV_COMMANDER: OnceLock<Theme> = OnceLock::new();
static THEME_WINDOWS_95: OnceLock<Theme> = OnceLock::new();
static THEME_WINDOWS_98: OnceLock<Theme> = OnceLock::new();
static THEME_WINDOWS_XP: OnceLock<Theme> = OnceLock::new();

/// Entry in the built-in themes registry.
/// Single source of truth for all built-in theme configurations.
struct BuiltinThemeEntry {
    name: &'static str,
    content: &'static str,
    storage: &'static OnceLock<Theme>,
}

/// Registry of all built-in themes.
/// Adding a new theme requires only adding an entry here
/// (plus the TOML file and static OnceLock above).
static BUILTIN_THEMES: &[BuiltinThemeEntry] = &[
    BuiltinThemeEntry {
        name: "atom-one-light",
        content: THEME_ATOM_ONE_LIGHT_TOML,
        storage: &THEME_ATOM_ONE_LIGHT,
    },
    BuiltinThemeEntry {
        name: "ayu-light",
        content: THEME_AYU_LIGHT_TOML,
        storage: &THEME_AYU_LIGHT,
    },
    BuiltinThemeEntry {
        name: "dos-navigator",
        content: THEME_DOS_NAVIGATOR_TOML,
        storage: &THEME_DOS_NAVIGATOR,
    },
    BuiltinThemeEntry {
        name: "dracula",
        content: THEME_DRACULA_TOML,
        storage: &THEME_DRACULA,
    },
    BuiltinThemeEntry {
        name: "far-manager",
        content: THEME_FAR_MANAGER_TOML,
        storage: &THEME_FAR_MANAGER,
    },
    BuiltinThemeEntry {
        name: "github-light",
        content: THEME_GITHUB_LIGHT_TOML,
        storage: &THEME_GITHUB_LIGHT,
    },
    BuiltinThemeEntry {
        name: "macos-dark",
        content: THEME_MACOS_DARK_TOML,
        storage: &THEME_MACOS_DARK,
    },
    BuiltinThemeEntry {
        name: "macos-light",
        content: THEME_MACOS_LIGHT_TOML,
        storage: &THEME_MACOS_LIGHT,
    },
    BuiltinThemeEntry {
        name: "material-lighter",
        content: THEME_MATERIAL_LIGHTER_TOML,
        storage: &THEME_MATERIAL_LIGHTER,
    },
    BuiltinThemeEntry {
        name: "midnight",
        content: THEME_MIDNIGHT_TOML,
        storage: &THEME_MIDNIGHT,
    },
    BuiltinThemeEntry {
        name: "monokai",
        content: THEME_MONOKAI_TOML,
        storage: &THEME_MONOKAI,
    },
    BuiltinThemeEntry {
        name: "nord",
        content: THEME_NORD_TOML,
        storage: &THEME_NORD,
    },
    BuiltinThemeEntry {
        name: "norton-commander",
        content: THEME_NORTON_COMMANDER_TOML,
        storage: &THEME_NORTON_COMMANDER,
    },
    BuiltinThemeEntry {
        name: "onedark",
        content: THEME_ONEDARK_TOML,
        storage: &THEME_ONEDARK,
    },
    BuiltinThemeEntry {
        name: "solarized-dark",
        content: THEME_SOLARIZED_DARK_TOML,
        storage: &THEME_SOLARIZED_DARK,
    },
    BuiltinThemeEntry {
        name: "solarized-light",
        content: THEME_SOLARIZED_LIGHT_TOML,
        storage: &THEME_SOLARIZED_LIGHT,
    },
    BuiltinThemeEntry {
        name: "volkov-commander",
        content: THEME_VOLKOV_COMMANDER_TOML,
        storage: &THEME_VOLKOV_COMMANDER,
    },
    BuiltinThemeEntry {
        name: "windows-95",
        content: THEME_WINDOWS_95_TOML,
        storage: &THEME_WINDOWS_95,
    },
    BuiltinThemeEntry {
        name: "windows-98",
        content: THEME_WINDOWS_98_TOML,
        storage: &THEME_WINDOWS_98,
    },
    BuiltinThemeEntry {
        name: "windows-xp",
        content: THEME_WINDOWS_XP_TOML,
        storage: &THEME_WINDOWS_XP,
    },
];

// Cache for user-loaded themes
static USER_THEMES: OnceLock<Mutex<HashMap<String, &'static Theme>>> = OnceLock::new();

// Themes directory path (set by app on startup)
static THEMES_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Set the themes directory path (call this at app startup).
pub fn set_themes_dir(path: PathBuf) {
    let _ = THEMES_DIR.set(path);
}

/// Get themes directory path.
fn get_themes_dir() -> Option<&'static PathBuf> {
    THEMES_DIR.get()
}

/// Hardcoded fallback theme in case of parse errors.
fn get_hardcoded_fallback_theme(name: &'static str) -> Theme {
    Theme {
        name,
        bg: Color::Black,
        fg: Color::White,
        accented_bg: Color::DarkGray,
        accented_fg: Color::Cyan,
        selected_bg: Color::Blue,
        selected_fg: Color::White,
        disabled: Color::Gray,
        success: Color::Green,
        warning: Color::Yellow,
        error: Color::Red,
        is_light: None,
    }
}

/// Load theme from embedded TOML content.
fn load_theme_from_toml(content: &str, name: &'static str) -> Theme {
    match loader::load_theme_from_str(content, name) {
        Ok(theme) => theme,
        Err(e) => {
            eprintln!(
                "Failed to parse built-in theme '{}': {}. Using fallback theme.",
                name, e
            );
            get_hardcoded_fallback_theme(name)
        }
    }
}

/// Get a built-in theme by name.
fn get_builtin_theme(name: &str) -> Option<&'static Theme> {
    BUILTIN_THEMES
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| {
            entry
                .storage
                .get_or_init(|| load_theme_from_toml(entry.content, entry.name))
        })
}

/// Get the default theme (windows-xp).
fn get_default_theme() -> &'static Theme {
    get_builtin_theme("windows-xp").expect("windows-xp theme must exist")
}

/// Try to load user theme from config directory.
fn try_load_user_theme(name: &str) -> Option<&'static Theme> {
    let cache = USER_THEMES.get_or_init(|| Mutex::new(HashMap::new()));

    // Check if theme is already cached
    // Use ok() to gracefully handle poisoned mutex (return None instead of panicking)
    {
        let cache_lock = cache.lock().ok()?;
        if let Some(theme) = cache_lock.get(name) {
            return Some(*theme);
        }
    }

    // Try to load from file
    let themes_dir = get_themes_dir()?;
    let theme_path = themes_dir.join(format!("{}.toml", name));

    if !theme_path.exists() {
        return None;
    }

    let theme = load_theme(&theme_path).ok()?;

    // Leak the theme to get 'static reference
    let static_theme: &'static Theme = Box::leak(Box::new(theme));

    // Cache it (ignore if mutex is poisoned - theme already loaded, just won't be cached)
    if let Ok(mut cache_lock) = cache.lock() {
        cache_lock.insert(name.to_string(), static_theme);
    }

    Some(static_theme)
}

impl Theme {
    /// Get theme by name.
    ///
    /// First tries to load from user's config directory.
    /// If not found, falls back to built-in themes.
    pub fn get_by_name(name: &str) -> &'static Theme {
        // Try to load user theme first
        if let Some(theme) = try_load_user_theme(name) {
            return theme;
        }

        // Fall back to built-in themes
        get_builtin_theme(name).unwrap_or_else(get_default_theme)
    }

    /// Get list of all available built-in themes.
    pub fn all_themes() -> Vec<&'static Theme> {
        BUILTIN_THEMES
            .iter()
            .map(|entry| {
                entry
                    .storage
                    .get_or_init(|| load_theme_from_toml(entry.content, entry.name))
            })
            .collect()
    }

    /// Get list of all theme names (built-in + user themes).
    /// Returns owned Vec since user themes are discovered at runtime.
    pub fn all_theme_names() -> Vec<String> {
        let mut names: Vec<String> = BUILTIN_THEMES.iter().map(|e| e.name.to_string()).collect();

        // Add user themes from config directory
        if let Some(themes_dir) = get_themes_dir() {
            if let Ok(entries) = std::fs::read_dir(themes_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|ext| ext == "toml") {
                        if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                            // Avoid duplicates (user theme would override built-in)
                            if !names.iter().any(|n| n == name) {
                                names.push(name.to_string());
                            }
                        }
                    }
                }
            }
        }

        names.sort();
        names
    }

    /// Get list of built-in theme names only.
    pub fn builtin_theme_names() -> Vec<&'static str> {
        BUILTIN_THEMES.iter().map(|e| e.name).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_loading() {
        let windows_xp = Theme::get_by_name("windows-xp");
        assert_eq!(windows_xp.name, "windows-xp");

        let midnight = Theme::get_by_name("midnight");
        assert_eq!(midnight.name, "midnight");

        // Test fallback for unknown theme (should return windows-xp as default)
        let unknown = Theme::get_by_name("nonexistent");
        assert_eq!(unknown.name, "windows-xp");
    }

    #[test]
    fn test_user_theme_loading() {
        if let Some(themes_dir) = get_themes_dir() {
            let darkgray_path = themes_dir.join("darkgray.toml");
            if darkgray_path.exists() {
                let darkgray = Theme::get_by_name("darkgray");
                assert_eq!(darkgray.name, "darkgray");
            }
        }
    }
}
