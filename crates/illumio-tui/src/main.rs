//! Entry point. No subcommand: the interactive TUI. Any subcommand: the
//! non-interactive CLI for scripts and agents.
use clap::Parser;
use illumio_tui::cli::{Cli, completions, policy, run};
use illumio_tui::tui::app::run as run_tui;

fn main() {
    // The completion helper: local files only, before clap and the runtime.
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.get(1).is_some_and(|a| a == completions::HELPER) {
        let _ = completions::helper(&args[2..], &mut std::io::stdout().lock());
        return;
    }
    // `policy search -deny`: a negated term is a term, not a flag.
    let cli = Cli::parse_from(policy::negated_terms_last(args));
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
