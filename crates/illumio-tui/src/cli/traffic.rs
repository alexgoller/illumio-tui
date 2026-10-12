//! `traffic query|groups|mesh`: build an Explorer query from human filters,
//! guard its cost, run it through the cached facade, and print flows,
//! label-pair groups, the blocked summary or the mesh.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{Meta, TableRows, Window};
use clap::{Args, Subcommand, ValueEnum};
use jiff::Timestamp;
use jiff::civil::Date;
use pce::models::{
    ActorMatch, PolicyDecision, ServiceMatch, TrafficQuery, WorkloadFilter, proto_number,
};
use pce::traffic::{
    AppSection, BackfillRun, CONFIRM_MAX_RESULTS, Decisions, Direction, FilterExpr, FlowGroup,
    MAX_SLICES, MeshColumns, MeshRow, Next, ResolvedFlow, Step, TrafficQueryBuilder, app_header,
    app_sections, app_sections_both, compact_duration, coverage, format_timestamp, group_by_labels,
    merge_flows, mesh, parse_window, plan_backfill, resolve_actor, resolve_flows,
};
use pce::{KeyOrder, NameIndex};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Subcommand, Debug)]
pub enum TrafficCmd {
    /// Run an Explorer traffic query
    Query(QueryArgs),
    /// Label-pair groups of the query's flows (connections, bytes, decisions)
    Groups(GroupsArgs),
    /// The mesh: user, process, source, destination, service, destination process
    Mesh(MeshArgs),
}

/// `traffic query`: the filter, then how to print the flows.
#[derive(Args, Debug, Clone, Default)]
pub struct QueryArgs {
    #[command(flatten)]
    pub filter: FilterArgs,
    /// Group by label keys, e.g. app,env
    #[arg(long, value_delimiter = ',', conflicts_with = "blocked")]
    pub group: Vec<String>,
    /// Only blocked and potentially blocked flows, in sections per destination group
    /// (as `traffic groups --by in --show blocked`; JSON: `{"sections": [...]}`)
    #[arg(long)]
    pub blocked: bool,
}

/// `traffic groups --by`: the flat pairs, or sections per app.
#[derive(ValueEnum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ByArg {
    /// One row per source and destination group
    #[default]
    Pairs,
    /// One section per destination app, its sources as rows
    In,
    /// One section per source app, its destinations as rows
    Out,
    /// Each app's inbound section, then its outbound one
    Both,
}

/// `traffic groups --show`: which decisions the sections count.
#[derive(ValueEnum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShowArg {
    #[default]
    All,
    /// Blocked and potentially blocked
    Blocked,
    /// Blocked only
    BlockedOnly,
}

impl From<ShowArg> for Decisions {
    fn from(s: ShowArg) -> Self {
        match s {
            ShowArg::All => Decisions::All,
            ShowArg::Blocked => Decisions::Blocked,
            ShowArg::BlockedOnly => Decisions::BlockedOnly,
        }
    }
}

