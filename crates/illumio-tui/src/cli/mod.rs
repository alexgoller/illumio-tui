//! clap definitions and dispatch. Each subcommand lives in its own module
//! and gets a `Context`.
pub mod app;
pub mod bulk;
pub mod columns;
pub mod completions;
pub mod context;
pub mod enforce;
pub mod error;
pub mod events;
pub mod exposure;
pub mod filters;
pub mod generate;
pub mod health;
pub mod hygiene;
pub mod marked;
pub mod marks;
pub mod names_cache;
pub mod objects;
pub mod output;
pub mod pairing_profiles;
pub mod policy;
pub mod policy_impact;
pub mod profile;
pub mod readiness;
pub mod schema;
pub mod snapshot;
pub mod traffic;
pub mod traffic_explain;
pub mod update;
pub mod where_used;
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
    /// Only these fields of --json `data` (dotted paths reach into nested objects and
    /// through lists: a,b,c.d; names from `illumio-tui schema COMMAND`), or these --csv
    /// columns in this order (the header row; `schema COMMAND` lists them in its $comment).
    /// Comma-separated or repeatable; checked before any request
    #[arg(long, global = true, value_name = "FIELDS", value_delimiter = ',')]
    pub fields: Vec<String>,
    /// Byte counts as 1.5K, 12M, 2.1G (1024-based, like ls -h) in --table and --csv, and in
    /// the stderr meta line; --json keeps integers (accepted, no effect there)
    #[arg(long, short = 'H', global = true)]
    pub human: bool,
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
    /// Which risky services (RDP, SMB, WinRM, SSH, databases …) reach what, and under which
    /// decision (read-only)
    ///
    /// One Explorer query over the window, its services every port of the risky list (the
    /// profile's [exposure], else rdp, smb, netbios, rpc, winrm, ssh, telnet, vnc, mssql,
    /// mysql, postgres, oracle, mongodb, redis, ldap), in its own cache slot, through the cost
    /// guard. One row per destination group and service: its sources (intra: the group's own,
    /// apps: other labelled groups, ip_lists: unlabelled hosts and IP lists, any and internet
    /// flagged) and flows per decision. Exposed rows (allowed from outside the group) first,
    /// then potentially blocked, blocked, intra; each exposed row names the `traffic explain`
    /// command that says which rule allows it (one query, not run here). --fail-on-exposed:
    /// exit 8 when a row is exposed.
    Exposure(exposure::ExposureArgs),
    /// Rulesets, rules, deny rules, pending changes, provisioning
    #[command(subcommand)]
    Policy(policy::PolicyCmd),
    /// Pairing profiles: list them; create or delete one (no pairing keys)
    #[command(subcommand, visible_alias = "pp")]
    PairingProfiles(pairing_profiles::PairingProfilesCmd),
    /// What names a label, label group, IP list or service in the draft policy (read-only)
    ///
    /// One row per reference: ruleset scopes, rules (provider, consumer or service side; allow,
    /// deny or override deny; pending and disabled marked) and the label groups holding it,
    /// directly or through a nested group (via); a rule naming such a group is listed via it.
    /// For a label, a last row counts the workloads carrying it. Rules are numbered as the
    /// TUI's Policy screen lists them. Requests, all cached: the object's own list (to find it
    /// by name or href), the draft rulesets, the label groups (labels and label groups only)
    /// and, for a label, one workloads read of at most one row filtered by it: no row is 0, one
    /// row is at least 1 (the PCE's X-Total-Count is the whole collection's size). Deny rules
    /// as the ruleset list carries them. Not searched: virtual services, enforcement
    /// boundaries, pairing profiles, container workload profiles.
    #[command(subcommand)]
    WhereUsed(where_used::WhereUsedCmd),
    /// Unused labels, label groups, IP lists and services, and workloads missing labels (read-only)
    ///
    /// The reverse of where-used, in one pass: an object nothing in the draft policy (every href
    /// a ruleset carries: scopes, rule sides and services, deny rules as the ruleset list
    /// carries them, custom iptables rules, egress services), the label groups, the pairing
    /// profiles (labels) or the workloads (labels) names is listed `unused`; a label also only
    /// when the PCE's own usage flags (labels?usage=true, the same one read) hold it nowhere.
    /// Never a guess: a kind a list not read in full may hold (cut short, or failed) is not
    /// listed (meta.hygiene.withheld), and rows that rest on rulesets whose deny rules were not
    /// read carry certain: false (meta.hygiene.uncertain). Built-ins (Any (0.0.0.0/0 and ::/0),
    /// All Services, ilo_defaults), deleted labels and objects pending deletion never are. An
    /// unused row is a candidate, not a verdict: meta.not_searched names what may still use it
    /// (active policy: draft only; virtual services and servers, enforcement boundaries,
    /// container workloads and their profiles, cloud resources, label mapping rules, RBAC
    /// scopes, firewall settings). Workloads, managed and unmanaged, lacking one or more of the
    /// profile's label keys ([profiles.x.labels] scope and tier keys; role, app, env for the
    /// default model, and loc when the PCE has a loc label) are listed with the keys missing.
    /// Requests, all cached and only for the kinds asked: labels, label groups, IP lists,
    /// services, the draft rulesets, the pairing profiles and the workload list. Exit 0
    /// whatever it finds.
    Hygiene(hygiene::HygieneArgs),
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
    /// Saved filters of the profile in the config file: list, show, rm (local, no request)
    ///
    /// --filter NAME on traffic query|groups|mesh, policy search, events and workloads starts
    /// from a saved filter (a flag given replaces that field); --save-filter NAME saves the
    /// filter the flags make and exits without contacting the PCE. The TUI saves and loads the
    /// same ones (:save, :filters, :load). Stored per profile as
    /// [profiles.<name>.filters.<screen>.<filter>], in the TUI editor's text.
    #[command(subcommand)]
    Filters(filters::FiltersCmd),
    /// Print the shell completion script: bash, zsh, fish or powershell (local, no request)
    ///
    /// e.g. `illumio-tui completions zsh > ~/.zfunc/_illumio-tui`. Profile names complete
    /// from the config file; labels, apps, hostnames, services, IP lists and label groups from
    /// the profile's names cache, which list commands, the TUI and `completions refresh` fill.
    /// Nothing a completion does contacts the PCE.
    Completions(completions::CompletionsArgs),
    /// The JSON Schema of a command's --json output, or the list of them (local, no request)
    ///
    /// `illumio-tui schema` lists every command and the schema its envelope follows (also
    /// meta.schema of each envelope); `illumio-tui schema workloads ven-summary` prints that
    /// envelope's schema (JSON Schema draft 2020-12): `data` of the command, `meta` shared.
    /// A schema name works too (`schema write`, `schema envelope`, `schema meta`,
    /// `schema error`); a row with a flag types as listed (`schema traffic query --dry-run`).
    /// --json --fields takes the schema's `data` properties; --csv --fields takes the CSV
    /// columns, which `schema COMMAND` names in its top-level $comment.
    Schema(schema::SchemaArgs),
    /// Local snapshots of the policy, labels and workloads; what changed between two
    ///
    /// `snapshot save` reads (cached, counted) and writes one file under
    /// $XDG_DATA_HOME/illumio-tui/snapshots/<profile>/ (else ~/.local/share/…); `snapshot diff`
    /// compares two of them, or one with now (a live read). `list`, `rm` and `prune` read only
    /// the files. The files hold what the reads returned, no credentials.
    #[command(subcommand)]
    Snapshot(snapshot::SnapshotCmd),
    /// Is a newer illumio-tui out? (GitHub's release list, never the PCE)
    ///
    /// `update check` asks GitHub once and prints the newest release and how to upgrade this
    /// install. Package managers install it: there is no self-update.
    #[command(subcommand)]
    Update(update::UpdateCmd),
    /// What's new: the release notes newer than this binary, else this version's
    ///
    /// RANGE `0.5.6..` (after 0.5.6) or `0.5.6..0.5.8` picks releases. This version's notes and
    /// older ones are built in (offline); newer ones come from GitHub's release list (cached a
    /// day, else one request; never the PCE). Markdown, or --json [{version, date, body}].
    News(update::NewsArgs),
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
        Command::Workloads(c) if c.filter_args().is_none() => {
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
        Command::PairingProfiles(c) if c.write_flags().is_some() => {
            Some("pairing-profiles create/delete")
        }
        Command::Traffic(t) if t.filter().is_some_and(|f| f.dry_run) => Some("traffic --dry-run"),
        Command::Profile(profile::ProfileCmd::Use { .. }) => Some("profile use"),
        Command::Profile(profile::ProfileCmd::Add(_)) => Some("profile add"),
        Command::Marks(marks::MarksCmd::Export(_)) => Some("marks export"),
        Command::Filters(filters::FiltersCmd::Rm(_)) => Some("filters rm"),
        Command::Filters(filters::FiltersCmd::Default(_)) => Some("filters default"),
        c if filters::saves_filter(c) => Some("--save-filter"),
        Command::Enforce(a) if !a.watch => Some("enforce --to/--rollback"),
        Command::Schema(a) if !a.command.is_empty() => Some("schema COMMAND"),
        Command::Update(_) => Some("update check"),
        Command::News(_) => Some("news"),
        _ => None,
    }
}

