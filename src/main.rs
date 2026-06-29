mod cli;
mod core;
mod platform;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{Event, EventStream},
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, SetTitle, disable_raw_mode, enable_raw_mode,
    },
};
use futures_util::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::time::Duration;
use tokio::time::{self, MissedTickBehavior};

use core::config::Config;
use tui::app::App;

#[derive(Parser)]
#[command(name = "zapusk", version, about = "Local dev project manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Check system dependencies
    Doctor,
    /// First-run setup wizard
    Init,
    /// Add a project interactively
    Add,
    /// Remove all zapusk configuration
    Destroy,
    /// Discover listening local services
    Discover {
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Import discovered service by port or pid
        #[arg(long, value_name = "PORT_OR_PID")]
        import: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Doctor) => cli::doctor::run().await,
        Some(Commands::Init) => cli::init::run().await,
        Some(Commands::Add) => cli::add::run().await,
        Some(Commands::Destroy) => cli::destroy::run().await,
        Some(Commands::Discover { json, import }) => cli::discover::run(json, import).await,
        None => run_tui().await,
    }
}

async fn run_tui() -> Result<()> {
    let config = match Config::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            let path = core::config::config_path();
            eprintln!("Could not load config at {}: {}\n", path.display(), e);
            eprintln!("Get started:");
            eprintln!("  zapusk init          Set up dnsmasq + Caddy");
            eprintln!("  zapusk add           Add your first project");
            eprintln!(
                "\nOr create {} manually — see config.example.toml for the format.",
                path.display()
            );
            std::process::exit(1);
        }
    };
    let mut app = App::new(config);
    app.detect_running().await;
    app.autostart().await;
    app.refresh_unmanaged().await;
    app.refresh_service_states().await;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, SetTitle("ZAPUSK"), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Main loop
    let result = run(&mut terminal, &mut app).await;

    // Always restore terminal even on error
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    // Own the channel receivers locally so `tokio::select!` can await them
    // independently of the `&mut app` the event handlers need.
    let mut rx = app.take_receivers();

    // Async terminal-input stream (replaces the old blocking event::poll/read).
    let mut input = EventStream::new();

    // Housekeeping cadence: adopted-process exit polling, background-refresh
    // scheduling, and spinner animation. Delay (not Burst) so a slow handler
    // can't trigger a catch-up flurry of ticks.
    let mut housekeeping = time::interval(Duration::from_millis(100));
    housekeeping.set_missed_tick_behavior(MissedTickBehavior::Delay);

    // Initial paint, before we ever await, so the UI shows immediately.
    terminal.draw(|frame| tui::ui::draw(frame, app))?;

    loop {
        // Park until exactly one wake source fires, then decide if we redraw.
        let mut needs_redraw = tokio::select! {
            maybe_event = input.next() => match maybe_event {
                Some(Ok(Event::Key(key))) => {
                    app.handle_key_event(key).await?;
                    true
                }
                // Resize must redraw: with conditional redraws we no longer
                // repaint every frame, so an idle resize would otherwise leave
                // a stale/garbled frame.
                Some(Ok(Event::Resize(_, _))) => true,
                // Mouse / paste / focus events: not handled, no redraw.
                Some(Ok(_)) => false,
                // Input read error — surface it like the old event::read()?.
                Some(Err(e)) => return Err(e.into()),
                // stdin closed: exit cleanly.
                None => {
                    app.should_quit = true;
                    false
                }
            },
            Some(event) = rx.manager_rx.recv() => {
                app.handle_manager_event(event);
                true
            }
            Some(event) = rx.background_rx.recv() => {
                app.handle_background_event(event);
                true
            }
            _ = housekeeping.tick() => app.housekeeping_tick(),
        };

        // Coalesce: drain anything else already queued before a single draw.
        if app.drain_pending(&mut rx) {
            needs_redraw = true;
        }

        if app.should_quit {
            break;
        }

        if needs_redraw {
            terminal.draw(|frame| tui::ui::draw(frame, app))?;
        }
    }

    Ok(())
}