#[derive(Args, Debug, Clone, Default)]
pub struct GroupsArgs {
    #[command(flatten)]
    pub filter: FilterArgs,
    /// Label keys to group by, e.g. app,env,role (default: the profile's scope keys)
    #[arg(long, value_delimiter = ',')]
    pub keys: Vec<String>,
    /// pairs (default), or sections per app: in, out, both (JSON: `{"sections": [...]}`)
    #[arg(long, value_enum, default_value_t)]
    pub by: ByArg,
    /// Sections only: count all flows, blocked (and potentially blocked) or blocked-only.
    /// Applied to the loaded flows; `--decision` filters the query itself
    #[arg(long, value_enum, default_value_t)]
    pub show: ShowArg,
    /// Sections only: keep this app's sections, e.g. pos/PCI
    #[arg(long)]
    pub app: Option<String>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct MeshArgs {
    #[command(flatten)]
    pub filter: FilterArgs,
    /// Label keys to group sources and destinations by (default: the profile's scope keys)
    #[arg(long, value_delimiter = ',')]
    pub keys: Vec<String>,
    /// Do not split rows by user
    #[arg(long)]
    pub no_user: bool,
    /// Do not split rows by the source process
    #[arg(long)]
    pub no_process: bool,
}

/// The Explorer filter every `traffic` command shares.
#[derive(Args, Debug, Clone, Default)]
pub struct FilterArgs {
    /// Relative window ending now, e.g. 1h, 24h, 7d (default from profile, 24h)
    #[arg(long, conflicts_with_all = ["from", "to"])]
    pub window: Option<String>,
    /// Start, RFC 3339 or YYYY-MM-DD (UTC)
    #[arg(long, requires = "to")]
    pub from: Option<String>,
    /// End, RFC 3339 or YYYY-MM-DD (end of that day, UTC)
    #[arg(long, requires = "from")]
    pub to: Option<String>,
    /// Source filter: key=value, ip:ADDR, ipl:NAME, wl:HOST, fqdn:NAME. Comma = AND, repeat = OR
    #[arg(long = "src")]
    pub src: Vec<String>,
    /// Destination filter, same syntax as --src
    #[arg(long = "dst")]
    pub dst: Vec<String>,
    /// Exclude sources matching this filter (same syntax as --src; repeatable)
    #[arg(long = "exclude-src")]
    pub exclude_src: Vec<String>,
    /// Exclude destinations matching this filter (same syntax as --src; repeatable)
    #[arg(long = "exclude-dst")]
    pub exclude_dst: Vec<String>,
    /// Destination port
    #[arg(long)]
    pub port: Option<u16>,
    /// tcp, udp, icmp or a protocol number
    #[arg(long)]
    pub proto: Option<String>,
    /// Process name on the destination
    #[arg(long)]
    pub process: Option<String>,
    /// allowed, potentially_blocked, blocked (comma-separated; default all)
    #[arg(long, value_delimiter = ',')]
    pub decision: Vec<String>,
    /// Row cap (default from profile, 1000; > 10000 needs --yes)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Print the request body instead of sending it
    #[arg(long)]
    pub dry_run: bool,
    /// Confirm an expensive query (window > 7d or max_results > 10000)
    #[arg(long)]
    pub yes: bool,
    /// When the result is truncated, fetch the window again in slices that fit the cap
    /// (raised up to the profile's if needed), newest first; meta.slices counts them
    #[arg(long)]
    pub backfill: bool,
    /// With --backfill: at most this many slice queries per run (1 to 50, default 50);
    /// stderr names the --to that continues
    #[arg(long, requires = "backfill", value_parser = clap::value_parser!(u32).range(1..=50))]
    pub max_slices: Option<u32>,
}

/// RFC 3339, or a date; a date-only `--to` means 23:59:59 of that day.
pub fn parse_time(s: &str, end_of_day: bool) -> Result<Timestamp, CliError> {
    let s = s.trim();
    if let Ok(t) = s.parse::<Timestamp>() {
        return Ok(t);
    }
    if let Ok(d) = s.parse::<Date>() {
        let dt = if end_of_day {
            d.at(23, 59, 59, 0)
        } else {
            d.at(0, 0, 0, 0)
        };
        return dt
            .to_zoned(jiff::tz::TimeZone::UTC)
            .map(|z| z.timestamp())
            .map_err(|e| CliError::Args(format!("bad time {s:?}: {e}")));
    }
    Err(CliError::Args(format!(
        "bad time {s:?}: use RFC 3339 (2026-10-09T08:00:00Z) or YYYY-MM-DD"
    )))
}

fn parse_groups(values: &[String]) -> Result<Vec<Vec<FilterExpr>>, CliError> {
    values
        .iter()
        .map(|v| {
            v.split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| FilterExpr::parse(s).map_err(CliError::from))
                .collect()
        })
        .collect()
}

/// The query plus the name index when filter resolution already loaded it,
/// so `query()` never fetches the object lists twice in one command.
pub struct BuiltQuery {
    pub query: TrafficQuery,
    pub index: Option<Arc<NameIndex>>,
}

