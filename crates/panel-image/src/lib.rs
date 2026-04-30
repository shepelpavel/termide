//! Image panel for native graphics rendering.
//!
//! Uses ratatui-image to render images directly to the parent terminal
//! via its graphics protocol (Kitty, Sixel, iTerm2, or halfblocks fallback).

use std::any::Any;
use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::KeyCode;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    prelude::Widget,
    style::{Color, Style},
    widgets::Paragraph,
};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol, StatefulImage};

use termide_core::{
    CommandResult, Config, Panel, PanelCommand, PanelEvent, RenderContext, SessionPanel, Theme,
    WidthPreference,
};

/// Image panel for displaying images using terminal graphics protocols.
pub struct ImagePanel {
    /// Path to the image file
    file_path: PathBuf,
    /// Display title (filename)
    title: String,
    /// Graphics protocol picker (detects best available protocol)
    picker: Option<Picker>,
    /// Stateful image protocol for rendering
    image_state: Option<StatefulProtocol>,
    /// Error message if image loading failed
    error: Option<String>,
}

impl ImagePanel {
    /// Create a new image panel for the given file path.
    pub fn new(path: PathBuf) -> Result<Self> {
        let t = termide_i18n::t();
        let title = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(t.panel_image())
            .to_string();

        let mut panel = Self {
            file_path: path.clone(),
            title,
            picker: None,
            image_state: None,
            error: None,
        };

        // Initialize picker and load image
        panel.load_image(&path);

        Ok(panel)
    }

    /// Check if graphics protocol is available in the current terminal.
    ///
    /// This queries the parent terminal for graphics capabilities.
    /// Returns true if Kitty, Sixel, or iTerm2 protocol is supported.
    pub fn graphics_available() -> bool {
        Picker::from_query_stdio().is_ok()
    }

    /// Update the displayed image to a new path.
    pub fn set_image(&mut self, path: PathBuf) {
        self.file_path = path.clone();
        let t = termide_i18n::t();
        self.title = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(t.panel_image())
            .to_string();
        self.load_image(&path);
    }

    /// Load image from file path.
    fn load_image(&mut self, path: &Path) {
        // Initialize picker if not already done
        if self.picker.is_none() {
            match Picker::from_query_stdio() {
                Ok(picker) => self.picker = Some(picker),
                Err(e) => {
                    let t = termide_i18n::t();
                    self.error = Some(t.image_error_fmt(&e.to_string()));
                    return;
                }
            }
        }

        // Load and decode image
        match image::open(path) {
            Ok(dyn_img) => {
                if let Some(picker) = &mut self.picker {
                    self.image_state = Some(picker.new_resize_protocol(dyn_img));
                    self.error = None;
                }
            }
            Err(e) => {
                let t = termide_i18n::t();
                self.error = Some(t.image_error_fmt(&e.to_string()));
            }
        }
    }
}

impl Panel for ImagePanel {
    fn name(&self) -> &'static str {
        "image"
    }

    fn width_preference(&self) -> WidthPreference {
        WidthPreference::PreferWide
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn prepare_render(&mut self, _theme: &Theme, _config: &std::sync::Arc<Config>) {
        // No preparation needed
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, _ctx: &RenderContext) {
        // Accordion already draws border with title, render directly to area

        // If there's an error, display it
        if let Some(ref error) = self.error {
            let error_text = Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
            error_text.render(area, buf);
            return;
        }

        // Render the image
        if let Some(ref mut state) = self.image_state {
            let image_widget = StatefulImage::default();
            ratatui::prelude::StatefulWidget::render(image_widget, area, buf, state);
        }
    }

    fn handle_key(&mut self, chord: termide_core::KeyChord) -> Vec<PanelEvent> {
        let key = chord.raw;
        match key.code {
            KeyCode::Char('q') => vec![PanelEvent::ClosePanel],
            _ => vec![],
        }
    }

    fn handle_command(&mut self, cmd: PanelCommand<'_>) -> CommandResult {
        match cmd {
            PanelCommand::Reload => {
                self.load_image(&self.file_path.clone());
                CommandResult::NeedsRedraw(true)
            }
            _ => CommandResult::None,
        }
    }

    fn to_session(&self, _session_dir: &Path) -> Option<SessionPanel> {
        Some(SessionPanel::Image {
            path: self.file_path.clone(),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn get_working_directory(&self) -> Option<PathBuf> {
        self.file_path.parent().map(|p| p.to_path_buf())
    }
}
