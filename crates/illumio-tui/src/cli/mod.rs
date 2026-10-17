//! clap definitions and dispatch. Each subcommand lives in its own module
//! and gets a `Context`.
pub mod app;
pub mod bulk;
pub mod context;
pub mod error;
pub mod events;
pub mod generate;
pub mod health;
pub mod marked;
pub mod marks;
pub mod objects;
pub mod output;
pub mod policy;
pub mod profile;
pub mod traffic;
pub mod workloads;
pub mod write;

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
    /// CSV output (RFC 4180, full columns) for list and report commands;
    /// the meta line goes to stderr
    #[arg(long, global = true, conflicts_with_all = ["json", "table"])]
    pub csv: bool,
    /// Ignore cached responses
    #[arg(long, global = true)]
    pub no_cache: bool,
    /// Accepted for compatibility; writes are gated by the profile's writes = true
    #[arg(long, global = true, hide = true)]
    pub allow_writes: bool,
    /// TUI colour theme: dark (default) or light (env: ILLUMIO_TUI_THEME)
    #[arg(long, global = true, env = "ILLUMIO_TUI_THEME")]
    pub theme: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Check connectivity and credentials
    Health,
    /// List labels; add, update or delete one
    Labels(objects::LabelsCmd),
    /// List label groups
    LabelGroups,
    /// List IP lists; add, update or delete one
    Iplists(objects::IplistsCmd),
    /// List services; add, update or delete one
    Services(objects::ServicesCmd),
    /// List audit events
    Events(events::EventsArgs),
    /// List workloads; add, update or delete an unmanaged one
    Workloads(workloads::WorkloadsCmd),
    /// One app: workloads and VENs, inbound and outbound peers, policy
    ///
    /// Peers: flows counts flow records; conns and bytes are the lifetime counts of those
    /// records, as in the PCE UI.
    App(app::AppArgs),
    /// Propose the allow rules one app needs, from its traffic (read-only)
    ///
    /// The TUI generator's rules for the app (G there), printed and never written: rows per
    /// peer, the rules at one granularity, and what is already in the draft.
    Gen(generate::GenArgs),
    /// Rulesets, rules, deny rules, pending changes, provisioning
    #[command(subcommand)]
    Policy(policy::PolicyCmd),
    /// Explorer traffic queries
    #[command(subcommand)]
    Traffic(traffic::TrafficCmd),
    /// Manage connection profiles
    #[command(subcommand)]
    Profile(profile::ProfileCmd),
    /// Flow marks in marks.toml: squelch, reviewed, critical … (local file, no request)
    ///
    /// Marks are applied to every traffic result (traffic, app; gen reports them): squelched
    /// flows are left out, the others carry their mark. --no-marks ignores the file.
    #[command(subcommand)]
    Marks(marks::MarksCmd),
}

/// The command's name for `--csv`'s refusal when it prints no table (a
/// write, a single record, a dry run's request body); `None` when it does.
fn csv_refusal(command: &Command) -> Option<&'static str> {
    use policy::{PolicyCmd as P, RulesAction};
    match command {
        Command::Health => Some("health"),
        Command::Labels(c) if c.action.is_some() => Some("labels add/update/delete"),
        Command::Iplists(c) if c.action.is_some() => Some("iplists add/update/delete"),
        Command::Services(c) if c.action.is_some() => Some("services add/update/delete"),
        Command::Workloads(c) if c.action.is_some() => {
            Some("workloads add/update/delete/enforce/relabel")
        }
        Command::Policy(P::Rulesets(c)) if c.action.is_some() => {
            Some("policy rulesets add/update/delete")
        }
        Command::Policy(P::Rules(c)) => match c.action {
            Some(RulesAction::Impact(_)) => Some("policy rules impact"),
            Some(_) => Some("policy rules add/update/delete"),
            None => None,
        },
        Command::Policy(P::DenyRules(_)) => Some("policy deny-rules"),
        Command::Policy(P::Provision(_)) => Some("policy provision"),
        Command::Policy(P::Revert(_)) => Some("policy revert"),
        Command::Traffic(t) if t.filter().dry_run => Some("traffic --dry-run"),
        Command::Profile(profile::ProfileCmd::Use { .. }) => Some("profile use"),
        Command::Marks(marks::MarksCmd::Export(_)) => Some("marks export"),
        _ => None,
    }
}

pub async fn run(global: GlobalArgs, command: Command) -> Result<(), CliError> {
    // Before any request: `--csv` only changes how a table is printed.
    if global.csv
        && let Some(name) = csv_refusal(&command)
    {
        return Err(CliError::Args(format!(
            "--csv: {name} prints no table; use --json or --table"
        )));
    }
    if let Command::Profile(p) = &command {
        return profile::run(&global, p);
    }
    if let Command::Marks(m) = &command {
        return marks::run(&global, m);
    }
    let ctx = Context::new(&global)?;
    match command {
        Command::Health => health::run(&ctx).await,
        Command::Labels(a) => objects::labels_cmd(&ctx, &a).await,
        Command::LabelGroups => objects::label_groups(&ctx).await,
        Command::Iplists(c) => objects::ip_lists_cmd(&ctx, &c).await,
        Command::Services(c) => objects::services_cmd(&ctx, &c).await,
        Command::Events(a) => events::run(&ctx, &a).await,
        Command::Workloads(a) => workloads::workloads_cmd(&ctx, &a).await,
        Command::App(a) => app::run(&ctx, &a).await,
        Command::Gen(a) => generate::run(&ctx, &a).await,
        Command::Policy(p) => policy::run(&ctx, &p).await,
        Command::Traffic(t) => traffic::run(&ctx, &t).await,
        Command::Profile(_) | Command::Marks(_) => unreachable!("handled above"),
    }
}
