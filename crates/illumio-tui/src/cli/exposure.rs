//! `illumio-tui exposure` (#82): which risky services (RDP, SMB, WinRM,
//! SSH, database ports …) reach each destination group, from where, and
//! under which decision. One Explorer query over every port of the list
//! (the profile's `[exposure]` or the defaults), in its own cache slot,
//! through the cost guard; `pce::traffic::exposure` makes the rows. Which
//! rule allows an exposed row is not looked up (a request per row): each
//! one names the `traffic explain` command that answers it.
use super::context::Context;
use super::error::CliError;
use super::marked::CliMarks;
use super::output::{OutputFormat, TableRows, join_list};
use super::traffic::{FilterArgs, fetch_exposure, keys_or_profile};
use crate::config::ConfigError;
use clap::{Args, ValueEnum};
use pce::marks::partition;
use pce::traffic::exposure::{self, ExposureRow, RiskyService, ServiceSummary, Status};
use pce::traffic::{ResolvedFlow, compact_duration, group_values, keys_or_default};
use serde::Serialize;
use std::io::Write;

/// `exposure --only`: the rows of these statuses.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnlyArg {
    /// Allowed from outside the destination's group
    Exposed,
    /// Potentially blocked (allowed until the destination is enforced)
    #[value(name = "potentially_blocked")]
    PotentiallyBlocked,
    /// Blocked only
    Blocked,
    /// Allowed from the destination's own group only
    Intra,
}

impl From<OnlyArg> for Status {
    fn from(o: OnlyArg) -> Self {
        match o {
            OnlyArg::Exposed => Status::Exposed,
            OnlyArg::PotentiallyBlocked => Status::PotentiallyBlocked,
            OnlyArg::Blocked => Status::Blocked,
            OnlyArg::Intra => Status::Intra,
        }
    }
}

#[derive(Args, Debug, Clone, Default)]
pub struct ExposureArgs {
    /// Relative window ending now, e.g. 1h, 24h, 7d (default from profile, 24h)
    #[arg(long)]
    pub window: Option<String>,
    /// Only these risky services, by name: rdp,ssh (comma-separated or repeatable; the
    /// profile's [exposure] list or the defaults)
    #[arg(long = "service", value_delimiter = ',', value_name = "NAMES")]
    pub services: Vec<String>,
    /// Only rows of these statuses: exposed, potentially_blocked, blocked, intra
    /// (comma-separated; the summary still counts every row)
    #[arg(long, value_enum, value_delimiter = ',')]
    pub only: Vec<OnlyArg>,
    /// The label keys a destination group is made of (default the profile's scope keys,
    /// app,env)
    #[arg(long, value_delimiter = ',')]
    pub keys: Vec<String>,
    /// Source filter, as on traffic: key=value, ip:ADDR, ipl:NAME, wl:HOST. Comma = AND,
    /// repeat = OR
    #[arg(long = "src")]
    pub src: Vec<String>,
    /// Destination filter, same syntax as --src, plus fqdn:NAME
    #[arg(long = "dst")]
    pub dst: Vec<String>,
    /// Exclude sources matching this filter (repeatable)
    #[arg(long = "exclude-src")]
    pub exclude_src: Vec<String>,
    /// Exclude destinations matching this filter (repeatable)
    #[arg(long = "exclude-dst")]
    pub exclude_dst: Vec<String>,
    /// Exclude a service: PORT, PORT/PROTO (22/tcp) or svc:NAME; repeatable
    #[arg(long = "exclude-service")]
    pub exclude_service: Vec<String>,
    /// Row cap, up to 200000 (default from profile, 10000; > 10000 needs --yes)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Confirm an expensive query (window > 7d or max_results > 10000)
    #[arg(long)]
    pub yes: bool,
    /// When the result is truncated, re-run it at a larger cap until it is complete, as
    /// on traffic (above 10000 only with --yes)
    #[arg(long)]
    pub backfill: bool,
    /// Also the decision the draft policy would make (one PUT update_rules on the query,
    /// status polls; it changes no policy): draft_allowed, draft_blocked per row
    #[arg(long)]
    pub draft: bool,
    /// Ignore marks.toml: squelched flows are shown too (they always count for
    /// --fail-on-exposed)
    #[arg(long)]
    pub no_marks: bool,
    /// Exit 8 (exposed) when any row is exposed, after the report: a CI or cron gate. Rows
    /// squelch hides count too. A truncated or incomplete result without an exposed row is
    /// exit 4
    #[arg(long)]
    pub fail_on_exposed: bool,
}

