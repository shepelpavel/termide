//! Layout types for terminal panel arrangement.

/// Layout mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// Single panel mode (width 1-80)
    Single,
    /// Multi-panel mode (width > 100)
    MultiPanel,
}

/// Layout information
#[derive(Debug, Clone)]
pub struct LayoutInfo {
    /// Layout mode
    pub mode: LayoutMode,
    /// Number of main panels
    pub main_panels_count: usize,
}

impl LayoutInfo {
    /// Calculate layout based on terminal width
    pub fn calculate(width: u16) -> Self {
        use termide_config::constants::*;

        if width <= MIN_WIDTH_MULTI_PANEL {
            // Single panel mode for narrow terminals
            Self {
                mode: LayoutMode::Single,
                main_panels_count: 1,
            }
        } else {
            // Multi-panel mode
            let main_panels_count = (width / MIN_MAIN_PANEL_WIDTH).max(1) as usize;
            Self {
                mode: LayoutMode::MultiPanel,
                main_panels_count,
            }
        }
    }

    /// Get recommended layout description
    pub fn recommended_layout_str(&self) -> &'static str {
        match self.mode {
            LayoutMode::Single => "Single panel",
            LayoutMode::MultiPanel => match self.main_panels_count {
                1 => "1 panel",
                2 => "2 panels",
                3 => "3 panels",
                4 => "4 panels",
                5 => "5 panels",
                6 => "6 panels",
                7 => "7 panels",
                8 => "8 panels",
                9 => "9 panels",
                _ => "Many panels",
            },
        }
    }
}
