//! Labels, label groups, IP lists, services: one cached list each.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{TableRows, join_list};
use super::write::{
    self, WriteFlags, parse_kv, resolve_ip_list, resolve_label, resolve_service, run_plan,
};
use clap::{Args, Subcommand};
use pce::models::{Fqdn, IpList, Label, LabelGroup, Service};
use pce::models::{PolicyScope, WorkloadFilter};
use pce::write::{
    IfMissing, IpListDraft, IpListPatch, IpRangeSpec, LabelDraft, PortSpec, ServiceDraft,
    ServicePatch,
};
use pce::{IpListSize, KeyOrder, LabelUsage, ip_list_size, label_usage};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Args, Debug, Clone, Default)]
pub struct LabelsArgs {
    /// Exact label key, e.g. app (repeatable: labels of any of them)
    #[arg(long)]
    pub key: Vec<String>,
    /// Exact label value
    #[arg(long)]
    pub value: Option<String>,
    /// Label value containing this text, case-insensitively
    #[arg(long)]
    pub value_contains: Option<String>,
    /// Only labels no workload, rule, ruleset scope or label group uses;
    /// reads the workloads, the draft rulesets and the label groups and adds
    /// workloads and rules counts per label
    #[arg(long)]
    pub unused: bool,
    /// Hide labels of the keys the profile's ignore_keys lists
    #[arg(long, overrides_with = "no_ignore_keys")]
    pub ignore_keys: bool,
    /// Show labels of every key (the default)
    #[arg(long, overrides_with = "ignore_keys")]
    pub no_ignore_keys: bool,
    /// Include labels the PCE marks deleted
    #[arg(long)]
    pub include_deleted: bool,
}

/// `labels` lists by default; `add`, `update` and `delete` write.
#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct LabelsCmd {
    #[command(subcommand)]
    pub action: Option<LabelsAction>,
    #[command(flatten)]
    pub list: LabelsArgs,
}

#[derive(Subcommand, Debug)]
pub enum LabelsAction {
    /// Create a label (needs writes = true in the profile)
    Add(LabelAddArgs),
    /// Change a label's value
    Update(LabelUpdateArgs),
    /// Delete a label
    Delete(LabelDeleteArgs),
}

#[derive(Args, Debug)]
pub struct LabelAddArgs {
    /// key=value
    pub label: String,
    /// Do nothing when a label with exactly this key and value exists
    #[arg(long)]
    pub if_missing: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct LabelUpdateArgs {
    /// Label href or key=value
    pub label: String,
    /// New value
    #[arg(long)]
    pub value: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct LabelDeleteArgs {
    /// Label href or key=value
    pub label: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

pub async fn labels_cmd(ctx: &Context, cmd: &LabelsCmd) -> Result<(), CliError> {
    match &cmd.action {
        None => labels(ctx, &cmd.list).await,
        Some(LabelsAction::Add(a)) => label_add(ctx, a).await,
        Some(LabelsAction::Update(a)) => label_update(ctx, a).await,
        Some(LabelsAction::Delete(a)) => label_delete(ctx, a).await,
    }
}

async fn label_add(ctx: &Context, a: &LabelAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let (k, v) = parse_kv(&a.label)?;
    let draft = LabelDraft::new(k, v);
    let if_missing = if a.if_missing {
        IfMissing::SkipExisting
    } else {
        IfMissing::Create
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_create_label(&draft, if_missing)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn label_update(ctx: &Context, a: &LabelUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = resolve_label(ctx, &a.label).await?;
    let plan = ctx.pce.writer(&guard).plan_update_label(&href, &a.value)?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn label_delete(ctx: &Context, a: &LabelDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = resolve_label(ctx, &a.label).await?;
    let plan = ctx.pce.writer(&guard).plan_delete_label(&href)?;
    run_plan(ctx, &a.write, &guard, plan).await
}

fn pending(update_type: &Option<String>) -> String {
    update_type.clone().unwrap_or_default()
}

impl TableRows for Vec<Label> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["key", "value", "deleted", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter().map(label_row).collect()
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.iter().map(label_full_row).collect()
    }
}

/// `label_row` with `deleted` as `true`/`false`.
fn label_full_row(l: &Label) -> Vec<String> {
    let mut row = label_row(l);
    row[2] = l.is_deleted().to_string();
    row
}

/// `key, value, deleted, href` of one label.
fn label_row(l: &Label) -> Vec<String> {
    vec![
        l.key.clone(),
        l.value.clone(),
        if l.is_deleted() {
            "yes".into()
        } else {
            String::new()
        },
        l.href.clone(),
    ]
}

impl TableRows for Vec<LabelGroup> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "key", "labels", "sub_groups", "pending", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|g| {
                vec![
                    g.name.clone(),
                    g.key.clone(),
                    g.labels.len().to_string(),
                    g.sub_groups.len().to_string(),
                    pending(&g.update_type),
                    g.href.clone(),
                ]
            })
            .collect()
    }
    /// The member hrefs themselves, as JSON has them, not their counts.
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|g| {
                vec![
                    g.name.clone(),
                    g.key.clone(),
                    join_list(g.labels.iter().map(|r| &r.href)),
                    join_list(g.sub_groups.iter().map(|r| &r.href)),
                    pending(&g.update_type),
                    g.href.clone(),
                ]
            })
            .collect()
    }
}

