mod app;
mod caddy;
mod config;
mod doctor;
mod init;
mod manager;
mod project;
mod ui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

use app::App;
use config::Config;

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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Doctor) => doctor::run().await,
        Some(Commands::Init) => init::run().await,
        Some(Commands::Add) => {
            eprintln!("zapusk add is not yet implemented");
            Ok(())
        }
        None => run_tui().await,
    }
}

async fn run_tui() -> Result<()> {
    let config = Config::load()?;
    let mut app = App::new(config);
    app.autostart().await;

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

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        app.tick().await?;

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