/// Builds the request body. Everything that can be checked offline (window,
/// caps, the `--yes` guard, filter syntax) is checked before the first PCE
/// request; the object lists are fetched once, and only when a filter names
/// something that needs resolving.
pub async fn build_query(ctx: &Context, args: &FilterArgs) -> Result<BuiltQuery, CliError> {
    let mut b = match (&args.from, &args.to) {
        (Some(f), Some(t)) => {
            TrafficQueryBuilder::between(parse_time(f, false)?, parse_time(t, true)?)
        }
        _ => {
            let w = args
                .window
                .clone()
                .unwrap_or_else(|| ctx.config.traffic_default_window.clone());
            TrafficQueryBuilder::last(parse_window(&w)?)?
        }
    };
    if args.port.is_some() || args.proto.is_some() || args.process.is_some() {
        let proto = match &args.proto {
            Some(p) => Some(
                proto_number(p).ok_or_else(|| CliError::Args(format!("unknown protocol {p:?}")))?,
            ),
            None => None,
        };
        b = b.service(ServiceMatch {
            port: args.port,
            proto,
            process_name: args.process.clone(),
        });
    }
    let decisions: Vec<PolicyDecision> = args
        .decision
        .iter()
        .map(|d| {
            PolicyDecision::parse(d).ok_or_else(|| {
                CliError::Args(format!(
                    "unknown decision {d:?}; use allowed, potentially_blocked, blocked"
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    b = b
        .decisions(decisions)
        .max_results(args.max_results.unwrap_or(ctx.config.traffic_max_results));

    // Cost guard on the bounds alone, before any request. A dry run only
    // prints the body, so it never needs confirming.
    let bounds = b.clone().build()?;
    if !args.dry_run
        && !args.yes
        && let Some(reason) = bounds.needs_confirmation()
    {
        return Err(CliError::Args(format!(
            "expensive query: {reason}; re-run with --yes to confirm"
        )));
    }

    let src = parse_groups(&args.src)?;
    let dst = parse_groups(&args.dst)?;
    let ex_src = parse_groups(&args.exclude_src)?;
    let ex_dst = parse_groups(&args.exclude_dst)?;
    let all = || {
        [&src, &dst, &ex_src, &ex_dst]
            .into_iter()
            .flatten()
            .flatten()
    };
    if all().next().is_none() {
        return Ok(BuiltQuery {
            query: bounds,
            index: None,
        });
    }
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let workloads = if all().any(FilterExpr::needs_workloads) {
        ctx.pce
            .workloads(&WorkloadFilter::default(), ctx.cache_mode)
            .await?
            .value
    } else {
        Vec::new()
    };
    let resolve = |groups: &[Vec<FilterExpr>]| -> Result<Vec<Vec<ActorMatch>>, CliError> {
        groups
            .iter()
            .map(|g| {
                g.iter()
                    .map(|e| resolve_actor(e, &idx, &workloads).map_err(CliError::from))
                    .collect()
            })
            .collect()
    };
    for g in resolve(&src)? {
        b = b.source(g);
    }
    for g in resolve(&dst)? {
        b = b.destination(g);
    }
    for m in resolve(&ex_src)?.into_iter().flatten() {
        b = b.exclude_source(m);
    }
    for m in resolve(&ex_dst)?.into_iter().flatten() {
        b = b.exclude_destination(m);
    }
    Ok(BuiltQuery {
        query: b.build()?,
        index: Some(idx),
    })
}

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct DryRun(pub TrafficQuery);

impl TableRows for DryRun {
    fn headers(&self) -> Vec<&'static str> {
        vec!["request body"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        serde_json::to_string_pretty(&self.0)
            .unwrap_or_default()
            .lines()
            .map(|l| vec![l.to_string()])
            .collect()
    }
}

/// Flows as printed: JSON is the flows themselves (every label); the
/// table's label columns follow `order` and hide its ignored keys.
struct FlowRows<'a> {
    flows: &'a [ResolvedFlow],
    order: &'a KeyOrder,
}

impl Serialize for FlowRows<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.flows.serialize(s)
    }
}

impl TableRows for FlowRows<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "last seen",
            "src",
            "src labels",
            "dst",
            "dst labels",
            "service",
            "conns",
            "decision",
            "process",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.flows
            .iter()
            .map(|f| {
                vec![
                    f.last_detected.clone().unwrap_or_default(),
                    f.src.display_name(),
                    f.src.label_text(self.order),
                    f.dst.display_name(),
                    f.dst.label_text(self.order),
                    f.service_text(),
                    f.num_connections.to_string(),
                    f.policy_decision.as_str().to_string(),
                    f.process_name.clone().unwrap_or_default(),
                ]
            })
            .collect()
    }
}

/// A bytes cell: `-` for every row when no row carried bytes.
fn bytes_cell(bytes: u64, any: bool) -> String {
    if any { bytes.to_string() } else { "-".into() }
}

impl TableRows for Vec<FlowGroup> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "src",
            "dst",
            "flows",
            "conns",
            "bytes",
            "allowed",
            "pot.blocked",
            "blocked",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let any = self.iter().any(|g| g.bytes > 0);
        self.iter()
            .map(|g| {
                vec![
                    g.src.join("/"),
                    g.dst.join("/"),
                    g.flows.to_string(),
                    g.connections.to_string(),
                    bytes_cell(g.bytes, any),
                    g.allowed.to_string(),
                    g.potentially_blocked.to_string(),
                    g.blocked.to_string(),
                ]
            })
            .collect()
    }
}

