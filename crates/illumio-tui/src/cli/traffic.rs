//! `traffic query|groups|mesh`: build an Explorer query from human filters,
//! guard its cost, run it through the cached facade, and print flows,
//! label-pair groups, the blocked summary or the mesh.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::marked::{CliMarks, FlowMark, MarkedFlow, mark_cell, squelched};
use super::output::{CompareMeta, CompareSide, Meta, SquelchedMeta, TableRows, Window, join_list};
use clap::{Args, Subcommand, ValueEnum};
use jiff::Timestamp;
use jiff::civil::Date;
use pce::marks::{ActiveMarks, annotate_groups, annotate_mesh, annotate_sections, partition};
use pce::models::{
    ActorMatch, PolicyDecision, ServiceMatch, TrafficQuery, Transmission, WorkloadFilter,
    proto_number,
};
use pce::traffic::{
    AppSection, CONFIRM_MAX_RESULTS, ChangeSet, CompareCounts, Decisions, Direction, DraftCounts,
    DraftSummary, FilterExpr, FlowGroup, Focus, FocusDirection, MAX_TRAFFIC_MAX_RESULTS,
    MeshColumns, MeshRow, ResolvedFlow, RowChange, ServiceExpr, Sides, TrafficQueryBuilder,
    app_header, app_sections, app_sections_both, backfill_cap, compact_duration, compare_flows,
    compare_groups, compare_mesh, compare_pairs, compare_sections, group_by_labels,
    group_by_service, in_app, keep_changes, keep_section_changes, keys_or_default, mesh, parse_app,
    parse_service, parse_window, resolve_actor, resolve_flows, resolve_service, without_keys,
};
use pce::{Evaluation, KeyOrder, NameIndex};
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
    /// allowed_conns, pot_blocked_conns, blocked_conns and unknown_conns are the lifetime
    /// counts of those records, as in the PCE UI. The per-decision connection columns (they
    /// sum to conns) are in JSON and CSV.
    Groups(GroupsArgs),
    /// The mesh: user, process, source, destination, service, destination process
    ///
    /// Units: flows counts flow records; conns and bytes are the lifetime counts of those
    /// records, as in the PCE UI.
    Mesh(MeshArgs),
    /// Which rule allows or blocks the flows of one source, destination and port
    ///
    /// Runs one Explorer query narrowed to --src and/or --dst (one side alone is a wider
    /// query), --port and --proto, evaluated against
    /// the draft policy (update_rules; it changes no policy), and resolves each flow's rules
    /// (allow, deny, override deny, essential) through the draft rulesets. Cost: 1 POST,
    /// status polls, 1 PUT, polls and 1 download; the draft rulesets (1 GET) when a flow names
    /// a rule; 1 GET per ruleset whose deny rules are not inline; the object lists (cached).
    /// JSON: data.flows[].rules[] (kind, ruleset, scopes, consumers, providers, services,
    /// enabled); meta.cost
    Explain(super::traffic_explain::ExplainArgs),
}

impl TrafficCmd {
    /// The Explorer filter the subcommand runs (`explain` builds its own).
    pub fn filter(&self) -> Option<&FilterArgs> {
        match self {
            TrafficCmd::Query(a) => Some(&a.filter),
            TrafficCmd::Groups(a) => Some(&a.filter),
            TrafficCmd::Mesh(a) => Some(&a.filter),
            TrafficCmd::Explain(_) => None,
        }
    }

