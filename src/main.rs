mod checks;
mod config;
mod state;
mod tui;

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(name = "alertpaca", version, about = "Proactive server health checker")]
struct Cli {
    /// Path to config file
    #[arg(short, long)]
    config: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = config::load_config(cli.config.as_deref())?;

    let mut terminal = ratatui::init();
    let result = tui::run(&mut terminal, config);
    ratatui::restore();

    result
}
