//! `workloads`: list workloads, with `--label key=value` filters resolved to
//! hrefs via the name index before querying.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::TableRows;
use clap::Args;
use pce::PceError;
use pce::models::{VenHealth, Workload, WorkloadFilter};
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
    /// full | visibility_only | selective | idle
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

/// A workload plus its derived VEN health, as printed by the CLI.
#[derive(Debug, Serialize)]
pub struct WorkloadRow {
    #[serde(flatten)]
    pub workload: Workload,
    pub health: VenHealth,
    pub sync_state: Option<String>,
    pub heartbeat_age_secs: Option<u64>,
}

impl WorkloadRow {
    pub fn new(workload: Workload, now: jiff::Timestamp) -> Self {
        let health = workload.ven_health(now);
        let a = workload.agent_status_view();
        Self {
            heartbeat_age_secs: workload.heartbeat_age(now).map(|d| d.as_secs()),
            sync_state: a.and_then(|a| a.sync_state),
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
                    w.label_text(),
                    w.enforcement_mode.clone().unwrap_or_default(),
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
    let now = jiff::Timestamp::now();
    let rows: Vec<WorkloadRow> = c
        .value
        .into_iter()
        .map(|w| WorkloadRow::new(w, now))
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