    pub fn filter_mut(&mut self) -> Option<&mut FilterArgs> {
        match self {
            TrafficCmd::Query(a) => Some(&mut a.filter),
            TrafficCmd::Groups(a) => Some(&mut a.filter),
            TrafficCmd::Mesh(a) => Some(&mut a.filter),
            TrafficCmd::Explain(_) => None,
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
    #[arg(long, conflicts_with_all = ["from", "to", "since"])]
    pub window: Option<String>,
    /// Start: RFC 3339, 2026-10-09T08:00 (local time) or YYYY-MM-DD (UTC)
    #[arg(long, requires = "to", group = "start")]
    pub from: Option<String>,
    /// End: RFC 3339, 2026-10-09T08:00 (local time) or YYYY-MM-DD (end of that day, UTC)
    #[arg(long, requires = "start")]
    pub to: Option<String>,
    /// Start, as --from, with --to defaulting to now: "what is new since". RFC 3339, or
    /// 2026-10-23T18:00 or 2026-10-23 in the local time zone
    #[arg(long, group = "start")]
    pub since: Option<String>,
    /// Compare mode: also query the baseline, the same filter over this long before the
    /// window's start ([start - DURATION, start), e.g. 7d, 30d), and mark each row new,
    /// changed (another set of decisions), gone (only in the baseline) or same. A second query,
    /// through the same cost guard (> 7d needs --yes); JSON: change, baseline_decision per row,
    /// meta.compare. New means not seen in the baseline window at Explorer's record
    /// granularity (a record spanning the cut-off is in both); counts stay each side's lifetime
    /// counters, never per window, never subtracted
    #[arg(long, value_name = "DURATION")]
    pub baseline: Option<String>,
    /// With --baseline: the rows to print, any of new,changed,gone,same (default
    /// new,changed,gone)
    #[arg(long, value_name = "LIST", requires = "baseline")]
    pub changes: Option<String>,
    /// Source filter: key=value, ip:ADDR, ipl:NAME, wl:HOST (fqdn:NAME is destination only).
    /// Comma = AND, repeat = OR
    #[arg(long = "src")]
    pub src: Vec<String>,
    /// Destination filter, same syntax as --src, plus fqdn:NAME
    #[arg(long = "dst")]
    pub dst: Vec<String>,
    /// Exclude sources matching this filter (same syntax as --src; repeatable)
    #[arg(long = "exclude-src")]
    pub exclude_src: Vec<String>,
    /// Exclude destinations matching this filter (same syntax as --dst; repeatable)
    #[arg(long = "exclude-dst")]
    pub exclude_dst: Vec<String>,
    /// Exclude a service: PORT, PROTO, PORT/PROTO (22/tcp), FROM-TO/PROTO (8000-8100/tcp) or
    /// svc:NAME, a service object's ports and port ranges; repeatable
    #[arg(long = "exclude-service")]
    pub exclude_service: Vec<String>,
    /// Destination port, or a range FROM-TO with --proto tcp or udp (8000-8100)
    #[arg(long)]
    pub port: Option<String>,
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
    /// Also the decision the draft policy would make for each flow (draft_policy_decision):
    /// the PCE's own Explorer draft view. Per query one PUT on that query (update_rules), more
    /// status polls, still one download; it changes no policy. JSON: meta.draft
    #[arg(long)]
    pub draft: bool,
    /// Keep squelched flows (marks.toml) in the output, marked `squelch`, instead of
    /// leaving them out; meta.squelched still counts them
    #[arg(long, conflicts_with = "no_marks")]
    pub show_squelched: bool,
    /// Ignore marks.toml: no mark fields, nothing left out, no meta.squelched
    #[arg(long)]
    pub no_marks: bool,
    #[command(flatten)]
    pub saved: super::filters::SavedFilterArgs,
    /// A saved filter's services to include (`443/tcp`, `svc:NAME`), any
    /// of them: the saved text's include terms, set by `--filter` only and
    /// never with `--port` (not `traffic groups --services`, a view flag).
    #[arg(skip)]
    pub saved_services: Vec<String>,
}

/// `value` of `flag` as one saved text term group: `|` and a leading `!`
/// are the text's own operators, so a value holding them would change
/// meaning; refused with the flag to use instead.
fn plain(flag: &str, value: &str, exclude: &str) -> Result<(), String> {
    if value.contains('|') {
        return Err(format!(
            "{flag} {value:?}: | separates alternatives in a saved filter; repeat {flag} instead"
        ));
    }
    if let Some(t) = arg_terms(value).into_iter().find(|t| t.starts_with('!')) {
        return Err(format!(
            "{flag} {value:?}: {t:?} would read as an exclusion in a saved filter; {exclude}"
        ));
    }
    Ok(())
}

/// `--proto` alone as a saved service term: a name (`tcp`), or 6, 17, 1
/// as `tcp`, `udp`, `icmp`. The text reads a bare number as a port, so any
/// other number is refused rather than saved as one.
fn bare_proto(proto: &str) -> Result<String, String> {
    let p = proto.trim().to_ascii_lowercase();
    match p.parse::<u16>() {
        Ok(6) => Ok("tcp".into()),
        Ok(17) => Ok("udp".into()),
        Ok(1) => Ok("icmp".into()),
        Ok(n) => Err(format!(
            "--proto {n} alone cannot be saved: a saved filter reads a bare number as a port \
             (tcp, udp and icmp can; or give --port with it)"
        )),
        Err(_) => Ok(p),
    }
}

/// One `--src`/`--dst` value's terms (`,` = AND); blanks dropped.
fn arg_terms(v: &str) -> Vec<String> {
    v.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

/// A saved side (`a,b | c, !d`) as `--src` values and `--exclude-src` ones.
fn side_args(text: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let t = crate::filters::parse_terms(text, |e| {
        FilterExpr::parse(e).map_err(|err| err.to_string())?;
        Ok(e.to_string())
    })?;
    Ok((t.include.iter().map(|g| g.join(",")).collect(), t.exclude))
}

impl FilterArgs {
    /// The saved filter `s` as the base: each of its fields where no flag
    /// sets that field (`--src`/`--exclude-src` the source, `--port`,
    /// `--proto`, `--process`, `--exclude-service` the service, `--window`,
    /// `--from`/`--to` the window).
    pub fn with_saved(&mut self, s: &crate::filters::TrafficSaved) -> Result<(), String> {
        if self.src.is_empty()
            && self.exclude_src.is_empty()
            && let Some(t) = &s.src
        {
            (self.src, self.exclude_src) = side_args(t).map_err(|e| format!("field src: {e}"))?;
        }
        if self.dst.is_empty()
            && self.exclude_dst.is_empty()
            && let Some(t) = &s.dst
        {
            (self.dst, self.exclude_dst) = side_args(t).map_err(|e| format!("field dst: {e}"))?;
        }
        if self.port.is_none()
            && self.proto.is_none()
            && self.process.is_none()
            && self.exclude_service.is_empty()
            && let Some(t) = &s.service
        {
            let bad = |e: String| format!("field service: {e}");
            let terms = crate::filters::parse_terms(t, |e| {
                ServiceExpr::parse(e)?;
                Ok(e.to_string())
            })
            .map_err(bad)?;
            if terms.include.iter().any(|g| g.len() > 1) {
                return Err(bad(
                    "a flow has one service; use | between services (443/tcp | 80/tcp)".into(),
                ));
            }
            self.saved_services = terms.include.into_iter().flatten().collect();
            self.exclude_service = terms.exclude;
        }
        if self.decision.is_empty()
            && let Some(d) = &s.decisions
        {
            if let Some(bad) = d.iter().find(|d| PolicyDecision::parse(d).is_none()) {
                return Err(format!(
                    "field decisions: unknown {bad:?} (allowed, potentially_blocked, blocked)"
                ));
            }
            self.decision = d.clone();
        }
        if self.transmission.is_empty()
            && let Some(list) = &s.transmission
        {
            self.transmission = list
                .iter()
                .map(|t| {
                    TransmissionArg::from_str(t, true)
                        .map_err(|_| format!("field transmission: unknown {t:?}"))
                })
                .collect::<Result<_, _>>()?;
        }
        if self.window.is_none()
            && self.from.is_none()
            && self.since.is_none()
            && let Some(w) = &s.window
        {
            parse_window(w).map_err(|e| format!("field window: {e}"))?;
            self.window = Some(w.clone());
        }
        if self.max_results.is_none()
            && let Some(n) = s.max_results
        {
            if !(1..=MAX_TRAFFIC_MAX_RESULTS).contains(&n) {
                return Err(format!(
                    "field max_results: {n} is not between 1 and {MAX_TRAFFIC_MAX_RESULTS}"
                ));
            }
            self.max_results = Some(n);
        }
        Ok(())
    }

    /// The filter as the TUI's text saves it, checked as the `f` editor
    /// checks it; what the text cannot hold is an error, never dropped.
    pub fn to_saved(&self) -> Result<crate::filters::TrafficSaved, String> {
        for (given, flag, why) in [
            (
                self.from.is_some() || self.since.is_some(),
                "--from/--to/--since",
                "a saved filter keeps a relative --window",
            ),
            (
                self.baseline.is_some(),
                "--baseline",
                "a saved filter keeps no baseline; give --baseline when you load it",
            ),
            (
                self.process.is_some(),
                "--process",
                "a saved traffic filter has no process field",
            ),
            (
                self.app.is_some(),
                "--app",
                "a focus is not part of a saved filter",
            ),
        ] {
            if given {
                return Err(format!("{flag}: {why}"));
            }
        }
        for (flag, values, use_instead) in [
            ("--src", &self.src, "use --exclude-src"),
            ("--dst", &self.dst, "use --exclude-dst"),
            (
                "--exclude-src",
                &self.exclude_src,
                "give the term without !",
            ),
            (
                "--exclude-dst",
                &self.exclude_dst,
                "give the term without !",
            ),
            (
                "--exclude-service",
                &self.exclude_service,
                "give the term without !",
            ),
        ] {
            for v in values {
                plain(flag, v, use_instead)?;
            }
        }
        for (flag, v) in [("--port", &self.port), ("--proto", &self.proto)] {
            if let Some(v) = v {
                plain(flag, v, "use --exclude-service")?;
            }
        }
        let side = |inc: &[String], exc: &[String]| {
            let include: Vec<Vec<String>> = inc
                .iter()
                .map(|v| arg_terms(v))
                .filter(|g| !g.is_empty())
                .collect();
            let exclude: Vec<String> = exc.iter().flat_map(|v| arg_terms(v)).collect();
            Some(crate::filters::terms_text(&include, &exclude)).filter(|t| !t.is_empty())
        };
        let services: Vec<Vec<String>> = match (&self.port, &self.proto) {
            (Some(port), Some(proto)) => vec![vec![format!("{port}/{proto}")]],
            (Some(port), None) => vec![vec![port.clone()]],
            (None, Some(proto)) => vec![vec![bare_proto(proto)?]],
            (None, None) => self
                .saved_services
                .iter()
                .map(|s| vec![s.clone()])
                .collect(),
        };
        let list = |v: Vec<String>| (!v.is_empty()).then_some(v);
        let saved = crate::filters::TrafficSaved {
            src: side(&self.src, &self.exclude_src),
            dst: side(&self.dst, &self.exclude_dst),
            service: {
                let exclude: Vec<String> = self
                    .exclude_service
                    .iter()
                    .flat_map(|v| arg_terms(v))
                    .collect();
                Some(crate::filters::terms_text(&services, &exclude)).filter(|t| !t.is_empty())
            },
            decisions: list(self.decision.iter().flat_map(|d| arg_terms(d)).collect()),
            transmission: list(
                self.transmission
                    .iter()
                    .map(|t| Transmission::from(*t).as_str().to_string())
                    .collect(),
            ),
            window: self.window.clone(),
            max_results: self.max_results,
        };
        let check = crate::tui::filter::TrafficFilter::defaults("24h", 10_000);
        crate::tui::saved::traffic_filter(&saved, &check)?;
        Ok(saved)
    }
}

/// `--from` / `--to`: RFC 3339; a date and time without an offset
/// (`2026-10-23T18:00`) in the local time zone; or a date, in UTC (a
/// date-only `--to` means 23:59:59 of that day).
pub fn parse_time(s: &str, end_of_day: bool) -> Result<Timestamp, CliError> {
    parse_time_in(s, end_of_day, false, &jiff::tz::TimeZone::system())
}

/// `--since`: as `--from`, but a date alone is local midnight too: "since
/// Friday" means the user's Friday.
pub fn parse_since(s: &str) -> Result<Timestamp, CliError> {
    parse_time_in(s, false, true, &jiff::tz::TimeZone::system())
}

/// `parse_time` in time zone `tz` (the system's, `TZ`, in use); dates in
/// it too when `local_dates`, else in UTC.
pub fn parse_time_in(
    s: &str,
    end_of_day: bool,
    local_dates: bool,
    tz: &jiff::tz::TimeZone,
) -> Result<Timestamp, CliError> {
    let s = s.trim();
    let bad = |e: jiff::Error| CliError::Args(format!("bad time {s:?}: {e}"));
    if let Ok(t) = s.parse::<Timestamp>() {
        return Ok(t);
    }
    // `2026-10-23T18:00`: a date and time without an offset, local.
    if s.contains('T')
        && let Ok(dt) = s.parse::<jiff::civil::DateTime>()
    {
        return dt.to_zoned(tz.clone()).map(|z| z.timestamp()).map_err(bad);
    }
    if let Ok(d) = s.parse::<Date>() {
        let dt = if end_of_day {
            d.at(23, 59, 59, 0)
        } else {
            d.at(0, 0, 0, 0)
        };
        let zone = if local_dates {
            tz.clone()
        } else {
            jiff::tz::TimeZone::UTC
        };
        return dt.to_zoned(zone).map(|z| z.timestamp()).map_err(bad);
    }
    Err(CliError::Args(format!(
        "bad time {s:?}: use RFC 3339 (2026-10-09T08:00:00Z), 2026-10-09T08:00 (local time) or YYYY-MM-DD"
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

/// ` (--app runs it as 2 queries)` for a query run once per side.
fn runs_text(queries: u32) -> String {
    if queries > 1 {
        format!(
            " (--app runs it as {})",
            pce::traffic::queries_text(queries)
        )
    } else {
        String::new()
    }
}

/// The query plus the name index when filter resolution already loaded it,
/// so `query()` never fetches the object lists twice in one command.
struct BuiltQuery {
    query: TrafficQuery,
    index: Option<Arc<NameIndex>>,
}

/// Builds the request body. Everything that can be checked offline (window,
/// caps, the `--yes` guard, filter syntax) is checked before the first PCE
/// request; the object lists are fetched once, and only when a filter names
/// something that needs resolving.
/// `queries` is how many Explorer queries the body will run as (`--app
/// --direction both`: 2); the guard's message names it.
async fn build_query(
    ctx: &Context,
    args: &FilterArgs,
    queries: u32,
) -> Result<BuiltQuery, CliError> {
    if args.max_slices.is_some() {
        return Err(CliError::Args(
            "time slices were removed: --backfill raises the cap".into(),
        ));
    }
    let mut b = match (&args.from, &args.since, &args.to) {
        (Some(f), _, Some(t)) => {
            TrafficQueryBuilder::between(parse_time(f, false)?, parse_time(t, true)?)
        }
        // `--since`: `--from` with `--to` now unless given.
        (None, Some(since), to) => TrafficQueryBuilder::between(
            parse_since(since)?,
            match to {
                Some(t) => parse_time(t, true)?,
                None => Timestamp::now(),
            },
        ),
        _ => {
            let w = args
                .window
                .clone()
                .unwrap_or_else(|| ctx.config.traffic_default_window.clone());
            TrafficQueryBuilder::last(parse_window(&w)?)?
        }
    };
    if args.port.is_some() || args.proto.is_some() || args.process.is_some() {
        // --port (a range too) and --proto read as one PORT/PROTO term.
        let bad = |e: String| CliError::Args(format!("--port/--proto: {e}"));
        let m = match (&args.port, &args.proto) {
            (Some(port), proto) => {
                let term = match proto {
                    Some(proto) => format!("{port}/{proto}"),
                    None => port.clone(),
                };
                let m = parse_service(&term).map_err(bad)?;
                if m.port.is_none() {
                    return Err(bad(format!("--port takes a port or a range, got {port:?}")));
                }
                m
            }
            (None, Some(p)) => ServiceMatch {
                proto: Some(proto_number(p).ok_or_else(|| bad(format!("unknown protocol {p:?}")))?),
                ..ServiceMatch::default()
            },
            (None, None) => ServiceMatch::default(),
        };
        b = b.service(ServiceMatch {
            process_name: args.process.clone(),
            ..m
        });
    }
    let ex_svc: Vec<ServiceExpr> = args
        .exclude_service
        .iter()
        .map(|s| {
            ServiceExpr::parse(s).map_err(|e| CliError::Args(format!("--exclude-service: {e}")))
        })
        .collect::<Result<_, _>>()?;
    // A saved filter's services (never with `--port`): any of them.
    let in_svc: Vec<ServiceExpr> = args
        .saved_services
        .iter()
        .map(|s| ServiceExpr::parse(s).map_err(|e| CliError::Args(format!("service: {e}"))))
        .collect::<Result<_, _>>()?;
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
        return Err(CliError::Args(format!(
            "expensive query: {reason}{}; re-run with --yes to confirm",
            runs_text(queries)
        )));
    }
    // Compare mode: the baseline through the same guard, on its own.
    if let Some(length) = baseline_length(args)? {
        let base = bounds.baseline(length)?;
        if !args.dry_run
            && !args.yes
            && let Some(reason) = base.needs_confirmation()
        {
            return Err(CliError::Args(format!(
                "expensive baseline query: {reason}{}; re-run with --yes to confirm",
                runs_text(queries)
            )));
        }
    }

    let src = parse_groups(&args.src)?;
    let dst = parse_groups(&args.dst)?;
    let ex_src = parse_groups(&args.exclude_src)?;
    let ex_dst = parse_groups(&args.exclude_dst)?;
    // Explorer takes no FQDN on the source side, to include or to exclude.
    for e in src.iter().chain(&ex_src).flatten() {
        e.check_source()?;
    }
    let all = || {
        [&src, &dst, &ex_src, &ex_dst]
            .into_iter()
            .flatten()
            .flatten()
    };
    // `svc:NAME` expands to the service's ports from the cached service
    // list: the index's when the actors load it, else that one list.
    let named = ex_svc.iter().chain(&in_svc).any(ServiceExpr::is_named);
    let apply_services = |mut b: TrafficQueryBuilder,
                          services: &[&pce::models::Service]|
     -> Result<TrafficQueryBuilder, CliError> {
        for e in &in_svc {
            for m in resolve_service(e, services)
                .map_err(|err| CliError::Args(format!("service: {err}")))?
            {
                b = b.service(m);
            }
        }
        for e in &ex_svc {
            for m in resolve_service(e, services)
                .map_err(|err| CliError::Args(format!("--exclude-service: {err}")))?
            {
                b = b.exclude_service(m);
            }
        }
        Ok(b)
    };
    if all().next().is_none() {
        let b = if named {
            let services = ctx.pce.services(ctx.cache_mode).await?.value;
            apply_services(b, &services.iter().collect::<Vec<_>>())?
        } else {
            apply_services(b, &[])?
        };
        return Ok(BuiltQuery {
            query: b.build()?,
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
    let b = apply_services(b, &idx.services())?;
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

/// Flows as printed: JSON is the flows themselves (every label), with
/// their `mark` while marks are active; the table's label columns follow
/// `order` and hide its ignored keys, and a last `mark` column the marks.
struct FlowRows<'a> {
    flows: &'a [ResolvedFlow],
    order: &'a KeyOrder,
    /// Each flow's mark, while marks are active.
    marks: Option<Vec<Option<FlowMark>>>,
    /// Each flow's change, in compare mode (`--baseline`).
    changes: Option<Vec<RowChange>>,
}

impl Serialize for FlowRows<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.marks.is_none() && self.changes.is_none() {
            return self.flows.serialize(s);
        }
        s.collect_seq(self.flows.iter().enumerate().map(|(i, flow)| MarkedFlow {
            flow,
            mark: self.marks.as_ref().and_then(|m| m[i].as_ref()),
            change: self.changes.as_ref().map(|c| &c[i]),
        }))
    }
}

/// Compare mode's two cells: the change (`new`, `changed`, `gone`,
/// `same`) and the baseline's decisions.
fn change_cells(c: Option<&RowChange>) -> [String; 2] {
    match c {
        Some(c) => [c.change.as_str().to_string(), c.baseline_text()],
        None => [String::new(), String::new()],
    }
}

impl FlowRows<'_> {
    /// Some flow carries a draft decision (`--draft`).
    fn draft(&self) -> bool {
        self.flows.iter().any(|f| f.draft_policy_decision.is_some())
    }
}

impl TableRows for FlowRows<'_> {
    fn units(&self) -> Option<&'static str> {
        Some(UNITS_FLOWS)
    }
    /// With draft decisions, a `draft` column after `decision`.
    fn headers(&self) -> Vec<&'static str> {
        let mut h = vec![
            "last seen",
            "src",
            "src labels",
            "dst",
            "dst labels",
            "service",
            "conns",
            "decision",
            "process",
        ];
        if self.draft() {
            h.insert(8, "draft");
        }
        if self.marks.is_some() {
            h.push("mark");
        }
        if self.changes.is_some() {
            h.extend(COMPARE_COLUMNS);
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let draft = self.draft();
        self.flows
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let mut row = vec![
                    f.last_detected.clone().unwrap_or_default(),
                    f.src.display_name(),
                    f.src.label_text(self.order),
                    f.dst.display_name(),
                    f.dst.label_text(self.order),
                    f.service_text(),
                    f.num_connections.to_string(),
                    f.policy_decision.as_str().to_string(),
                    f.process_name.clone().unwrap_or_default(),
                ];
                if draft {
                    row.insert(8, f.draft_text());
                }
                if let Some(marks) = &self.marks {
                    row.push(
                        marks[i]
                            .as_ref()
                            .map(|m| m.action.as_str().to_string())
                            .unwrap_or_default(),
                    );
                }
                if let Some(changes) = &self.changes {
                    row.extend(change_cells(Some(&changes[i])));
                }
                row
            })
            .collect()
    }
    /// Every label (ignored keys too, as in JSON), one list entry each;
    /// the draft decision plain.
    fn full_rows(&self) -> Vec<Vec<String>> {
        let labels = |e: &pce::traffic::ResolvedEndpoint| {
            join_list(e.labels.iter().map(|l| format!("{}={}", l.key, l.value)))
        };
        let draft = self.draft();
        self.flows
            .iter()
            .zip(self.rows())
            .map(|(f, mut row)| {
                row[2] = labels(&f.src);
                row[4] = labels(&f.dst);
                if draft {
                    row[8] = f
                        .draft_policy_decision
                        .as_ref()
                        .map(|d| d.as_str().to_string())
                        .unwrap_or_default();
                }
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
pub const CONNS_BY_DECISION: [&str; 4] = [
    "allowed_conns",
    "pot_blocked_conns",
    "blocked_conns",
    "unknown_conns",
];

/// The `--csv` columns `--draft` adds to pairs and sections (flow records).
pub const DRAFT_COUNTS: [&str; 2] = ["draft_allowed", "draft_blocked"];

/// `DRAFT_COUNTS`' cells: the counts, `0` for a row without draft data.
pub fn draft_cells(d: Option<&DraftCounts>) -> [String; 2] {
    let d = d.copied().unwrap_or_default();
    [d.draft_allowed.to_string(), d.draft_blocked.to_string()]
}

fn mesh_draft(rows: &[MeshRow]) -> bool {
    rows.iter().any(|r| r.draft.is_some())
}

fn mesh_compared(rows: &[MeshRow]) -> bool {
    rows.iter().any(|r| r.change.is_some())
}

fn groups_draft(groups: &[FlowGroup]) -> bool {
    groups.iter().any(|g| g.draft.is_some())
}

/// Some row carries marks: the tables get a last `mark` column.
fn groups_marked(groups: &[FlowGroup]) -> bool {
    groups.iter().any(|g| g.marks.is_some())
}

/// Compare mode: the tables end with `COMPARE_COLUMNS`.
fn groups_compared(groups: &[FlowGroup]) -> bool {
    groups.iter().any(|g| g.change.is_some())
}

fn group_cells(groups: &[FlowGroup]) -> Vec<Vec<String>> {
    let any = groups.iter().any(|g| g.bytes > 0);
    groups
        .iter()
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

impl TableRows for Vec<FlowGroup> {
    fn units(&self) -> Option<&'static str> {
        Some(UNITS_GROUPS)
    }
    /// With draft decisions, a `draft` column: `2 would allow · 1 would
    /// block` (flow records).
    fn headers(&self) -> Vec<&'static str> {
        let mut h = vec![
            "src",
            "dst",
            "flows",
            "conns",
            "bytes",
            "allowed",
            "pot.blocked",
            "blocked",
        ];
        if groups_draft(self) {
            h.push("draft");
        }
        if groups_marked(self) {
            h.push("mark");
        }
        if groups_compared(self) {
            h.extend(COMPARE_COLUMNS);
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let draft = groups_draft(self);
        let marked = groups_marked(self);
        let compared = groups_compared(self);
        self.iter()
            .zip(group_cells(self))
            .map(|(g, mut row)| {
                if draft {
                    row.push(DraftCounts::text(g.draft.as_ref()));
                }
                if marked {
                    row.push(mark_cell(g.marks.as_ref()));
                }
                if compared {
                    row.extend(change_cells(g.change.as_ref()));
                }
                row
            })
            .collect()
    }
    /// The table's counts, then the connections per decision, then with
    /// draft decisions `draft_allowed` and `draft_blocked`, then `mark`,
    /// then compare mode's columns.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        h.retain(|c| !["draft", "mark"].contains(c) && !COMPARE_COLUMNS.contains(c));
        h.extend(CONNS_BY_DECISION);
        if groups_draft(self) {
            h.extend(DRAFT_COUNTS);
        }
        if groups_marked(self) {
            h.push("mark");
        }
        if groups_compared(self) {
            h.extend(COMPARE_COLUMNS);
        }
        h
    }
    /// `bytes` always a number (0 when no flow carried any), then the
    /// connections per decision and the draft counts.
    fn full_rows(&self) -> Vec<Vec<String>> {
        let draft = groups_draft(self);
        self.iter()
            .zip(group_cells(self))
            .map(|(g, mut row)| {
                row[4] = g.bytes.to_string();
                row.extend(g.conns.cells());
                if draft {
                    row.extend(draft_cells(g.draft.as_ref()));
                }
                if groups_marked(self) {
                    row.push(mark_cell(g.marks.as_ref()));
                }
                if groups_compared(self) {
                    row.extend(change_cells(g.change.as_ref()));
                }
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
    /// With draft decisions, a `draft` column (`DraftCounts::text`).
    fn headers(&self) -> Vec<&'static str> {
        let mut h = vec![
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
        ];
        if mesh_draft(self) {
            h.push("draft");
        }
        if self.iter().any(|r| r.marks.is_some()) {
            h.push("mark");
        }
        if mesh_compared(self) {
            h.extend(COMPARE_COLUMNS);
        }
        h
    }
    /// The table's, with the draft counts as `draft_allowed` and
    /// `draft_blocked` numbers, then `mark`, then compare mode's columns.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        let marked = h.contains(&"mark");
        h.retain(|c| !["draft", "mark"].contains(c) && !COMPARE_COLUMNS.contains(c));
        if mesh_draft(self) {
            h.extend(DRAFT_COUNTS);
        }
        if marked {
            h.push("mark");
        }
        if mesh_compared(self) {
            h.extend(COMPARE_COLUMNS);
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let draft = mesh_draft(self);
        let compared = mesh_compared(self);
        let marked = self.iter().any(|r| r.marks.is_some());
        let any = self.iter().any(|r| r.bytes > 0);
        let or_dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".into());
        self.iter()
            .map(|r| {
                let mut row = vec![
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
                ];
                if draft {
                    row.push(DraftCounts::text(r.draft.as_ref()));
                }
                if marked {
                    row.push(mark_cell(r.marks.as_ref()));
                }
                if compared {
                    row.extend(change_cells(r.change.as_ref()));
                }
                row
            })
            .collect()
    }
    /// Empty cells for no user or process, `bytes` always a number, the
    /// draft counts as numbers.
    fn full_rows(&self) -> Vec<Vec<String>> {
        let draft = mesh_draft(self);
        let compared = mesh_compared(self);
        let marked = self.iter().any(|r| r.marks.is_some());
        let or_empty = |v: &Option<String>| v.clone().unwrap_or_default();
        self.iter()
            .zip(self.rows())
            .map(|(r, mut row)| {
                row[0] = or_empty(&r.user);
                row[1] = or_empty(&r.process);
                row[5] = or_empty(&r.dst_process);
                row[8] = r.bytes.to_string();
                if compared {
                    row.truncate(row.len() - COMPARE_COLUMNS.len());
                }
                let mark = if marked { row.pop() } else { None };
                if draft {
                    row.pop();
                    row.extend(draft_cells(r.draft.as_ref()));
                }
                row.extend(mark);
                if compared {
                    row.extend(change_cells(r.change.as_ref()));
                }
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
    /// With draft decisions, a `draft` column (`DraftCounts::text`).
    fn headers(&self) -> Vec<&'static str> {
        let mut h = vec![
            "app / peer",
            "peer hosts",
            "app hosts",
            "service",
            "flows",
            "conns",
            "bytes",
            "last seen",
            "decision",
        ];
        if self.draft() {
            h.push("draft");
        }
        if self.marked() {
            h.push("mark");
        }
        if self.compared() {
            h.extend(COMPARE_COLUMNS);
        }
        h
    }
    /// A header row is one cell: `render_table` prints it whole in the
    /// first column with the other cells empty.
    fn rows(&self) -> Vec<Vec<String>> {
        let draft = self.draft();
        let marked = self.marked();
        let compared = self.compared();
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
                    if draft && let Some(row) = out.last_mut() {
                        row.push(DraftCounts::text(r.draft.as_ref()));
                    }
                    if marked && let Some(row) = out.last_mut() {
                        row.push(mark_cell(r.marks.as_ref()));
                    }
                    if compared && let Some(row) = out.last_mut() {
                        row.extend(change_cells(r.change.as_ref()));
                    }
                }
            }
        }
        out
    }
    /// One row per peer, no header rows: the app and the direction are
    /// columns, the host names lists beside their counts.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = section_csv_headers(self.draft());
        if self.marked() {
            h.push("mark");
        }
        if self.compared() {
            h.extend(COMPARE_COLUMNS);
        }
        h
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        let (draft, marked, compared) = (self.draft(), self.marked(), self.compared());
        self.sections
            .iter()
            .flat_map(|s| {
                s.rows.iter().map(move |r| {
                    let mut row = section_csv_row(&s.app, s.direction, r, draft);
                    if marked {
                        row.push(mark_cell(r.marks.as_ref()));
                    }
                    if compared {
                        row.extend(change_cells(r.change.as_ref()));
                    }
                    row
                })
            })
            .collect()
    }
}

impl Sections {
    /// Some row carries draft counts (`--draft`).
    fn draft(&self) -> bool {
        self.sections
            .iter()
            .flat_map(|s| &s.rows)
            .any(|r| r.draft.is_some())
    }

    /// Some row carries marks: a last `mark` column.
    fn marked(&self) -> bool {
        self.sections
            .iter()
            .flat_map(|s| &s.rows)
            .any(|r| r.marks.is_some())
    }

    /// Compare mode: the tables end with `COMPARE_COLUMNS`.
    fn compared(&self) -> bool {
        self.sections
            .iter()
            .flat_map(|s| &s.rows)
            .any(|r| r.change.is_some())
    }
}

/// `SECTION_CSV_HEADERS`, then with `draft` the `DRAFT_COUNTS`.
pub fn section_csv_headers(draft: bool) -> Vec<&'static str> {
    let mut h = SECTION_CSV_HEADERS.to_vec();
    if draft {
        h.extend(DRAFT_COUNTS);
    }
    h
}

/// `peer_csv_row`, then with `draft` the row's draft counts.
pub fn section_csv_row(
    app: &[String],
    direction: Direction,
    r: &pce::traffic::PeerRow,
    draft: bool,
) -> Vec<String> {
    let mut row = peer_csv_row(app, direction, r);
    if draft {
        row.extend(draft_cells(r.draft.as_ref()));
    }
    row
}

/// The CSV columns of app sections, one row per peer (`--csv` and the
/// TUI's `:export`).
pub const SECTION_CSV_HEADERS: [&str; 17] = [
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
    CONNS_BY_DECISION[3],
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
        r.conns.unknown_conns.to_string(),
    ]
}

/// `no app matches "pos/PCI"` when `--app` kept no section, or, when the
/// app has flows but squelch left none of them (`squelched_only`), `all of
/// "pos/PCI"'s flows are squelched (--show-squelched shows them)`.
pub fn app_miss(args: &GroupsArgs, s: &Sections, squelched_only: bool) -> Option<String> {
    args.filter
        .app
        .as_ref()
        .filter(|_| s.sections.is_empty())
        .map(|a| {
            if squelched_only {
                format!("all of {a:?}'s flows are squelched (--show-squelched shows them)")
            } else {
                format!("no app matches {a:?}")
            }
        })
}

/// `--by`, `--show` and `--app` over the loaded flows: the sections (only
/// `--app`'s with it), or `None` for `pairs` (see `pairs_for`).
pub fn sections_for(
    flows: &[ResolvedFlow],
    keys: &[String],
    args: &GroupsArgs,
) -> Option<Sections> {
    sections_with(flows, keys, args, Decisions::from(args.show))
}

/// `sections_for` with `decisions` in place of `--show` (compare mode
/// judges each key on every decision).
fn sections_with(
    flows: &[ResolvedFlow],
    keys: &[String],
    args: &GroupsArgs,
    decisions: Decisions,
) -> Option<Sections> {
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
        TrafficCmd::Explain(a) => super::traffic_explain::run(ctx, a).await,
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
    /// `--draft`: what the draft decisions change.
    pub(crate) draft: Option<DraftSummary>,
    /// The name index the flows were resolved with.
    pub(crate) index: Arc<NameIndex>,
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
        m.draft = self.draft;
        m
    }
}

/// Where one side of a command's results is cached, and how its
/// messages name it: the recent result in `main` (`focus:in|out` with
/// `--app`), compare mode's baseline in its own slots, so neither evicts
/// the other.
#[derive(Debug, Clone, Copy)]
struct Side {
    slot: &'static str,
    focus_slot: &'static str,
    /// `baseline ` in front of a backfill message.
    label: &'static str,
}

const RECENT: Side = Side {
    slot: "main",
    focus_slot: "focus",
    label: "",
};

const BASELINE: Side = Side {
    slot: pce::BASELINE_SLOT,
    focus_slot: pce::FOCUS_BASELINE_SLOT,
    label: "baseline ",
};

/// `--backfill` on a truncated result: the same window again at the next
/// cap (`backfill_cap`: 10000, then 25000 to 200000), each answer replacing
/// the last, until one is complete or the next cap needs `--yes`. Explorer
/// cannot page a result, so this is the only way to the rest of it.
/// Returns the query of the result kept, and its async query href.
async fn backfill(
    ctx: &Context,
    args: &FilterArgs,
    q: &TrafficQuery,
    href: String,
    idx: &NameIndex,
    f: &mut Fetched,
    side: Side,
) -> Result<(TrafficQuery, String), CliError> {
    let ceiling = if args.yes {
        MAX_TRAFFIC_MAX_RESULTS
    } else {
        CONFIRM_MAX_RESULTS
    };
    let what = side.label;
    let mut q = q.clone();
    let mut href = href;
    let mut steps = vec![q.max_results];
    while f.truncated {
        let Some(next) = backfill_cap(q.max_results) else {
            eprintln!(
                "{what}backfill stopped: truncated at the maximum cap of {MAX_TRAFFIC_MAX_RESULTS}; narrow the filter"
            );
            break;
        };
        if next > ceiling {
            eprintln!(
                "{what}backfill stopped: truncated at cap {}; a cap of {next} needs --yes",
                q.max_results
            );
            break;
        }
        eprintln!("{what}backfill raises the cap to {next}");
        q.max_results = next;
        let r = ctx
            .pce
            .traffic_slot(
                side.slot,
                &q,
                ctx.cache_mode,
                &CancellationToken::new(),
                None,
            )
            .await?;
        steps.push(next);
        href = r.value.query_href.clone();
        f.flows = resolve_flows(&r.value.flows, idx);
        f.cache = cache_label(r.hit, ctx.cache_mode);
        f.truncated = r.truncated;
        f.incomplete = r.value.incomplete;
        f.cap = next;
    }
    f.cap_steps = Some(steps);
    Ok((q, href))
}

/// Builds and runs the query (cost guard, cache, one name index): with a
/// focused `--app`, one query per side through `Pce::focused_traffic`,
/// after the app's labels are checked (an unknown one is exit 2 before any
/// traffic request). An unfocused query is submitted while the name index
/// loads, so the PCE computes meanwhile. A dry run prints the request body
/// (with `--app`, the bodies) and returns `None`. `--baseline` is not read
/// here (`fetch_compare`).
pub(crate) async fn fetch(
    ctx: &Context,
    args: &FilterArgs,
    focus: Option<&CliFocus>,
) -> Result<Option<Fetched>, CliError> {
    let n = focus
        .filter(|f| f.query)
        .map_or(1, |f| f.focus.direction.query_count());
    let BuiltQuery { query: q, index } = build_query(ctx, args, n).await?;
    let focus = focus.filter(|f| f.query).map(|f| &f.focus);
    if args.dry_run {
        dry_run(ctx, &[q], focus, index).await?;
        return Ok(None);
    }
    run_side(ctx, args, focus, &q, index, RECENT, args.draft)
        .await
        .map(Some)
}

/// Compare mode (`--baseline`): the recent query as `fetch` runs it, then
/// the baseline, the same body over `[start - length, start)` of the
/// recent query's bounds, in its own cache slots, backfilled with
/// `--backfill` too. The draft (`--draft`) is the recent side's only: the
/// comparison reads the reported decisions. A dry run prints every body,
/// the recent ones first, and returns `None`.
pub(crate) async fn fetch_compare(
    ctx: &Context,
    args: &FilterArgs,
    focus: Option<&CliFocus>,
    length: Duration,
) -> Result<Option<(Fetched, Fetched)>, CliError> {
    let n = focus
        .filter(|f| f.query)
        .map_or(1, |f| f.focus.direction.query_count());
    let BuiltQuery { query: q, index } = build_query(ctx, args, n).await?;
    let base = q.baseline(length)?;
    let focus = focus.filter(|f| f.query).map(|f| &f.focus);
    if args.dry_run {
        dry_run(ctx, &[q, base], focus, index).await?;
        return Ok(None);
    }
    let recent = run_side(ctx, args, focus, &q, index, RECENT, args.draft).await?;
    let idx = Some(recent.index.clone());
    let baseline = run_side(ctx, args, focus, &base, idx, BASELINE, false).await?;
    Ok(Some((recent, baseline)))
}

/// `--dry-run`: the request bodies, nothing sent. One unfocused query is
/// its body; with `--app` (one body per side, `in` first) or a baseline,
/// a list.
async fn dry_run(
    ctx: &Context,
    queries: &[TrafficQuery],
    focus: Option<&Focus>,
    index: Option<Arc<NameIndex>>,
) -> Result<(), CliError> {
    let window = queries.first().map(|q| Window {
        start: q.start_date.clone(),
        end: q.end_date.clone(),
    });
    let Some(focus) = focus else {
        if let [q] = queries {
            return ctx.emit(&DryRun(q.clone()), "none", false, window);
        }
        return ctx.emit(&DryRuns(queries.to_vec()), "none", false, window);
    };
    let idx = match index {
        Some(idx) => idx,
        None => ctx.pce.name_index(ctx.cache_mode).await?,
    };
    let mut bodies = Vec::new();
    for q in queries {
        let sides = focus.queries(q, &idx).map_err(CliError::Args)?;
        bodies.extend(sides.into_iter().map(|(_, q)| q));
    }
    ctx.emit(&DryRuns(bodies), "none", false, window)
}

/// One side's query run (or reused) in `side`'s slots: focused, one query
/// per side of the app; else one query (with the name index alongside
/// when `index` is not loaded yet), backfilled with `--backfill`, with the
/// draft decisions when `draft`.
async fn run_side(
    ctx: &Context,
    args: &FilterArgs,
    focus: Option<&Focus>,
    q: &TrafficQuery,
    index: Option<Arc<NameIndex>>,
    side: Side,
    draft: bool,
) -> Result<Fetched, CliError> {
    let window = Some(Window {
        start: q.start_date.clone(),
        end: q.end_date.clone(),
    });
    if let Some(focus) = focus {
        let idx = match index {
            Some(idx) => idx,
            None => ctx.pce.name_index(ctx.cache_mode).await?,
        };
        // `--draft`: each side run and then re-evaluated (no href known).
        let none: &[String] = &[];
        let ft = ctx
            .pce
            .focused_traffic_eval(
                side.focus_slot,
                focus,
                q,
                &idx,
                draft.then_some(none),
                ctx.cache_mode,
                &CancellationToken::new(),
                None,
            )
            .await?;
        return Ok(Fetched {
            draft: draft.then(|| DraftSummary::of(&ft.flows)),
            queries: Some(ft.query_count()),
            cache: cache_label(ft.hit, ctx.cache_mode),
            truncated: ft.truncated,
            incomplete: ft.incomplete,
            window,
            span: q.window(),
            cap: q.max_results,
            cap_steps: None,
            flows: ft.flows,
            index: idx,
        });
    }
    let cancel = CancellationToken::new();
    // `--draft` in one go (POST, polls, PUT, polls, one download), except
    // with `--backfill`: its caps run plain and only the result kept is
    // re-evaluated, by its href.
    let run = async {
        let eval = if draft && !args.backfill {
            Evaluation::Draft(None)
        } else {
            Evaluation::Reported
        };
        ctx.pce
            .traffic_eval(side.slot, q, eval, ctx.cache_mode, &cancel, None)
            .await
    };
    let (result, idx) = match index {
        Some(idx) => (run.await?, idx),
        // The first error stops the other: a failed name index drops the
        // traffic query's polls and download.
        None => tokio::try_join!(run, ctx.pce.name_index(ctx.cache_mode))?,
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
        draft: None,
        index: idx.clone(),
    };
    if args.backfill {
        let (last, href) = backfill(
            ctx,
            args,
            q,
            result.value.query_href.clone(),
            &idx,
            &mut f,
            side,
        )
        .await?;
        if draft {
            let r = ctx
                .pce
                .traffic_eval(
                    side.slot,
                    &last,
                    Evaluation::Draft(Some(&href)),
                    ctx.cache_mode,
                    &cancel,
                    None,
                )
                .await?;
            f.flows = resolve_flows(&r.value.flows, &idx);
            f.truncated = r.truncated;
            f.incomplete = r.value.incomplete;
        }
    }
    if draft {
        f.draft = Some(DraftSummary::of(&f.flows));
    }
    Ok(f)
}

/// `--baseline` as a length, checked before any request.
pub fn baseline_length(args: &FilterArgs) -> Result<Option<Duration>, CliError> {
    let Some(b) = &args.baseline else {
        return Ok(None);
    };
    let d = parse_window(b).map_err(|e| CliError::Args(format!("--baseline: {e}")))?;
    if d.is_zero() {
        return Err(CliError::Args("--baseline must be longer than 0".into()));
    }
    Ok(Some(d))
}

/// The `--csv` columns compare mode adds to every traffic view.
pub const COMPARE_COLUMNS: [&str; 2] = ["change", "baseline_decision"];

/// Compare mode's other half: the baseline's result and what to print.
pub(crate) struct CompareRun {
    pub(crate) baseline: Fetched,
    /// `--changes`.
    pub(crate) show: ChangeSet,
    /// `--baseline` as given.
    pub(crate) length: String,
}

/// What compare mode compares, squelch applied to both sides alike.
pub(crate) struct CompareSides {
    /// The recent records the views show and compare: squelch's kept ones
    /// without `hidden`.
    pub(crate) recent: Vec<ResolvedFlow>,
    /// The baseline's, likewise, merged on their own.
    pub(crate) baseline: Vec<ResolvedFlow>,
    /// The keys with a squelched record on either side (`squelched_keys`).
    pub(crate) hidden: std::collections::BTreeSet<pce::traffic::FlowKey>,
}

/// The Explorer queries a side took: each cap step (`--backfill`), one
/// per side of `--app`.
fn side_queries(f: &Fetched) -> u32 {
    let steps = f.cap_steps.as_ref().map_or(1, |s| s.len() as u32);
    steps * f.queries.unwrap_or(1)
}

impl CompareRun {
    /// Both sides as the views compare them: squelch (unless
    /// `--show-squelched`) leaves out every key with a squelched record on
    /// either side, so a mark matching one side only (a draft decision the
    /// baseline never carries, a decision that changed) never makes a row
    /// new or gone. The baseline is merged on its own (`merge_flows`: one
    /// row per record, never summed), never with the recent side.
    fn sides(
        &self,
        kept: &[ResolvedFlow],
        part: Option<&pce::marks::Partition>,
        act: Option<&ActiveMarks<'_>>,
        show: bool,
    ) -> CompareSides {
        let base_part = act.map(|a| partition(&self.baseline.flows, a, show));
        let base_kept: &[ResolvedFlow] =
            base_part.as_ref().map_or(&self.baseline.flows, |p| &p.kept);
        let hidden = match (part, &base_part) {
            (Some(r), Some(b)) if !show => {
                pce::traffic::squelched_keys(r.marked.squelched(), b.marked.squelched())
            }
            _ => Default::default(),
        };
        let mut merged = Vec::with_capacity(base_kept.len());
        pce::traffic::merge_flows(&mut merged, without_keys(base_kept, &hidden));
        CompareSides {
            recent: without_keys(kept, &hidden),
            baseline: merged,
            hidden,
        }
    }

    /// `meta.compare`, the cost of both sides (`meta.queries`, `meta.cost`),
    /// a warning (stderr and `meta.warnings`) for each truncated side; the
    /// exit code stays 0.
    fn fill(&self, meta: &mut Meta, recent: &Fetched, counts: CompareCounts) {
        let side = |f: &Fetched| CompareSide {
            start: f
                .window
                .as_ref()
                .map(|w| w.start.clone())
                .unwrap_or_default(),
            end: f.window.as_ref().map(|w| w.end.clone()).unwrap_or_default(),
            truncated: f.truncated,
            incomplete: f.incomplete,
            cap: f.cap,
            cap_steps: f.cap_steps.clone(),
            queries: side_queries(f),
            cache: f.cache,
        };
        for (name, f) in [("recent", recent), ("baseline", &self.baseline)] {
            if f.truncated {
                let w = format!(
                    "{name} truncated, capped at {}: new/gone over-report; --backfill raises the cap",
                    f.cap
                );
                eprintln!("warning: {w}");
                meta.warnings.get_or_insert_with(Vec::new).push(w);
            }
        }
        let (r, b) = (side_queries(recent), side_queries(&self.baseline));
        meta.queries = Some(r + b);
        meta.cost = Some(
            format!(
                "{} Explorer queries: {r} for the recent window, {b} for the baseline (each 1 POST, status polls, 1 download)",
                r + b
            )
            .into(),
        );
        meta.compare = Some(CompareMeta {
            length: self.length.clone(),
            changes: self.show.text(),
            recent: side(recent),
            baseline: side(&self.baseline),
            counts,
        });
    }
}

/// The recent result, and with `--baseline` the baseline too. `--baseline`
/// and `--changes` are checked before any request.
async fn fetch_sides(
    ctx: &Context,
    args: &FilterArgs,
    focus: Option<&CliFocus>,
) -> Result<Option<(Fetched, Option<CompareRun>)>, CliError> {
    let show = match &args.changes {
        Some(c) => ChangeSet::parse(c).map_err(|e| CliError::Args(format!("--changes: {e}")))?,
        None => ChangeSet::default(),
    };
    let Some(length) = baseline_length(args)? else {
        return Ok(fetch(ctx, args, focus).await?.map(|f| (f, None)));
    };
    let Some((recent, baseline)) = fetch_compare(ctx, args, focus, length).await? else {
        return Ok(None);
    };
    let length = args.baseline.clone().unwrap_or_default();
    Ok(Some((
        recent,
        Some(CompareRun {
            baseline,
            show,
            length,
        }),
    )))
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
    let Some((f, cmp)) = fetch_sides(ctx, &args.filter, focus.as_ref()).await? else {
        return Ok(());
    };
    // Marks: classify, then aggregate the records kept (all of them with
    // --show-squelched); the marks join the rows by their keys. Compare
    // mode squelches both sides alike (`CompareRun::sides`).
    let cm = CliMarks::load(ctx, args.filter.no_marks, args.filter.show_squelched);
    let act = cm.active(args.filter.draft);
    let part = act.as_ref().map(|a| partition(&f.flows, a, cm.show));
    let kept: &[ResolvedFlow] = part.as_ref().map_or(&f.flows, |p| &p.kept);
    let both = cmp
        .as_ref()
        .map(|c| c.sides(kept, part.as_ref(), act.as_ref(), cm.show));
    let flows: &[ResolvedFlow] = both.as_ref().map_or(kept, |b| &b.recent);
    let mut meta = f.meta(ctx);
    if args.blocked {
        let keys = keys_or_profile(ctx, &[]);
        let sec = |x: &[ResolvedFlow], d| app_sections(x, &keys, Direction::In, d);
        let agg = |x: &[ResolvedFlow]| sec(x, Decisions::Blocked);
        let mut sections = Sections {
            sections: agg(flows),
            both: false,
        };
        let sq = squelched(
            part.as_ref(),
            act.as_ref(),
            |p, a| annotate_sections(&mut sections.sections, p, a, agg),
            |x| Decisions::Blocked.keeps(x),
        );
        cm.fill(&mut meta, act.as_ref(), sq);
        if let (Some(c), Some(b)) = (&cmp, &both) {
            // Each key judged on every decision, then Blocked's rows.
            let (all_r, all_b, kept_b) = (
                sec(&b.recent, Decisions::All),
                sec(&b.baseline, Decisions::All),
                agg(&b.baseline),
            );
            let counts = compare_sections(
                &mut sections.sections,
                Sides {
                    all_recent: &all_r,
                    all_baseline: &all_b,
                    kept_baseline: &kept_b,
                },
            );
            keep_section_changes(&mut sections.sections, c.show);
            c.fill(&mut meta, &f, counts);
        }
        ctx.emit_meta(&sections, &meta)
    } else if !args.group.is_empty() {
        let agg = |x: &[ResolvedFlow]| group_by_labels(x, &args.group);
        let mut groups = agg(flows);
        let sq = squelched(
            part.as_ref(),
            act.as_ref(),
            |p, a| annotate_groups(&mut groups, p, a, agg),
            |_| true,
        );
        cm.fill(&mut meta, act.as_ref(), sq);
        if let (Some(c), Some(b)) = (&cmp, &both) {
            let by_service = |x: &[ResolvedFlow]| group_by_service(x, &args.group);
            let counts = compare_pairs_of(
                &mut groups,
                (&b.recent, &b.baseline),
                (&b.recent, &b.baseline),
                by_service,
                &agg(&b.baseline),
            );
            keep_changes(&mut groups, c.show, |g| g.change.as_ref());
            c.fill(&mut meta, &f, counts);
        }
        ctx.emit_meta(&groups, &meta)
    } else {
        let order = ctx.config.key_order();
        // Flows: rows are flows.
        let (marks, sq): (Option<Vec<Option<FlowMark>>>, _) = match (&part, &act) {
            (Some(p), Some(a)) => {
                let n = p.marked.squelched_where(|_| true);
                (
                    Some(
                        p.kept_hits
                            .iter()
                            .map(|h| FlowMark::of(h.as_ref(), a))
                            .collect(),
                    ),
                    Some(SquelchedMeta {
                        rows: Some(n),
                        flows: n,
                    }),
                )
            }
            _ => (None, None),
        };
        cm.fill(&mut meta, act.as_ref(), sq);
        let (Some(c), Some(b)) = (&cmp, &both) else {
            let rows = FlowRows {
                flows,
                order: &order,
                marks,
                changes: None,
            };
            return ctx.emit_meta(&rows, &meta);
        };
        // The marks of the records `sides` kept (`kept` without the hidden
        // keys), in their order.
        let marks = marks.map(|m| {
            kept.iter()
                .zip(m)
                .filter(|(fl, _)| !b.hidden.contains(&pce::traffic::flow_key(fl)))
                .map(|(_, m)| m)
                .collect()
        });
        let rows = compared_flows(&b.recent, &b.baseline, marks, c.show);
        c.fill(&mut meta, &f, rows.counts);
        let out = FlowRows {
            flows: &rows.flows,
            order: &order,
            marks: rows.marks,
            changes: Some(rows.changes),
        };
        ctx.emit_meta(&out, &meta)
    }
}

/// Plain pairs in compare mode (`compare_pairs`): their service keys
/// compared, each on every decision (`all`: both sides unfiltered), the
/// view's own rows through its filter (`kept`: both sides filtered);
/// `pairs` (built from the kept recent flows) annotated, the gone pairs
/// from `kept_baseline_pairs`. The pair rows' counts (#79).
fn compare_pairs_of(
    pairs: &mut Vec<FlowGroup>,
    all: (&[ResolvedFlow], &[ResolvedFlow]),
    kept: (&[ResolvedFlow], &[ResolvedFlow]),
    by_service: impl Fn(&[ResolvedFlow]) -> Vec<FlowGroup>,
    kept_baseline_pairs: &[FlowGroup],
) -> CompareCounts {
    let mut services = by_service(kept.0);
    let (all_r, all_b, kept_b) = (by_service(all.0), by_service(all.1), by_service(kept.1));
    compare_groups(
        &mut services,
        Sides {
            all_recent: &all_r,
            all_baseline: &all_b,
            kept_baseline: &kept_b,
        },
    );
    compare_pairs(pairs, kept_baseline_pairs, &services)
}

/// Flow records in compare mode: the recent ones then the gone ones (each
/// with its mark, none for a gone one), kept by `show`.
struct ComparedFlows {
    flows: Vec<ResolvedFlow>,
    marks: Option<Vec<Option<FlowMark>>>,
    changes: Vec<RowChange>,
    /// Before `show`.
    counts: CompareCounts,
}

fn compared_flows(
    recent: &[ResolvedFlow],
    baseline: &[ResolvedFlow],
    marks: Option<Vec<Option<FlowMark>>>,
    show: ChangeSet,
) -> ComparedFlows {
    let c = compare_flows(recent, baseline);
    let mut marks = marks.map(|m| m.into_iter());
    let mut out = ComparedFlows {
        flows: Vec::new(),
        marks: marks.as_ref().map(|_| Vec::new()),
        changes: Vec::new(),
        counts: c.counts,
    };
    let gone = c.gone.into_iter().map(|(f, rc)| (f, rc, None));
    let rows = recent
        .iter()
        .cloned()
        .zip(c.recent)
        .map(|(f, rc)| (f, rc, marks.as_mut().and_then(Iterator::next).flatten()))
        .chain(gone);
    for (f, rc, mark) in rows.filter(|(_, rc, _)| show.contains(rc.change)) {
        out.flows.push(f);
        out.changes.push(rc);
        if let Some(m) = out.marks.as_mut() {
            m.push(mark);
        }
    }
    out
}

async fn groups(ctx: &Context, args: &GroupsArgs) -> Result<(), CliError> {
    let keys = keys_or_profile(ctx, &args.keys);
    let focus = cli_focus(&args.filter, &keys, args.by)?;
    let Some((f, cmp)) = fetch_sides(ctx, &args.filter, focus.as_ref()).await? else {
        return Ok(());
    };
    let cm = CliMarks::load(ctx, args.filter.no_marks, args.filter.show_squelched);
    let act = cm.active(args.filter.draft);
    let part = act.as_ref().map(|a| partition(&f.flows, a, cm.show));
    let kept: &[ResolvedFlow] = part.as_ref().map_or(&f.flows, |p| &p.kept);
    let both = cmp
        .as_ref()
        .map(|c| c.sides(kept, part.as_ref(), act.as_ref(), cm.show));
    let flows: &[ResolvedFlow] = both.as_ref().map_or(kept, |b| &b.recent);
    let decisions = Decisions::from(args.show);
    let mut meta = f.meta(ctx);
    match sections_for(flows, &keys, args) {
        Some(mut sections) => {
            // The squelched records of the sections shown: `--app`'s only.
            let resolved = keys_or_default(&keys);
            let app = args.filter.app.as_deref().map(parse_app);
            let dir = match args.by {
                ByArg::In => Some(Direction::In),
                ByArg::Out => Some(Direction::Out),
                _ => None,
            };
            let agg = |x: &[ResolvedFlow]| {
                sections_for(x, &keys, args)
                    .map(|s| s.sections)
                    .unwrap_or_default()
            };
            let sq = squelched(
                part.as_ref(),
                act.as_ref(),
                |p, a| annotate_sections(&mut sections.sections, p, a, agg),
                |x| match &app {
                    Some(app) => in_app(x, &resolved, app, dir, decisions),
                    None => decisions.keeps(x),
                },
            );
            cm.fill(&mut meta, act.as_ref(), sq);
            // An app emptied by squelch is not a miss: judge it on the
            // unsquelched flows (only then, and only when squelch hid some).
            let squelched_only = sections.sections.is_empty()
                && part.as_ref().is_some_and(|p| p.kept.len() < f.flows.len())
                && sections_for(&f.flows, &keys, args).is_some_and(|s| !s.sections.is_empty());
            if let (Some(c), Some(b)) = (&cmp, &both) {
                // Each key judged on every decision, then `--show`'s rows.
                let all = |x: &[ResolvedFlow]| {
                    sections_with(x, &keys, args, Decisions::All)
                        .map(|s| s.sections)
                        .unwrap_or_default()
                };
                let (all_r, all_b, kept_b) = (all(&b.recent), all(&b.baseline), agg(&b.baseline));
                let counts = compare_sections(
                    &mut sections.sections,
                    Sides {
                        all_recent: &all_r,
                        all_baseline: &all_b,
                        kept_baseline: &kept_b,
                    },
                );
                c.fill(&mut meta, &f, counts);
            }
            if let Some(miss) = app_miss(args, &sections, squelched_only) {
                eprintln!("{miss}");
            }
            if let Some(c) = &cmp {
                keep_section_changes(&mut sections.sections, c.show);
            }
            ctx.emit_meta(&sections, &meta)
        }
        None => {
            let agg = |x: &[ResolvedFlow]| pairs_for(x, &keys, args);
            let mut pairs = agg(flows);
            let sq = squelched(
                part.as_ref(),
                act.as_ref(),
                |p, a| annotate_groups(&mut pairs, p, a, agg),
                |x| decisions.keeps(x),
            );
            cm.fill(&mut meta, act.as_ref(), sq);
            if let (Some(c), Some(b)) = (&cmp, &both) {
                let by_service = |x: &[ResolvedFlow]| group_by_service(x, &keys);
                let (kr, kb) = (decisions.kept(&b.recent), decisions.kept(&b.baseline));
                let counts = if args.services {
                    let (all_r, all_b, kept_b) = (
                        by_service(&b.recent),
                        by_service(&b.baseline),
                        agg(&b.baseline),
                    );
                    compare_groups(
                        &mut pairs,
                        Sides {
                            all_recent: &all_r,
                            all_baseline: &all_b,
                            kept_baseline: &kept_b,
                        },
                    )
                } else {
                    compare_pairs_of(
                        &mut pairs,
                        (&b.recent, &b.baseline),
                        (&kr, &kb),
                        by_service,
                        &agg(&b.baseline),
                    )
                };
                keep_changes(&mut pairs, c.show, |g| g.change.as_ref());
                c.fill(&mut meta, &f, counts);
            }
            if args.services {
                ctx.emit_meta(&ServiceGroups(pairs), &meta)
            } else {
                ctx.emit_meta(&pairs, &meta)
            }
        }
    }
}

async fn mesh_cmd(ctx: &Context, args: &MeshArgs) -> Result<(), CliError> {
    let keys = keys_or_profile(ctx, &args.keys);
    let focus = cli_focus(&args.filter, &keys, ByArg::Pairs)?;
    let Some((f, cmp)) = fetch_sides(ctx, &args.filter, focus.as_ref()).await? else {
        return Ok(());
    };
    let cols = MeshColumns {
        user: !args.no_user,
        process: !args.no_process,
    };
    let cm = CliMarks::load(ctx, args.filter.no_marks, args.filter.show_squelched);
    let act = cm.active(args.filter.draft);
    let part = act.as_ref().map(|a| partition(&f.flows, a, cm.show));
    let kept: &[ResolvedFlow] = part.as_ref().map_or(&f.flows, |p| &p.kept);
    let both = cmp
        .as_ref()
        .map(|c| c.sides(kept, part.as_ref(), act.as_ref(), cm.show));
    let flows: &[ResolvedFlow] = both.as_ref().map_or(kept, |b| &b.recent);
    let agg = |x: &[ResolvedFlow]| mesh(x, &keys, cols);
    let mut rows = agg(flows);
    let sq = squelched(
        part.as_ref(),
        act.as_ref(),
        |p, a| annotate_mesh(&mut rows, p, a, agg),
        |_| true,
    );
    let mut meta = f.meta(ctx);
    cm.fill(&mut meta, act.as_ref(), sq);
    if let (Some(c), Some(b)) = (&cmp, &both) {
        let counts = compare_mesh(&mut rows, &agg(&b.baseline));
        keep_changes(&mut rows, c.show, |r| r.change.as_ref());
        c.fill(&mut meta, &f, counts);
    }
    ctx.emit_meta(&rows, &meta)
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
            draft: None,
            marks: None,
            change: None,
        }
    }

