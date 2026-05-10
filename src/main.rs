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
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
    },
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

use termide_app::App;
use termide_config::Config;
use termide_core::{init_icon_mode, init_terminal_caps};
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

/// Restore terminal to a usable state (raw mode off, alternate screen off, etc.).
/// Called both on normal exit and from the panic handler.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableFocusChange,
        DisableBracketedPaste,
        SetTitle("")
    );
    let _ = execute!(io::stdout(), crossterm::cursor::Show);
}

fn main() -> Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Install panic handler that restores terminal before printing the panic.
    // Without this, a panic leaves the terminal in raw mode + alternate screen,
    // which looks like a frozen blank screen (especially over SSH).
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    // Detect terminal capabilities first (before loading themes)
    let caps = init_terminal_caps();

    // Enable ANSI-16 color adaptation for limited color terminals (Linux TTY)
    if caps.needs_color_adaptation() {
        set_ansi16_mode(true);
    }

    // Resolve project root early so the layered config loader can pick up
    // a `<project>/.termide/config.toml` overlay.
    let project_root = std::env::current_dir()
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")));

    // Layered load: defaults → global file → project overlay (if any).
    // `--config <PATH>` bypasses layering and treats the file as the whole
    // effective config (historical semantics). The second tuple element is
    // the `defaults + global` snapshot used later as the diff baseline for
    // the per-project override file.
    let (mut config, mut global_baseline) = if let Some(ref path) = cli.config {
        let cfg = Config::load_from(path)?;
        (cfg.clone(), cfg)
    } else {
        Config::load_layered(None, &project_root).unwrap_or_else(|e| {
            eprintln!("Could not load config: {}. Using defaults.", e);
            (Config::default(), Config::default())
        })
    };

    // Apply CLI overrides on top of the layered config. These are runtime-only
    // — they do NOT propagate into the diff-against-baseline saves.
    if let Some(ref level) = cli.log_level {
        config.logging.min_level = level.clone();
        global_baseline.logging.min_level = config.logging.min_level.clone();
    }
    if cli.no_lsp {
        config.lsp.enabled = false;
        global_baseline.lsp.enabled = false;
    }

    // On Linux VT, use norton-commander theme by default (better for 16-color)
    if caps.is_linux_console && config.general.theme == "default" {
        config.general.theme = "norton-commander".to_string();
    }

    // Initialize icon mode based on config + terminal capabilities
    init_icon_mode(config.general.icon_mode);

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

    // Check if terminal supports enhanced keyboard protocol (kitty protocol).
    // This enables proper Alt+Cyrillic handling in modern terminals like Ghostty, Kitty, WezTerm.
    // Skip on SSH: the detection sends escape sequences and waits for a response,
    // which can hang indefinitely if the SSH terminal doesn't reply.
    let keyboard_caps = termide_keyboard::KeyboardCaps::detect();
    let keyboard_enhanced = keyboard_caps.kitty_full;

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
        //
        // REPORT_EVENT_TYPES exposes `KeyEventState::CAPS_LOCK` on every key
        // event, which the hotkey matcher uses to ignore the spurious Shift
        // modifier that Caps Lock attaches to letters. The main loop filters
        // out Release/Repeat so the rest of the app keeps its press-only
        // assumption.
        execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            )
        )?;
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Get terminal size and use it to initialize app with correct dimensions.
    // Guard against 0x0 (can happen on SSH before PTY size negotiation completes).
    let size = terminal.size()?;
    let width = size.width.max(20);
    let height = size.height.max(5);

    // Create application with pre-loaded config (avoids double config loading)
    let mut app = App::new_with_config(config, global_baseline, width, height, keyboard_caps);

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
    if keyboard_enhanced {
        let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    restore_terminal();

    // Print error if there was one
    if let Err(err) = result {
        log::error!("Error: {:?}", err);
    }

    Ok(())
}
