//! `traffic query|groups|mesh`: build an Explorer query from human filters,
//! guard its cost, run it through the cached facade, and print flows,
//! label-pair groups, the blocked summary or the mesh.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{TableRows, Window};
use clap::{Args, Subcommand};
use jiff::Timestamp;
use jiff::civil::Date;
use pce::NameIndex;
use pce::models::{
    ActorMatch, PolicyDecision, ServiceMatch, TrafficQuery, WorkloadFilter, proto_number,
};
use pce::traffic::{
    BlockedGroup, FilterExpr, FlowGroup, MeshColumns, MeshRow, ResolvedFlow, TrafficQueryBuilder,
    blocked_summary, group_by_labels, mesh, parse_window, resolve_actor, resolve_flows,
};
use serde::Serialize;
use std::sync::Arc;
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
    /// Only blocked and potentially blocked flows, grouped by destination group and port
    #[arg(long)]
    pub blocked: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct GroupsArgs {
    #[command(flatten)]
    pub filter: FilterArgs,
    /// Label keys to group by, e.g. app,env,role (default: the profile's scope keys)
    #[arg(long, value_delimiter = ',')]
    pub keys: Vec<String>,
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

impl TableRows for Vec<ResolvedFlow> {
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
        self.iter()
            .map(|f| {
                vec![
                    f.last_detected.clone().unwrap_or_default(),
                    f.src.display_name(),
                    f.src.label_text(),
                    f.dst.display_name(),
                    f.dst.label_text(),
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

impl TableRows for Vec<BlockedGroup> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "last seen",
            "dst",
            "dst labels",
            "service",
            "flows",
            "conns",
            "sources",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|g| {
                let service = match g.port {
                    Some(p) => format!("{p}/{}", g.proto),
                    None => g.proto.clone(),
                };
                vec![
                    g.last_detected.clone().unwrap_or_default(),
                    g.dst.clone(),
                    g.dst_labels.clone(),
                    service,
                    g.flows.to_string(),
                    g.connections.to_string(),
                    g.sources.join(", "),
                ]
            })
            .collect()
    }
}

pub async fn run(ctx: &Context, cmd: &TrafficCmd) -> Result<(), CliError> {
    match cmd {
        TrafficCmd::Query(a) => query(ctx, a).await,
        TrafficCmd::Groups(a) => groups(ctx, a).await,
        TrafficCmd::Mesh(a) => mesh_cmd(ctx, a).await,
    }
}

/// One query's flows, resolved, and what the envelope reports about them.
struct Fetched {
    flows: Vec<ResolvedFlow>,
    cache: &'static str,
    truncated: bool,
    window: Option<Window>,
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
    Ok(Some(Fetched {
        flows: resolve_flows(&result.value.flows, &idx),
        cache: cache_label(result.hit, ctx.cache_mode),
        truncated: result.truncated,
        window,
    }))
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
        ctx.emit(
            &blocked_summary(&f.flows, &keys),
            f.cache,
            f.truncated,
            f.window,
        )
    } else if !args.group.is_empty() {
        ctx.emit(
            &group_by_labels(&f.flows, &args.group),
            f.cache,
            f.truncated,
            f.window,
        )
    } else {
        ctx.emit(&f.flows, f.cache, f.truncated, f.window)
    }
}

async fn groups(ctx: &Context, args: &GroupsArgs) -> Result<(), CliError> {
    let Some(f) = fetch(ctx, &args.filter).await? else {
        return Ok(());
    };
    let keys = keys_or_profile(ctx, &args.keys);
    ctx.emit(
        &group_by_labels(&f.flows, &keys),
        f.cache,
        f.truncated,
        f.window,
    )
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
    ctx.emit(&mesh(&f.flows, &keys, cols), f.cache, f.truncated, f.window)
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
}
