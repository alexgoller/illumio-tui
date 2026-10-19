//! `illumio-tui readiness`: per-app enforcement readiness (#54). One
//! traffic query (with `--app`, narrowed to the apps on either side)
//! re-evaluated against the draft policy, the workloads, the
//! pending changes and the draft rulesets (cached reads), and marks.toml;
//! `pce::readiness` judges each app. Read-only.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::marked::CliMarks;
use super::output::Window;
use super::output::{OutputFormat, TableRows, join_list};
use super::traffic::{Fetched, FilterArgs, TransmissionArg, fetch};
use clap::Args;
use jiff::Timestamp;
use pce::NameIndex;
use pce::models::{ActorMatch, PolicyScope, Transmission, WorkloadFilter};
use pce::readiness::{AppReadiness, Gap, ReadinessInput, readiness};
use pce::traffic::{
    DraftSummary, GroupKey, TrafficQueryBuilder, app_labels, keys_or_default, parse_app,
    parse_window, resolve_flows,
};
use serde::Serialize;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// The window readiness looks at unless told otherwise: under 7 days is
/// little evidence, over it needs `--yes`.
pub const DEFAULT_READINESS_WINDOW: &str = "7d";

#[derive(Args, Debug, Clone, Default)]
pub struct ReadinessArgs {
    /// Relative window ending now (default 7d; under 7d is little evidence, over 7d needs --yes)
    #[arg(long)]
    pub window: Option<String>,
    /// Only this app, e.g. shop/prod: one value per scope key (repeatable). The one query is
    /// narrowed to the apps (on either side) and the gaps and pending changes are printed per
    /// app. An app with a `-` slot (a workload missing a scope label) is listed without --app
    /// but cannot be named here
    #[arg(long = "app")]
    pub apps: Vec<String>,
    /// Row cap per traffic query, up to 200000 (default from profile; > 10000 needs --yes)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Confirm an expensive query (window > 7d or max_results > 10000)
    #[arg(long)]
    pub yes: bool,
    /// Ignore marks.toml: no flow counts as accepted (squelch, accept-blocked)
    #[arg(long)]
    pub no_marks: bool,
}

/// What `data` holds.
#[derive(Debug, Serialize)]
pub struct ReadinessReport {
    /// The window the traffic covers, `7d`.
    pub window: String,
    /// The traffic was re-evaluated against the draft policy.
    pub draft: bool,
    pub apps: Vec<AppReadiness>,
}

pub(crate) struct Apps<'a>(pub &'a [AppReadiness]);

impl TableRows for Apps<'_> {
    fn units(&self) -> Option<&'static str> {
        Some(
            "flows: unicast flow records · conns: their lifetime counts · coverage: draft-allowed or accepted conns",
        )
    }
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "app",
            "workloads",
            "enforcement",
            "flows",
            "conns in",
            "conns out",
            "would block",
            "coverage",
            "pending",
            "verdict",
            "warnings",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|r| {
                let c = &r.coverage;
                vec![
                    r.app.clone(),
                    r.workloads.total.to_string(),
                    r.workloads.enforcement.text(),
                    r.evidence.flows.to_string(),
                    c.inbound.connections.to_string(),
                    c.outbound.connections.to_string(),
                    c.total.would_block.to_string(),
                    c.percent_text(),
                    r.pending.count.to_string(),
                    r.verdict.text().to_string(),
                    r.warnings_text(),
                ]
            })
            .collect()
    }
    fn full_headers(&self) -> Vec<&'static str> {
        vec![
            "app",
            "workloads",
            "managed",
            "unmanaged",
            "enforced",
            "selective",
            "visibility",
            "idle",
            "offline",
            "missing_tier",
            "window",
            "flows",
            "connections",
            "little_evidence",
            "conns_in",
            "conns_out",
            "allowed",
            "would_block",
            "accepted",
            "would_block_flows",
            "coverage",
            "pending",
            "verdict",
            "warnings",
        ]
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|r| {
                let (w, c, e) = (&r.workloads, &r.coverage, &r.evidence);
                vec![
                    r.app.clone(),
                    w.total.to_string(),
                    w.managed.to_string(),
                    w.unmanaged.to_string(),
                    w.enforcement.enforced.to_string(),
                    w.enforcement.selective.to_string(),
                    w.enforcement.visibility.to_string(),
                    w.enforcement.idle.to_string(),
                    w.offline.to_string(),
                    w.missing_tier.to_string(),
                    e.window.clone(),
                    e.flows.to_string(),
                    e.connections.to_string(),
                    e.little.to_string(),
                    c.inbound.connections.to_string(),
                    c.outbound.connections.to_string(),
                    c.total.allowed.to_string(),
                    c.total.would_block.to_string(),
                    c.total.accepted.to_string(),
                    c.total.would_block_flows.to_string(),
                    c.ratio.map(|r| format!("{r:.4}")).unwrap_or_default(),
                    r.pending.count.to_string(),
                    r.verdict.as_str().to_string(),
                    join_list(r.warnings.iter().map(|w| w.text())),
                ]
            })
            .collect()
    }
}

