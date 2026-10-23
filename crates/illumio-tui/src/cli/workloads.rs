//! `workloads`: list workloads, with `--label key=value` filters resolved to
//! hrefs via the name index before querying.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{TableRows, join_list};
use super::write::{
    self, WriteFlags, label_in, parse_kv, resolve_labels, resolve_workload, run_plan,
};
use clap::{Args, Subcommand};
use pce::label_filter::{
    LabelFilter, LabelTerm, MAX_COMBINATIONS, QueryError, group_filter, group_key, group_title,
};
use pce::models::{UNMANAGED, VenHealth, Workload, WorkloadFilter};
use pce::write::{
    IfMissing, InterfaceSpec, LabelRemoval, LabelsPatch, WorkloadDraft, WorkloadPatch,
};
use pce::{KeyOrder, PceError};
use serde::Serialize;

#[derive(Args, Debug, Clone, Default)]
pub struct WorkloadsArgs {
    /// Hostname (PCE substring match)
    #[arg(long)]
    pub hostname: Option<String>,
    /// Workload name (PCE substring match)
    #[arg(long)]
    pub name: Option<String>,
    /// Interface IP address
    #[arg(long)]
    pub ip: Option<String>,
    /// Label key=value; a comma lists values any of which matches
    /// (app=shop,crm); repeat to AND several. !key=value excludes and
    /// key=- means no label for the key (both filtered locally)
    #[arg(long = "label")]
    pub labels: Vec<String>,
    /// Effective enforcement: unmanaged (no VEN) asks for managed=false; a
    /// mode also asks for managed=true, as the PCE gives workloads without a
    /// VEN a mode too (often visibility_only)
    #[arg(long, value_parser = ["full", "visibility_only", "selective", "idle", "unmanaged"])]
    pub enforcement: Option<String>,
    /// Only online (true) or offline (false) workloads
    #[arg(long)]
    pub online: Option<bool>,
    /// Only managed (true) or unmanaged (false) workloads
    #[arg(long)]
    pub managed: Option<bool>,
    /// Row cap sent to the PCE (default 10000)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Only workloads whose VEN is in warning, error or offline state (local filter)
    #[arg(long)]
    pub unhealthy: bool,
    /// Only workloads whose VEN is offline: online false, or no heartbeat
    /// for 15 minutes (local filter, the app's `o`)
    #[arg(long)]
    pub offline: bool,
    /// Only workloads without a label for one of these keys (comma-separated);
    /// alone, the profile's scope keys and tier key (local filter, the app's `u`)
    #[arg(
        long,
        value_name = "KEYS",
        num_args = 0..=1,
        value_delimiter = ',',
        default_missing_value = ""
    )]
    pub unlabeled: Option<Vec<String>>,
}

impl WorkloadsArgs {
    /// Any filter that narrows the selection (`--max-results` is a cap,
    /// not a filter).
    pub fn narrows(&self) -> bool {
        self.hostname.is_some()
            || self.name.is_some()
            || self.ip.is_some()
            || !self.labels.is_empty()
            || self.enforcement.is_some()
            || self.online.is_some()
            || self.managed.is_some()
            || self.unhealthy
            || self.offline
            || self.unlabeled.is_some()
    }
}

/// `workloads` lists by default; `add`, `update` and `delete` write
/// unmanaged workloads.
#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct WorkloadsCmd {
    #[command(subcommand)]
    pub action: Option<WorkloadsAction>,
    #[command(flatten)]
    pub list: WorkloadsArgs,
    /// Group the listed workloads by these label keys (comma-separated);
    /// alone, the profile's scope keys (app,env). Table: a heading per
    /// group; JSON: data.groups; CSV: a leading group column (app=shop env=-)
    #[arg(
        long,
        value_name = "KEYS",
        num_args = 0..=1,
        value_delimiter = ',',
        default_missing_value = ""
    )]
    pub group: Option<Vec<String>>,
    #[command(flatten)]
    pub saved: super::filters::SavedFilterArgs,
}

impl WorkloadsArgs {
    /// The saved filter `s` as the base: its labels where no `--label` is
    /// given, its enforcement, managed, offline and unlabeled where those
    /// flags are not.
    pub fn with_saved(&mut self, s: &crate::filters::WorkloadsSaved) -> Result<(), String> {
        if self.labels.is_empty()
            && let Some(text) = &s.labels
        {
            self.labels = LabelFilter::parse(text)
                .map_err(|e| format!("field labels: {e}"))?
                .terms
                .iter()
                .map(ToString::to_string)
                .collect();
        }
        if self.enforcement.is_none() {
            self.enforcement = s.enforcement.clone();
        }
        if self.managed.is_none() {
            self.managed = s.managed;
        }
        self.offline |= s.offline == Some(true);
        if self.unlabeled.is_none() && s.unlabeled == Some(true) {
            // `--unlabeled` alone: the label model's keys.
            self.unlabeled = Some(Vec::new());
        }
        Ok(())
    }

