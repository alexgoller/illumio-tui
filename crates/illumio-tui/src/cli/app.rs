//! `illumio-tui app <app>`: one app's profile in one envelope: its
//! workloads (enforcement and VEN health), its inbound and outbound peers
//! by service and decision, the policy that covers it, and blocked traffic
//! over time (approximate: from the flows' last-detected times; exact with
//! `--timeline`: one blocked-only query per bucket and side).
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{OutputFormat, TableRows};
use super::traffic::{BuiltQuery, ByArg, DirectionArg, FilterArgs, build_query, cli_focus, fetch};
use clap::{Args, ValueEnum};
use jiff::Timestamp;
use pce::app::{
    AppPeer, AppPolicyRow, AppWorkload, PolicyReason, WorkloadSummary, app_peers, app_policy,
    app_workload_filter, app_workloads, enforcement_word, label_hrefs, workload_in_app,
};
use pce::models::{PolicyScope, Workload};
use pce::traffic::{
    APPROX_BUCKETS, Direction, FocusDirection, TIMELINE_CONFIRM_QUERIES, Timeline, approx_timeline,
    bucket_count, bucket_plan, equal_buckets, exact_timeline, parse_window,
};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionArg {
    /// The workloads, enforcement and VEN health
    Workloads,
    /// The inbound peers
    Inbound,
    /// The outbound peers
    Outbound,
    /// The rulesets covering the app
    Policy,
    /// Blocked traffic over time
    Timeline,
}

#[derive(Args, Debug, Clone, Default)]
pub struct AppArgs {
    /// The app: one value per label key (--keys, else the profile's scope keys), e.g. shop/prod
    pub app: String,
    /// in (the app is the destination), out (the source) or both (default; two queries)
    #[arg(long, value_enum)]
    pub direction: Option<DirectionArg>,
    /// Relative window ending now, e.g. 1h, 24h, 7d (default from profile, 24h)
    #[arg(long)]
    pub window: Option<String>,
    /// Label keys the app is a group of, e.g. app,env (default: the profile's scope keys)
    #[arg(long, value_delimiter = ',')]
    pub keys: Vec<String>,
    /// Row cap per traffic query, up to 200000 (default from profile; > 10000 needs --yes)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Match the active policy instead of the draft
    #[arg(long)]
    pub active: bool,
    /// Only this section (JSON: `app`, `direction` and it; --csv needs one)
    #[arg(long, value_enum)]
    pub section: Option<SectionArg>,
    /// The exact blocked timeline: one blocked-only query per bucket and side (24x1h for 1d,
    /// 28x6h for 1w, 28x1d for 4w, 30x1d for 30d, else up to 24 of at least 5m: 1h is 12x5m),
    /// one at a time; above 24 queries needs --yes
    #[arg(long)]
    pub timeline: bool,
    /// Confirm an expensive query (window > 7d, max_results > 10000, --timeline above 24 queries)
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Serialize)]
pub struct WorkloadsSection {
    /// The workloads read hit its cap: the rows and the summary are partial.
    pub truncated: bool,
    pub summary: WorkloadSummary,
    pub rows: Vec<AppWorkload>,
}

#[derive(Debug, Serialize)]
pub struct PolicySection {
    /// `draft`, or `active` with `--active`.
    pub scope: PolicyScope,
    /// The rulesets read hit its cap: rulesets may be missing.
    pub truncated: bool,
    pub rows: Vec<AppPolicyRow>,
}

#[derive(Debug, Serialize)]
pub struct AppReport {
    pub app: String,
    pub direction: FocusDirection,
    pub workloads: WorkloadsSection,
    pub inbound: Vec<AppPeer>,
    pub outbound: Vec<AppPeer>,
    pub policy: PolicySection,
    pub timeline: Timeline,
}

impl TableRows for WorkloadsSection {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "role", "enforcement", "ven"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.rows
            .iter()
            .map(|w| {
                vec![
                    w.name.clone(),
                    w.role.clone().unwrap_or_default(),
                    enforcement_word(&w.enforcement).to_string(),
                    w.ven.as_str().to_string(),
                ]
            })
            .collect()
    }
    fn full_headers(&self) -> Vec<&'static str> {
        vec![
            "href",
            "name",
            "role",
            "enforcement",
            "enforcement_mode",
            "ven",
        ]
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.rows
            .iter()
            .map(|w| {
                vec![
                    w.href.clone(),
                    w.name.clone(),
                    w.role.clone().unwrap_or_default(),
                    w.enforcement.clone(),
                    w.enforcement_mode.clone().unwrap_or_default(),
                    w.ven.as_str().to_string(),
                ]
            })
            .collect()
    }
}

struct Peers<'a>(&'a [AppPeer]);

