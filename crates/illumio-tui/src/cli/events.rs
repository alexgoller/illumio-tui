//! `events`: PCE audit events with server-side severity/status/window filters
//! and a local event-type substring filter.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{TableRows, Window};
use clap::Args;
use pce::models::{Event, EventFilter};
use pce::traffic::{format_timestamp, parse_window};

#[derive(Args, Debug, Clone, Default)]
pub struct EventsArgs {
    /// emerg, alert, crit, err, warning, notice, info, debug (PCE-side filter)
    #[arg(long)]
    pub severity: Option<String>,
    /// Event type substring, case-insensitive, e.g. agent. or sec_policy (local filter)
    #[arg(long = "type")]
    pub event_type: Option<String>,
    /// success | failure (PCE-side filter)
    #[arg(long)]
    pub status: Option<String>,
    /// Window ending now, e.g. 1h, 24h, 7d
    #[arg(long, default_value = "24h")]
    pub since: String,
    /// Row cap sent to the PCE (default 500)
    #[arg(long)]
    pub max_results: Option<u32>,
}

impl TableRows for Vec<Event> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "timestamp",
            "severity",
            "type",
            "actor",
            "resource",
            "status",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|e| {
                vec![
                    e.timestamp.clone().unwrap_or_default(),
                    e.severity.clone().unwrap_or_default(),
                    e.event_type.clone().unwrap_or_default(),
                    e.actor(),
                    e.resource(),
                    e.status.clone().unwrap_or_default(),
                ]
            })
            .collect()
    }
}

pub async fn run(ctx: &Context, args: &EventsArgs) -> Result<(), CliError> {
    let window = parse_window(&args.since).map_err(|e| CliError::Args(e.to_string()))?;
    let until = jiff::Timestamp::now();
    let since = jiff::Span::try_from(window)
        .and_then(|span| until.checked_sub(span))
        .map_err(|e| CliError::Args(format!("bad window {:?}: {e}", args.since)))?;
    let filter = EventFilter {
        severity: args.severity.clone(),
        event_type: None,
        status: args.status.clone(),
        since: Some(since),
        until: Some(until),
        max_results: args.max_results,
    };
    let c = ctx.pce.events(&filter, ctx.cache_mode).await?;
    let needle = args.event_type.as_deref().map(str::to_ascii_lowercase);
    let data: Vec<Event> = c
        .value
        .into_iter()
        .filter(|e| {
            needle.as_deref().is_none_or(|n| {
                e.event_type
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains(n)
            })
        })
        .collect();
    ctx.emit(
        &data,
        cache_label(c.hit, ctx.cache_mode),
        c.truncated,
        Some(Window {
            start: format_timestamp(since),
            end: format_timestamp(until),
        }),
    )
}