    /// The filter as the TUI's Workloads screen saves it; flags it has no
    /// field for are an error, never dropped.
    pub fn to_saved(&self) -> Result<crate::filters::WorkloadsSaved, String> {
        for (given, flag) in [
            (self.hostname.is_some(), "--hostname"),
            (self.name.is_some(), "--name"),
            (self.ip.is_some(), "--ip"),
            (self.online.is_some(), "--online"),
            (self.max_results.is_some(), "--max-results"),
            (self.unhealthy, "--unhealthy"),
        ] {
            if given {
                return Err(format!("{flag} is not part of a saved workloads filter"));
            }
        }
        if self
            .unlabeled
            .as_ref()
            .is_some_and(|keys| keys.iter().any(|k| !k.trim().is_empty()))
        {
            return Err(
                "--unlabeled KEYS: a saved filter's unlabeled is the label model's keys \
                 (--unlabeled alone)"
                    .into(),
            );
        }
        let labels = LabelFilter::from_terms(&self.labels)?.to_string();
        Ok(crate::filters::WorkloadsSaved {
            labels: Some(labels).filter(|l| !l.is_empty()),
            enforcement: self.enforcement.clone(),
            managed: self.managed,
            offline: self.offline.then_some(true),
            unlabeled: self.unlabeled.is_some().then_some(true),
        })
    }
}

#[derive(Subcommand, Debug)]
pub enum WorkloadsAction {
    /// Create an unmanaged workload (needs writes = true in the profile)
    Add(WorkloadAddArgs),
    /// Change an unmanaged workload's name, hostname, interfaces or labels; with --managed-labels, a VEN-managed workload's labels
    Update(WorkloadUpdateArgs),
    /// Delete an unmanaged workload
    Delete(WorkloadDeleteArgs),
    /// Set the enforcement mode (and visibility level) of every workload the
    /// filters select, in bulk_update requests of 100; unmanaged ones are skipped
    Enforce(WorkloadEnforceArgs),
    /// Set or remove labels on every workload the filters select, managed
    /// and unmanaged, in bulk_update requests of 100
    Relabel(WorkloadRelabelArgs),
}

