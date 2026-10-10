//! Labels, label groups, IP lists, services: one cached list each.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::TableRows;
use clap::Args;
use pce::models::{IpList, Label, LabelGroup, Service};

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