impl TableRows for Vec<MeshRow> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "user",
            "process",
            "src",
            "dst",
            "service",
            "dst process",
            "flows",
            "conns",
            "bytes",
            "decision",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let any = self.iter().any(|r| r.bytes > 0);
        let or_dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".into());
        self.iter()
            .map(|r| {
                vec![
                    or_dash(&r.user),
                    or_dash(&r.process),
                    r.src.join("/"),
                    r.dst.join("/"),
                    r.service_text(),
                    or_dash(&r.dst_process),
                    r.flows.to_string(),
                    r.connections.to_string(),
                    bytes_cell(r.bytes, any),
                    r.dominant().as_str().to_string(),
                ]
            })
            .collect()
    }
}

/// App sections as printed: JSON `{"sections": [{app, direction, rows,
/// totals}]}`; the table one header row per app like the TUI (`shop/prod
/// in · 3 groups · …`, or `← … → …` with both directions) in the first
/// column, then its peers indented (`← ` / `→ ` with `both`).
#[derive(Debug, Serialize)]
pub struct Sections {
    pub sections: Vec<AppSection>,
    /// `--by both`: mark each peer row's direction.
    #[serde(skip)]
    pub both: bool,
}

impl TableRows for Sections {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "app / peer",
            "peer hosts",
            "app hosts",
            "service",
            "flows",
            "conns",
            "bytes",
            "last seen",
            "decision",
        ]
    }
    /// A header row is one cell: `render_table` prints it whole in the
    /// first column with the other cells empty.
    fn rows(&self) -> Vec<Vec<String>> {
        let any = self
            .sections
            .iter()
            .flat_map(|s| &s.rows)
            .any(|r| r.bytes > 0);
        let mut out = Vec::new();
        // An app's sections are adjacent (`app_sections_both`).
        for group in self.sections.chunk_by(|a, b| a.app == b.app) {
            let parts: Vec<(Direction, &pce::traffic::Totals)> =
                group.iter().map(|s| (s.direction, &s.totals)).collect();
            out.push(vec![format!(
                "{}  {}",
                group[0].app_text(),
                app_header(&parts)
            )]);
            for s in group {
                let mark = if self.both {
                    format!("{} ", s.direction.arrow())
                } else {
                    String::new()
                };
                for r in &s.rows {
                    out.push(vec![
                        format!("  {mark}{}", r.peer_text()),
                        r.peer_name_text(),
                        r.app_name_text(),
                        r.service.clone(),
                        r.flows.to_string(),
                        r.connections.to_string(),
                        bytes_cell(r.bytes, any),
                        r.last_detected.clone().unwrap_or_default(),
                        r.dominant().as_str().to_string(),
                    ]);
                }
            }
        }
        out
    }
}

/// `no app matches "pos/PCI"` when `--app` kept no section.
pub fn app_miss(args: &GroupsArgs, s: &Sections) -> Option<String> {
    args.app
        .as_ref()
        .filter(|_| s.sections.is_empty())
        .map(|a| format!("no app matches {a:?}"))
}

/// `--by`, `--show` and `--app` over the loaded flows: the sections, or
/// `None` for `pairs`. `--show` and `--app` need sections.
pub fn sections_for(
    flows: &[ResolvedFlow],
    keys: &[String],
    args: &GroupsArgs,
) -> Result<Option<Sections>, CliError> {
    let decisions = Decisions::from(args.show);
    let mut sections = match args.by {
        ByArg::Pairs => {
            if args.show != ShowArg::All || args.app.is_some() {
                return Err(CliError::Args(
                    "--show and --app need sections: add --by in, out or both".into(),
                ));
            }
            return Ok(None);
        }
        ByArg::In => app_sections(flows, keys, Direction::In, decisions),
        ByArg::Out => app_sections(flows, keys, Direction::Out, decisions),
        ByArg::Both => app_sections_both(flows, keys, decisions),
    };
    if let Some(app) = &args.app {
        sections.retain(|s| &s.app_text() == app);
    }
    Ok(Some(Sections {
        sections,
        both: args.by == ByArg::Both,
    }))
}