/// The envelope's schema and `--fields`, checked before any request: JSON
/// names against the schema (so an empty result checks too), CSV names
/// against the header row when it prints.
fn select_output(global: &GlobalArgs, command: &Command) -> Result<(), CliError> {
    let schema = schema::schema_of(command);
    let csv_columns = columns::csv_columns(command);
    let fields = output::parse_fields(&global.fields);
    if !global.fields.is_empty() && fields.is_empty() {
        return Err(CliError::Args("--fields: no field names".into()));
    }
    if let Some(f) = fields.iter().find(|f| f.split('.').any(str::is_empty)) {
        return Err(CliError::Args(format!("--fields {f}: an empty name")));
    }
    if !fields.is_empty() {
        let format = output::choose_format(global.json, global.table, global.csv);
        match (format, schema) {
            (output::OutputFormat::Table, _) => {
                return Err(CliError::Args("--fields needs --json or --csv".into()));
            }
            (_, None) => {
                return Err(CliError::Args(
                    "--fields: this command prints no JSON envelope or CSV".into(),
                ));
            }
            (output::OutputFormat::Csv, Some(_)) => {
                if let Some(f) = fields.iter().find(|f| f.contains('.')) {
                    return Err(CliError::Args(format!(
                        "--fields {f}: a dotted path is for --json; --csv takes column names"
                    )));
                }
                if let Some(f) = fields.iter().find(|f| !csv_columns.contains(&f.as_str())) {
                    return Err(CliError::Args(format!(
                        "--fields: no CSV column {f:?}; the columns are: {}",
                        csv_columns.join(", ")
                    )));
                }
            }
            (output::OutputFormat::Json, Some(name)) => {
                schema::check_fields(name, schema::branch_of(command), &fields)?
            }
        }
    }
    output::select(output::Selection {
        schema,
        fields,
        csv_columns,
    });
    Ok(())
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
    output::set_human(global.human);
    select_output(&global, &command)?;
    if let Command::Schema(s) = &command {
        return schema::run(&global, s);
    }
    // GitHub's release list and the built-in notes: no profile, no PCE.
    if let Command::Update(u) = &command {
        return update::run_update(&global, u).await;
    }
    if let Command::News(n) = &command {
        return update::run_news(&global, n).await;
    }
    if let Command::Profile(p) = &command {
        return profile::run(&global, p);
    }
    if let Command::Marks(m) = &command {
        return marks::run(&global, m);
    }
    if let Command::Filters(f) = &command {
        return filters::run(&global, f);
    }
    // Snapshot files only: the profile by name, no client, no credentials.
    if let Command::Snapshot(s) = &command
        && snapshot::is_local(s)
    {
        return snapshot::run_local(&global, s);
    }
    // Saved filters, from the config file alone: `--save-filter` ends here.
    let Some(command) = filters::prepare(&global, command)? else {
        return Ok(());
    };
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
        Command::Exposure(a) => exposure::run(&ctx, &a).await,
        Command::Policy(p) => policy::run(&ctx, &p).await,
        Command::PairingProfiles(p) => pairing_profiles::run(&ctx, &p).await,
        Command::WhereUsed(w) => where_used::run(&ctx, &w).await,
        Command::Hygiene(a) => hygiene::run(&ctx, &a).await,
        Command::Traffic(t) => traffic::run(&ctx, &t).await,
        Command::Completions(_) => names_cache::refresh(&ctx).await,
        Command::Snapshot(s) => snapshot::run(&ctx, &s).await,
        Command::Profile(_)
        | Command::Marks(_)
        | Command::Filters(_)
        | Command::Schema(_)
        | Command::Update(_)
        | Command::News(_) => {
            unreachable!("handled above")
        }
    }
}