impl ExposureArgs {
    /// The Explorer filter it runs: the given sides, every port of `list`.
    fn filter(&self, list: &[RiskyService]) -> FilterArgs {
        FilterArgs {
            window: self.window.clone(),
            src: self.src.clone(),
            dst: self.dst.clone(),
            exclude_src: self.exclude_src.clone(),
            exclude_dst: self.exclude_dst.clone(),
            exclude_service: self.exclude_service.clone(),
            max_results: self.max_results,
            yes: self.yes,
            backfill: self.backfill,
            draft: self.draft,
            no_marks: self.no_marks,
            saved_services: exposure::service_terms(list),
            ..FilterArgs::default()
        }
    }
}

/// One row as printed: the row, and for an exposed one the `traffic
/// explain` arguments that say which rule allows it.
#[derive(Debug, Serialize)]
pub struct ExposureOut {
    #[serde(flatten)]
    pub row: ExposureRow,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explain: Option<Vec<String>>,
    /// An exposed row squelch hides from the table (marks.toml): still in
    /// JSON, and still counted by `--fail-on-exposed`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub squelched: bool,
}

/// What `data` holds.
#[derive(Debug, Serialize)]
pub struct ExposureReport {
    /// The window the traffic covers, `24h`.
    pub window: String,
    /// Draft decisions were loaded (`--draft`).
    pub draft: bool,
    /// One per risky service looked for, in the list's order.
    pub services: Vec<ServiceSummary>,
    pub rows: Vec<ExposureOut>,
}

/// `value` for a shell, quoted when it needs it.
fn shell_word(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.,=:/@+".contains(c))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

/// The `traffic explain` arguments for an exposed row: its destination
/// (the group's labels, or the sample's address) and the exposing flow's
/// port; `None` for another row or one it cannot name.
pub fn explain_args(r: &ExposureRow, keys: &[String], window: Option<&str>) -> Option<Vec<String>> {
    if r.status != Status::Exposed {
        return None;
    }
    let f: &ResolvedFlow = &r.sample;
    let dst = side_term(&f.dst, keys)?;
    let proto = matches!(f.proto.as_str(), "tcp" | "udp").then_some(f.proto.clone())?;
    let port = f.port?.to_string();
    let mut args: Vec<String> = vec!["traffic".into(), "explain".into()];
    // One source group: its exposing flow's source, so the answer is
    // about that flow. Several: none, so it covers every source (dst and
    // port), never a part of the row.
    let s = &r.sources;
    let groups = u64::from(s.intra > 0) + s.apps + s.ip_lists;
    if groups == 1
        && let Some(src) = side_term(&f.src, keys)
    {
        args.extend(["--src".into(), src]);
    }
    args.extend(["--dst", &dst, "--port", &port, "--proto", &proto].map(String::from));
    if let Some(w) = window {
        args.extend(["--window".into(), w.to_string()]);
    }
    Some(args)
}

/// One side of an explain command: a labelled workload's group labels
/// (the keys it has), else its address.
fn side_term(ep: &pce::traffic::ResolvedEndpoint, keys: &[String]) -> Option<String> {
    if ep.is_workload() && !ep.labels.is_empty() {
        let terms: Vec<String> = keys
            .iter()
            .zip(group_values(ep, keys))
            .filter(|(_, v)| v != "-")
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        if !terms.is_empty() {
            return Some(terms.join(","));
        }
    }
    ep.ip.as_ref().map(|ip| format!("ip:{ip}"))
}

/// The command line of `explain_args`.
pub fn explain_command(args: &[String]) -> String {
    let words: Vec<String> = args.iter().map(|a| shell_word(a)).collect();
    format!("illumio-tui {}", words.join(" "))
}

/// The `--csv` columns, in order (`columns::csv_columns` too).
pub const CSV_COLUMNS: [&str; 22] = [
    "status",
    "service",
    "ports",
    "dst",
    "intra",
    "apps",
    "ip_lists",
    "any",
    "internet",
    "flows",
    "connections",
    "allowed",
    "potentially_blocked",
    "blocked",
    "exposed_flows",
    "allowed_conns",
    "pot_blocked_conns",
    "blocked_conns",
    "unknown_conns",
    "draft_allowed",
    "draft_blocked",
    "explain",
];

struct Rows<'a> {
    rows: &'a [ExposureOut],
    draft: bool,
}

