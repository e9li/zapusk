mod cli;
mod core;
mod i18n;
mod platform;
mod tui;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
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

use core::config::{Config, config_path};
use tui::app::App;

#[derive(Debug, Parser)]
#[command(name = "zapusk", version, about = "Local dev project manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
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
    /// Print a shell completion script to stdout
    Completions {
        /// Target shell
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Start a project (leaves the process running)
    Start {
        /// Project name from config.toml
        name: String,
        /// Do not wait for the domain to answer
        #[arg(long)]
        no_wait: bool,
    },
    /// Stop a running project
    Stop {
        /// Project name from config.toml
        name: String,
    },
    /// Stop then start a project
    Restart {
        /// Project name from config.toml
        name: String,
        /// Do not wait for the domain to answer
        #[arg(long)]
        no_wait: bool,
    },
    /// Show project status
    #[command(visible_alias = "list")]
    Status {
        /// Project name (omit to list all)
        name: Option<String>,
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// Open a project domain in the browser
    Open {
        /// Project name from config.toml
        name: String,
    },
    /// Scaffold a user recipe TOML
    Recipe {
        #[command(subcommand)]
        command: RecipeCommand,
    },
}

#[derive(Debug, Subcommand)]
enum RecipeCommand {
    /// Write a recipe into ~/.config/zapusk/frameworks/
    Init {
        /// rails, laravel, express, or a new id
        id: Option<String>,
        /// Overwrite an existing user file
        #[arg(long)]
        force: bool,
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
        Some(Commands::Completions { shell }) => cli::completions::run(shell, Cli::command()),
        Some(Commands::Start { name, no_wait }) => cli::lifecycle::start(&name, no_wait).await,
        Some(Commands::Stop { name }) => cli::lifecycle::stop(&name).await,
        Some(Commands::Restart { name, no_wait }) => cli::lifecycle::restart(&name, no_wait).await,
        Some(Commands::Status { name, json }) => {
            cli::lifecycle::status(name.as_deref(), json).await
        }
        Some(Commands::Open { name }) => cli::lifecycle::open(&name),
        Some(Commands::Recipe { command }) => match command {
            RecipeCommand::Init { id, force } => cli::recipe::run(id, force),
        },
        None => run_tui().await,
    }
}

async fn run_tui() -> Result<()> {
    let path = config_path();
    if !path.exists() {
        eprintln!("No config at {}.", path.display());
        eprintln!();
        eprintln!("Get started:");
        eprintln!("  zapusk init          Set up dnsmasq + Caddy");
        eprintln!("  zapusk add           Add your first project");
        eprintln!();
        eprintln!("Then run `zapusk` to open the TUI.");
        std::process::exit(1);
    }

    let config = match Config::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Could not load config at {}: {}\n", path.display(), e);
            eprintln!("Fix the TOML, or create one based on config.example.toml.");
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
            _ = housekeeping.tick() => app.housekeeping_tick().await,
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_start_no_wait() {
        let cli = Cli::try_parse_from(["zapusk", "start", "shop", "--no-wait"]).unwrap();
        match cli.command {
            Some(Commands::Start { name, no_wait }) => {
                assert_eq!(name, "shop");
                assert!(no_wait);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_status_alias_list_and_json() {
        let cli = Cli::try_parse_from(["zapusk", "list", "api", "--json"]).unwrap();
        match cli.command {
            Some(Commands::Status { name, json }) => {
                assert_eq!(name.as_deref(), Some("api"));
                assert!(json);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_stop_and_open() {
        let stop = Cli::try_parse_from(["zapusk", "stop", "blog"]).unwrap();
        match stop.command {
            Some(Commands::Stop { name }) => assert_eq!(name, "blog"),
            other => panic!("unexpected {other:?}"),
        }
        let open = Cli::try_parse_from(["zapusk", "open", "blog"]).unwrap();
        match open.command {
            Some(Commands::Open { name }) => assert_eq!(name, "blog"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_recipe_init() {
        let cli = Cli::try_parse_from(["zapusk", "recipe", "init", "rails", "--force"]).unwrap();
        match cli.command {
            Some(Commands::Recipe {
                command: RecipeCommand::Init { id, force },
            }) => {
                assert_eq!(id.as_deref(), Some("rails"));
                assert!(force);
            }
            other => panic!("unexpected {other:?}"),
        }
        let bare = Cli::try_parse_from(["zapusk", "recipe", "init"]).unwrap();
        match bare.command {
            Some(Commands::Recipe {
                command: RecipeCommand::Init { id, force },
            }) => {
                assert!(id.is_none());
                assert!(!force);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
