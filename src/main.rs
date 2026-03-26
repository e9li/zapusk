mod cli;
mod core;
mod platform;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

use core::config::Config;
use tui::app::App;

#[derive(Parser)]
#[command(name = "zapusk", about = "Local dev project manager")]
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
        Err(_) => {
            let path = core::config::config_path();
            eprintln!("No config found at {}\n", path.display());
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
    execute!(stdout, EnterAlternateScreen)?;
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
    loop {
        terminal.draw(|frame| tui::ui::draw(frame, app))?;

        app.tick().await?;

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