impl Rows<'_> {
    /// The rows the table and CSV print: squelched ones left out.
    fn shown(&self) -> impl Iterator<Item = &ExposureOut> {
        self.rows.iter().filter(|o| !o.squelched)
    }
}

impl TableRows for Rows<'_> {
    fn units(&self) -> Option<&'static str> {
        Some(
            "one row per destination group and risky service · flows: flow records · conns: their lifetime counts",
        )
    }
    fn headers(&self) -> Vec<&'static str> {
        let mut h = vec![
            "status",
            "service",
            "ports",
            "dst",
            "sources",
            "flows",
            "conns",
            "allowed",
            "pot.blocked",
            "blocked",
        ];
        if self.draft {
            h.push("draft");
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.shown()
            .map(|o| {
                let r = &o.row;
                let mut cells = vec![
                    r.status.short().to_string(),
                    r.service.clone(),
                    r.ports.join(","),
                    r.dst_text(),
                    r.sources.text(),
                    r.flows.to_string(),
                    r.connections.to_string(),
                    r.allowed.to_string(),
                    r.potentially_blocked.to_string(),
                    r.blocked.to_string(),
                ];
                if self.draft {
                    cells.push(pce::traffic::DraftCounts::text(r.draft.as_ref()));
                }
                cells
            })
            .collect()
    }
    fn full_headers(&self) -> Vec<&'static str> {
        CSV_COLUMNS.to_vec()
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.shown()
            .map(|o| {
                let r = &o.row;
                let s = &r.sources;
                let c = &r.conns;
                let d = |f: fn(&pce::traffic::DraftCounts) -> u64| {
                    r.draft
                        .as_ref()
                        .map(f)
                        .map(|n| n.to_string())
                        .unwrap_or_default()
                };
                vec![
                    r.status.as_str().to_string(),
                    r.service.clone(),
                    join_list(&r.ports),
                    r.dst_text(),
                    s.intra.to_string(),
                    s.apps.to_string(),
                    s.ip_lists.to_string(),
                    s.any.to_string(),
                    s.internet.to_string(),
                    r.flows.to_string(),
                    r.connections.to_string(),
                    r.allowed.to_string(),
                    r.potentially_blocked.to_string(),
                    r.blocked.to_string(),
                    r.exposed_flows.to_string(),
                    c.allowed_conns.to_string(),
                    c.pot_blocked_conns.to_string(),
                    c.blocked_conns.to_string(),
                    c.unknown_conns.to_string(),
                    d(|x| x.draft_allowed),
                    d(|x| x.draft_blocked),
                    o.explain
                        .as_deref()
                        .map(explain_command)
                        .unwrap_or_default(),
                ]
            })
            .collect()
    }
}

/// The footer lines printed under the table at most.
pub const EXPLAIN_LINES: usize = 10;

/// The footer under the table: which rule allows each exposed row, one
/// plain line each (`rdp  jump-infra/Production  illumio-tui traffic
/// explain …`), the first `EXPLAIN_LINES`, then how many more `--json`
/// holds. Empty without an exposed row shown.
pub fn explain_footer(rows: &[ExposureOut]) -> Vec<String> {
    let named: Vec<(&ExposureOut, &[String])> = rows
        .iter()
        .filter(|o| !o.squelched)
        .filter_map(|o| o.explain.as_deref().map(|a| (o, a)))
        .collect();
    if named.is_empty() {
        return Vec::new();
    }
    let shown = &named[..named.len().min(EXPLAIN_LINES)];
    let sw = shown
        .iter()
        .map(|(o, _)| o.row.service.chars().count())
        .max()
        .unwrap_or(0);
    let dw = shown
        .iter()
        .map(|(o, _)| o.row.dst_text().chars().count())
        .max()
        .unwrap_or(0);
    let mut lines =
        vec!["which rule allows it (one Explorer query each, run on demand):".to_string()];
    for (o, a) in shown {
        lines.push(format!(
            "{:sw$}  {:dw$}  {}",
            o.row.service,
            o.row.dst_text(),
            explain_command(a)
        ));
    }
    if named.len() > shown.len() {
        lines.push(format!(
            "{} more: --json has every command",
            named.len() - shown.len()
        ));
    }
    lines
}