/// An IP list as `iplists` prints it: the list as the PCE returned it plus
/// its `size` (no request).
#[derive(Debug, Serialize)]
pub struct IpListOut {
    #[serde(flatten)]
    pub list: IpList,
    pub size: IpListSize,
}

impl TableRows for Vec<IpListOut> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "size", "ranges", "pending", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|o| {
                vec![
                    o.list.name.clone(),
                    o.size.display(),
                    o.list.summary(),
                    pending(&o.list.update_type),
                    o.list.href.clone(),
                ]
            })
            .collect()
    }
    /// `size` as the exact address count (`any (0.0.0.0/0)` when unbounded),
    /// the ranges and FQDNs one list entry each.
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|o| {
                let size = if o.size.unbounded.is_some() {
                    o.size.display()
                } else {
                    o.size.total().to_string()
                };
                vec![
                    o.list.name.clone(),
                    size,
                    join_list(o.list.entries()),
                    pending(&o.list.update_type),
                    o.list.href.clone(),
                ]
            })
            .collect()
    }
}

impl TableRows for Vec<Service> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "ports", "pending", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|s| {
                vec![
                    s.name.clone(),
                    s.summary(),
                    pending(&s.update_type),
                    s.href.clone(),
                ]
            })
            .collect()
    }
    /// One list entry per port; no `-` for a service without ports.
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|s| {
                vec![
                    s.name.clone(),
                    join_list(s.service_ports.iter().map(|p| p.display())),
                    pending(&s.update_type),
                    s.href.clone(),
                ]
            })
            .collect()
    }
}

/// A label as `labels --unused` prints it: the label as the PCE returned
/// it plus how many workloads carry it and how many rules name it.
#[derive(Debug, Serialize)]
pub struct LabelOut {
    #[serde(flatten)]
    pub label: Label,
    pub workloads: usize,
    pub rules: usize,
}

impl TableRows for Vec<LabelOut> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["key", "value", "workloads", "rules", "deleted", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|o| label_out_row(o, label_row(&o.label)))
            .collect()
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|o| label_out_row(o, label_full_row(&o.label)))
            .collect()
    }
}

/// A label's row with its two counts after `value`.
fn label_out_row(o: &LabelOut, mut row: Vec<String>) -> Vec<String> {
    row.splice(2..2, [o.workloads.to_string(), o.rules.to_string()]);
    row
}

/// Whether `l` passes the list flags (all of them; `--key`s are any of):
/// `order` knows the profile's ignored keys. `--unused` is applied apart.
fn keep_label(l: &Label, args: &LabelsArgs, order: &KeyOrder) -> bool {
    let contains = args.value_contains.as_deref().map(str::to_lowercase);
    (args.include_deleted || !l.is_deleted())
        && (args.key.is_empty() || args.key.contains(&l.key))
        && args.value.as_deref().is_none_or(|v| l.value == v)
        && contains.is_none_or(|c| l.value.to_lowercase().contains(&c))
        && (!args.ignore_keys || order.shows(&l.key))
}

/// The labels `usage` says nothing uses, with their (zero) counts.
fn unused_labels(labels: Vec<Label>, usage: &HashMap<String, LabelUsage>) -> Vec<LabelOut> {
    labels
        .into_iter()
        .filter_map(|label| {
            let u = usage.get(&label.href).copied().unwrap_or_default();
            (!u.is_used()).then_some(LabelOut {
                label,
                workloads: u.workloads,
                rules: u.rules,
            })
        })
        .collect()
}

