//! Entry point. No subcommand: the interactive TUI. Any subcommand: the
//! non-interactive CLI for scripts and agents.
use clap::Parser;
use illumio_tui::cli::{Cli, run};
use illumio_tui::tui::app::run as run_tui;

fn main() {
    let cli = Cli::parse();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = match cli.command {
        None => rt.block_on(run_tui(cli.global)),
        Some(command) => rt.block_on(run(cli.global, command)),
    };
    if let Err(e) = result {
        eprintln!("{}", e.to_json());
        std::process::exit(e.exit_code());
    }
}
