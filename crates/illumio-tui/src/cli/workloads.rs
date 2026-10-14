//! `workloads`: list workloads, with `--label key=value` filters resolved to
//! hrefs via the name index before querying.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{TableRows, join_list};
use super::write::{
    self, WriteFlags, label_in, parse_kv, resolve_labels, resolve_workload, run_plan,
};
use clap::{Args, Subcommand};
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
    /// Label key=value; repeat to AND several
    #[arg(long = "label")]
    pub labels: Vec<String>,
    /// full | visibility_only | selective | idle | unmanaged. Effective
    /// enforcement: unmanaged (no VEN) asks for managed=false; a mode also
    /// asks for managed=true, as the PCE reports workloads without a VEN as
    /// visibility_only
    #[arg(long)]
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
}

#[derive(Subcommand, Debug)]
pub enum WorkloadsAction {
    /// Create an unmanaged workload (needs writes = true in the profile)
    Add(WorkloadAddArgs),
    /// Change an unmanaged workload's name, hostname, interfaces or labels; with --managed-labels, a VEN-managed workload's labels
    Update(WorkloadUpdateArgs),
    /// Delete an unmanaged workload
    Delete(WorkloadDeleteArgs),
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
        None => run(ctx, &cmd.list).await,
        Some(WorkloadsAction::Add(a)) => workload_add(ctx, a).await,
        Some(WorkloadsAction::Update(a)) => workload_update(ctx, a).await,
        Some(WorkloadsAction::Delete(a)) => workload_delete(ctx, a).await,
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
            health,
            workload,
        }
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

impl TableRows for Vec<WorkloadRow> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
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
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|r| {
                let w = &r.workload;
                vec![
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
                ]
            })
            .collect()
    }
    /// The heartbeat age in seconds, as JSON's `heartbeat_age_secs`.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        h[8] = "heartbeat_age_secs";
        h
    }
    /// Every interface address and every label; the heartbeat in seconds.
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .zip(self.rows())
            .map(|(r, mut row)| {
                row[2] = join_list(r.workload.interfaces.iter().map(|i| &i.address));
                row[3] = r.label_list.clone();
                row[8] = r
                    .heartbeat_age_secs
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                row
            })
            .collect()
    }
}

/// `--enforcement` (effective) and `--managed` as the PCE's
/// `(enforcement_mode, managed)` query: `unmanaged` is `managed=false` with
/// no mode; a mode adds `managed=true`, since the PCE reports workloads
/// without a VEN as `visibility_only`. A contradiction is an argument error.
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

pub async fn run(ctx: &Context, args: &WorkloadsArgs) -> Result<(), CliError> {
    let (enforcement_mode, managed) = enforcement_query(args.enforcement.as_deref(), args.managed)?;
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
    if !args.labels.is_empty() {
        let idx = ctx.pce.name_index(ctx.cache_mode).await?;
        let mut hrefs = Vec::new();
        for spec in &args.labels {
            let (k, v) = spec.split_once('=').ok_or_else(|| {
                CliError::Args(format!("--label expects key=value, got {spec:?}"))
            })?;
            let href = idx.label_href(k.trim(), v.trim()).ok_or_else(|| {
                let known = idx.values_for_key(k.trim());
                PceError::Invalid(format!(
                    "unknown label {spec}; known values for {}: {}",
                    k.trim(),
                    known.join(", ")
                ))
            })?;
            hrefs.push(href.to_string());
        }
        filter.labels = vec![hrefs];
    }
    let c = ctx.pce.workloads(&filter, ctx.cache_mode).await?;
    let now = jiff::Timestamp::now();
    let order = ctx.config.key_order();
    let rows: Vec<WorkloadRow> = c
        .value
        .into_iter()
        .map(|w| WorkloadRow::new(w, now, &order))
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
        .collect();
    ctx.emit(&rows, cache_label(c.hit, ctx.cache_mode), c.truncated, None)
}

#[cfg(test)]
mod tests {
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
        let json = serde_json::to_value(&rows[2]).unwrap();
        assert_eq!(json["enforcement"], "unmanaged");
        assert_eq!(json["enforcement_mode"], "visibility_only");
        let json = serde_json::to_value(&rows[0]).unwrap();
        assert_eq!(json["enforcement"], "full");
        assert_eq!(json["enforcement_mode"], "full");
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