pub(crate) struct Gaps<'a>(pub &'a [Gap]);

impl TableRows for Gaps<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["direction", "peer", "service", "flows", "conns"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|g| {
                vec![
                    g.direction.as_str().to_string(),
                    g.peer_text.clone(),
                    g.service.clone(),
                    g.flows.to_string(),
                    g.connections.to_string(),
                ]
            })
            .collect()
    }
}

struct Pending<'a>(&'a [pce::models::PendingChange]);

impl TableRows for Pending<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["type", "change", "name", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|p| {
                vec![
                    p.object_type.clone(),
                    p.change_type.clone(),
                    p.name.clone().unwrap_or_default(),
                    p.href.clone(),
                ]
            })
            .collect()
    }
}

/// The traffic filter readiness runs: unicast only, with the draft
/// decision; `src` / `dst` narrow it to the apps.
fn filter(args: &ReadinessArgs) -> FilterArgs {
    FilterArgs {
        window: Some(
            args.window
                .clone()
                .unwrap_or_else(|| DEFAULT_READINESS_WINDOW.into()),
        ),
        max_results: args.max_results,
        yes: args.yes,
        transmission: vec![TransmissionArg::Unicast],
        draft: true,
        ..FilterArgs::default()
    }
}

/// The cost guard before any request (the window, the cap), as
/// `traffic query`'s: `--app` validates its labels with the name index,
/// which must not be loaded for a query that is then refused.
fn guard(ctx: &Context, args: &ReadinessArgs) -> Result<(), CliError> {
    let w = args
        .window
        .clone()
        .unwrap_or_else(|| DEFAULT_READINESS_WINDOW.into());
    let q = TrafficQueryBuilder::last(parse_window(&w)?)?
        .max_results(args.max_results.unwrap_or(ctx.config.traffic_max_results))
        .build()?;
    if !args.yes
        && let Some(reason) = q.needs_confirmation()
    {
        return Err(CliError::Args(format!(
            "expensive query: {reason}; re-run with --yes to confirm"
        )));
    }
    Ok(())
}

/// One query for the apps: each app's labels as an include group on both
/// sides, joined with `or` (`sources_destinations_query_op`, as the bulk
/// draft check does), so a flow with an app on either side comes back
/// once; re-evaluated against the draft (one `update_rules`).
async fn fetch_apps(
    ctx: &Context,
    args: &ReadinessArgs,
    groups: &[Vec<ActorMatch>],
    idx: Arc<NameIndex>,
) -> Result<Fetched, CliError> {
    let w = args
        .window
        .clone()
        .unwrap_or_else(|| DEFAULT_READINESS_WINDOW.into());
    let mut b = TrafficQueryBuilder::last(parse_window(&w)?)?
        .transmission(&[Transmission::Unicast])
        .max_results(args.max_results.unwrap_or(ctx.config.traffic_max_results));
    for g in groups {
        b = b.source(g.clone()).destination(g.clone());
    }
    let mut q = b.build()?;
    q.sources_destinations_query_op = "or".into();
    let r = ctx
        .pce
        .traffic_draft(&q, None, ctx.cache_mode, &CancellationToken::new(), None)
        .await?;
    let flows = resolve_flows(&r.value.flows, &idx);
    Ok(Fetched {
        draft: Some(DraftSummary::of(&flows)),
        cache: cache_label(r.hit, ctx.cache_mode),
        truncated: r.truncated,
        incomplete: r.value.incomplete,
        window: Some(Window {
            start: q.start_date.clone(),
            end: q.end_date.clone(),
        }),
        span: q.window(),
        cap: q.max_results,
        cap_steps: None,
        queries: Some(1),
        flows,
        index: idx,
    })
}

/// Everything readiness read and judged: the report, the
/// workloads read (and when it started), and the meta so far.
pub(crate) struct Evaluated {
    pub report: ReadinessReport,
    pub workloads: pce::Cached<Vec<pce::models::Workload>>,
    /// When the workloads read started: a send right after it may skip
    /// the bulk re-read (`BulkPlan::selection_read`).
    pub workloads_read_at: std::time::Instant,
    /// Each `--app`'s labels (as the traffic query and the workloads read
    /// name them).
    pub app_labels: Vec<Vec<ActorMatch>>,
    /// The name index the flows were resolved with.
    pub index: Arc<NameIndex>,
    pub meta: super::output::Meta,
}