impl TableRows for Peers<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["peer", "service", "decision", "flows", "conns", "bytes"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|p| {
                vec![
                    p.peer_text(),
                    p.service.clone(),
                    p.decision.as_str().to_string(),
                    p.flows.to_string(),
                    p.connections.to_string(),
                    p.bytes.to_string(),
                ]
            })
            .collect()
    }
}

struct Policy<'a>(&'a [AppPolicyRow]);

impl TableRows for Policy<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["ruleset", "enabled", "reason", "rules"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|r| {
                vec![
                    r.name.clone(),
                    r.enabled.to_string(),
                    match r.reason {
                        PolicyReason::Scope => "scope",
                        PolicyReason::AllWorkloads => "all_workloads",
                        PolicyReason::Rules => "rules",
                    }
                    .to_string(),
                    r.rules.to_string(),
                ]
            })
            .collect()
    }
}

struct Buckets<'a>(&'a Timeline);

impl TableRows for Buckets<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["start", "end", "count", "truncated"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .buckets
            .iter()
            .map(|b| {
                vec![
                    pce::traffic::format_timestamp(b.start),
                    pce::traffic::format_timestamp(b.end),
                    b.count.map(|c| c.to_string()).unwrap_or_default(),
                    b.truncated.to_string(),
                ]
            })
            .collect()
    }
}

