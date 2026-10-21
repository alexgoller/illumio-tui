//! clap definitions and dispatch. Each subcommand lives in its own module
//! and gets a `Context`.
pub mod app;
pub mod bulk;
pub mod completions;
pub mod context;
pub mod enforce;
pub mod error;
pub mod events;
pub mod generate;
pub mod health;
pub mod marked;
pub mod marks;
pub mod names_cache;
pub mod objects;
pub mod output;
pub mod policy;
pub mod profile;
pub mod readiness;
pub mod traffic;
pub mod traffic_explain;
pub mod workloads;
pub mod write;

use clap::{Args, Parser, Subcommand, ValueHint};
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
    #[arg(long, global = true, value_hint = ValueHint::FilePath)]
    pub config: Option<PathBuf>,
    /// dotenv file with PCE_HOST, PCE_ORG_ID, API_KEY, API_SECRET, ...
    #[arg(long, global = true, value_hint = ValueHint::FilePath)]
    pub env_file: Option<PathBuf>,
    /// PCE hostname (env: PCE_HOST)
    #[arg(long, global = true, value_hint = ValueHint::Hostname)]
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
    /// Per-app enforcement readiness: draft coverage, gaps, pending changes, verdict (read-only)
    ///
    /// Requests: one Explorer query over the window (with --app, narrowed to the apps on either
    /// side), re-evaluated against the draft policy (one PUT update_rules, more status polls,
    /// one download; changes no policy); the workloads, pending changes and draft rulesets (one
    /// GET each, cached); the object lists for names (cached). Unicast flows only. Apps with a
    /// `-` slot (a workload missing a scope label) are listed but cannot be named with --app. Verdict: not_evaluated, needs_rules (flows the draft would block, not accepted by a
    /// squelch or accept-blocked mark), needs_provision (pending changes in rulesets whose scope
    /// covers the app, or whose rules reach its workloads), ready; enforced when every managed workload is in full enforcement.
    Readiness(readiness::ReadinessArgs),
    /// Move one app to enforcement, watch what is blocked since, roll back (change records)
    ///
    /// --to full|selective: readiness for the app first (one Explorer query narrowed to the
    /// app, re-evaluated against the draft: one PUT update_rules; the workloads, pending
    /// changes and draft rulesets, cached; the object lists, cached); refused (exit 6) unless
    /// the verdict is ready or enforced, or --force. Then the bulk move of the app's managed
    /// workloads: one PUT bulk_update per 100 (plus a re-read when the workloads came from the
    /// cache), with --dry-run / --yes and --confirm-count as `workloads enforce`. Writes a
    /// change record to $XDG_STATE_HOME/illumio-tui/enforcements/ (else
    /// ~/.local/state/illumio-tui/enforcements/).
    ///
    /// --watch: one Explorer query (blocked flows only, the app on either side) from the
    /// change time (the app's newest record, or --since) to now. Exit 0 when none, exit 7
    /// (blocked_since_enforcement) when some. Read-only; runs only when asked.
    ///
    /// --rollback: one workloads list read of the app past the cache, then one PUT
    /// bulk_update per (mode, visibility) of the record's `before` (per 100). A workload no
    /// longer as the move set it was changed by someone else: skipped and reported. Writes a
    /// rollback record; no record is ever deleted.
    Enforce(enforce::EnforceArgs),
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
    /// Print the shell completion script: bash, zsh, fish or powershell (local, no request)
    ///
    /// e.g. `illumio-tui completions zsh > ~/.zfunc/_illumio-tui`. Profile names complete
    /// from the config file; labels, apps, hostnames, services, IP lists and label groups from
    /// the profile's names cache, which list commands, the TUI and `completions refresh` fill.
    /// Nothing a completion does contacts the PCE.
    Completions(completions::CompletionsArgs),
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
            Some("policy rulesets add/update/delete/enable/disable")
        }
        Command::Policy(P::Rules(c)) => match &c.action {
            Some(RulesAction::Impact(a)) if a.list.is_none() => Some("policy rules impact"),
            Some(RulesAction::Impact(_)) => None,
            Some(_) => Some("policy rules add/update/delete/enable/disable"),
            None => None,
        },
        Command::Policy(P::DenyRules(_)) => Some("policy deny-rules"),
        Command::Policy(P::Provision(_)) => Some("policy provision"),
        Command::Policy(P::Revert(_)) => Some("policy revert"),
        Command::Traffic(t) if t.filter().is_some_and(|f| f.dry_run) => Some("traffic --dry-run"),
        Command::Profile(profile::ProfileCmd::Use { .. }) => Some("profile use"),
        Command::Marks(marks::MarksCmd::Export(_)) => Some("marks export"),
        Command::Enforce(a) if !a.watch => Some("enforce --to/--rollback"),
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
    if let Command::Completions(c) = &command {
        match &c.action {
            None => return completions::run(c),
            Some(completions::CompletionsAction::Refresh(r)) if r.clear => {
                return names_cache::clear(&global);
            }
            Some(_) => {}
        }
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
        Command::Readiness(a) => readiness::run(&ctx, &a).await,
        Command::Enforce(a) => enforce::run(&ctx, &a).await,
        Command::Policy(p) => policy::run(&ctx, &p).await,
        Command::Traffic(t) => traffic::run(&ctx, &t).await,
        Command::Completions(_) => names_cache::refresh(&ctx).await,
        Command::Profile(_) | Command::Marks(_) => {
            unreachable!("handled above")
        }
    }
}
