//! `workloads`: list workloads, with `--label key=value` filters resolved to
//! hrefs via the name index before querying.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{MetaCompare, SnapshotCompareMeta, SnapshotCounts, TableRows, join_list};
use super::write::{
    self, WriteFlags, label_in, parse_kv, resolve_labels, resolve_workload, run_plan,
};
use crate::snapshot::baseline::{WlChange, compare_workloads};
use clap::{Args, Subcommand};
use pce::label_filter::{
    LabelFilter, LabelTerm, MAX_COMBINATIONS, QueryError, group_filter, group_key, group_title,
};
use pce::models::{UNMANAGED, VenHealth, Workload, WorkloadFilter};
use pce::traffic::{Change, ChangeSet, CompareCounts};
use pce::ven::{HealthSet, VenSummary, VenTally};
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
    /// Only workloads whose VEN health is one of these (comma-separated, any
    /// case; local filter, the app's `v`). With --offline or --unhealthy
    /// their states are added to these (health is one filter)
    #[arg(
        long,
        value_name = "STATES",
        value_delimiter = ',',
        ignore_case = true,
        value_parser = ["healthy", "warning", "error", "offline", "unmanaged"]
    )]
    pub health: Vec<String>,
    /// Only workloads whose VEN reports this version: exact, or a `*` glob
    /// such as 23.2.* (local filter)
    #[arg(long, value_name = "VERSION")]
    pub ven_version: Option<String>,
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
            || !self.health.is_empty()
            || self.ven_version.is_some()
            || self.unlabeled.is_some()
    }

    /// `--health` as a set (clap checked the names).
    pub fn health_set(&self) -> HealthSet {
        HealthSet::from_names(&self.health).unwrap_or_default()
    }

    /// The health filter in force as `(unhealthy, offline, health)`: health
    /// is one dimension, so with `--health` the states of `--offline` and
    /// `--unhealthy` join its set (a union) and those two are off.
    pub fn health_filter(&self) -> (bool, bool, HealthSet) {
        let mut set = self.health_set();
        if set.is_empty() {
            return (self.unhealthy, self.offline, set);
        }
        if self.offline {
            set.insert(VenHealth::Offline);
        }
        if self.unhealthy {
            for h in [VenHealth::Warning, VenHealth::Error, VenHealth::Offline] {
                set.insert(h);
            }
        }
        (false, false, set)
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
    /// Compare the listed workloads with a snapshot's (`snapshot save`): latest, previous, or
    /// its time (a prefix is enough), among the profile's snapshots that hold workloads. Rows
    /// gain change (new, changed, gone, same) and was (a changed one's transitions: app:
    /// shop→cart · mode: visibility→selective · ven: active→stopped · health:
    /// healthy→offline); a gone one has the snapshot's values. The filters apply to both
    /// sides. Reads the file only, no extra request; another PCE's snapshot is refused (exit 2).
    /// JSON: meta.compare names the snapshot
    #[arg(long, value_name = "SNAPSHOT")]
    pub baseline_snapshot: Option<String>,
    /// With --baseline-snapshot: the rows to print, any of new,changed,gone,same (default
    /// new,changed,gone)
    #[arg(long, value_name = "LIST", requires = "baseline_snapshot")]
    pub changes: Option<String>,
    #[command(flatten)]
    pub saved: super::filters::SavedFilterArgs,
}

impl WorkloadsArgs {
    /// The saved filter `s` as the base: its labels where no `--label` is
    /// given, its enforcement, managed, offline, unlabeled, health and VEN
    /// version where those flags are not.
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
        if self.health.is_empty()
            && let Some(h) = &s.health
        {
            self.health = HealthSet::from_names(h)
                .map_err(|e| format!("field health: {e}"))?
                .names();
        }
        if self.ven_version.is_none() {
            self.ven_version = s.ven_version.clone();
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
            health: Some(self.health_set().names()).filter(|h| !h.is_empty()),
            ven_version: self
                .ven_version
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(String::from),
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
    /// The VEN fleet of the workloads the filters select: counts per health
    /// state, VEN version (newest first, how many behind), enforcement and
    /// health message. The one workloads read `workloads` makes (cached);
    /// nothing else. CSV: one row per bucket with a section column
    VenSummary(VenSummaryArgs),
}

#[derive(Args, Debug)]
pub struct VenSummaryArgs {
    #[command(flatten)]
    pub select: WorkloadsArgs,
    #[command(flatten)]
    pub saved: super::filters::SavedFilterArgs,
}

impl WorkloadsCmd {
    /// The list filters and the saved filter flags: of the list, or of
    /// `ven-summary`; `None` for the other subcommands.
    pub fn filter_args(&self) -> Option<(&WorkloadsArgs, &super::filters::SavedFilterArgs)> {
        match &self.action {
            None => Some((&self.list, &self.saved)),
            Some(WorkloadsAction::VenSummary(a)) => Some((&a.select, &a.saved)),
            Some(_) => None,
        }
    }

