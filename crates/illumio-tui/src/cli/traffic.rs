//! `traffic query|groups|mesh`: build an Explorer query from human filters,
//! guard its cost, run it through the cached facade, and print flows,
//! label-pair groups, the blocked summary or the mesh.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{Meta, TableRows, Window, join_list};
use clap::{Args, Subcommand, ValueEnum};
use jiff::Timestamp;
use jiff::civil::Date;
use pce::models::{
    ActorMatch, PolicyDecision, ServiceMatch, TrafficQuery, Transmission, WorkloadFilter,
    proto_number,
};
use pce::traffic::{
    AppSection, CONFIRM_MAX_RESULTS, Decisions, Direction, FilterExpr, FlowGroup, Focus,
    FocusDirection, MAX_TRAFFIC_MAX_RESULTS, MeshColumns, MeshRow, ResolvedFlow,
    TrafficQueryBuilder, app_header, app_sections, app_sections_both, backfill_cap,
    compact_duration, group_by_labels, group_by_service, mesh, parse_app, parse_window,
    resolve_actor, resolve_flows,
};
use pce::{KeyOrder, NameIndex};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The unit line above a grouped traffic table, and `--help`'s.
pub const UNITS_GROUPS: &str =
    "flows: flow records · conns, bytes: lifetime counts of those records";
/// The unit line above a table of flow records.
pub const UNITS_FLOWS: &str = "one row per flow record · conns: its lifetime count";

#[derive(Subcommand, Debug)]
pub enum TrafficCmd {
    /// Run an Explorer traffic query
    ///
    /// Each row is one flow record, an aggregate the PCE keeps: its conns (and bytes) are
    /// lifetime counts of that record, as in the PCE UI, not counts for the window.
    Query(QueryArgs),
    /// Label-pair groups of the query's flows (connections, bytes, decisions)
    ///
    /// Units: flows, allowed, pot.blocked and blocked count flow records; conns, bytes,
    /// allowed_conns, pot_blocked_conns and blocked_conns are the lifetime counts of those
    /// records, as in the PCE UI. The per-decision connection columns are in JSON and CSV.
    Groups(GroupsArgs),
    /// The mesh: user, process, source, destination, service, destination process
    ///
    /// Units: flows counts flow records; conns and bytes are the lifetime counts of those
    /// records, as in the PCE UI.
    Mesh(MeshArgs),
}

impl TrafficCmd {
    /// The Explorer filter the subcommand runs.
    pub fn filter(&self) -> &FilterArgs {
        match self {
            TrafficCmd::Query(a) => &a.filter,
            TrafficCmd::Groups(a) => &a.filter,
            TrafficCmd::Mesh(a) => &a.filter,
        }
    }
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

/// `traffic groups --show`: which decisions the pairs or sections count.
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

/// `--transmission`: how the flows were sent.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransmissionArg {
    Unicast,
    Broadcast,
    Multicast,
}

impl From<TransmissionArg> for Transmission {
    fn from(t: TransmissionArg) -> Self {
        match t {
            TransmissionArg::Unicast => Transmission::Unicast,
            TransmissionArg::Broadcast => Transmission::Broadcast,
            TransmissionArg::Multicast => Transmission::Multicast,
        }
    }
}

/// `--direction`: which side of the flows `--app` is on.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionArg {
    /// The app is the destination
    In,
    /// The app is the source
    Out,
    /// Either (two queries, merged)
    Both,
}

impl From<DirectionArg> for FocusDirection {
    fn from(d: DirectionArg) -> Self {
        match d {
            DirectionArg::In => FocusDirection::In,
            DirectionArg::Out => FocusDirection::Out,
            DirectionArg::Both => FocusDirection::Both,
        }
    }
}

/// `--app` / `--direction` against the keys the command groups by: the
/// focus, and whether its labels go into the query. A name-keyed group
/// (`unmanaged-nas/-`, `ipl:Corp/-`) has no labels to query by: with `--by`
/// sections it keeps narrowing an unfocused query's sections, as before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliFocus {
    pub focus: Focus,
    pub query: bool,
}

