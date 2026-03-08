mod checks;
mod config;
mod exitcode;
mod mcpserver;
mod output;
mod state;
mod tui;

use std::process;

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(name = "alertpaca", version, about = "Proactive server health checker")]
struct Cli {
    /// Path to config file
    #[arg(short, long)]
    config: Option<String>,

    /// Run one check and output JSON to stdout
    #[arg(long)]
    json: bool,

    /// Run one check and print a plain-text table to stdout
    #[arg(long)]
    once: bool,

    /// Run as an MCP server on stdio
    #[arg(long)]
    mcp: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Mutual exclusion: at most one output mode.
    let modes = cli.json as u8 + cli.once as u8 + cli.mcp as u8;
    if modes > 1 {
        eprintln!("Error: --json, --once, and --mcp are mutually exclusive");
        process::exit(exitcode::GENERAL_ERROR);
    }

    let config = config::load_config(cli.config.as_deref())?;

    if cli.mcp {
        if let Err(e) = mcpserver::run(&config) {
            eprintln!("Error: {e}");
            process::exit(exitcode::GENERAL_ERROR);
        }
        return Ok(());
    }

    if cli.json || cli.once {
        let results = checks::run_all_checks(&config);
        let code = output::exit_code(&results);

        if cli.json {
            if let Err(e) = output::write_json(&results) {
                eprintln!("Error: {e}");
                process::exit(exitcode::GENERAL_ERROR);
            }
        } else {
            if let Err(e) = output::write_table(&results) {
                eprintln!("Error: {e}");
                process::exit(exitcode::GENERAL_ERROR);
            }
        }

        process::exit(code);
    }

    // Default: interactive TUI.
    let mut terminal = ratatui::init();
    let result = tui::run(&mut terminal, config);
    ratatui::restore();

    result
}
