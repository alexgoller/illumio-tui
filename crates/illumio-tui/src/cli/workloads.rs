//! `workloads`: list workloads, with `--label key=value` filters resolved to
//! hrefs via the name index before querying.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::TableRows;
use clap::Args;
use pce::PceError;
use pce::models::{Workload, WorkloadFilter};

#[derive(Args, Debug, Clone, Default)]
pub struct WorkloadsArgs {
    /// Hostname (PCE substring match)
    #[arg(long)]
    pub hostname: Option<String>,
    #[arg(long)]
    pub name: Option<String>,
    /// Interface IP address
    #[arg(long)]
    pub ip: Option<String>,
    /// Label key=value; repeat to AND several
    #[arg(long = "label")]
    pub labels: Vec<String>,
    /// full | visibility_only | selective | idle
    #[arg(long)]
    pub enforcement: Option<String>,
    #[arg(long)]
    pub online: Option<bool>,
    #[arg(long)]
    pub managed: Option<bool>,
    #[arg(long)]
    pub max_results: Option<u32>,
}

impl TableRows for Vec<Workload> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "hostname",
            "name",
            "ip",
            "labels",
            "enforcement",
            "online",
            "agent",
            "href",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|w| {
                vec![
                    w.hostname.clone().unwrap_or_default(),
                    w.name.clone().unwrap_or_default(),
                    w.primary_ip().unwrap_or("").to_string(),
                    w.label_text(),
                    w.enforcement_mode.clone().unwrap_or_default(),
                    w.online.map(|b| b.to_string()).unwrap_or_default(),
                    w.agent_status().unwrap_or_default(),
                    w.href.clone(),
                ]
            })
            .collect()
    }
}

pub async fn run(ctx: &Context, args: &WorkloadsArgs) -> Result<(), CliError> {
    let mut filter = WorkloadFilter {
        name: args.name.clone(),
        hostname: args.hostname.clone(),
        ip_address: args.ip.clone(),
        enforcement_mode: args.enforcement.clone(),
        managed: args.managed,
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
    ctx.emit(
        &c.value,
        cache_label(c.hit, ctx.cache_mode),
        c.truncated,
        None,
    )
}