/// The offline checks of `--app`: `--direction` with `--by` sections (the
/// sections already choose a direction), `--backfill` with a focused query
/// (it re-runs the unfocused query). Without `--direction` the direction is
/// `--by`'s (`in`, `out`), else `both`.
pub fn cli_focus(
    args: &FilterArgs,
    keys: &[String],
    by: ByArg,
) -> Result<Option<CliFocus>, CliError> {
    let Some(text) = &args.app else {
        return Ok(None);
    };
    if args.direction.is_some() && by != ByArg::Pairs {
        return Err(CliError::Args(
            "--direction conflicts with --by: the sections already choose a direction".into(),
        ));
    }
    let direction = match (args.direction, by) {
        (Some(d), _) => d.into(),
        (None, ByArg::In) => FocusDirection::In,
        (None, ByArg::Out) => FocusDirection::Out,
        (None, _) => FocusDirection::Both,
    };
    let focus = Focus::new(parse_app(text), keys.to_vec()).with_direction(direction);
    let query = focus.is_label_group() || by == ByArg::Pairs;
    if query && args.backfill {
        return Err(CliError::Args(
            "--backfill does not combine with --app: raise --max-results instead".into(),
        ));
    }
    Ok(Some(CliFocus { focus, query }))
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
    /// Count all flows, blocked (and potentially blocked) or blocked-only, in the pairs
    /// (with `--services` too) or the sections. Applied to the loaded flows; `--decision`
    /// filters the query itself
    #[arg(long, value_enum, default_value_t)]
    pub show: ShowArg,
    /// Pairs only: one row per source group, destination group and service
    /// (port/proto), with a `service` column (sections are already per service)
    #[arg(long, conflicts_with = "by")]
    pub services: bool,
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
    /// Only flows sent this way: unicast, broadcast, multicast (repeatable or comma-separated;
    /// default all). Explorer excludes the other kinds on the destination side. workloader
    /// counts unicast only by default: add --transmission unicast to compare
    #[arg(long, value_enum, value_delimiter = ',')]
    pub transmission: Vec<TransmissionArg>,
    /// Row cap, up to 200000 (default from profile, 10000; > 10000 needs --yes)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Print the request body instead of sending it
    #[arg(long)]
    pub dry_run: bool,
    /// Confirm an expensive query (window > 7d or max_results > 10000)
    #[arg(long)]
    pub yes: bool,
    /// When the result is truncated, re-run the same window at a larger cap until it is
    /// complete: 10000, then 25000, 50000, 100000, 200000 (above 10000 only with --yes).
    /// Each run replaces the last; meta.cap_steps lists the caps tried
    #[arg(long)]
    pub backfill: bool,
    /// Removed: time slices gave wrong counts. Passing it exits 2
    #[arg(long, hide = true, num_args = 0..=1)]
    pub max_slices: Option<Option<String>>,
    /// Focus on one app, e.g. shop/prod: a group of the keys the command groups by (groups and
    /// mesh: --keys; query: --group; else the profile's scope keys). Its labels go into the
    /// query, so the cap is spent on it
    #[arg(long)]
    pub app: Option<String>,
    /// With --app: in (the app is the destination), out (the source) or both
    /// (default; two queries, merged)
    #[arg(long, value_enum, requires = "app")]
    pub direction: Option<DirectionArg>,
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
/// `queries` is how many Explorer queries the body will run as (`--app
/// --direction both`: 2); the guard's message names it.
pub async fn build_query(
    ctx: &Context,
    args: &FilterArgs,
    queries: u32,
) -> Result<BuiltQuery, CliError> {
    if args.max_slices.is_some() {
        return Err(CliError::Args(
            "time slices were removed: --backfill raises the cap".into(),
        ));
    }
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
    let transmission: Vec<Transmission> = args
        .transmission
        .iter()
        .copied()
        .map(Transmission::from)
        .collect();
    b = b
        .decisions(decisions)
        .transmission(&transmission)
        .max_results(args.max_results.unwrap_or(ctx.config.traffic_max_results));

    // Cost guard on the bounds alone, before any request. A dry run only
    // prints the body, so it never needs confirming.
    let bounds = b.clone().build()?;
    if !args.dry_run
        && !args.yes
        && let Some(reason) = bounds.needs_confirmation()
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

/// `--dry-run` with `--app`: one request body per side, `in` first.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct DryRuns(pub Vec<TrafficQuery>);

impl TableRows for DryRuns {
    fn headers(&self) -> Vec<&'static str> {
        vec!["request body"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .flat_map(|q| {
                DryRun(q.clone())
                    .rows()
                    .into_iter()
                    .chain(std::iter::once(vec![String::new()]))
            })
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
    fn units(&self) -> Option<&'static str> {
        Some(UNITS_FLOWS)
    }
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
    /// Every label (ignored keys too, as in JSON), one list entry each.
    fn full_rows(&self) -> Vec<Vec<String>> {
        let labels = |e: &pce::traffic::ResolvedEndpoint| {
            join_list(e.labels.iter().map(|l| format!("{}={}", l.key, l.value)))
        };
        self.flows
            .iter()
            .zip(self.rows())
            .map(|(f, mut row)| {
                row[2] = labels(&f.src);
                row[4] = labels(&f.dst);
                row
            })
            .collect()
    }
}

/// A bytes cell: `-` for every row when no row carried bytes.
fn bytes_cell(bytes: u64, any: bool) -> String {
    if any { bytes.to_string() } else { "-".into() }
}

/// The per-decision connection columns `--csv` adds to pairs and sections.
pub const CONNS_BY_DECISION: [&str; 3] = ["allowed_conns", "pot_blocked_conns", "blocked_conns"];

impl TableRows for Vec<FlowGroup> {
    fn units(&self) -> Option<&'static str> {
        Some(UNITS_GROUPS)
    }
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
    /// The table's, then the connections per decision.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        h.extend(CONNS_BY_DECISION);
        h
    }
    /// `bytes` always a number (0 when no flow carried any), then the
    /// connections per decision.
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .zip(self.rows())
            .map(|(g, mut row)| {
                row[4] = g.bytes.to_string();
                row.extend(g.conns.cells());
                row
            })
            .collect()
    }
}

