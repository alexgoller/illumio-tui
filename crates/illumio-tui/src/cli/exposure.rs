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
    /// Ignore marks.toml: squelched flows count too
    #[arg(long)]
    pub no_marks: bool,
    /// Exit 8 (exposed) when any row is exposed, after the report: a CI or cron gate. A
    /// truncated or incomplete result without an exposed row is exit 4
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
    let dst = if f.dst.is_workload() && !f.dst.labels.is_empty() {
        let terms: Vec<String> = keys
            .iter()
            .zip(group_values(&f.dst, keys))
            .filter(|(_, v)| v != "-")
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        (!terms.is_empty()).then(|| terms.join(","))
    } else {
        None
    };
    let dst = dst.or_else(|| f.dst.ip.as_ref().map(|ip| format!("ip:{ip}")))?;
    let proto = matches!(f.proto.as_str(), "tcp" | "udp").then_some(f.proto.clone())?;
    let mut args: Vec<String> = [
        "traffic",
        "explain",
        "--dst",
        &dst,
        "--port",
        &f.port?.to_string(),
        "--proto",
        &proto,
    ]
    .map(String::from)
    .to_vec();
    if let Some(w) = window {
        args.extend(["--window".into(), w.to_string()]);
    }
    Some(args)
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
        self.rows
            .iter()
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
        self.rows
            .iter()
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

/// The footer: which rule allows each exposed row, one query each.
struct Explains<'a>(&'a [ExposureOut]);

impl Explains<'_> {
    fn named(&self) -> impl Iterator<Item = (&ExposureOut, &[String])> {
        self.0
            .iter()
            .filter_map(|o| o.explain.as_deref().map(|a| (o, a)))
    }
}

impl TableRows for Explains<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["service", "dst"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.named()
            .map(|(o, _)| vec![o.row.service.clone(), o.row.dst_text()])
            .collect()
    }
    /// The command under its row, whole.
    fn notes(&self) -> Vec<Vec<String>> {
        self.named()
            .map(|(_, a)| vec![format!("  {}", explain_command(a))])
            .collect()
    }
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
    let sq = part.as_ref().map(|p| super::output::SquelchedMeta {
        rows: None,
        flows: p
            .marked
            .squelched_where(|x| exposure::service_of(&list, x).is_some()),
    });
    cm.fill(&mut meta, act.as_ref(), sq);
    let summaries = exposure::summarize(&all, &list);
    let exposed = all.iter().filter(|r| r.status == Status::Exposed).count();
    let only: Vec<Status> = args.only.iter().map(|o| Status::from(*o)).collect();
    let report = ExposureReport {
        window: compact_duration(f.span),
        draft: f.draft.is_some(),
        services: summaries,
        rows: all
            .into_iter()
            .filter(|r| only.is_empty() || only.contains(&r.status))
            .map(|row| ExposureOut {
                explain: explain_args(&row, &keys, args.window.as_deref()),
                row,
            })
            .collect(),
    };
    let rows = Rows {
        rows: &report.rows,
        draft: report.draft,
    };
    let explains = Explains(&report.rows);
    // The title, then a line per service some flow reached (never clipped
    // as a table cell would be).
    let mut title = format!(
        "exposure · {} · {} row{} · {exposed} exposed",
        report.window,
        report.rows.len(),
        if report.rows.len() == 1 { "" } else { "s" }
    );
    for s in report.services.iter().filter(|s| s.destinations > 0) {
        title.push_str(&format!("\n  {}", s.text()));
    }
    // `--csv`: the rows alone, one table per output.
    let mut tables: Vec<(String, &dyn TableRows)> = vec![(title, &rows)];
    if ctx.format != OutputFormat::Csv && explains.named().next().is_some() {
        tables.push((
            "which rule allows it (one Explorer query each, run on demand):".into(),
            &explains,
        ));
    }
    ctx.emit_report(&report, &tables, &meta)?;
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

    #[test]
    fn shell_words_are_quoted_when_needed() {
        assert_eq!(shell_word("app=shop,env=prod"), "app=shop,env=prod");
        assert_eq!(shell_word("app=my shop"), "'app=my shop'");
        assert_eq!(shell_word("it's"), r"'it'\''s'");
    }
}