/// Runs readiness (the requests and the judgement) without printing:
/// `run`, and `enforce` before a move.
pub(crate) async fn evaluate(ctx: &Context, args: &ReadinessArgs) -> Result<Evaluated, CliError> {
    let keys = keys_or_default(&ctx.config.scope_keys);
    let mut apps: Vec<GroupKey> = Vec::new();
    for a in &args.apps {
        let app = parse_app(a);
        if !apps.contains(&app) {
            apps.push(app);
        }
    }
    let narrowed = !apps.is_empty();
    guard(ctx, args)?;
    // The apps' labels, checked before any traffic request (exit 2 with
    // the closest values), and the workload filter they make.
    let mut label_groups = Vec::new();
    let mut app_labels_ = Vec::new();
    let fetched = if narrowed {
        let idx = ctx.pce.name_index(ctx.cache_mode).await?;
        for app in &apps {
            let m = app_labels(app, &keys, &idx).map_err(CliError::Args)?;
            label_groups.push(pce::app::label_hrefs(&m));
            app_labels_.push(m);
        }
        fetch_apps(ctx, args, &app_labels_, idx).await?
    } else {
        fetch(ctx, &filter(args), None)
            .await?
            .ok_or_else(|| CliError::Other("not a dry run".into()))?
    };
    let wl_filter = WorkloadFilter {
        labels: label_groups,
        ..WorkloadFilter::default()
    };
    let workloads_read_at = std::time::Instant::now();
    let wl = ctx.pce.workloads(&wl_filter, ctx.cache_mode).await?;
    let pending = ctx.pce.pending_changes(ctx.cache_mode).await?;
    let rs = ctx
        .pce
        .rule_sets(PolicyScope::Draft, ctx.cache_mode)
        .await?;
    let cm = CliMarks::load(ctx, args.no_marks, false);
    let act = cm.active(true);
    let input = ReadinessInput {
        flows: &fetched.flows,
        draft: true,
        truncated: fetched.truncated,
        incomplete: fetched.incomplete,
        gaps: narrowed,
        window: fetched.span,
        workloads: &wl.value,
        pending: &pending.value,
        rule_sets: &rs.value,
        marks: act.as_ref(),
        idx: &fetched.index,
        scope_keys: &keys,
        tier_key: &ctx.config.tier_key,
        now: Timestamp::now(),
    };
    let report = ReadinessReport {
        window: pce::traffic::compact_duration(fetched.span),
        draft: true,
        apps: readiness(&input, &apps),
    };
    let mut meta = fetched.meta(ctx);
    cm.fill(&mut meta, act.as_ref(), None);
    let all_hit = fetched.cache == "hit" && wl.hit && pending.hit && rs.hit;
    meta.cache = cache_label(all_hit, ctx.cache_mode);
    for (cut, what) in [
        (
            wl.truncated,
            "the workloads read hit its cap: workload counts are partial",
        ),
        (
            rs.truncated,
            "the rulesets read hit its cap: pending matching may miss some",
        ),
    ] {
        if cut {
            meta.warnings.get_or_insert_with(Vec::new).push(what.into());
        }
    }
    Ok(Evaluated {
        report,
        workloads: wl,
        workloads_read_at,
        app_labels: app_labels_,
        index: fetched.index.clone(),
        meta,
    })
}

pub async fn run(ctx: &Context, args: &ReadinessArgs) -> Result<(), CliError> {
    let narrowed = !args.apps.is_empty();
    let Evaluated { report, meta, .. } = evaluate(ctx, args).await?;
    let apps_table = Apps(&report.apps);
    let mut tables: Vec<(String, &dyn TableRows)> = vec![(
        format!(
            "readiness · {} · {} app{}",
            report.window,
            report.apps.len(),
            if report.apps.len() == 1 { "" } else { "s" }
        ),
        &apps_table,
    )];
    let gaps: Vec<Gaps> = report
        .apps
        .iter()
        .map(|r| Gaps(r.gaps.as_deref().unwrap_or_default()))
        .collect();
    let pend: Vec<Pending> = report
        .apps
        .iter()
        .map(|r| Pending(&r.pending.changes))
        .collect();
    // `--app`: each app's gaps and pending changes too (`--csv`: the apps
    // table only, one table per output).
    if narrowed && ctx.format != OutputFormat::Csv {
        for ((r, g), p) in report.apps.iter().zip(&gaps).zip(&pend) {
            tables.push((format!("gaps · {} · {}", r.app, r.gap_count), g));
            if r.pending.count > 0 {
                tables.push((format!("pending · {} · {}", r.app, r.pending.count), p));
            }
        }
    }
    ctx.emit_report(&report, &tables, &meta)
}
