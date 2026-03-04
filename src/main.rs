mod ui;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
        LeaveAlternateScreen, SetTitle,
    },
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

use termide_app::App;
use termide_config::Config;
use termide_core::init_terminal_caps;
use termide_git::is_available as check_git_available;
use termide_i18n::init_with_language;
use termide_theme::{set_ansi16_mode, set_themes_dir};

#[derive(Parser)]
#[command(name = "termide", version, about = "Terminal IDE")]
struct Cli {
    /// Override minimum log level (trace, debug, info, warn, error)
    #[arg(long)]
    log_level: Option<String>,

    /// Disable LSP support
    #[arg(long)]
    no_lsp: bool,

    /// Path to config file (default: ~/.config/termide/config.toml)
    #[arg(long, value_name = "PATH")]
    config: Option<std::path::PathBuf>,
}

fn main() -> Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Detect terminal capabilities first (before loading themes)
    let caps = init_terminal_caps();

    // Enable ANSI-16 color adaptation for limited color terminals (Linux TTY)
    if caps.needs_color_adaptation() {
        set_ansi16_mode(true);
    }

    // Load config: from custom path if specified, otherwise default
    let mut config = if let Some(ref path) = cli.config {
        Config::load_from(path)?
    } else {
        Config::load().unwrap_or_default()
    };

    // Apply CLI overrides
    if let Some(ref level) = cli.log_level {
        config.logging.min_level = level.clone();
    }
    if cli.no_lsp {
        config.lsp.enabled = false;
    }

    // Initialize theme system with themes directory from config
    if let Ok(themes_dir) = Config::get_themes_dir() {
        set_themes_dir(themes_dir);
    }

    // Initialize translation system with language from config
    init_with_language(&config.general.language)?;

    // Check for git on the system
    let git_available = check_git_available();

    // Initialize terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();

    // Check if terminal supports enhanced keyboard protocol (kitty protocol)
    // This enables proper Alt+Cyrillic handling in modern terminals like Ghostty, Kitty, WezTerm
    let keyboard_enhanced = supports_keyboard_enhancement().unwrap_or(false);

    let title = format!(
        "Termide: {}",
        termide_core::util::shorten_home_path(
            &std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        )
    );

    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableFocusChange,
        EnableBracketedPaste,
        SetTitle(title)
    )?;

    if keyboard_enhanced {
        // Note: REPORT_ALL_KEYS_AS_ESCAPE_CODES causes modifier keys (Shift, Ctrl, Alt)
        // to generate separate events, which breaks combinations like Shift+Home.
        // We only use DISAMBIGUATE_ESCAPE_CODES and REPORT_ALTERNATE_KEYS.
        execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            )
        )?;
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Get terminal size and use it to initialize app with correct dimensions
    let size = terminal.size()?;

    // Create application with pre-loaded config (avoids double config loading)
    let mut app = App::new_with_config(config, size.width, size.height);

    // Log git availability to journal (not to stderr)
    app.log_git_status(git_available);

    // Try to load session, fallback to default layout on error
    if let Err(_e) = app.load_session() {
        // Session file doesn't exist or is corrupted - use default layout
        app.setup_default_layout();
    }

    // Run application
    let result = app.run(&mut terminal, |frame, state, layout_manager| {
        ui::render_layout_with_accordion(frame, state, layout_manager);
    });

    // Restore terminal
    disable_raw_mode()?;
    if keyboard_enhanced {
        let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableFocusChange,
        DisableBracketedPaste,
        SetTitle("")
    )?;
    terminal.show_cursor()?;

    // Print error if there was one
    if let Err(err) = result {
        eprintln!("Error: {:?}", err);
    }

    Ok(())
}
