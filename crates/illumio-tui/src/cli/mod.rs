//! clap definitions and dispatch. Each subcommand lives in its own module
//! and gets a `Context`.
pub mod context;
pub mod error;
pub mod health;
pub mod objects;
pub mod output;

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

pub use context::Context;
pub use error::CliError;

#[derive(Parser, Debug)]
#[command(
    name = "illumio-tui",
    version,
    about = "Terminal UI and CLI for the Illumio Core PCE"
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct GlobalArgs {
    /// Profile name from the config file (env: ILLUMIO_PROFILE)
    #[arg(long, global = true, env = "ILLUMIO_PROFILE")]
    pub profile: Option<String>,
    /// Config file (default: ~/.config/illumio-tui/config.toml)
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    /// dotenv file with PCE_HOST, PCE_ORG_ID, API_KEY, API_SECRET, ...
    #[arg(long, global = true)]
    pub env_file: Option<PathBuf>,
    #[arg(long, global = true)]
    pub host: Option<String>,
    #[arg(long, global = true)]
    pub port: Option<u16>,
    #[arg(long, global = true)]
    pub org_id: Option<u64>,
    /// Skip TLS certificate verification
    #[arg(long, global = true)]
    pub insecure: bool,
    /// JSON output (default when stdout is not a terminal)
    #[arg(long, global = true, conflicts_with = "table")]
    pub json: bool,
    /// Table output (default when stdout is a terminal)
    #[arg(long, global = true)]
    pub table: bool,
    /// Ignore cached responses
    #[arg(long, global = true)]
    pub no_cache: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Check connectivity and credentials
    Health,
    /// List labels
    Labels(objects::LabelsArgs),
    /// List label groups
    LabelGroups,
    /// List IP lists
    Iplists,
    /// List services
    Services,
}

pub async fn run(global: GlobalArgs, command: Command) -> Result<(), CliError> {
    let ctx = Context::new(&global)?;
    match command {
        Command::Health => health::run(&ctx).await,
        Command::Labels(a) => objects::labels(&ctx, &a).await,
        Command::LabelGroups => objects::label_groups(&ctx).await,
        Command::Iplists => objects::ip_lists(&ctx).await,
        Command::Services => objects::services(&ctx).await,
    }
}
