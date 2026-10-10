//! Entry point. No subcommand: the interactive TUI (Plan 2). Any subcommand:
//! the non-interactive CLI for scripts and agents.
use clap::Parser;
use illumio_tui::cli::{Cli, run};

fn main() {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        println!(
            "illumio-tui: the interactive TUI arrives in Plan 2; run `illumio-tui --help` for CLI commands"
        );
        return;
    };
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    if let Err(e) = rt.block_on(run(cli.global, command)) {
        eprintln!("{}", e.to_json());
        std::process::exit(e.exit_code());
    }
}