pub async fn run(ctx: &Context, cmd: &TrafficCmd) -> Result<(), CliError> {
    match cmd {
        TrafficCmd::Query(a) => query(ctx, a).await,
        TrafficCmd::Groups(a) => groups(ctx, a).await,
        TrafficCmd::Mesh(a) => mesh_cmd(ctx, a).await,
    }
}

/// One query's flows (with `--backfill`, every slice's merged), resolved,
/// and what the envelope reports about them.
struct Fetched {
    flows: Vec<ResolvedFlow>,
    cache: &'static str,
    truncated: bool,
    window: Option<Window>,
    /// How much of the window the flows cover.
    covered: Duration,
    /// `--backfill`: the queries merged.
    slices: Option<u32>,
}

impl Fetched {
    fn meta(&self, ctx: &Context) -> Meta {
        let mut m = ctx.meta(self.cache, self.truncated, self.window.clone());
        m.covered = Some(compact_duration(self.covered));
        m.slices = self.slices;
        m
    }
}

/// What a backfill left: the slices merged, the covered span, and whether
/// the window is still not fully covered (or a slice was cut).
struct Backfilled {
    slices: u32,
    covered: Duration,
    truncated: bool,
}

/// `--backfill` on a truncated result: the whole window again, newest
/// first, in slices sized from the result's rate at the smallest cap that
/// fits it in `MAX_SLICES` (up to the profile's, and past 10000 only with
/// `--yes`). The first slice replaces the truncated rows (an unordered cut
/// of the whole window), later ones merge. A truncated slice is dropped and
/// its range fetched again in halves; one that truncates at 1m stops. At
/// most `max` queries; the rest is reported on stderr. `None` when there
/// is nothing to measure the rate on.
async fn backfill(
    ctx: &Context,
    args: &FilterArgs,
    q: &TrafficQuery,
    idx: &NameIndex,
    flows: &mut Vec<ResolvedFlow>,
) -> Result<Option<Backfilled>, CliError> {
    let (Ok(start), Ok(end)) = (
        q.start_date.parse::<Timestamp>(),
        q.end_date.parse::<Timestamp>(),
    ) else {
        return Ok(None);
    };
    let window = q.window();
    let cov = coverage(flows, window);
    let mut max_cap = ctx.config.traffic_max_results.max(q.max_results);
    if !args.yes {
        max_cap = max_cap.min(CONFIRM_MAX_RESULTS).max(q.max_results);
    }
    let Some(plan) = plan_backfill(&cov, true, q.max_results, max_cap, window) else {
        eprintln!("cannot measure coverage: no last-seen times");
        return Ok(None);
    };
    if plan.cap > q.max_results {
        eprintln!("backfill raises the cap to {}", plan.cap);
    }
    let max = args.max_slices.unwrap_or(MAX_SLICES);
    let mut run = BackfillRun::new(plan.slice, start, end, max);
    let mut slice_q = q.clone();
    slice_q.max_results = plan.cap;
    let mut slices = 0;
    let mut cut = false;
    loop {
        let (s, e) = match run.next_slice() {
            Next::Done => break,
            Next::OverBudget { remaining } => {
                eprintln!(
                    "backfill stopped after {max} queries: {remaining} slices of {} left; \
                     --to {} continues",
                    compact_duration(run.slice),
                    format_timestamp(run.next_end)
                );
                break;
            }
            Next::Slice(s, e) => (s, e),
        };
        slice_q.start_date = format_timestamp(s);
        slice_q.end_date = format_timestamp(e);
        let r = ctx
            .pce
            .traffic(&slice_q, ctx.cache_mode, &CancellationToken::new(), None)
            .await?;
        let step = run.answered(r.truncated);
        if let Step::Retry { slice } = step {
            eprintln!(
                "slice {} to {} truncated at {}: halved to {}",
                format_timestamp(s),
                format_timestamp(e),
                plan.cap,
                compact_duration(slice)
            );
            continue;
        }
        let more = resolve_flows(&r.value.flows, idx);
        if slices == 0 {
            *flows = more;
        } else {
            merge_flows(flows, more);
        }
        slices += 1;
        if step == Step::TooSmall {
            cut = true;
            eprintln!(
                "cap {} is too small for this PCE at 1m: raise --max-results or traffic_max_results",
                plan.cap
            );
            break;
        }
    }
    if slices == 0 {
        return Ok(None); // nothing merged: the truncated result stays
    }
    Ok(Some(Backfilled {
        slices,
        covered: end.duration_since(run.next_end).unsigned_abs().min(window),
        truncated: cut || run.next_end > start,
    }))
}