    #[test]
    fn times_without_an_offset_are_local_and_dates_utc_except_for_since() {
        let berlin = jiff::tz::TimeZone::get("Europe/Berlin").unwrap();
        let t = |s: &str, end: bool, local: bool| {
            pce::traffic::format_timestamp(parse_time_in(s, end, local, &berlin).unwrap())
        };
        // CEST (UTC+2) on 2026-10-23.
        assert_eq!(t("2026-10-23T18:00", false, false), "2026-10-23T16:00:00Z");
        assert_eq!(t("2026-10-23T18:00", false, true), "2026-10-23T16:00:00Z");
        assert_eq!(
            t("2026-10-23T18:00:00Z", false, true),
            "2026-10-23T18:00:00Z"
        );
        assert_eq!(
            t("2026-10-23T18:00:00+01:00", false, false),
            "2026-10-23T17:00:00Z"
        );
        // --from / --to: a date in UTC, as before.
        assert_eq!(t("2026-10-23", false, false), "2026-10-23T00:00:00Z");
        assert_eq!(t("2026-10-23", true, false), "2026-10-23T23:59:59Z");
        // --since: local midnight.
        assert_eq!(t("2026-10-23", false, true), "2026-10-22T22:00:00Z");
        // After the switch to CET (UTC+1).
        assert_eq!(t("2026-11-02T09:00", false, true), "2026-11-02T08:00:00Z");
        assert!(parse_time_in("friday", false, true, &berlin).is_err());
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
            draft: None,
            marks: None,
            change: None,
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
            marks: None,
            changes: None,
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
            marks: None,
            changes: None,
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
        assert_eq!(app_miss(&a, &s, false), None);
        let a = groups_args(&["--by", "in", "--app", "pos/PCI"]);
        let s = sections_for(&flows, &app_env(), &a).unwrap();
        assert!(s.sections.is_empty());
        assert_eq!(
            app_miss(&a, &s, false).as_deref(),
            Some("no app matches \"pos/PCI\"")
        );
        assert_eq!(
            app_miss(&a, &s, true).as_deref(),
            Some("all of \"pos/PCI\"'s flows are squelched (--show-squelched shows them)")
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

    fn query_filter(argv: &[&str]) -> FilterArgs {
        use clap::Parser;
        let mut full = vec!["illumio-tui", "traffic", "query"];
        full.extend_from_slice(argv);
        let cli = crate::cli::Cli::try_parse_from(full).expect("parses");
        match cli.command {
            Some(crate::cli::Command::Traffic(TrafficCmd::Query(a))) => a.filter,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn flags_save_as_the_tui_text_and_the_text_reads_back_as_flags() {
        let f = query_filter(&[
            "--src",
            "app=web,env=prod",
            "--src",
            "ipl:Corp",
            "--exclude-src",
            "app=backup,role=db",
            "--dst",
            "app=db",
            "--port",
            "8000-8100",
            "--proto",
            "tcp",
            "--exclude-service",
            "53/udp",
            "--decision",
            "blocked,potentially_blocked",
            "--transmission",
            "unicast",
            "--window",
            "7d",
            "--max-results",
            "20000",
        ]);
        let s = f.to_saved().unwrap();
        assert_eq!(
            s.src.as_deref(),
            Some("app=web,env=prod | ipl:Corp, !app=backup, !role=db"),
            "repeat = |, --exclude-src = !"
        );
        assert_eq!(s.service.as_deref(), Some("8000-8100/tcp, !53/udp"));
        assert_eq!(
            s.decisions,
            Some(vec!["blocked".into(), "potentially_blocked".into()])
        );
        assert_eq!(s.transmission, Some(vec!["unicast".into()]));
        assert_eq!(
            (s.window.as_deref(), s.max_results),
            (Some("7d"), Some(20000))
        );

        // Back: the same flags, the services as the saved alternatives.
        let mut back = FilterArgs::default();
        back.with_saved(&s).unwrap();
        assert_eq!(back.src, vec!["app=web,env=prod", "ipl:Corp"]);
        assert_eq!(back.exclude_src, vec!["app=backup", "role=db"]);
        assert_eq!(back.saved_services, vec!["8000-8100/tcp"]);
        assert_eq!(back.exclude_service, vec!["53/udp"]);
        assert_eq!(back.transmission, vec![TransmissionArg::Unicast]);
        assert_eq!(back.to_saved().unwrap(), s, "round trip");
        // A TUI-only form: several services, svc:NAME.
        let tui = crate::filters::TrafficSaved {
            service: Some("443/tcp | svc:HTTPS, !udp".into()),
            ..Default::default()
        };
        let mut f = FilterArgs::default();
        f.with_saved(&tui).unwrap();
        assert_eq!(f.saved_services, vec!["443/tcp", "svc:HTTPS"]);
        assert_eq!(f.to_saved().unwrap().service, tui.service);
    }

    #[test]
    fn a_flag_replaces_its_saved_field_and_the_rest_stays() {
        let saved = crate::filters::TrafficSaved {
            src: Some("app=web".into()),
            dst: Some("!app=backup".into()),
            service: Some("443/tcp".into()),
            window: Some("7d".into()),
            max_results: Some(500),
            ..Default::default()
        };
        let mut f = query_filter(&[
            "--exclude-src",
            "app=db",
            "--port",
            "22",
            "--from",
            "2026-10-01",
            "--to",
            "2026-10-02",
        ]);
        f.with_saved(&saved).unwrap();
        assert!(f.src.is_empty(), "--exclude-src replaces the whole source");
        assert_eq!(f.exclude_src, vec!["app=db"]);
        assert_eq!(f.exclude_dst, vec!["app=backup"], "kept");
        assert!(f.saved_services.is_empty(), "--port replaces the service");
        assert_eq!(f.window, None, "--from/--to replace the window");
        assert_eq!(f.max_results, Some(500));
    }

    #[test]
    fn what_the_text_cannot_hold_is_said_not_dropped() {
        for (argv, flag) in [
            (vec!["--process", "nginx"], "--process"),
            (
                vec!["--from", "2026-10-01", "--to", "2026-10-02"],
                "--from/--to",
            ),
            (vec!["--app", "shop/prod"], "--app"),
        ] {
            let e = query_filter(&argv).to_saved().unwrap_err();
            assert!(e.starts_with(flag), "{e}");
        }
        let e = query_filter(&["--src", "fqdn:x.example"])
            .to_saved()
            .unwrap_err();
        assert!(e.contains("source"), "checked as the f editor checks: {e}");
        for (bad, field) in [
            (
                crate::filters::TrafficSaved {
                    transmission: Some(vec!["anycast".into()]),
                    ..Default::default()
                },
                "field transmission",
            ),
            (
                crate::filters::TrafficSaved {
                    decisions: Some(vec!["nope".into()]),
                    ..Default::default()
                },
                "field decisions",
            ),
            (
                crate::filters::TrafficSaved {
                    window: Some("soon".into()),
                    ..Default::default()
                },
                "field window",
            ),
            (
                crate::filters::TrafficSaved {
                    max_results: Some(0),
                    ..Default::default()
                },
                "field max_results",
            ),
        ] {
            let e = FilterArgs::default().with_saved(&bad).unwrap_err();
            assert!(e.starts_with(field), "{e}");
        }
        // The text's own operators in a flag value: refused, naming the flag.
        for (argv, flag) in [
            (vec!["--src", "!app=x"], "--exclude-src"),
            (vec!["--dst", "app=a | app=b"], "repeat --dst"),
            (vec!["--exclude-src", "!app=x"], "without !"),
        ] {
            let e = query_filter(&argv).to_saved().unwrap_err();
            assert!(e.contains(flag), "{e}");
        }
    }

    #[test]
    fn a_bare_protocol_saves_as_its_name_and_other_numbers_are_refused() {
        for (proto, text) in [("6", "tcp"), ("17", "udp"), ("1", "icmp"), ("TCP", "tcp")] {
            let s = query_filter(&["--proto", proto]).to_saved().unwrap();
            assert_eq!(s.service.as_deref(), Some(text), "{proto}");
            let mut back = FilterArgs::default();
            back.with_saved(&s).unwrap();
            assert_eq!(back.saved_services, vec![text]);
            let e = ServiceExpr::parse(text).unwrap();
            assert!(
                format!("{e:?}").to_lowercase().contains("proto"),
                "{text} reads back as a protocol: {e:?}"
            );
        }
        let e = query_filter(&["--proto", "47"]).to_saved().unwrap_err();
        assert!(e.contains("--proto 47 alone cannot be saved"), "{e}");
        let s = query_filter(&["--port", "443", "--proto", "6"])
            .to_saved()
            .unwrap();
        assert_eq!(s.service.as_deref(), Some("443/6"));
    }
}