/// The profile's risky services narrowed to `--service`: a malformed
/// `[exposure]` is a config error (exit 2), an unknown name an argument
/// one, both before any request.
pub fn services(ctx: &Context, args: &ExposureArgs) -> Result<Vec<RiskyService>, CliError> {
    let list = ctx
        .config
        .exposure_services
        .clone()
        .map_err(|e| CliError::Config(ConfigError::Message(e)))?;
    exposure::select(&list, &args.services).map_err(CliError::Args)
}

pub async fn run(ctx: &Context, args: &ExposureArgs) -> Result<(), CliError> {
    let list = services(ctx, args)?;
    let keys = keys_or_default(&keys_or_profile(ctx, &args.keys));
    let f = fetch_exposure(ctx, &args.filter(&list)).await?;
    // Squelched flows (marks.toml) are left out, as on traffic.
    let cm = CliMarks::load(ctx, args.no_marks, false);
    let act = cm.active(args.draft);
    let part = act.as_ref().map(|a| partition(&f.flows, a, false));
    let kept: &[ResolvedFlow] = part.as_ref().map_or(&f.flows, |p| &p.kept);
    let all = exposure::exposure_rows(kept, &list, &keys);
    let mut meta = f.meta(ctx);
    // One Explorer query (one per cap step with --backfill), as traffic.
    let queries = super::traffic::side_queries(&f);
    meta.queries = Some(queries);
    meta.cost = Some(
        format!(
            "{queries} Explorer {} (each 1 POST, status polls, 1 download{})",
            if queries == 1 { "query" } else { "queries" },
            if args.draft {
                "; the result kept re-evaluated against the draft: 1 PUT update_rules, polls, 1 download"
            } else {
                ""
            }
        )
        .into(),
    );
    let sq = part.as_ref().map(|p| super::output::SquelchedMeta {
        rows: None,
        flows: p
            .marked
            .squelched_where(|x| exposure::service_of(&list, x).is_some()),
    });
    cm.fill(&mut meta, act.as_ref(), sq);
    // Squelch hides rows from the table, never an exposure from the gate:
    // the exposed rows of every flow that no shown row is exposed for.
    let hidden: Vec<ExposureRow> = match &part {
        Some(_) => exposure::exposure_rows(&f.flows, &list, &keys)
            .into_iter()
            .filter(|h| {
                h.status == Status::Exposed
                    && !all.iter().any(|r| {
                        r.status == Status::Exposed && r.service == h.service && r.dst == h.dst
                    })
            })
            .collect(),
        None => Vec::new(),
    };
    if !hidden.is_empty() {
        meta.warnings.get_or_insert_with(Vec::new).push(format!(
            "{} exposed row{} squelched (marks.toml): {}; --fail-on-exposed counts them",
            hidden.len(),
            if hidden.len() == 1 { "" } else { "s" },
            hidden
                .iter()
                .map(|h| format!("{} {}", h.service, h.dst_text()))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let summaries = exposure::summarize(&all, &list);
    let shown_exposed = all.iter().filter(|r| r.status == Status::Exposed).count();
    let exposed = shown_exposed + hidden.len();
    let only: Vec<Status> = args.only.iter().map(|o| Status::from(*o)).collect();
    let out = |row: ExposureRow, squelched: bool| ExposureOut {
        explain: explain_args(&row, &keys, args.window.as_deref()),
        row,
        squelched,
    };
    let report = ExposureReport {
        window: compact_duration(f.span),
        draft: f.draft.is_some(),
        services: summaries,
        rows: all
            .into_iter()
            .map(|r| out(r, false))
            .chain(hidden.into_iter().map(|r| out(r, true)))
            .filter(|o| only.is_empty() || only.contains(&o.row.status))
            .collect(),
    };
    let squelched = exposed - shown_exposed;
    let rows = Rows {
        rows: &report.rows,
        draft: report.draft,
    };
    // The title, then a line per service some flow reached (never clipped
    // as a table cell would be).
    let shown = report.rows.iter().filter(|o| !o.squelched).count();
    let mut title = format!(
        "exposure · {} · {shown} row{} · {shown_exposed} exposed",
        report.window,
        if shown == 1 { "" } else { "s" }
    );
    if squelched > 0 {
        title.push_str(&format!(
            " · {squelched} exposed row{} squelched (in --json, counted by --fail-on-exposed)",
            if squelched == 1 { "" } else { "s" }
        ));
    }
    for s in report.services.iter().filter(|s| s.destinations > 0) {
        title.push_str(&format!("\n  {}", s.text()));
    }
    // `--csv`: the rows alone, one table per output.
    let tables: Vec<(String, &dyn TableRows)> = vec![(title, &rows)];
    ctx.emit_report(&report, &tables, &meta)?;
    if ctx.format == OutputFormat::Table {
        let footer = explain_footer(&report.rows);
        if !footer.is_empty() {
            let mut out = std::io::stdout().lock();
            writeln!(out)?;
            for l in footer {
                writeln!(out, "{l}")?;
            }
        }
    }
    if args.fail_on_exposed {
        if exposed > 0 {
            return Err(CliError::Exposed(format!(
                "{exposed} exposed row{} (allowed from outside the destination's group)",
                if exposed == 1 { "" } else { "s" }
            )));
        }
        if f.truncated || f.incomplete {
            return Err(CliError::Incomplete(format!(
                "no exposed row found, but the result is {}: the answer is not complete",
                if f.truncated {
                    "truncated (cap reached; --backfill raises it)"
                } else {
                    "incomplete (a region did not answer)"
                }
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use pce::traffic::ResolvedEndpoint;
    use pce::traffic::default_services;
    use pce::traffic::exposure::exposure_rows;

    fn wl(app: &str) -> ResolvedEndpoint {
        ResolvedEndpoint {
            ip: Some(format!("10.0.0.{}", app.len())),
            workload_href: Some(format!("/orgs/5/workloads/{app}")),
            labels: ["app", "env"]
                .iter()
                .map(|k| pce::ResolvedLabel {
                    href: format!("/orgs/5/labels/{k}-{app}"),
                    key: (*k).into(),
                    value: if *k == "app" {
                        app.into()
                    } else {
                        "prod".into()
                    },
                })
                .collect(),
            ..ResolvedEndpoint::default()
        }
    }

    fn rdp(src: &str, dst: &str) -> ResolvedFlow {
        ResolvedFlow {
            src: wl(src),
            dst: wl(dst),
            port: Some(3389),
            proto: "tcp".into(),
            process_name: None,
            user_name: None,
            num_connections: 1,
            policy_decision: pce::models::PolicyDecision::Allowed,
            draft_policy_decision: None,
            flow_direction: None,
            first_detected: None,
            last_detected: None,
            rules: Vec::new(),
            side_rules: Vec::new(),
            bytes_in: None,
            bytes_out: None,
        }
    }

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    #[test]
    fn explain_names_the_source_only_when_the_row_has_one() {
        let list = default_services();
        let one = &exposure_rows(&[rdp("vdi", "jump")], &list, &keys())[0];
        let args = explain_args(one, &keys(), Some("24h")).unwrap();
        assert_eq!(
            explain_command(&args),
            "illumio-tui traffic explain --src app=vdi,env=prod --dst app=jump,env=prod --port 3389 --proto tcp --window 24h"
        );
        let two = &exposure_rows(&[rdp("vdi", "jump"), rdp("ops", "jump")], &list, &keys())[0];
        assert_eq!(
            explain_command(&explain_args(two, &keys(), None).unwrap()),
            "illumio-tui traffic explain --dst app=jump,env=prod --port 3389 --proto tcp",
            "two source groups: dst and port only"
        );
    }

    #[test]
    fn the_footer_is_one_line_per_row_capped_at_ten() {
        let list = default_services();
        let flows: Vec<ResolvedFlow> = (0..12).map(|i| rdp("vdi", &format!("d{i:02}"))).collect();
        let rows: Vec<ExposureOut> = exposure_rows(&flows, &list, &keys())
            .into_iter()
            .map(|row| ExposureOut {
                explain: explain_args(&row, &keys(), None),
                row,
                squelched: false,
            })
            .collect();
        let footer = explain_footer(&rows);
        assert_eq!(footer.len(), 1 + EXPLAIN_LINES + 1);
        assert!(
            footer[1].starts_with("rdp  d00/prod  illumio-tui traffic explain --src app=vdi"),
            "{}",
            footer[1]
        );
        assert_eq!(footer[11], "2 more: --json has every command");
        assert!(explain_footer(&rows[..0]).is_empty());
    }

    #[test]
    fn shell_words_are_quoted_when_needed() {
        assert_eq!(shell_word("app=shop,env=prod"), "app=shop,env=prod");
        assert_eq!(shell_word("app=my shop"), "'app=my shop'");
        assert_eq!(shell_word("it's"), r"'it'\''s'");
    }
}
