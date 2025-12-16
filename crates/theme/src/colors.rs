//! Theme color definitions.

use ratatui::style::Color;

/// Application theme with semantic color assignments.
///
/// The theme uses a minimal 10-color palette:
/// - 2 base colors (bg, fg)
/// - 2 accented colors (accented_bg, accented_fg)
/// - 2 selection colors (selected_bg, selected_fg)
/// - 1 disabled color
/// - 3 semantic colors (success, warning, error)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// Theme name for display
    pub name: &'static str,

    // === Base (2 colors) ===
    /// Panel backgrounds
    pub bg: Color,
    /// Main text
    pub fg: Color,

    // === Accented (2 colors) ===
    /// Menu, status bar, cursor line background
    pub accented_bg: Color,
    /// Active borders, first letter in menu, selected file marker
    pub accented_fg: Color,

    // === Selection (2 colors) ===
    /// Selected item background (FM cursor, menu selection)
    pub selected_bg: Color,
    /// Selected item text
    pub selected_fg: Color,

    // === Disabled (1 color) ===
    /// Inactive elements, secondary text, separators
    pub disabled: Color,

    // === Semantic (3 colors) ===
    /// Success, git added, resource indicators <50%
    pub success: Color,
    /// Warning, git modified, resource indicators 50-75%, search highlight
    pub warning: Color,
    /// Error, git deleted, resource indicators >75%
    pub error: Color,

    // === Theme classification ===
    /// Optional override for light/dark classification (auto-detected from bg if None)
    pub is_light: Option<bool>,
}

impl Theme {
    /// Determine if this is a light theme.
    /// Uses explicit is_light field if set, otherwise auto-detects from bg luminance.
    pub fn is_light_theme(&self) -> bool {
        if let Some(is_light) = self.is_light {
            return is_light;
        }
        // Auto-detect using ITU-R BT.601 relative luminance formula
        match self.bg {
            Color::Rgb(r, g, b) => {
                let luminance = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
                luminance > 128.0
            }
            // Named light colors
            Color::White
            | Color::Gray
            | Color::LightRed
            | Color::LightGreen
            | Color::LightYellow
            | Color::LightBlue
            | Color::LightMagenta
            | Color::LightCyan => true,
            // Black, DarkGray, and saturated colors are considered dark
            _ => false,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        *Self::get_by_name("default")
    }
}