    pub fn filter_args_mut(&mut self) -> Option<&mut WorkloadsArgs> {
        match &mut self.action {
            None => Some(&mut self.list),
            Some(WorkloadsAction::VenSummary(a)) => Some(&mut a.select),
            Some(_) => None,
        }
    }
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
        None => {
            let base = baseline(ctx, cmd)?;
            run(ctx, &cmd.list, cmd.group.as_deref(), base.as_ref()).await
        }
        Some(WorkloadsAction::Add(a)) => workload_add(ctx, a).await,
        Some(WorkloadsAction::Update(a)) => workload_update(ctx, a).await,
        Some(WorkloadsAction::Delete(a)) => workload_delete(ctx, a).await,
        Some(WorkloadsAction::Enforce(a)) => super::bulk::enforce(ctx, a).await,
        Some(WorkloadsAction::Relabel(a)) => super::bulk::relabel(ctx, a).await,
        Some(WorkloadsAction::VenSummary(a)) => ven_summary(ctx, &a.select).await,
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
    /// With `--baseline-snapshot`: `change` and `was`.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub compare: Option<RowCompare>,
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
            compare: None,
            health,
            workload,
        }
    }
}

/// A row's change against the snapshot (`--baseline-snapshot`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RowCompare {
    /// `new`, `changed`, `gone` (the snapshot's values) or `same`.
    pub change: Change,
    /// A changed row's transitions (`app: shop→cart · mode:
    /// visibility→selective`); `null` on any other.
    pub was: Option<String>,
}

