//! `illumio-tui readiness`: per-app enforcement readiness (#54). One
//! traffic query (two with `--app`: the apps as destination, then as
//! source) re-evaluated against the draft policy, the workloads, the
//! pending changes and the draft rulesets (cached reads), and marks.toml;
//! `pce::readiness` judges each app. Read-only.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::marked::CliMarks;
use super::output::{OutputFormat, TableRows, join_list};
use super::traffic::{Fetched, FilterArgs, TransmissionArg, fetch};
use clap::Args;
use jiff::Timestamp;
use pce::models::{PolicyScope, WorkloadFilter};
use pce::readiness::{AppReadiness, Gap, ReadinessInput, readiness};
use pce::traffic::{
    DraftSummary, GroupKey, TrafficQueryBuilder, app_labels, keys_or_default, parse_app,
    parse_window, union_flows,
};
use serde::Serialize;

/// The window readiness looks at unless told otherwise: under 7 days is
/// little evidence, over it needs `--yes`.
pub const DEFAULT_READINESS_WINDOW: &str = "7d";

#[derive(Args, Debug, Clone, Default)]
pub struct ReadinessArgs {
    /// Relative window ending now (default 7d; under 7d is little evidence, over 7d needs --yes)
    #[arg(long)]
    pub window: Option<String>,
    /// Only this app, e.g. shop/prod: one value per scope key (repeatable). The query is narrowed
    /// to the apps (two queries: the apps as destination, then as source) and the gaps and
    /// pending changes are printed per app
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

struct Apps<'a>(&'a [AppReadiness]);

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

struct Gaps<'a>(&'a [Gap]);

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
                    g.peer_text(),
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
fn guard(ctx: &Context, args: &ReadinessArgs, queries: u32) -> Result<(), CliError> {
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
        let runs = if queries > 1 {
            format!(
                " (--app runs it as {})",
                pce::traffic::queries_text(queries)
            )
        } else {
            String::new()
        };
        return Err(CliError::Args(format!(
            "expensive query: {reason}{runs}; re-run with --yes to confirm"
        )));
    }
    Ok(())
}

/// `shop/prod` → `app=shop,env=prod`: one AND group of a traffic filter.
fn filter_group(app: &[String], keys: &[String]) -> String {
    keys.iter()
        .zip(app)
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// The apps as destination, then as source: two queries, merged without
/// duplicates (a flow between two of the apps is in both).
async fn fetch_apps(
    ctx: &Context,
    args: &ReadinessArgs,
    apps: &[GroupKey],
    keys: &[String],
) -> Result<Fetched, CliError> {
    let groups: Vec<String> = apps.iter().map(|a| filter_group(a, keys)).collect();
    let inbound = FilterArgs {
        dst: groups.clone(),
        ..filter(args)
    };
    let outbound = FilterArgs {
        src: groups,
        ..filter(args)
    };
    let some = |f: Option<Fetched>| f.ok_or_else(|| CliError::Other("not a dry run".into()));
    let a = some(fetch(ctx, &inbound, None).await?)?;
    let b = some(fetch(ctx, &outbound, None).await?)?;
    let flows = union_flows(a.flows, b.flows);
    Ok(Fetched {
        draft: Some(DraftSummary::of(&flows)),
        cache: if a.cache == b.cache { a.cache } else { "miss" },
        truncated: a.truncated || b.truncated,
        incomplete: a.incomplete || b.incomplete,
        queries: Some(2),
        flows,
        ..a
    })
}

pub async fn run(ctx: &Context, args: &ReadinessArgs) -> Result<(), CliError> {
    let keys = keys_or_default(&ctx.config.scope_keys);
    let mut apps: Vec<GroupKey> = Vec::new();
    for a in &args.apps {
        let app = parse_app(a);
        if !apps.contains(&app) {
            apps.push(app);
        }
    }
    let narrowed = !apps.is_empty();
    guard(ctx, args, if narrowed { 2 } else { 1 })?;
    // The apps' labels, checked before any traffic request (exit 2 with
    // the closest values), and the workload filter they make.
    let mut label_groups = Vec::new();
    if narrowed {
        let idx = ctx.pce.name_index(ctx.cache_mode).await?;
        for app in &apps {
            let m = app_labels(app, &keys, &idx).map_err(CliError::Args)?;
            label_groups.push(pce::app::label_hrefs(&m));
        }
    }
    let fetched = if narrowed {
        fetch_apps(ctx, args, &apps, &keys).await?
    } else {
        fetch(ctx, &filter(args), None)
            .await?
            .ok_or_else(|| CliError::Other("not a dry run".into()))?
    };
    let wl_filter = WorkloadFilter {
        labels: label_groups,
        ..WorkloadFilter::default()
    };
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
    let gaps: Vec<Gaps> = report.apps.iter().map(|r| Gaps(&r.gaps)).collect();
    let pend: Vec<Pending> = report
        .apps
        .iter()
        .map(|r| Pending(&r.pending.changes))
        .collect();
    // `--app`: each app's gaps and pending changes too (`--csv`: the apps
    // table only, one table per output).
    if narrowed && ctx.format != OutputFormat::Csv {
        for ((r, g), p) in report.apps.iter().zip(&gaps).zip(&pend) {
            tables.push((format!("gaps · {} · {}", r.app, r.gaps.len()), g));
            if r.pending.count > 0 {
                tables.push((format!("pending · {} · {}", r.app, r.pending.count), p));
            }
        }
    }
    if narrowed {
        ctx.emit_report(&report, &tables, &meta)
    } else {
        // Without `--app` the gaps stay out of JSON too: every app's
        // tuples would swamp the envelope.
        let mut data = serde_json::to_value(&report).map_err(|e| CliError::Other(e.to_string()))?;
        if let Some(list) = data.get_mut("apps").and_then(|a| a.as_array_mut()) {
            for a in list {
                if let Some(o) = a.as_object_mut() {
                    o.remove("gaps");
                }
            }
        }
        ctx.emit_report(&data, &tables, &meta)
    }
}