/// `traffic groups --services`: the pairs split by service, a `service`
/// column after `dst` (JSON: the pairs with a `service` field).
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct ServiceGroups(pub Vec<FlowGroup>);

impl ServiceGroups {
    fn with_service(&self, rows: Vec<Vec<String>>) -> Vec<Vec<String>> {
        self.0
            .iter()
            .zip(rows)
            .map(|(g, mut row)| {
                row.insert(2, g.service.clone().unwrap_or_default());
                row
            })
            .collect()
    }
}

impl TableRows for ServiceGroups {
    fn units(&self) -> Option<&'static str> {
        Some(UNITS_GROUPS)
    }
    fn headers(&self) -> Vec<&'static str> {
        let mut h = self.0.headers();
        h.insert(2, "service");
        h
    }
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.0.full_headers();
        h.insert(2, "service");
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.with_service(self.0.rows())
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.with_service(self.0.full_rows())
    }
}

impl TableRows for Vec<MeshRow> {
    fn units(&self) -> Option<&'static str> {
        Some(UNITS_GROUPS)
    }
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
    /// Empty cells for no user or process, `bytes` always a number.
    fn full_rows(&self) -> Vec<Vec<String>> {
        let or_empty = |v: &Option<String>| v.clone().unwrap_or_default();
        self.iter()
            .zip(self.rows())
            .map(|(r, mut row)| {
                row[0] = or_empty(&r.user);
                row[1] = or_empty(&r.process);
                row[5] = or_empty(&r.dst_process);
                row[8] = r.bytes.to_string();
                row
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
    fn units(&self) -> Option<&'static str> {
        Some(UNITS_GROUPS)
    }
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
    /// One row per peer, no header rows: the app and the direction are
    /// columns, the host names lists beside their counts.
    fn full_headers(&self) -> Vec<&'static str> {
        SECTION_CSV_HEADERS.to_vec()
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.sections
            .iter()
            .flat_map(|s| s.rows.iter().map(|r| peer_csv_row(&s.app, s.direction, r)))
            .collect()
    }
}

/// The CSV columns of app sections, one row per peer (`--csv` and the
/// TUI's `:export`).
pub const SECTION_CSV_HEADERS: [&str; 16] = [
    "app",
    "direction",
    "peer",
    "peer hosts",
    "peer names",
    "app hosts",
    "app names",
    "service",
    "flows",
    "conns",
    "bytes",
    "last seen",
    "decision",
    CONNS_BY_DECISION[0],
    CONNS_BY_DECISION[1],
    CONNS_BY_DECISION[2],
];

/// One peer row of an app's section under `SECTION_CSV_HEADERS`.
pub fn peer_csv_row(
    app: &[String],
    direction: Direction,
    r: &pce::traffic::PeerRow,
) -> Vec<String> {
    vec![
        app.join("/"),
        direction.as_str().to_string(),
        r.peer_text(),
        r.peer_hosts.to_string(),
        join_list(&r.peer_names),
        r.app_hosts.to_string(),
        join_list(&r.app_names),
        r.service.clone(),
        r.flows.to_string(),
        r.connections.to_string(),
        r.bytes.to_string(),
        r.last_detected.clone().unwrap_or_default(),
        r.dominant().as_str().to_string(),
        r.conns.allowed_conns.to_string(),
        r.conns.pot_blocked_conns.to_string(),
        r.conns.blocked_conns.to_string(),
    ]
}

