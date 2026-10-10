//! Entry point. With no subcommand this will open the TUI (Plan 2); for now
//! it prints help. Subcommands are the non-interactive CLI for scripts and
//! agents.
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "illumio-tui",
    version,
    about = "Terminal UI and CLI for the Illumio Core PCE"
)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    println!("illumio-tui: the interactive TUI arrives in Plan 2; run with --help");
}