pub async fn labels(ctx: &Context, args: &LabelsArgs) -> Result<(), CliError> {
    let c = ctx.pce.labels(ctx.cache_mode).await?;
    let order = ctx.config.key_order();
    let data: Vec<Label> = c
        .value
        .into_iter()
        .filter(|l| keep_label(l, args, &order))
        .collect();
    let cache = cache_label(c.hit, ctx.cache_mode);
    if !args.unused {
        return ctx.emit(&data, cache, c.truncated, None);
    }
    // Usage: three more cached reads, counted in `meta`.
    let w = ctx
        .pce
        .workloads(&WorkloadFilter::default(), ctx.cache_mode)
        .await?;
    let sets = ctx
        .pce
        .rule_sets(PolicyScope::Draft, ctx.cache_mode)
        .await?;
    let groups = ctx.pce.label_groups(ctx.cache_mode).await?;
    // Inline deny rules only: the per-ruleset fallback would cost a read each.
    let usage = label_usage(&w.value, &sets.value, [], &groups.value);
    let truncated = c.truncated || w.truncated || sets.truncated || groups.truncated;
    ctx.emit(&unused_labels(data, &usage), cache, truncated, None)
}

pub async fn label_groups(ctx: &Context) -> Result<(), CliError> {
    let c = ctx.pce.label_groups(ctx.cache_mode).await?;
    ctx.emit(
        &c.value,
        cache_label(c.hit, ctx.cache_mode),
        c.truncated,
        None,
    )
}

#[derive(Args, Debug)]
pub struct IplistsCmd {
    #[command(subcommand)]
    pub action: Option<IplistsAction>,
}

#[derive(Subcommand, Debug)]
pub enum IplistsAction {
    /// Create an IP list in draft (needs writes = true in the profile)
    Add(IpListAddArgs),
    /// Change an IP list's name, description, ranges or FQDNs; a changed list is re-sent whole
    Update(IpListUpdateArgs),
    /// Delete an IP list from draft
    Delete(IpListDeleteArgs),
}