/// `no app matches "pos/PCI"` when `--app` kept no section.
pub fn app_miss(args: &GroupsArgs, s: &Sections) -> Option<String> {
    args.filter
        .app
        .as_ref()
        .filter(|_| s.sections.is_empty())
        .map(|a| format!("no app matches {a:?}"))
}

/// `--by`, `--show` and `--app` over the loaded flows: the sections (only
/// `--app`'s with it), or `None` for `pairs` (see `pairs_for`).
pub fn sections_for(
    flows: &[ResolvedFlow],
    keys: &[String],
    args: &GroupsArgs,
) -> Option<Sections> {
    let decisions = Decisions::from(args.show);
    let mut sections = match args.by {
        ByArg::Pairs => return None,
        ByArg::In => app_sections(flows, keys, Direction::In, decisions),
        ByArg::Out => app_sections(flows, keys, Direction::Out, decisions),
        ByArg::Both => app_sections_both(flows, keys, decisions),
    };
    if let Some(app) = &args.filter.app {
        sections.retain(|s| &s.app_text() == app);
    }
    Some(Sections {
        sections,
        both: args.by == ByArg::Both,
    })
}

/// The pairs (by service with `--services`) of the loaded flows `--show`
/// keeps.
pub fn pairs_for(flows: &[ResolvedFlow], keys: &[String], args: &GroupsArgs) -> Vec<FlowGroup> {
    let kept = Decisions::from(args.show).kept(flows);
    if args.services {
        group_by_service(&kept, keys)
    } else {
        group_by_labels(&kept, keys)
    }
}

pub async fn run(ctx: &Context, cmd: &TrafficCmd) -> Result<(), CliError> {
    match cmd {
        TrafficCmd::Query(a) => query(ctx, a).await,
        TrafficCmd::Groups(a) => groups(ctx, a).await,
        TrafficCmd::Mesh(a) => mesh_cmd(ctx, a).await,
    }
}

/// One query's flows (with `--backfill`, the last cap's), resolved, and
/// what the envelope reports about them.
pub(crate) struct Fetched {
    pub(crate) flows: Vec<ResolvedFlow>,
    pub(crate) cache: &'static str,
    pub(crate) truncated: bool,
    /// A supercluster region did not respond.
    pub(crate) incomplete: bool,
    pub(crate) window: Option<Window>,
    /// The window's length: what a complete result covers.
    pub(crate) span: Duration,
    /// The cap the flows were fetched at.
    pub(crate) cap: u32,
    /// `--backfill`: every cap tried, the first query's included.
    pub(crate) cap_steps: Option<Vec<u32>>,
    /// `--app`: the Explorer queries the result took.
    pub(crate) queries: Option<u32>,
}

impl Fetched {
    pub(crate) fn meta(&self, ctx: &Context) -> Meta {
        let mut m = ctx.meta(self.cache, self.truncated, self.window.clone());
        // A complete result covers its window; a truncated one an unknown
        // part of it, so it claims no span.
        m.covered = Some((!self.truncated).then(|| compact_duration(self.span)));
        m.cap = Some(self.cap);
        m.cap_steps = self.cap_steps.clone();
        m.incomplete = Some(self.incomplete);
        m.queries = self.queries;
        m
    }
}

/// `--backfill` on a truncated result: the same window again at the next
/// cap (`backfill_cap`: 10000, then 25000 to 200000), each answer replacing
/// the last, until one is complete or the next cap needs `--yes`. Explorer
/// cannot page a result, so this is the only way to the rest of it.
async fn backfill(
    ctx: &Context,
    args: &FilterArgs,
    q: &TrafficQuery,
    idx: &NameIndex,
    f: &mut Fetched,
) -> Result<(), CliError> {
    let ceiling = if args.yes {
        MAX_TRAFFIC_MAX_RESULTS
    } else {
        CONFIRM_MAX_RESULTS
    };
    let mut q = q.clone();
    let mut steps = vec![q.max_results];
    while f.truncated {
        let Some(next) = backfill_cap(q.max_results) else {
            eprintln!(
                "backfill stopped: truncated at the maximum cap of {MAX_TRAFFIC_MAX_RESULTS}; narrow the filter"
            );
            break;
        };
        if next > ceiling {
            eprintln!(
                "backfill stopped: truncated at cap {}; a cap of {next} needs --yes",
                q.max_results
            );
            break;
        }
        eprintln!("backfill raises the cap to {next}");
        q.max_results = next;
        let r = ctx
            .pce
            .traffic(&q, ctx.cache_mode, &CancellationToken::new(), None)
            .await?;
        steps.push(next);
        f.flows = resolve_flows(&r.value.flows, idx);
        f.cache = cache_label(r.hit, ctx.cache_mode);
        f.truncated = r.truncated;
        f.incomplete = r.value.incomplete;
        f.cap = next;
    }
    f.cap_steps = Some(steps);
    Ok(())
}

