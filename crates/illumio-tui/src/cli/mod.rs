//! clap definitions and dispatch. Each subcommand lives in its own module
//! and gets a `Context`.
pub mod context;
pub mod error;
pub mod health;
pub mod objects;
pub mod output;
pub mod policy;
pub mod profile;
pub mod traffic;
pub mod workloads;

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
    /// PCE hostname (env: PCE_HOST)
    #[arg(long, global = true)]
    pub host: Option<String>,
    /// PCE port override (named --pce-port: `traffic query` has its own --port)
    #[arg(long = "pce-port", global = true)]
    pub pce_port: Option<u16>,
    /// PCE organisation id (env: PCE_ORG_ID)
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
    /// List workloads
    Workloads(workloads::WorkloadsArgs),
    /// Rulesets, rules, pending changes
    #[command(subcommand)]
    Policy(policy::PolicyCmd),
    /// Explorer traffic queries
    #[command(subcommand)]
    Traffic(traffic::TrafficCmd),
    /// Manage connection profiles
    #[command(subcommand)]
    Profile(profile::ProfileCmd),
}

pub async fn run(global: GlobalArgs, command: Command) -> Result<(), CliError> {
    if let Command::Profile(p) = &command {
        return profile::run(&global, p);
    }
    let ctx = Context::new(&global)?;
    match command {
        Command::Health => health::run(&ctx).await,
        Command::Labels(a) => objects::labels(&ctx, &a).await,
        Command::LabelGroups => objects::label_groups(&ctx).await,
        Command::Iplists => objects::ip_lists(&ctx).await,
        Command::Services => objects::services(&ctx).await,
        Command::Workloads(a) => workloads::run(&ctx, &a).await,
        Command::Policy(p) => policy::run(&ctx, &p).await,
        Command::Traffic(t) => traffic::run(&ctx, &t).await,
        Command::Profile(_) => unreachable!("handled above"),
    }
}
