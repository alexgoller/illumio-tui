//! Labels, label groups, IP lists, services: one cached list each.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::TableRows;
use super::write::{self, WriteFlags, parse_kv, resolve_label, run_plan};
use clap::{Args, Subcommand};
use pce::models::{IpList, Label, LabelGroup, Service};
use pce::write::{IfMissing, LabelDraft};

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

pub async fn ip_lists(ctx: &Context) -> Result<(), CliError> {
    let c = ctx.pce.ip_lists(ctx.cache_mode).await?;
    ctx.emit(
        &c.value,
        cache_label(c.hit, ctx.cache_mode),
        c.truncated,
        None,
    )
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