/// Builds and runs the query (cost guard, cache, one name index). A dry
/// run prints the request body and returns `None`.
async fn fetch(ctx: &Context, args: &FilterArgs) -> Result<Option<Fetched>, CliError> {
    let BuiltQuery { query: q, index } = build_query(ctx, args).await?;
    let window = Some(Window {
        start: q.start_date.clone(),
        end: q.end_date.clone(),
    });
    if args.dry_run {
        ctx.emit(&DryRun(q), "none", false, window)?;
        return Ok(None);
    }
    let idx = match index {
        Some(idx) => idx,
        None => ctx.pce.name_index(ctx.cache_mode).await?,
    };
    let result = ctx
        .pce
        .traffic(&q, ctx.cache_mode, &CancellationToken::new(), None)
        .await?;
    let mut f = Fetched {
        flows: resolve_flows(&result.value.flows, &idx),
        cache: cache_label(result.hit, ctx.cache_mode),
        truncated: result.truncated,
        window,
        covered: Duration::ZERO,
        slices: None,
    };
    // A complete result covers its window, whatever its rows' times.
    f.covered = if f.truncated {
        coverage(&f.flows, q.window()).covered
    } else {
        q.window()
    };
    if args.backfill {
        f.slices = Some(0);
        if f.truncated
            && let Some(b) = backfill(ctx, args, &q, &idx, &mut f.flows).await?
        {
            f.slices = Some(b.slices);
            f.covered = b.covered;
            f.truncated = b.truncated;
        }
    }
    Ok(Some(f))
}

/// `--keys`, else the profile's scope keys.
fn keys_or_profile(ctx: &Context, keys: &[String]) -> Vec<String> {
    if keys.is_empty() {
        ctx.config.scope_keys.clone()
    } else {
        keys.to_vec()
    }
}

async fn query(ctx: &Context, args: &QueryArgs) -> Result<(), CliError> {
    let Some(f) = fetch(ctx, &args.filter).await? else {
        return Ok(());
    };
    if args.blocked {
        let keys = keys_or_profile(ctx, &[]);
        let sections = Sections {
            sections: app_sections(&f.flows, &keys, Direction::In, Decisions::Blocked),
            both: false,
        };
        ctx.emit_meta(&sections, &f.meta(ctx))
    } else if !args.group.is_empty() {
        ctx.emit_meta(&group_by_labels(&f.flows, &args.group), &f.meta(ctx))
    } else {
        let order = ctx.config.key_order();
        let rows = FlowRows {
            flows: &f.flows,
            order: &order,
        };
        ctx.emit_meta(&rows, &f.meta(ctx))
    }
}

async fn groups(ctx: &Context, args: &GroupsArgs) -> Result<(), CliError> {
    // Bad flag combinations fail before any request.
    sections_for(&[], &[], args)?;
    let Some(f) = fetch(ctx, &args.filter).await? else {
        return Ok(());
    };
    let keys = keys_or_profile(ctx, &args.keys);
    match sections_for(&f.flows, &keys, args)? {
        Some(sections) => {
            if let Some(miss) = app_miss(args, &sections) {
                eprintln!("{miss}");
            }
            ctx.emit_meta(&sections, &f.meta(ctx))
        }
        None => ctx.emit_meta(&group_by_labels(&f.flows, &keys), &f.meta(ctx)),
    }
}