/// Builds and runs the query (cost guard, cache, one name index): with a
/// focused `--app`, one query per side through `Pce::focused_traffic`,
/// after the app's labels are checked (an unknown one is exit 2 before any
/// traffic request). An unfocused query is submitted while the name index
/// loads, so the PCE computes meanwhile. A dry run prints the request body
/// (with `--app`, the bodies) and returns `None`.
pub(crate) async fn fetch(
    ctx: &Context,
    args: &FilterArgs,
    focus: Option<&CliFocus>,
) -> Result<Option<Fetched>, CliError> {
    let n = focus
        .filter(|f| f.query)
        .map_or(1, |f| f.focus.direction.query_count());
    let BuiltQuery { query: q, index } = build_query(ctx, args, n).await?;
    let window = Some(Window {
        start: q.start_date.clone(),
        end: q.end_date.clone(),
    });
    let focus = focus.filter(|f| f.query).map(|f| &f.focus);
    if args.dry_run && focus.is_none() {
        ctx.emit(&DryRun(q), "none", false, window)?;
        return Ok(None);
    }
    if let Some(focus) = focus {
        let idx = match index {
            Some(idx) => idx,
            None => ctx.pce.name_index(ctx.cache_mode).await?,
        };
        let queries = focus.queries(&q, &idx).map_err(CliError::Args)?;
        if args.dry_run {
            let bodies = DryRuns(queries.into_iter().map(|(_, q)| q).collect());
            ctx.emit(&bodies, "none", false, window)?;
            return Ok(None);
        }
        let ft = ctx
            .pce
            .focused_traffic(
                "focus",
                focus,
                &q,
                &idx,
                ctx.cache_mode,
                &CancellationToken::new(),
                None,
            )
            .await?;
        return Ok(Some(Fetched {
            queries: Some(ft.query_count()),
            cache: cache_label(ft.hit, ctx.cache_mode),
            truncated: ft.truncated,
            incomplete: ft.incomplete,
            window,
            span: q.window(),
            cap: q.max_results,
            cap_steps: None,
            flows: ft.flows,
        }));
    }
    let cancel = CancellationToken::new();
    let (result, idx) = match index {
        Some(idx) => (
            ctx.pce.traffic(&q, ctx.cache_mode, &cancel, None).await?,
            idx,
        ),
        // The first error stops the other: a failed name index drops the
        // traffic query's polls and download.
        None => tokio::try_join!(
            ctx.pce.traffic(&q, ctx.cache_mode, &cancel, None),
            ctx.pce.name_index(ctx.cache_mode)
        )?,
    };
    let mut f = Fetched {
        flows: resolve_flows(&result.value.flows, &idx),
        cache: cache_label(result.hit, ctx.cache_mode),
        truncated: result.truncated,
        incomplete: result.value.incomplete,
        window,
        span: q.window(),
        cap: q.max_results,
        cap_steps: None,
        queries: None,
    };
    if args.backfill {
        backfill(ctx, args, &q, &idx, &mut f).await?;
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
    let keys = if args.group.is_empty() {
        keys_or_profile(ctx, &[])
    } else {
        args.group.clone()
    };
    let focus = cli_focus(&args.filter, &keys, ByArg::Pairs)?;
    let Some(f) = fetch(ctx, &args.filter, focus.as_ref()).await? else {
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
    let keys = keys_or_profile(ctx, &args.keys);
    let focus = cli_focus(&args.filter, &keys, args.by)?;
    let Some(f) = fetch(ctx, &args.filter, focus.as_ref()).await? else {
        return Ok(());
    };
    match sections_for(&f.flows, &keys, args) {
        Some(sections) => {
            if let Some(miss) = app_miss(args, &sections) {
                eprintln!("{miss}");
            }
            ctx.emit_meta(&sections, &f.meta(ctx))
        }
        None if args.services => ctx.emit_meta(
            &ServiceGroups(pairs_for(&f.flows, &keys, args)),
            &f.meta(ctx),
        ),
        None => ctx.emit_meta(&pairs_for(&f.flows, &keys, args), &f.meta(ctx)),
    }
}

async fn mesh_cmd(ctx: &Context, args: &MeshArgs) -> Result<(), CliError> {
    let keys = keys_or_profile(ctx, &args.keys);
    let focus = cli_focus(&args.filter, &keys, ByArg::Pairs)?;
    let Some(f) = fetch(ctx, &args.filter, focus.as_ref()).await? else {
        return Ok(());
    };
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
            service: None,
            flows: 1,
            connections: 2,
            bytes,
            allowed: 1,
            potentially_blocked: 0,
            blocked: 0,
            conns: Default::default(),
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
            (a.by, a.show, a.filter.app.as_deref()),
            (ByArg::Pairs, ShowArg::All, None)
        );
        let flows = crate::tui::test_support::fixture_flows_rich();
        assert!(sections_for(&flows, &app_env(), &a).is_none());
        for (arg, by) in [
            ("in", ByArg::In),
            ("out", ByArg::Out),
            ("both", ByArg::Both),
        ] {
            assert_eq!(groups_args(&["--by", arg]).by, by);
        }
        let ins = sections_for(&flows, &app_env(), &groups_args(&["--by", "in"])).unwrap();
        assert!(ins.sections.iter().all(|s| s.direction == Direction::In));
        assert_eq!(ins.sections[0].app_text(), "crm/prod");
        let both = sections_for(&flows, &app_env(), &groups_args(&["--by", "both"])).unwrap();
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
                .sections
                .iter()
                .map(|s| s.totals.flows)
                .sum()
        };
        assert_eq!(count(&["--by", "in"]), 30);
        assert_eq!(count(&["--by", "in", "--show", "blocked"]), 14);
        assert_eq!(count(&["--by", "in", "--show", "blocked-only"]), 7);
        let a = groups_args(&["--by", "both", "--app", "shop/prod"]);
        let s = sections_for(&flows, &app_env(), &a).unwrap();
        assert_eq!(s.sections.len(), 2);
        assert!(s.sections.iter().all(|s| s.app_text() == "shop/prod"));
        assert_eq!(
            (s.sections[0].direction, s.sections[1].direction),
            (Direction::In, Direction::Out)
        );
        assert!(
            sections_for(&flows, &app_env(), &groups_args(&["--app", "shop/prod"])).is_none(),
            "--app without --by gives the pairs of the focused flows"
        );
        let a = groups_args(&["--show", "blocked"]);
        assert!(sections_for(&flows, &app_env(), &a).is_none());
    }

    #[test]
    fn show_filters_the_pairs_and_the_pairs_by_service() {
        let flows = crate::tui::test_support::fixture_flows_rich();
        let pairs = |argv: &[&str]| pairs_for(&flows, &app_env(), &groups_args(argv));
        let total = |rows: &[FlowGroup]| rows.iter().map(|g| g.flows).sum::<u64>();
        let all = pairs(&[]);
        assert_eq!(total(&all), 30);
        let blocked = pairs(&["--show", "blocked"]);
        assert_eq!(total(&blocked), 14);
        assert!(blocked.iter().all(|g| g.allowed == 0));
        assert!(blocked.iter().any(|g| g.potentially_blocked > 0));
        assert!(blocked.len() < all.len(), "allowed-only pairs drop out");
        let only = pairs(&["--show", "blocked-only", "--services"]);
        assert_eq!(total(&only), 7);
        assert!(only.iter().all(|g| g.service.is_some()));
        assert!(
            only.iter()
                .all(|g| g.allowed == 0 && g.potentially_blocked == 0)
        );
        let services = pairs(&["--services"]);
        assert_eq!(total(&services), 30);
        assert!(services.len() > all.len());
    }

    #[test]
    fn both_prints_one_header_per_app_and_marks_the_direction() {
        let flows = crate::tui::test_support::fixture_flows_rich();
        let a = groups_args(&["--by", "both", "--app", "shop/prod"]);
        let s = sections_for(&flows, &app_env(), &a).unwrap();
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
        let s = sections_for(&flows, &app_env(), &a).unwrap();
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