#[derive(Args, Debug)]
pub struct IpListAddArgs {
    /// IP list name
    pub name: String,
    /// Range: A, A-B or A/N (repeatable)
    #[arg(long = "range")]
    pub ranges: Vec<String>,
    /// Excluded range, same forms (repeatable)
    #[arg(long = "exclude")]
    pub excludes: Vec<String>,
    /// FQDN, wildcards allowed (repeatable)
    #[arg(long = "fqdn")]
    pub fqdns: Vec<String>,
    /// Description
    #[arg(long)]
    pub description: Option<String>,
    /// Do nothing when an IP list with exactly this name exists
    #[arg(long)]
    pub if_missing: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct IpListUpdateArgs {
    /// IP list name or href
    pub target: String,
    /// New name
    #[arg(long)]
    pub name: Option<String>,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    /// Range to add (repeatable)
    #[arg(long = "add-range")]
    pub add_ranges: Vec<String>,
    /// Range to remove; must match an existing range exactly (repeatable)
    #[arg(long = "remove-range")]
    pub remove_ranges: Vec<String>,
    /// Excluded range to add (repeatable)
    #[arg(long = "add-exclude")]
    pub add_excludes: Vec<String>,
    /// Excluded range to remove (repeatable)
    #[arg(long = "remove-exclude")]
    pub remove_excludes: Vec<String>,
    /// FQDN to add (repeatable)
    #[arg(long = "add-fqdn")]
    pub add_fqdns: Vec<String>,
    /// FQDN to remove (repeatable)
    #[arg(long = "remove-fqdn")]
    pub remove_fqdns: Vec<String>,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct IpListDeleteArgs {
    /// IP list name or href
    pub target: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

fn parse_ranges(specs: &[String], exclusion: bool) -> Result<Vec<IpRangeSpec>, CliError> {
    specs
        .iter()
        .map(|s| IpRangeSpec::parse(s, exclusion).map_err(CliError::Args))
        .collect()
}

pub async fn ip_lists_cmd(ctx: &Context, cmd: &IplistsCmd) -> Result<(), CliError> {
    match &cmd.action {
        None => ip_lists(ctx).await,
        Some(IplistsAction::Add(a)) => ip_list_add(ctx, a).await,
        Some(IplistsAction::Update(a)) => ip_list_update(ctx, a).await,
        Some(IplistsAction::Delete(a)) => ip_list_delete(ctx, a).await,
    }
}

async fn ip_list_add(ctx: &Context, a: &IpListAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let mut ranges = parse_ranges(&a.ranges, false)?;
    ranges.extend(parse_ranges(&a.excludes, true)?);
    let draft = IpListDraft {
        name: a.name.clone(),
        description: a.description.clone(),
        ranges,
        fqdns: a.fqdns.iter().map(Fqdn::new).collect(),
    };
    let if_missing = if a.if_missing {
        IfMissing::SkipExisting
    } else {
        IfMissing::Create
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_create_ip_list(&draft, if_missing)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn ip_list_update(ctx: &Context, a: &IpListUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let mut add_ranges = parse_ranges(&a.add_ranges, false)?;
    add_ranges.extend(parse_ranges(&a.add_excludes, true)?);
    let mut remove_ranges = parse_ranges(&a.remove_ranges, false)?;
    remove_ranges.extend(parse_ranges(&a.remove_excludes, true)?);
    let patch = IpListPatch {
        name: a.name.clone(),
        description: a.description.clone(),
        add_ranges,
        remove_ranges,
        add_fqdns: a.add_fqdns.clone(),
        remove_fqdns: a.remove_fqdns.clone(),
    };
    if patch.is_empty() {
        return Err(CliError::Args(
            "nothing to change; pass --name, --description, --add-range, --remove-range, --add-exclude, --remove-exclude, --add-fqdn or --remove-fqdn".into(),
        ));
    }
    let href = resolve_ip_list(ctx, &a.target).await?;
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_update_ip_list(&href, &patch)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn ip_list_delete(ctx: &Context, a: &IpListDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = resolve_ip_list(ctx, &a.target).await?;
    let plan = ctx.pce.writer(&guard).plan_delete_ip_list(&href)?;
    run_plan(ctx, &a.write, &guard, plan).await
}

pub async fn ip_lists(ctx: &Context) -> Result<(), CliError> {
    let c = ctx.pce.ip_lists(ctx.cache_mode).await?;
    let data: Vec<IpListOut> = c
        .value
        .into_iter()
        .map(|list| IpListOut {
            size: ip_list_size(&list),
            list,
        })
        .collect();
    ctx.emit(&data, cache_label(c.hit, ctx.cache_mode), c.truncated, None)
}

#[derive(Args, Debug)]
pub struct ServicesCmd {
    #[command(subcommand)]
    pub action: Option<ServicesAction>,
}

#[derive(Subcommand, Debug)]
pub enum ServicesAction {
    /// Create a service in draft (needs writes = true in the profile)
    Add(ServiceAddArgs),
    /// Change a service's name, description or ports; a changed port list is re-sent whole
    Update(ServiceUpdateArgs),
    /// Delete a service from draft
    Delete(ServiceDeleteArgs),
}

#[derive(Args, Debug)]
pub struct ServiceAddArgs {
    /// Service name
    pub name: String,
    /// Port: P/PROTO, P-Q/PROTO, or a protocol name such as icmp (repeatable, at least one)
    #[arg(long = "port", required = true)]
    pub ports: Vec<String>,
    /// Description
    #[arg(long)]
    pub description: Option<String>,
    /// Do nothing when a service with exactly this name exists
    #[arg(long)]
    pub if_missing: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct ServiceUpdateArgs {
    /// Service name or href
    pub target: String,
    /// New name
    #[arg(long)]
    pub name: Option<String>,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    /// Port to add (repeatable)
    #[arg(long = "add-port")]
    pub add_ports: Vec<String>,
    /// Port to remove; must match an existing entry exactly (repeatable)
    #[arg(long = "remove-port")]
    pub remove_ports: Vec<String>,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct ServiceDeleteArgs {
    /// Service name or href
    pub target: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

fn parse_ports(specs: &[String]) -> Result<Vec<PortSpec>, CliError> {
    specs
        .iter()
        .map(|s| PortSpec::parse(s).map_err(CliError::Args))
        .collect()
}

pub async fn services_cmd(ctx: &Context, cmd: &ServicesCmd) -> Result<(), CliError> {
    match &cmd.action {
        None => services(ctx).await,
        Some(ServicesAction::Add(a)) => service_add(ctx, a).await,
        Some(ServicesAction::Update(a)) => service_update(ctx, a).await,
        Some(ServicesAction::Delete(a)) => service_delete(ctx, a).await,
    }
}

async fn service_add(ctx: &Context, a: &ServiceAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let draft = ServiceDraft {
        name: a.name.clone(),
        description: a.description.clone(),
        ports: parse_ports(&a.ports)?,
    };
    let if_missing = if a.if_missing {
        IfMissing::SkipExisting
    } else {
        IfMissing::Create
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_create_service(&draft, if_missing)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn service_update(ctx: &Context, a: &ServiceUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let patch = ServicePatch {
        name: a.name.clone(),
        description: a.description.clone(),
        add_ports: parse_ports(&a.add_ports)?,
        remove_ports: parse_ports(&a.remove_ports)?,
    };
    if patch.is_empty() {
        return Err(CliError::Args(
            "nothing to change; pass --name, --description, --add-port or --remove-port".into(),
        ));
    }
    let href = resolve_service(ctx, &a.target).await?;
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_update_service(&href, &patch)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn service_delete(ctx: &Context, a: &ServiceDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = resolve_service(ctx, &a.target).await?;
    let plan = ctx.pce.writer(&guard).plan_delete_service(&href)?;
    run_plan(ctx, &a.write, &guard, plan).await
}

pub async fn services(ctx: &Context) -> Result<(), CliError> {
    let c = ctx.pce.services(ctx.cache_mode).await?;
    ctx.emit(
        &c.value,
        cache_label(c.hit, ctx.cache_mode),
        c.truncated,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<Label> {
        serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap()
    }

    /// The fixture labels `args` keeps, as `key=value`.
    fn kept(args: &LabelsArgs, order: &KeyOrder) -> Vec<String> {
        fixture()
            .iter()
            .filter(|l| keep_label(l, args, order))
            .map(|l| format!("{}={}", l.key, l.value))
            .collect()
    }

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn list_flags_combine_with_and_and_keys_with_or() {
        let order = KeyOrder::new(&keys(&["app", "env"]), "role").with_ignored(&keys(&["loc"]));
        let none = LabelsArgs::default();
        assert_eq!(
            kept(&none, &order),
            vec!["role=web", "role=db", "app=shop", "env=prod", "loc=eu"],
            "deleted labels hidden; ignored keys shown by default"
        );
        let by_keys = LabelsArgs {
            key: keys(&["role", "env"]),
            ..LabelsArgs::default()
        };
        assert_eq!(
            kept(&by_keys, &order),
            vec!["role=web", "role=db", "env=prod"]
        );
        let contains = LabelsArgs {
            value_contains: Some("E".into()),
            ..LabelsArgs::default()
        };
        assert_eq!(kept(&contains, &order), vec!["role=web", "loc=eu"]);
        let both = LabelsArgs {
            key: keys(&["role", "env"]),
            value_contains: Some("e".into()),
            ..LabelsArgs::default()
        };
        assert_eq!(kept(&both, &order), vec!["role=web"]);
        let exact = LabelsArgs {
            key: keys(&["role"]),
            value: Some("db".into()),
            ..LabelsArgs::default()
        };
        assert_eq!(kept(&exact, &order), vec!["role=db"]);
        let ignore = LabelsArgs {
            ignore_keys: true,
            ..LabelsArgs::default()
        };
        assert_eq!(
            kept(&ignore, &order),
            vec!["role=web", "role=db", "app=shop", "env=prod"]
        );
        let deleted = LabelsArgs {
            key: keys(&["app"]),
            include_deleted: true,
            ..LabelsArgs::default()
        };
        assert_eq!(kept(&deleted, &order), vec!["app=shop", "app="]);
    }

    #[test]
    fn unused_keeps_the_labels_nothing_uses_with_their_counts() {
        let workloads: Vec<pce::models::Workload> =
            serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
        let sets: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        let groups: Vec<LabelGroup> =
            serde_json::from_str(include_str!("../../../../fixtures/label_groups.json")).unwrap();
        let usage = label_usage(&workloads, &sets, [], &groups);
        let out = unused_labels(fixture(), &usage);
        let hrefs: Vec<&str> = out.iter().map(|o| o.label.href.as_str()).collect();
        assert_eq!(hrefs, vec!["/orgs/5/labels/5", "/orgs/5/labels/6"]);
        let json = serde_json::to_value(&out[0]).unwrap();
        assert_eq!(json["key"], "loc", "the label's own fields stay flat");
        assert_eq!(
            (json["workloads"].as_u64(), json["rules"].as_u64()),
            (Some(0), Some(0))
        );
        assert_eq!(
            out.headers(),
            vec!["key", "value", "workloads", "rules", "deleted", "href"]
        );
        assert_eq!(
            out.rows()[1],
            vec!["app", "", "0", "0", "yes", "/orgs/5/labels/6"]
        );
        assert!(unused_labels(fixture(), &HashMap::new()).len() == 6);
    }
}