async fn mesh_cmd(ctx: &Context, args: &MeshArgs) -> Result<(), CliError> {
    let Some(f) = fetch(ctx, &args.filter).await? else {
        return Ok(());
    };
    let keys = keys_or_profile(ctx, &args.keys);
    let cols = MeshColumns {
        user: !args.no_user,
        process: !args.no_process,
    };
    ctx.emit_meta(&mesh(&f.flows, &keys, cols), &f.meta(ctx))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(bytes: u64) -> FlowGroup {
        FlowGroup {
            src: vec!["a".into()],
            dst: vec!["b".into()],
            flows: 1,
            connections: 2,
            bytes,
            allowed: 1,
            potentially_blocked: 0,
            blocked: 0,
        }
    }

    #[test]
    fn bytes_cells_are_a_dash_only_when_no_row_has_bytes() {
        assert_eq!(vec![group(0), group(0)].rows()[0][4], "-");
        let rows = vec![group(5), group(0)].rows();
        assert_eq!((rows[0][4].as_str(), rows[1][4].as_str()), ("5", "0"));
        let row = MeshRow {
            user: None,
            process: Some("java".into()),
            src: vec!["a".into()],
            dst: vec!["b".into()],
            port: None,
            proto: "icmp".into(),
            dst_process: None,
            flows: 1,
            connections: 3,
            bytes: 0,
            allowed: 0,
            potentially_blocked: 1,
            blocked: 0,
        };
        assert_eq!(
            vec![row].rows()[0],
            vec![
                "-",
                "java",
                "a",
                "b",
                "icmp",
                "-",
                "1",
                "3",
                "-",
                "potentially_blocked"
            ]
        );
    }

    #[test]
    fn table_label_columns_follow_the_index_key_order() {
        let labels: Vec<pce::models::Label> =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        let raw: Vec<pce::models::Flow> =
            serde_json::from_str(include_str!("../../../../fixtures/traffic_flows.json")).unwrap();
        let idx = NameIndex::new(&labels, &[], &[], &[])
            .with_key_order(pce::KeyOrder::new(&["env".into()], "role"));
        let flows = pce::traffic::resolve_flows(&raw, &idx);
        let table = FlowRows {
            flows: &flows,
            order: idx.key_order(),
        };
        let headers = table.headers();
        let col = |h: &str| headers.iter().position(|x| *x == h).unwrap();
        let rows = table.rows();
        assert_eq!(rows[0][col("src labels")], "env=prod role=web app=shop");
        assert_eq!(rows[0][col("dst labels")], "env=prod role=db app=shop");
    }

    #[test]
    fn table_label_columns_hide_ignored_keys_and_json_keeps_them() {
        let labels: Vec<pce::models::Label> =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        let raw: Vec<pce::models::Flow> =
            serde_json::from_str(include_str!("../../../../fixtures/traffic_flows.json")).unwrap();
        let idx = NameIndex::new(&labels, &[], &[], &[]);
        let flows = pce::traffic::resolve_flows(&raw, &idx);
        let order = KeyOrder::new(&["app".into()], "role").with_ignored(&["env".into()]);
        let table = FlowRows {
            flows: &flows,
            order: &order,
        };
        let headers = table.headers();
        let col = |h: &str| headers.iter().position(|x| *x == h).unwrap();
        let rows = table.rows();
        assert_eq!(rows[0][col("src labels")], "app=shop role=web");
        assert_eq!(rows[0][col("dst labels")], "app=shop role=db");
        let json = serde_json::to_value(&table).unwrap();
        assert_eq!(
            json,
            serde_json::to_value(&flows).unwrap(),
            "JSON is the flows"
        );
        let keys: Vec<&str> = json[0]["src"]["labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|l| l["key"].as_str())
            .collect();
        assert!(keys.contains(&"env"), "{keys:?}");
    }
    fn groups_args(argv: &[&str]) -> GroupsArgs {
        use clap::Parser;
        let mut full = vec!["illumio-tui", "traffic", "groups"];
        full.extend_from_slice(argv);
        let cli = crate::cli::Cli::try_parse_from(full).expect("parses");
        match cli.command {
            Some(crate::cli::Command::Traffic(TrafficCmd::Groups(a))) => a,
            other => panic!("{other:?}"),
        }
    }

    fn app_env() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    #[test]
    fn by_parses_and_defaults_to_pairs_which_has_no_sections() {
        let a = groups_args(&[]);
        assert_eq!(
            (a.by, a.show, a.app.as_deref()),
            (ByArg::Pairs, ShowArg::All, None)
        );
        let flows = crate::tui::test_support::fixture_flows_rich();
        assert!(sections_for(&flows, &app_env(), &a).unwrap().is_none());
        for (arg, by) in [
            ("in", ByArg::In),
            ("out", ByArg::Out),
            ("both", ByArg::Both),
        ] {
            assert_eq!(groups_args(&["--by", arg]).by, by);
        }
        let ins = sections_for(&flows, &app_env(), &groups_args(&["--by", "in"]))
            .unwrap()
            .unwrap();
        assert!(ins.sections.iter().all(|s| s.direction == Direction::In));
        assert_eq!(ins.sections[0].app_text(), "crm/prod");
        let both = sections_for(&flows, &app_env(), &groups_args(&["--by", "both"]))
            .unwrap()
            .unwrap();
        assert!(both.sections.iter().any(|s| s.direction == Direction::Out));
        assert!(
            <crate::cli::Cli as clap::Parser>::try_parse_from([
                "illumio-tui",
                "traffic",
                "groups",
                "--by",
                "x"
            ])
            .is_err()
        );
    }

    #[test]
    fn show_filters_the_decisions_and_app_keeps_one_apps_sections() {
        let flows = crate::tui::test_support::fixture_flows_rich();
        let count = |argv: &[&str]| -> u64 {
            sections_for(&flows, &app_env(), &groups_args(argv))
                .unwrap()
                .unwrap()
                .sections
                .iter()
                .map(|s| s.totals.flows)
                .sum()
        };
        assert_eq!(count(&["--by", "in"]), 30);
        assert_eq!(count(&["--by", "in", "--show", "blocked"]), 14);
        assert_eq!(count(&["--by", "in", "--show", "blocked-only"]), 7);
        let a = groups_args(&["--by", "both", "--app", "shop/prod"]);
        let s = sections_for(&flows, &app_env(), &a).unwrap().unwrap();
        assert_eq!(s.sections.len(), 2);
        assert!(s.sections.iter().all(|s| s.app_text() == "shop/prod"));
        assert_eq!(
            (s.sections[0].direction, s.sections[1].direction),
            (Direction::In, Direction::Out)
        );
        for argv in [&["--app", "shop/prod"][..], &["--show", "blocked"][..]] {
            let err = sections_for(&flows, &app_env(), &groups_args(argv)).unwrap_err();
            assert!(err.to_string().contains("--by"), "{err}");
        }
    }

    #[test]
    fn both_prints_one_header_per_app_and_marks_the_direction() {
        let flows = crate::tui::test_support::fixture_flows_rich();
        let a = groups_args(&["--by", "both", "--app", "shop/prod"]);
        let s = sections_for(&flows, &app_env(), &a).unwrap().unwrap();
        let rows = s.rows();
        let headers: Vec<&Vec<String>> = rows.iter().filter(|r| r.len() == 1).collect();
        assert_eq!(
            headers,
            vec![&vec![
                "shop/prod  ← 1545 conns · 2 blocked · 1 pot.blocked  → 1004 conns".to_string()
            ]]
        );
        assert!(rows[1][0].starts_with("  ← "), "{:?}", rows[1]);
        assert!(rows.last().unwrap()[0].starts_with("  → "));
        assert_eq!(app_miss(&a, &s), None);
        let a = groups_args(&["--by", "in", "--app", "pos/PCI"]);
        let s = sections_for(&flows, &app_env(), &a).unwrap().unwrap();
        assert!(s.sections.is_empty());
        assert_eq!(
            app_miss(&a, &s).as_deref(),
            Some("no app matches \"pos/PCI\"")
        );
    }

    #[test]
    fn sections_print_header_rows_then_indented_peers_and_json_has_sections() {
        let flows = crate::tui::test_support::fixture_flows_rich();
        let s = Sections {
            sections: app_sections(&flows, &app_env(), Direction::In, Decisions::Blocked),
            both: false,
        };
        let rows = s.rows();
        assert_eq!(
            rows[0],
            vec!["erp/prod  in · 3 groups · 102 conns · 2 blocked · 3 pot.blocked"],
            "a header row: the first column only"
        );
        assert_eq!(rows[1][0], "  erp/dev");
        assert_eq!(rows[1][2], "erp-api-p.example");
        assert_eq!(rows[1][3], "8443/tcp");
        let text = crate::cli::output::render_table(&s.headers(), &rows);
        assert!(text.lines().next().unwrap().starts_with("app / peer"));
        assert_eq!(
            text.lines().nth(2),
            Some("erp/prod  in · 3 groups · 102 conns · 2 blocked · 3 pot.blocked"),
            "printed whole, not clipped"
        );
        assert!(
            text.lines().nth(3).unwrap().starts_with("  erp/dev   "),
            "the header does not widen the first column: {text}"
        );
        assert!(text.contains("\n  ipl:Corp/-"), "{text}");
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(
            json["sections"][0]["app"],
            serde_json::json!(["erp", "prod"])
        );
        assert_eq!(json["sections"][0]["direction"], "in");
        assert_eq!(json["sections"][0]["totals"]["connections"], 102);
        assert_eq!(json["sections"][0]["rows"][0]["service"], "8443/tcp");
    }
}