pub async fn run(ctx: &Context, args: &AppArgs) -> Result<(), CliError> {
    if ctx.format == OutputFormat::Csv && args.section.is_none() {
        return Err(CliError::Args(
            "--csv prints one table: add --section workloads|inbound|outbound|policy|timeline"
                .into(),
        ));
    }
    // The bucket queries' count, checked before any request.
    let sides = args
        .direction
        .map_or(FocusDirection::Both, FocusDirection::from)
        .query_count();
    if args.timeline {
        if args.section.is_some_and(|s| s != SectionArg::Timeline) {
            return Err(CliError::Args(
                "--timeline only adds to the timeline section: drop --section or use --section timeline"
                    .into(),
            ));
        }
        let window = args
            .window
            .clone()
            .unwrap_or_else(|| ctx.config.traffic_default_window.clone());
        let buckets = u32::try_from(bucket_count(parse_window(&window)?)).unwrap_or(u32::MAX);
        let queries = buckets.saturating_mul(sides);
        if queries > TIMELINE_CONFIRM_QUERIES && !args.yes {
            return Err(CliError::Args(format!(
                "--timeline sends {} ({buckets} buckets{}, one at a time; more than {TIMELINE_CONFIRM_QUERIES}): re-run with --yes to confirm",
                pce::traffic::queries_text(queries),
                if sides == 2 { " x in and out" } else { "" },
            )));
        }
    }
    let keys = if args.keys.is_empty() {
        ctx.config.scope_keys.clone()
    } else {
        args.keys.clone()
    };
    let filter = FilterArgs {
        window: args.window.clone(),
        max_results: args.max_results,
        yes: args.yes,
        app: Some(args.app.clone()),
        direction: args.direction,
        ..FilterArgs::default()
    };
    let focus = cli_focus(&filter, &keys, ByArg::Pairs)?
        .ok_or_else(|| CliError::Other("--app is set".into()))?;
    let Some(fetched) = fetch(ctx, &filter, Some(&focus)).await? else {
        return Ok(()); // a dry run; `app` has none, kept for completeness
    };
    let focus = focus.focus;
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let hrefs = label_hrefs(&focus.label_matches(&idx).map_err(CliError::Args)?);
    let wl = ctx
        .pce
        .workloads(&app_workload_filter(&hrefs), ctx.cache_mode)
        .await?;
    let scope = if args.active {
        PolicyScope::Active
    } else {
        PolicyScope::Draft
    };
    let rs = ctx.pce.rule_sets(scope, ctx.cache_mode).await?;
    let mine: Vec<Workload> = wl
        .value
        .iter()
        .filter(|w| workload_in_app(w, &hrefs))
        .cloned()
        .collect();
    let (rows, summary) = app_workloads(&mine, &idx, &ctx.config.tier_key, Timestamp::now());
    let bounds = fetched.window.as_ref().and_then(|w| {
        Some((
            w.start.parse::<Timestamp>().ok()?,
            w.end.parse::<Timestamp>().ok()?,
        ))
    });
    let buckets = bounds
        .map(|(s, e)| equal_buckets(s, e, APPROX_BUCKETS))
        .unwrap_or_default();
    // --timeline: each bucket's blocked-only focused query (per side), one
    // at a time, each in its own cache slot; a capped bucket is a lower
    // bound (its `truncated`, and the timeline's).
    let mut bucket_queries = 0u32;
    let mut buckets_hit = true;
    let timeline = if args.timeline {
        let BuiltQuery { query: base, .. } = build_query(ctx, &filter, sides).await?;
        let plan = bounds.map(|(s, e)| bucket_plan(s, e)).unwrap_or_default();
        let mut counts = Vec::with_capacity(plan.len());
        for (i, b) in plan.iter().enumerate() {
            let c = ctx
                .pce
                .timeline_bucket(
                    i,
                    &focus,
                    &base,
                    b,
                    &idx,
                    ctx.cache_mode,
                    &CancellationToken::new(),
                )
                .await?;
            bucket_queries += c.queries;
            buckets_hit &= c.hit;
            counts.push(Some((c.connections, c.truncated)));
        }
        exact_timeline(&plan, &counts)
    } else {
        approx_timeline(&fetched.flows, &focus, &buckets)
    };
    let d = focus.direction;
    let report = AppReport {
        app: focus.app_text(),
        direction: d,
        workloads: WorkloadsSection {
            truncated: wl.truncated,
            summary,
            rows,
        },
        inbound: if d == FocusDirection::Out {
            Vec::new()
        } else {
            app_peers(&fetched.flows, &focus, Direction::In)
        },
        outbound: if d == FocusDirection::In {
            Vec::new()
        } else {
            app_peers(&fetched.flows, &focus, Direction::Out)
        },
        policy: PolicySection {
            scope,
            truncated: rs.truncated,
            rows: app_policy(&rs.value, &hrefs, &idx),
        },
        timeline,
    };
    // `meta.truncated` is the traffic's; the workloads and the rulesets
    // say so in their sections.
    // `meta.queries` counts every Explorer query, the buckets' too.
    let mut meta = fetched.meta(ctx);
    meta.queries = Some(meta.queries.unwrap_or(0) + bucket_queries);
    let all_hit = fetched.cache == "hit" && wl.hit && rs.hit && buckets_hit;
    meta.cache = cache_label(all_hit, ctx.cache_mode);
    let s = &report.workloads.summary;
    let (inbound, outbound) = (Peers(&report.inbound), Peers(&report.outbound));
    let (policy, buckets) = (Policy(&report.policy.rows), Buckets(&report.timeline));
    let cut = |t: bool| if t { " · truncated" } else { "" };
    let tables: Vec<(SectionArg, String, &dyn TableRows)> = vec![
        (
            SectionArg::Workloads,
            format!(
                "workloads · {} · {} · VENs {}{}",
                s.workloads,
                s.enforcement.text(),
                s.ven_text(),
                cut(report.workloads.truncated)
            ),
            &report.workloads,
        ),
        (
            SectionArg::Inbound,
            format!("inbound · {}", report.inbound.len()),
            &inbound,
        ),
        (
            SectionArg::Outbound,
            format!("outbound · {}", report.outbound.len()),
            &outbound,
        ),
        (
            SectionArg::Policy,
            format!(
                "policy · {} · {}{}",
                report.policy.scope.segment(),
                report.policy.rows.len(),
                cut(report.policy.truncated)
            ),
            &policy,
        ),
        (
            SectionArg::Timeline,
            format!(
                "timeline · {} · blocked {}{}{}",
                if report.timeline.exact {
                    "exact"
                } else {
                    "approx"
                },
                if report.timeline.exact {
                    "connections"
                } else {
                    "flow records"
                },
                report
                    .timeline
                    .spike
                    .as_ref()
                    .map(|s| format!(" · {}", s.text()))
                    .unwrap_or_default(),
                if report.timeline.truncated {
                    " · truncated (capped buckets are lower bounds)"
                } else {
                    ""
                }
            ),
            &buckets,
        ),
    ];
    let chosen: Vec<(String, &dyn TableRows)> = tables
        .into_iter()
        .filter(|(sec, _, _)| args.section.is_none_or(|want| want == *sec))
        .map(|(_, title, table)| (title, table))
        .collect();
    match args.section {
        // JSON with --section: `app`, `direction` and that section only.
        Some(sec) if ctx.format == OutputFormat::Json => {
            let mut data =
                serde_json::to_value(&report).map_err(|e| CliError::Other(e.to_string()))?;
            if let Some(o) = data.as_object_mut() {
                let keep = sec.key();
                o.retain(|k, _| k == "app" || k == "direction" || k == keep);
            }
            ctx.emit_report(&data, &chosen, &meta)
        }
        _ => ctx.emit_report(&report, &chosen, &meta),
    }
}

impl SectionArg {
    /// The section's key in the envelope's `data`.
    pub fn key(self) -> &'static str {
        match self {
            SectionArg::Workloads => "workloads",
            SectionArg::Inbound => "inbound",
            SectionArg::Outbound => "outbound",
            SectionArg::Policy => "policy",
            SectionArg::Timeline => "timeline",
        }
    }
}