#[derive(Args, Debug)]
pub struct WorkloadEnforceArgs {
    /// The enforcement mode
    #[arg(value_parser = ["idle", "visibility_only", "selective", "full"])]
    pub mode: String,
    /// The visibility level; without it each workload keeps its own
    #[arg(long, value_parser = ["flow_summary", "flow_drops", "flow_off", "enhanced_data_collection"])]
    pub visibility: Option<String>,
    /// The number of workloads that change: required with --yes for full or
    /// selective on more than one (the dry run prints it)
    #[arg(long, value_name = "N")]
    pub confirm_count: Option<usize>,
    /// Every workload the read returns when no filter narrows it (needed
    /// without filters; with filters it changes nothing)
    #[arg(long)]
    pub all: bool,
    #[command(flatten)]
    pub select: WorkloadsArgs,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct WorkloadRelabelArgs {
    /// Labels per key, comma-separated: key=value sets it (replacing that
    /// key's label), key=- removes it; other keys are kept
    #[arg(long, value_delimiter = ',', value_name = "KEY=VALUE,KEY=-")]
    pub set: Vec<String>,
    /// Label key=value to set (repeatable; as --set)
    #[arg(long = "add-label")]
    pub add_labels: Vec<String>,
    /// Label to remove: key (whatever its value), or key=value (only from
    /// workloads that have that value) (repeatable)
    #[arg(long = "remove-label")]
    pub remove_labels: Vec<String>,
    /// Every workload the read returns when no filter narrows it (needed
    /// without filters; with filters it changes nothing)
    #[arg(long)]
    pub all: bool,
    #[command(flatten)]
    pub select: WorkloadsArgs,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct WorkloadAddArgs {
    /// Workload name (also the hostname unless --hostname is given)
    pub name: String,
    /// Hostname
    #[arg(long)]
    pub hostname: Option<String>,
    /// Interface address, ADDR or ADDR/N (repeatable; named eth0, eth1, ...)
    #[arg(long = "ip")]
    pub ips: Vec<String>,
    /// Label key=value (repeatable)
    #[arg(long = "label")]
    pub labels: Vec<String>,
    /// Description
    #[arg(long)]
    pub description: Option<String>,
    /// Do nothing when a workload with exactly this hostname exists
    #[arg(long)]
    pub if_missing: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct WorkloadUpdateArgs {
    /// Hostname, name or href
    pub target: String,
    /// New name
    #[arg(long)]
    pub name: Option<String>,
    /// New hostname
    #[arg(long)]
    pub hostname: Option<String>,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    /// Interface address to add, ADDR or ADDR/N (repeatable)
    #[arg(long = "add-ip")]
    pub add_ips: Vec<String>,
    /// Interface address to remove (repeatable)
    #[arg(long = "remove-ip")]
    pub remove_ips: Vec<String>,
    /// Replace all labels: key=value,key=value (conflicts with --add-label/--remove-label)
    #[arg(long, value_delimiter = ',', conflicts_with_all = ["add_labels", "remove_labels"])]
    pub set_labels: Option<Vec<String>>,
    /// Label key=value to add (repeatable)
    #[arg(long = "add-label")]
    pub add_labels: Vec<String>,
    /// Label to remove: key (any value) or key=value (repeatable)
    #[arg(long = "remove-label")]
    pub remove_labels: Vec<String>,
    /// Change labels on a VEN-managed workload (label flags only; the PUT carries only labels)
    #[arg(long)]
    pub managed_labels: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct WorkloadDeleteArgs {
    /// Hostname, name or href
    pub target: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

pub async fn workloads_cmd(ctx: &Context, cmd: &WorkloadsCmd) -> Result<(), CliError> {
    match &cmd.action {
        None => run(ctx, &cmd.list, cmd.group.as_deref()).await,
        Some(WorkloadsAction::Add(a)) => workload_add(ctx, a).await,
        Some(WorkloadsAction::Update(a)) => workload_update(ctx, a).await,
        Some(WorkloadsAction::Delete(a)) => workload_delete(ctx, a).await,
        Some(WorkloadsAction::Enforce(a)) => super::bulk::enforce(ctx, a).await,
        Some(WorkloadsAction::Relabel(a)) => super::bulk::relabel(ctx, a).await,
    }
}

fn parse_interfaces(specs: &[String]) -> Result<Vec<InterfaceSpec>, CliError> {
    specs
        .iter()
        .enumerate()
        .map(|(i, s)| InterfaceSpec::parse(s, format!("eth{i}")).map_err(CliError::Args))
        .collect()
}

async fn workload_add(ctx: &Context, a: &WorkloadAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let draft = WorkloadDraft {
        name: a.name.clone(),
        hostname: a.hostname.clone(),
        description: a.description.clone(),
        interfaces: parse_interfaces(&a.ips)?,
        labels: resolve_labels(ctx, &a.labels).await?,
    };
    let if_missing = if a.if_missing {
        IfMissing::SkipExisting
    } else {
        IfMissing::Create
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_create_workload(&draft, if_missing)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn workload_update(ctx: &Context, a: &WorkloadUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    if a.managed_labels
        && (a.name.is_some()
            || a.hostname.is_some()
            || a.description.is_some()
            || !a.add_ips.is_empty()
            || !a.remove_ips.is_empty())
    {
        return Err(CliError::Args(
            "--managed-labels allows label changes only".into(),
        ));
    }
    let remove_labels = a
        .remove_labels
        .iter()
        .map(|s| match s.split_once('=') {
            Some(_) => parse_kv(s).map(|(k, v)| LabelRemoval {
                key: k,
                value: Some(v),
            }),
            None if s.trim().is_empty() => Err(CliError::Args("--remove-label needs a key".into())),
            None => Ok(LabelRemoval {
                key: s.trim().to_string(),
                value: None,
            }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Nothing to change: refuse before any lookup or label resolution.
    let nothing = WorkloadPatch {
        name: a.name.clone(),
        hostname: a.hostname.clone(),
        description: a.description.clone(),
        add_addresses: a.add_ips.clone(),
        remove_addresses: a.remove_ips.clone(),
        set_labels: a.set_labels.as_ref().map(|_| Vec::new()),
        add_labels: a.add_labels.clone(),
        remove_labels: remove_labels.clone(),
    }
    .is_empty();
    if nothing {
        return Err(CliError::Args(
            "nothing to change; pass --name, --hostname, --description, --add-ip, --remove-ip, --set-labels, --add-label or --remove-label".into(),
        ));
    }
    // One labels list (one GET), only when labels change: it resolves the
    // specs and supplies keys for the workload's current labels.
    let touches_labels =
        a.set_labels.is_some() || !a.add_labels.is_empty() || !a.remove_labels.is_empty();
    let labels = if touches_labels {
        ctx.pce.labels(ctx.cache_mode).await?.value
    } else {
        Vec::new()
    };
    let resolve = |specs: &[String]| -> Result<Vec<String>, CliError> {
        specs.iter().map(|s| label_in(&labels, s)).collect()
    };
    // `--set-labels ""` clears every label: clap yields `Some(vec![""])`.
    let set_labels = match &a.set_labels {
        Some(specs) => {
            let specs: Vec<String> = specs
                .iter()
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .collect();
            Some(resolve(&specs)?)
        }
        None => None,
    };
    let add_labels = resolve(&a.add_labels)?;
    let href = resolve_workload(ctx, &guard, &a.target).await?;
    let idx = pce::NameIndex::new(&labels, &[], &[], &[]);
    let w = ctx.pce.writer(&guard);
    let plan = if a.managed_labels {
        let patch = LabelsPatch {
            set_labels,
            add_labels,
            remove_labels,
        };
        w.plan_relabel_workload(&href, &patch, &idx).await?
    } else {
        let patch = WorkloadPatch {
            name: a.name.clone(),
            hostname: a.hostname.clone(),
            description: a.description.clone(),
            add_addresses: a.add_ips.clone(),
            remove_addresses: a.remove_ips.clone(),
            set_labels,
            add_labels,
            remove_labels,
        };
        w.plan_update_workload(&href, &patch, &idx).await?
    };
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn workload_delete(ctx: &Context, a: &WorkloadDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = resolve_workload(ctx, &guard, &a.target).await?;
    let plan = ctx.pce.writer(&guard).plan_delete_workload(&href).await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

/// A workload plus its derived VEN health, as printed by the CLI.
#[derive(Debug, Serialize)]
pub struct WorkloadRow {
    #[serde(flatten)]
    pub workload: Workload,
    /// The effective enforcement ([`Workload::enforcement`]): `unmanaged`
    /// without a VEN, else the PCE's mode. JSON keeps the raw value in
    /// `enforcement_mode`; the table and CSV `enforcement` cell is this one.
    pub enforcement: Option<String>,
    pub health: VenHealth,
    pub sync_state: Option<String>,
    pub heartbeat_age_secs: Option<u64>,
    /// With `--unlabeled`: the keys checked that have no label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_labels: Option<Vec<String>>,
    /// The table's `labels` cell, keys in the profile's [`KeyOrder`]; JSON
    /// carries the `labels` array itself.
    #[serde(skip)]
    pub label_text: String,
    /// CSV's `labels` cell: every label (ignored keys too, as in JSON) in
    /// the key order, `; `-joined.
    #[serde(skip)]
    pub label_list: String,
}

impl WorkloadRow {
    pub fn new(workload: Workload, now: jiff::Timestamp, order: &KeyOrder) -> Self {
        let health = workload.ven_health(now);
        let a = workload.agent_status_view();
        Self {
            label_text: workload.label_text(order),
            label_list: join_list(workload.label_entries(order)),
            heartbeat_age_secs: workload.heartbeat_age(now).map(|d| d.as_secs()),
            sync_state: a.and_then(|a| a.sync_state),
            enforcement: workload.enforcement().map(str::to_string),
            missing_labels: None,
            health,
            workload,
        }
    }
}

/// A warning for each named `--unlabeled` key that is neither in the label
/// `model` nor on any `seen` workload label: likely a typo, as every
/// workload is then missing it. Keys match exactly, as label keys do;
/// one that differs only in case is named.
fn unknown_key_warnings(named: &[String], model: &[String], seen: &[&str]) -> Vec<String> {
    named
        .iter()
        .filter(|k| !model.contains(k) && !seen.contains(&k.as_str()))
        .map(|k| {
            let near = model
                .iter()
                .map(String::as_str)
                .chain(seen.iter().copied())
                .find(|s| s.eq_ignore_ascii_case(k));
            match near {
                Some(n) => format!(
                    "unknown label key {k}: not in the label model or on any workload; did you mean {n}?"
                ),
                None => format!(
                    "unknown label key {k}: not in the label model or on any workload, so every workload is missing it"
                ),
            }
        })
        .collect()
}

/// `--unlabeled`'s keys: the ones named (blanks dropped, repeats once),
/// else `model` (the profile's scope keys, then its tier key).
fn unlabeled_keys(named: &[String], model: &[String]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for k in named.iter().map(|k| k.trim()).filter(|k| !k.is_empty()) {
        if !keys.iter().any(|x| x == k) {
            keys.push(k.to_string());
        }
    }
    if keys.is_empty() {
        model.to_vec()
    } else {
        keys
    }
}

/// `45s`, `2m`, `3h`, `2d`; blank when the age is unknown.
fn compact_age(secs: Option<u64>) -> String {
    match secs {
        None => String::new(),
        Some(s) if s < 60 => format!("{s}s"),
        Some(s) if s < 3600 => format!("{}m", s / 60),
        Some(s) if s < 86_400 => format!("{}h", s / 3600),
        Some(s) => format!("{}d", s / 86_400),
    }
}

/// Workloads printed as the list: the plain list's rows, or one group's.
pub struct Rows<'a>(pub Vec<&'a WorkloadRow>);

impl TableRows for Vec<WorkloadRow> {
    fn headers(&self) -> Vec<&'static str> {
        Rows(self.iter().collect()).headers()
    }
    fn rows(&self) -> Vec<Vec<String>> {
        Rows(self.iter().collect()).rows()
    }
    fn full_headers(&self) -> Vec<&'static str> {
        Rows(self.iter().collect()).full_headers()
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        Rows(self.iter().collect()).full_rows()
    }
}

impl TableRows for Rows<'_> {
    /// `missing` last when `--unlabeled` filled it.
    fn headers(&self) -> Vec<&'static str> {
        let mut h = vec![
            "hostname",
            "name",
            "ip",
            "labels",
            "enforcement",
            "online",
            "agent",
            "sync",
            "heartbeat",
            "health",
            "href",
        ];
        if self.0.iter().any(|r| r.missing_labels.is_some()) {
            h.push("missing");
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let missing = self.0.iter().any(|r| r.missing_labels.is_some());
        self.0
            .iter()
            .map(|r| {
                let w = &r.workload;
                let mut row = vec![
                    w.hostname.clone().unwrap_or_default(),
                    w.name.clone().unwrap_or_default(),
                    w.primary_ip().unwrap_or("").to_string(),
                    r.label_text.clone(),
                    r.enforcement.clone().unwrap_or_default(),
                    w.online.map(|b| b.to_string()).unwrap_or_default(),
                    w.agent_status().unwrap_or_default(),
                    r.sync_state.clone().unwrap_or_default(),
                    compact_age(r.heartbeat_age_secs),
                    r.health.as_str().to_string(),
                    w.href.clone(),
                ];
                if missing {
                    row.push(r.missing_labels.as_deref().unwrap_or_default().join(", "));
                }
                row
            })
            .collect()
    }
    /// The heartbeat age in seconds, as JSON's `heartbeat_age_secs`; the
    /// PCE's raw `enforcement_mode` appended, as JSON carries it.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        h[8] = "heartbeat_age_secs";
        h.push("enforcement_mode");
        h
    }
    /// Every interface address and every label; the heartbeat in seconds;
    /// the raw `enforcement_mode` last.
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .zip(self.rows())
            .map(|(r, mut row)| {
                row[2] = join_list(r.workload.interfaces.iter().map(|i| &i.address));
                row[3] = r.label_list.clone();
                row[8] = r
                    .heartbeat_age_secs
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                if let Some(m) = &r.missing_labels
                    && let Some(cell) = row.last_mut()
                {
                    *cell = join_list(m);
                }
                row.push(r.workload.enforcement_mode.clone().unwrap_or_default());
                row
            })
            .collect()
    }
}

/// `--enforcement` (effective) and `--managed` as the PCE's
/// `(enforcement_mode, managed)` query: `unmanaged` is `managed=false` with
/// no mode; a mode adds `managed=true`, since the PCE gives workloads
/// without a VEN a mode too (often `visibility_only`). A contradiction is an argument error.
fn enforcement_query(
    enforcement: Option<&str>,
    managed: Option<bool>,
) -> Result<(Option<&str>, Option<bool>), CliError> {
    match (enforcement, managed) {
        (None, m) => Ok((None, m)),
        (Some(UNMANAGED), Some(true)) => Err(CliError::Args(
            "--enforcement unmanaged means no VEN; it contradicts --managed true".into(),
        )),
        (Some(UNMANAGED), _) => Ok((None, Some(false))),
        (Some(mode), Some(false)) => Err(CliError::Args(format!(
            "--enforcement {mode} needs a VEN; a workload without one is unmanaged (--enforcement unmanaged)"
        ))),
        (Some(mode), _) => Ok((Some(mode), Some(true))),
    }
}

/// The workloads the list filters select: one list request (the plain
/// any-of `--label` terms resolved through the name index, cached), the
/// local filters applied to its answer.
pub struct Selection {
    /// The query the request carried.
    pub filter: WorkloadFilter,
    pub rows: Vec<WorkloadRow>,
    pub hit: bool,
    pub truncated: bool,
    /// No workload can match: no request was sent.
    pub nothing: bool,
    pub warnings: Vec<String>,
    pub local_filter: Option<Vec<String>>,
}

pub async fn select(
    ctx: &Context,
    args: &WorkloadsArgs,
    mode: pce::CacheMode,
) -> Result<Selection, CliError> {
    let (enforcement_mode, managed) = enforcement_query(args.enforcement.as_deref(), args.managed)?;
    // `online` is the PCE's flag; offline is the VEN health (a stale
    // heartbeat too), so `--offline` is never sent as `online=false`.
    if args.offline && args.online == Some(true) {
        return Err(CliError::Args("--offline contradicts --online true".into()));
    }
    let order = ctx.config.key_order();
    let unlabeled = args
        .unlabeled
        .as_deref()
        .map(|named| unlabeled_keys(named, order.first()));
    let mut filter = WorkloadFilter {
        name: args.name.clone(),
        hostname: args.hostname.clone(),
        ip_address: args.ip.clone(),
        enforcement_mode: enforcement_mode.map(str::to_string),
        managed,
        online: args.online,
        labels: Vec::new(),
        max_results: args.max_results,
    };
    // One parser for `--label` and the app's `f`; the positive any-of
    // terms go into the one request, `!` and `-` are applied to its answer.
    let labels = LabelFilter::from_terms(&args.labels)
        .map_err(|e| CliError::Args(format!("--label {e}")))?;
    // `app=shop app=crm`: no workload has both, so nothing is asked (an
    // empty labels parameter would mean every workload).
    let nothing = labels.matches_nothing();
    if !nothing && labels.terms.iter().any(|t| !t.negate && !t.none) {
        // The labels list alone resolves them (one read, cached).
        let labels_list = ctx.pce.labels(ctx.cache_mode).await?.value;
        let idx = pce::NameIndex::new(&labels_list, &[], &[], &[]);
        let sets = labels
            .query_sets(|k, v| {
                idx.label_href(k, v).map(str::to_string).ok_or_else(|| {
                    PceError::Invalid(format!(
                        "unknown label {k}={v}; known values for {k}: {}",
                        idx.values_for_key(k).join(", ")
                    ))
                })
            })
            .map_err(|e| match e {
                QueryError::TooMany(n) => CliError::Args(format!(
                    "--label: the any-of values make {n} label combinations; at most {MAX_COMBINATIONS} fit one request"
                )),
                QueryError::Unresolved(e) => e.into(),
            })?;
        filter.labels = sets.unwrap_or_default();
    }
    let local_labels: Vec<&LabelTerm> =
        labels.terms.iter().filter(|t| t.negate || t.none).collect();
    let c = if nothing {
        pce::Cached {
            value: Vec::new(),
            age: std::time::Duration::ZERO,
            hit: false,
            truncated: false,
        }
    } else {
        ctx.pce.workloads(&filter, mode).await?
    };
    let mut warnings = match &unlabeled {
        Some(keys) if keys.as_slice() != order.first() => {
            let seen: Vec<&str> = c
                .value
                .iter()
                .flat_map(|w| w.labels.iter().filter_map(|l| l.key.as_deref()))
                .collect();
            unknown_key_warnings(keys, order.first(), &seen)
        }
        _ => Vec::new(),
    };
    if nothing {
        warnings.push(
            "--label: a key's values have nothing in common across its terms, so no workload can match; no request sent"
                .to_string(),
        );
    }
    let now = jiff::Timestamp::now();
    let rows: Vec<WorkloadRow> = c
        .value
        .into_iter()
        .map(|w| {
            let mut r = WorkloadRow::new(w, now, &order);
            r.missing_labels = unlabeled
                .as_deref()
                .map(|keys| r.workload.missing_label_keys(keys));
            r
        })
        // The PCE filtered already; this holds the effective mode should it not.
        .filter(|r| {
            args.enforcement
                .as_deref()
                .is_none_or(|e| r.enforcement.as_deref() == Some(e))
        })
        .filter(|r| {
            !args.unhealthy
                || matches!(
                    r.health,
                    VenHealth::Warning | VenHealth::Error | VenHealth::Offline
                )
        })
        // Offline is the VEN health, as the app's `o`.
        .filter(|r| !args.offline || r.health == VenHealth::Offline)
        .filter(|r| r.missing_labels.as_ref().is_none_or(|m| !m.is_empty()))
        // The whole label filter, as the app applies it: the query carried
        // the plain any-of terms, `!` and `-` are only checked here.
        .filter(|r| labels.keeps(&r.workload))
        .collect();
    Ok(Selection {
        filter,
        rows,
        hit: c.hit,
        truncated: c.truncated,
        nothing,
        warnings,
        local_filter: local_filter(
            args.unhealthy,
            args.offline,
            unlabeled.as_deref(),
            &local_labels,
        ),
    })
}

pub async fn run(
    ctx: &Context,
    args: &WorkloadsArgs,
    group: Option<&[String]>,
) -> Result<(), CliError> {
    let sel = select(ctx, args, ctx.cache_mode).await?;
    for w in &sel.warnings {
        eprintln!("warning: {w}");
    }
    let cache = if sel.nothing {
        "none"
    } else {
        cache_label(sel.hit, ctx.cache_mode)
    };
    let mut meta = ctx.meta(cache, sel.truncated, None);
    meta.local_filter = sel.local_filter;
    meta.warnings = Some(sel.warnings).filter(|w| !w.is_empty());
    let rows = sel.rows;
    match group {
        None => ctx.emit_meta(&rows, &meta),
        Some(named) => {
            let keys = unlabeled_keys(
                named,
                &pce::traffic::keys_or_default(&ctx.config.scope_keys),
            );
            let grouped = Grouped::new(rows, keys);
            // One table: a heading row per group, columns aligned across them.
            ctx.emit_report(&grouped, &[(String::new(), &grouped)], &meta)
        }
    }
}

/// `workloads --group`: the listed workloads under their label groups,
/// largest first (then by name). JSON is `{group_by, groups: [{key,
/// filter, count, health, enforcement, workloads}]}`; the table has a
/// heading row per group; CSV prepends a `group` column in the filter
/// grammar (`app=shop env=-`), so a row's group pastes into `--label`.
#[derive(Debug, Serialize)]
pub struct Grouped {
    pub group_by: Vec<String>,
    pub groups: Vec<WorkloadGroup>,
}

#[derive(Debug, Serialize)]
pub struct WorkloadGroup {
    /// The group's value per key, `null` where its workloads have no label.
    pub key: serde_json::Map<String, serde_json::Value>,
    /// The filter that narrows to the group (`--label` / the app's `f`).
    pub filter: String,
    pub count: usize,
    /// Workloads per VEN health, every state.
    pub health: std::collections::BTreeMap<&'static str, usize>,
    /// Workloads per effective enforcement (`unmanaged` without a VEN).
    pub enforcement: std::collections::BTreeMap<String, usize>,
    pub workloads: Vec<WorkloadRow>,
    #[serde(skip)]
    title: String,
}

impl Grouped {
    pub fn new(rows: Vec<WorkloadRow>, keys: Vec<String>) -> Self {
        let mut groups: Vec<(Vec<Option<String>>, Vec<WorkloadRow>)> = Vec::new();
        let mut at: std::collections::HashMap<Vec<Option<String>>, usize> =
            std::collections::HashMap::new();
        for r in rows {
            let k = group_key(&r.workload, &keys);
            match at.get(&k) {
                Some(&i) => groups[i].1.push(r),
                None => {
                    at.insert(k.clone(), groups.len());
                    groups.push((k, vec![r]));
                }
            }
        }
        // Largest first, then by name with a missing label last.
        groups.sort_by(|a, b| {
            b.1.len().cmp(&a.1.len()).then_with(|| {
                let name = |k: &[Option<String>]| -> Vec<(bool, String)> {
                    k.iter()
                        .map(|v| (v.is_none(), v.clone().unwrap_or_default().to_lowercase()))
                        .collect()
                };
                name(&a.0).cmp(&name(&b.0))
            })
        });
        let groups = groups
            .into_iter()
            .map(|(k, workloads)| {
                let mut health: std::collections::BTreeMap<&'static str, usize> = [
                    VenHealth::Healthy,
                    VenHealth::Warning,
                    VenHealth::Error,
                    VenHealth::Offline,
                    VenHealth::Unmanaged,
                ]
                .into_iter()
                .map(|h| (h.as_str(), 0))
                .collect();
                let mut enforcement = std::collections::BTreeMap::new();
                for w in &workloads {
                    *health.entry(w.health.as_str()).or_default() += 1;
                    if let Some(e) = &w.enforcement {
                        *enforcement.entry(e.clone()).or_default() += 1;
                    }
                }
                WorkloadGroup {
                    key: keys
                        .iter()
                        .zip(&k)
                        .map(|(key, v)| {
                            (
                                key.clone(),
                                v.clone().map_or(serde_json::Value::Null, Into::into),
                            )
                        })
                        .collect(),
                    filter: group_filter(&k, &keys),
                    count: workloads.len(),
                    health,
                    enforcement,
                    title: group_title(&k, &keys),
                    workloads,
                }
            })
            .collect();
        Self {
            group_by: keys,
            groups,
        }
    }
}

impl WorkloadGroup {
    /// `shop | prod · 2 workloads · healthy 1, offline 1 · full 1, visibility_only 1`.
    fn heading(&self) -> String {
        let n = |n: usize, what: &str| format!("{what} {n}");
        let health: Vec<String> = self
            .health
            .iter()
            .filter(|(_, c)| **c > 0)
            .map(|(h, c)| n(*c, h))
            .collect();
        let enforcement: Vec<String> = self
            .enforcement
            .iter()
            .filter(|(e, _)| e.as_str() != UNMANAGED)
            .map(|(e, c)| n(*c, e))
            .collect();
        let mut parts = vec![
            self.title.clone(),
            format!(
                "{} workload{}",
                self.count,
                if self.count == 1 { "" } else { "s" }
            ),
        ];
        if !health.is_empty() {
            parts.push(health.join(", "));
        }
        if !enforcement.is_empty() {
            parts.push(enforcement.join(", "));
        }
        parts.join(" · ")
    }
}

impl TableRows for Grouped {
    fn headers(&self) -> Vec<&'static str> {
        self.flat().headers()
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        for g in &self.groups {
            out.push(vec![g.heading()]);
            out.extend(Rows(g.workloads.iter().collect()).rows());
        }
        out
    }
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = vec!["group"];
        h.extend(self.flat().full_headers());
        h
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        let filters = self
            .groups
            .iter()
            .flat_map(|g| g.workloads.iter().map(move |_| g.filter.clone()));
        self.flat()
            .full_rows()
            .into_iter()
            .zip(filters)
            .map(|(row, f)| std::iter::once(f).chain(row).collect())
            .collect()
    }
}

impl Grouped {
    /// Every workload in group order, as the plain list prints them.
    fn flat(&self) -> Rows<'_> {
        Rows(self.groups.iter().flat_map(|g| &g.workloads).collect())
    }
}

/// `meta.local_filter`: the filters applied here after the one request,
/// as the app's filter line names them.
fn local_filter(
    unhealthy: bool,
    offline: bool,
    unlabeled: Option<&[String]>,
    labels: &[&LabelTerm],
) -> Option<Vec<String>> {
    let mut out = Vec::new();
    if unhealthy {
        out.push("unhealthy".to_string());
    }
    if offline {
        out.push("offline".to_string());
    }
    if let Some(keys) = unlabeled {
        out.push(format!("unlabeled ({})", keys.join(", ")));
    }
    if !labels.is_empty() {
        let terms: Vec<String> = labels.iter().map(ToString::to_string).collect();
        out.push(format!("labels {}", terms.join(" ")));
    }
    Some(out).filter(|o| !o.is_empty())
}

#[cfg(test)]
mod tests {

    fn list_args(argv: &[&str]) -> WorkloadsArgs {
        use clap::Parser;
        let mut full = vec!["illumio-tui", "workloads"];
        full.extend_from_slice(argv);
        match crate::cli::Cli::try_parse_from(full)
            .expect("parses")
            .command
        {
            Some(crate::cli::Command::Workloads(w)) => w.list,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn workloads_flags_save_as_the_screens_filter_and_back() {
        let a = list_args(&[
            "--label",
            "app=shop,crm",
            "--label",
            "!role=db",
            "--enforcement",
            "visibility_only",
            "--managed",
            "true",
            "--offline",
            "--unlabeled",
        ]);
        let s = a.to_saved().unwrap();
        assert_eq!(s.labels.as_deref(), Some("app=shop,crm !role=db"));
        assert_eq!(
            (s.enforcement.as_deref(), s.managed, s.offline, s.unlabeled),
            (Some("visibility_only"), Some(true), Some(true), Some(true))
        );
        let mut b = list_args(&["--enforcement", "idle"]);
        b.with_saved(&s).unwrap();
        assert_eq!(b.labels, vec!["app=shop,crm", "!role=db"]);
        assert_eq!(b.enforcement.as_deref(), Some("idle"), "the flag wins");
        assert!(b.offline && b.unlabeled.is_some());
        for (argv, flag) in [
            (vec!["--hostname", "web"], "--hostname"),
            (vec!["--unhealthy"], "--unhealthy"),
            (vec!["--max-results", "5"], "--max-results"),
        ] {
            assert!(list_args(&argv).to_saved().unwrap_err().starts_with(flag));
        }
        let e = list_args(&["--unlabeled", "app"]).to_saved().unwrap_err();
        assert!(e.contains("label model"), "{e}");
    }
    use super::*;

    fn fixture() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap()
    }

    #[test]
    fn rows_show_the_effective_enforcement_and_json_keeps_the_raw_mode() {
        let now: jiff::Timestamp = "2026-10-10T08:00:00Z".parse().unwrap();
        let mut wls = fixture();
        wls[2].enforcement_mode = Some("visibility_only".into());
        let rows: Vec<WorkloadRow> = wls
            .into_iter()
            .map(|w| WorkloadRow::new(w, now, &KeyOrder::default()))
            .collect();
        let col = rows
            .headers()
            .iter()
            .position(|h| *h == "enforcement")
            .unwrap();
        let shown: Vec<String> = rows.rows().into_iter().map(|r| r[col].clone()).collect();
        assert_eq!(shown, ["full", "visibility_only", "unmanaged", "selective"]);
        assert_eq!(rows.full_rows()[2][col], "unmanaged");
        // CSV appends the PCE's raw mode, as the app CSV does.
        assert_eq!(rows.full_headers().last(), Some(&"enforcement_mode"));
        let full = rows.full_rows();
        assert_eq!(full[2].last().unwrap(), "visibility_only");
        assert_eq!(full[0].last().unwrap(), "full");
        assert_eq!(full[0].len(), rows.full_headers().len());
        let json = serde_json::to_value(&rows[2]).unwrap();
        assert_eq!(json["enforcement"], "unmanaged");
        assert_eq!(json["enforcement_mode"], "visibility_only");
        let json = serde_json::to_value(&rows[0]).unwrap();
        assert_eq!(json["enforcement"], "full");
        assert_eq!(json["enforcement_mode"], "full");
    }

    #[test]
    fn unlabeled_keys_default_to_the_label_model_and_local_filters_are_named() {
        let model: Vec<String> = ["app", "env", "role"].map(String::from).to_vec();
        // `--unlabeled` alone: clap's missing value is one blank.
        assert_eq!(unlabeled_keys(&["".into()], &model), model);
        assert_eq!(
            unlabeled_keys(&[" env".into(), "os".into(), "env".into()], &model),
            ["env", "os"]
        );
        assert_eq!(local_filter(false, false, None, &[]), None);
        assert_eq!(
            local_filter(true, true, Some(&model), &[]).unwrap(),
            ["unhealthy", "offline", "unlabeled (app, env, role)"]
        );
        let t = LabelTerm::parse("!role=db").unwrap();
        let none = LabelTerm::parse("app=-").unwrap();
        assert_eq!(
            local_filter(false, false, None, &[&t, &none]).unwrap(),
            ["labels !role=db app=-"]
        );
    }

    #[test]
    fn the_enforcement_filter_is_effective() {
        let q = |e: Option<&str>, m: Option<bool>| {
            enforcement_query(e, m).map(|(e, m)| (e.map(str::to_string), m))
        };
        // unmanaged is no VEN: managed=false, no mode sent.
        assert_eq!(q(Some("unmanaged"), None).unwrap(), (None, Some(false)));
        assert_eq!(
            q(Some("unmanaged"), Some(false)).unwrap(),
            (None, Some(false))
        );
        // A mode keeps the unmanaged workloads (which report
        // visibility_only) out: managed=true.
        assert_eq!(
            q(Some("visibility_only"), None).unwrap(),
            (Some("visibility_only".into()), Some(true))
        );
        assert_eq!(
            q(Some("full"), Some(true)).unwrap(),
            (Some("full".into()), Some(true))
        );
        assert_eq!(q(None, None).unwrap(), (None, None));
        assert_eq!(q(None, Some(false)).unwrap(), (None, Some(false)));
        // Contradictions match nothing: an argument error, no request.
        assert!(matches!(
            q(Some("unmanaged"), Some(true)),
            Err(CliError::Args(_))
        ));
        assert!(matches!(
            q(Some("visibility_only"), Some(false)),
            Err(CliError::Args(_))
        ));
    }

    #[test]
    fn rows_print_labels_in_the_profile_key_order() {
        let now: jiff::Timestamp = "2026-10-10T08:00:00Z".parse().unwrap();
        let order = KeyOrder::new(&["env".into()], "role");
        let rows: Vec<WorkloadRow> = fixture()
            .into_iter()
            .map(|w| WorkloadRow::new(w, now, &order))
            .collect();
        let labels = rows.headers().iter().position(|h| *h == "labels").unwrap();
        let table = rows.rows();
        assert_eq!(table[0][labels], "env=prod role=web app=shop");
        let json = serde_json::to_value(&rows[0]).unwrap();
        assert!(json.get("label_text").is_none(), "JSON keeps its shape");
        assert!(json["labels"].is_array());
    }

    #[test]
    fn table_rows_hide_ignored_keys_and_json_keeps_them() {
        let now: jiff::Timestamp = "2026-10-10T08:00:00Z".parse().unwrap();
        let order = KeyOrder::new(&["app".into()], "role").with_ignored(&["env".into()]);
        let rows: Vec<WorkloadRow> = fixture()
            .into_iter()
            .map(|w| WorkloadRow::new(w, now, &order))
            .collect();
        let labels = rows.headers().iter().position(|h| *h == "labels").unwrap();
        assert_eq!(rows.rows()[0][labels], "app=shop role=web");
        let json = serde_json::to_value(&rows[0]).unwrap();
        let keys: Vec<&str> = json["labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|l| l["key"].as_str())
            .collect();
        assert!(keys.contains(&"env"), "{keys:?}");
    }
}