impl RowCompare {
    pub fn of(c: &WlChange) -> Self {
        Self {
            change: c.change,
            was: (c.change == Change::Changed).then(|| c.was_text()),
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

impl Rows<'_> {
    /// Against a snapshot: the `change` and `was` columns last.
    fn compared(&self) -> bool {
        self.0.iter().any(|r| r.compare.is_some())
    }
}

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
        if self.compared() {
            h.extend(["change", "was"]);
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let missing = self.0.iter().any(|r| r.missing_labels.is_some());
        let compared = self.compared();
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
                if compared {
                    let c = r.compare.as_ref();
                    row.push(c.map(|c| c.change.as_str()).unwrap_or_default().into());
                    row.push(c.and_then(|c| c.was.clone()).unwrap_or_default());
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
                    && let Some(cell) = row.get_mut(11)
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
    /// Against a baseline: the rows per change (the filters applied).
    pub counts: Option<CompareCounts>,
    /// Against a baseline with the list read cut by its row cap: how many
    /// were read. No row is then called gone (one past the cap would).
    pub gone_withheld: Option<usize>,
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
    select_against(ctx, args, mode, None).await
}

/// [`select`], and with a baseline (a snapshot's workloads and when it was
/// taken) each row's change against it, the baseline's workloads the
/// answer lacks added as `gone` rows (the snapshot's values, its time's
/// health) when the same filters keep them.
pub async fn select_against(
    ctx: &Context,
    args: &WorkloadsArgs,
    mode: pce::CacheMode,
    base: Option<(&[Workload], jiff::Timestamp)>,
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
    // Against a snapshot every filter is applied here, to both sides: the
    // one read is unfiltered (as `snapshot save` reads it), so a workload
    // the filters no longer match is never taken for gone.
    let against = base.is_some();
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
    if against {
        filter = WorkloadFilter {
            max_results: args.max_results,
            ..WorkloadFilter::default()
        };
    }
    // One parser for `--label` and the app's `f`; the positive any-of
    // terms go into the one request, `!` and `-` are applied to its answer.
    let labels = LabelFilter::from_terms(&args.labels)
        .map_err(|e| CliError::Args(format!("--label {e}")))?;
    // Names spelt as the labels are (`app=ordering` finds `Ordering`, #90),
    // for the request and the local checks alike. The labels list alone
    // does it (one read, cached); a filter with no value reads nothing.
    let labels_idx = if labels.terms.iter().any(|t| !t.values.is_empty()) {
        let labels_list = ctx.pce.labels(ctx.cache_mode).await?.value;
        Some(pce::NameIndex::new(&labels_list, &[], &[], &[]))
    } else {
        None
    };
    // The terms as typed are what `meta` names.
    let typed = labels;
    let labels = match &labels_idx {
        Some(idx) => typed
            .resolved(idx)
            .map_err(|e| CliError::Args(format!("--label {e}")))?,
        None => typed.clone(),
    };
    // `app=shop app=crm`: no workload has both, so nothing is asked (an
    // empty labels parameter would mean every workload).
    let nothing = labels.matches_nothing();
    if let Some(idx) = &labels_idx
        && !against
        && !nothing
        && labels.terms.iter().any(|t| !t.negate && !t.none)
    {
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
    let local_labels: Vec<&LabelTerm> = typed
        .terms
        .iter()
        .filter(|t| against || t.negate || t.none)
        .collect();
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
    let (unhealthy, offline, health) = args.health_filter();
    // Compared on the whole (unfiltered) answer, before the filters: a
    // workload they hide now is not gone.
    let compared = base.map(|(ws, at)| (compare_workloads(&c.value, now, ws, at), at));
    let make = |w: Workload, at: jiff::Timestamp| {
        let mut r = WorkloadRow::new(w, at, &order);
        r.missing_labels = unlabeled
            .as_deref()
            .map(|keys| r.workload.missing_label_keys(keys));
        r
    };
    let keep = |r: &WorkloadRow| {
        // The PCE filtered already; this holds the effective mode should it not.
        args.enforcement
            .as_deref()
            .is_none_or(|e| r.enforcement.as_deref() == Some(e))
            && (!unhealthy
                || matches!(
                    r.health,
                    VenHealth::Warning | VenHealth::Error | VenHealth::Offline
                ))
            // Offline is the VEN health, as the app's `o`.
            && (!offline || r.health == VenHealth::Offline)
            && health.keeps(r.health)
            && args.ven_version.as_deref().is_none_or(|p| {
                r.health != VenHealth::Unmanaged
                    && r.workload
                        .agent_version()
                        .is_some_and(|v| pce::ven::version_matches(p, v))
            })
            && r.missing_labels.as_ref().is_none_or(|m| !m.is_empty())
            // The whole label filter, as the app applies it: the query carried
            // the plain any-of terms, `!` and `-` are only checked here.
            && labels.keeps(&r.workload)
    };
    // Against a snapshot: what the request would have asked the PCE,
    // checked here on both sides (a snapshot keeps no interfaces, so with
    // `--ip` no snapshot workload matches).
    let asked = |w: &Workload| {
        let has = |field: Option<&str>, want: &Option<String>| {
            want.as_deref()
                .is_none_or(|x| field.is_some_and(|f| f.to_lowercase().contains(&x.to_lowercase())))
        };
        !against
            || (has(w.hostname.as_deref(), &args.hostname)
                && has(w.name.as_deref(), &args.name)
                && args
                    .ip
                    .as_deref()
                    .is_none_or(|ip| w.interfaces.iter().any(|i| i.address == ip))
                && args.online.is_none_or(|o| w.online == Some(o))
                && managed.is_none_or(|m| m != w.is_unmanaged()))
    };
    // A list cut by its cap cannot say what is gone.
    let read = c.value.len();
    let withheld = (compared.is_some() && c.truncated).then_some(read);
    let mut rows: Vec<WorkloadRow> = c
        .value
        .into_iter()
        .filter(|w| asked(w))
        .map(|w| make(w, now))
        .filter(|r| keep(r))
        .collect();
    let mut counts = None;
    if let Some((cmp, at)) = compared {
        for r in &mut rows {
            r.compare = cmp.of(&r.workload.href).map(RowCompare::of);
        }
        rows.extend(
            cmp.gone
                .into_iter()
                .filter(|_| withheld.is_none())
                .filter(|w| asked(w))
                .map(|w| make(w, at))
                .filter(|r| keep(r))
                .map(|mut r| {
                    r.compare = Some(RowCompare::of(&WlChange::of(Change::Gone)));
                    r
                }),
        );
        let mut n = CompareCounts::default();
        for c in rows.iter().filter_map(|r| r.compare.as_ref()) {
            n.add(c.change);
        }
        counts = Some(n);
        if args.ip.is_some() {
            warnings.push(
                "--ip: a snapshot keeps no interfaces, so no workload is listed as gone"
                    .to_string(),
            );
        }
    }
    Ok(Selection {
        filter,
        rows,
        counts,
        gone_withheld: withheld,
        hit: c.hit,
        truncated: c.truncated,
        nothing,
        warnings,
        local_filter: local_filter(
            unhealthy,
            offline,
            (health, args.ven_version.as_deref()),
            unlabeled.as_deref(),
            &local_labels,
        ),
    })
}

/// `--baseline-snapshot`'s snapshot and `--changes`, checked and the
/// file read before any request.
pub struct Baseline {
    pub id: String,
    pub snapshot: crate::snapshot::Snapshot,
    pub show: ChangeSet,
}

fn baseline(ctx: &Context, cmd: &WorkloadsCmd) -> Result<Option<Baseline>, CliError> {
    use crate::snapshot::baseline::{BaselineError, load_baseline};
    let Some(spec) = &cmd.baseline_snapshot else {
        return Ok(None);
    };
    let show = match &cmd.changes {
        Some(c) => ChangeSet::parse(c).map_err(|e| CliError::Args(format!("--changes: {e}")))?,
        None => ChangeSet::default(),
    };
    let root = crate::snapshot::default_root().ok_or_else(|| {
        CliError::Other("no data directory for snapshots: set HOME or XDG_DATA_HOME".into())
    })?;
    let dir = crate::snapshot::profile_dir(&root, &ctx.config.profile_name);
    let me = crate::snapshot::Identity::of(&ctx.config.profile_name, ctx.pce.client());
    let (m, snapshot) = load_baseline(&dir, spec, &me).map_err(|e| match e {
        BaselineError::Usage(e) => CliError::Args(format!("--baseline-snapshot: {e}")),
        BaselineError::File(e) => CliError::Other(e),
    })?;
    Ok(Some(Baseline {
        id: m.id,
        snapshot,
        show,
    }))
}

pub async fn run(
    ctx: &Context,
    args: &WorkloadsArgs,
    group: Option<&[String]>,
    base: Option<&Baseline>,
) -> Result<(), CliError> {
    let against = base.map(|b| {
        (
            b.snapshot.sections.workloads.as_deref().unwrap_or_default(),
            b.snapshot.taken_at,
        )
    });
    let mut sel = select_against(ctx, args, ctx.cache_mode, against).await?;
    if let Some(b) = base {
        if b.snapshot.truncated.iter().any(|t| t == "workloads") {
            sel.warnings.push(format!(
                "snapshot {}: its workloads read hit its row cap: new over-reports",
                b.id
            ));
        }
        if let Some(n) = sel.gone_withheld {
            sel.warnings.push(format!(
                "gone not judged: list cut at {n} (the workloads read hit its row cap; --max-results raises it)"
            ));
        }
    }
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
    let mut rows = sel.rows;
    if let Some(b) = base {
        rows.retain(|r| r.compare.as_ref().is_none_or(|c| b.show.contains(c.change)));
        meta.compare = Some(MetaCompare::Snapshot(SnapshotCompareMeta {
            snapshot: b.id.clone(),
            taken_at: b.snapshot.taken_at,
            label: b.snapshot.label.clone(),
            changes: b.show.text(),
            counts: {
                let c = sel.counts.unwrap_or_default();
                SnapshotCounts {
                    new: c.new,
                    changed: c.changed,
                    gone: sel.gone_withheld.is_none().then_some(c.gone),
                    same: c.same,
                }
            },
            truncated: b.snapshot.truncated.iter().any(|t| t == "workloads"),
            list_cut_at: sel.gone_withheld,
        }));
    }
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

/// `workloads ven-summary`: the selection `workloads` would list (one
/// read, cached), summarised. No other request.
async fn ven_summary(ctx: &Context, args: &WorkloadsArgs) -> Result<(), CliError> {
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
    let summary = summarise(&sel.rows, sel.truncated);
    let sections = VenSections(&summary);
    if ctx.format == super::output::OutputFormat::Csv {
        return ctx.emit_report(&summary, &[(String::new(), &sections)], &meta);
    }
    let tables = sections.tables();
    let refs: Vec<(String, &dyn TableRows)> = tables
        .iter()
        .map(|(title, t)| (title.clone(), t as &dyn TableRows))
        .collect();
    ctx.emit_report(&summary, &refs, &meta)
}

/// The fleet of `rows`, their health as listed.
pub fn summarise(rows: &[WorkloadRow], truncated: bool) -> VenSummary {
    let mut t = VenTally::default();
    for r in rows {
        t.add(&r.workload, r.health);
    }
    t.finish(truncated)
}

/// One section of `ven-summary`'s table.
pub struct Section {
    headers: Vec<&'static str>,
    rows: Vec<Vec<String>>,
}

impl TableRows for Section {
    fn headers(&self) -> Vec<&'static str> {
        self.headers.clone()
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.rows.clone()
    }
}

/// `ven-summary` as tables (one per section) and as one CSV with a
/// `section` column: `section,name,count,note`.
pub struct VenSections<'a>(pub &'a VenSummary);

impl VenSections<'_> {
    /// `(section, name, count, note)` rows, in the overlay's order.
    fn entries(&self) -> Vec<(&'static str, String, usize, String)> {
        let s = self.0;
        let mut out: Vec<(&'static str, String, usize, String)> = s
            .health
            .iter()
            .map(|c| {
                (
                    "health",
                    c.health.as_str().to_string(),
                    c.count,
                    String::new(),
                )
            })
            .collect();
        out.extend(s.versions.iter().map(|v| {
            let note = if v.newest { "newest" } else { "behind" };
            ("version", v.version.clone(), v.count, note.to_string())
        }));
        if s.no_version > 0 {
            out.push(("version", String::new(), s.no_version, "no version".into()));
        }
        out.extend(
            s.enforcement
                .iter()
                .map(|e| ("enforcement", e.enforcement.clone(), e.count, String::new())),
        );
        out.extend(s.messages.iter().map(|m| {
            (
                "message",
                m.message.clone(),
                m.count,
                m.severity.to_string(),
            )
        }));
        out
    }

    /// One titled table per section.
    pub fn tables(&self) -> Vec<(String, Section)> {
        let s = self.0;
        let entries = self.entries();
        let of = |section: &str, note: bool| -> Vec<Vec<String>> {
            entries
                .iter()
                .filter(|e| e.0 == section)
                .map(|(_, name, n, extra)| {
                    let mut row = vec![name.clone(), n.to_string()];
                    if note {
                        row.push(extra.clone());
                    }
                    row
                })
                .collect()
        };
        let loaded = if s.truncated {
            format!(
                "{} VENs of {} workloads loaded (capped)",
                s.vens, s.workloads
            )
        } else {
            format!("{} VENs · {} workloads", s.vens, s.workloads)
        };
        let versions = match &s.newest {
            Some(v) => format!("versions · newest {v} · {} behind", s.behind),
            None => "versions".to_string(),
        };
        vec![
            (
                format!("health · {loaded}"),
                Section {
                    headers: vec!["health", "count"],
                    rows: of("health", false),
                },
            ),
            (
                versions,
                Section {
                    headers: vec!["version", "count", "note"],
                    rows: of("version", true),
                },
            ),
            (
                "enforcement".to_string(),
                Section {
                    headers: vec!["enforcement", "count"],
                    rows: of("enforcement", false),
                },
            ),
            (
                "messages".to_string(),
                Section {
                    headers: vec!["message", "count", "severity"],
                    rows: of("message", true),
                },
            ),
        ]
    }
}

impl TableRows for VenSections<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["section", "name", "count", "note"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.entries()
            .into_iter()
            .map(|(section, name, n, note)| vec![section.to_string(), name, n.to_string(), note])
            .collect()
    }
}

/// `meta.local_filter`: the filters applied here after the one request,
/// as the app's filter line names them.
fn local_filter(
    unhealthy: bool,
    offline: bool,
    (health, ven_version): (HealthSet, Option<&str>),
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
    if !health.is_empty() {
        out.push(format!("health {health}"));
    }
    if let Some(v) = ven_version {
        out.push(format!("ven {v}"));
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
        let none = (HealthSet::default(), None);
        assert_eq!(local_filter(false, false, none, None, &[]), None);
        assert_eq!(
            local_filter(true, true, none, Some(&model), &[]).unwrap(),
            ["unhealthy", "offline", "unlabeled (app, env, role)"]
        );
        let ven = (HealthSet::parse("error,warning").unwrap(), Some("23.2.*"));
        assert_eq!(
            local_filter(false, false, ven, None, &[]).unwrap(),
            ["health warning,error", "ven 23.2.*"]
        );
        let t = LabelTerm::parse("!role=db").unwrap();
        let none = LabelTerm::parse("app=-").unwrap();
        assert_eq!(
            local_filter(
                false,
                false,
                (HealthSet::default(), None),
                None,
                &[&t, &none]
            )
            .unwrap(),
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
