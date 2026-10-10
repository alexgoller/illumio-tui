//! Labels, label groups, IP lists, services: one cached list each.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::TableRows;
use super::write::{
    self, WriteFlags, parse_kv, resolve_ip_list, resolve_label, resolve_service, run_plan,
};
use clap::{Args, Subcommand};
use pce::models::{IpList, Label, LabelGroup, Service};
use pce::write::{
    IfMissing, IpListDraft, IpListPatch, IpRangeSpec, LabelDraft, PortSpec, ServiceDraft,
    ServicePatch,
};

#[derive(Args, Debug, Clone, Default)]
pub struct LabelsArgs {
    /// Exact label key (e.g. app)
    #[arg(long)]
    pub key: Option<String>,
    /// Exact label value
    #[arg(long)]
    pub value: Option<String>,
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
    /// Create a label (needs writes enabled and --allow-writes)
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
        self.iter()
            .map(|l| {
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
            })
            .collect()
    }
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
}

impl TableRows for Vec<IpList> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "ranges", "pending", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|l| {
                vec![
                    l.name.clone(),
                    l.summary(),
                    pending(&l.update_type),
                    l.href.clone(),
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
}

pub async fn labels(ctx: &Context, args: &LabelsArgs) -> Result<(), CliError> {
    let c = ctx.pce.labels(ctx.cache_mode).await?;
    let data: Vec<Label> = c
        .value
        .into_iter()
        .filter(|l| args.include_deleted || !l.is_deleted())
        .filter(|l| args.key.as_deref().is_none_or(|k| l.key == k))
        .filter(|l| args.value.as_deref().is_none_or(|v| l.value == v))
        .collect();
    ctx.emit(&data, cache_label(c.hit, ctx.cache_mode), c.truncated, None)
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
    /// Create an IP list in draft (needs writes enabled and --allow-writes)
    Add(IpListAddArgs),
    /// Add or remove ranges and FQDNs; the full list is re-sent
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
        fqdns: a.fqdns.clone(),
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
    ctx.emit(
        &c.value,
        cache_label(c.hit, ctx.cache_mode),
        c.truncated,
        None,
    )
}

#[derive(Args, Debug)]
pub struct ServicesCmd {
    #[command(subcommand)]
    pub action: Option<ServicesAction>,
}

#[derive(Subcommand, Debug)]
pub enum ServicesAction {
    /// Create a service in draft (needs writes enabled and --allow-writes)
    Add(ServiceAddArgs),
    /// Add or remove ports; the full port list is re-sent
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
