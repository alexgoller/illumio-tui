//! `illumio-tui app <app>`: one app's profile in one envelope: its
//! workloads (enforcement and VEN health), its inbound and outbound peers
//! by service and decision, and the policy that covers it. Explorer returns
//! lifetime aggregates, so there is no traffic-over-time view.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::marked::{CliMarks, mark_cell, squelched};
use super::output::{OutputFormat, TableRows};
use super::traffic::{ByArg, DirectionArg, FilterArgs, TransmissionArg, cli_focus, fetch};
use clap::{Args, ValueEnum};
use jiff::Timestamp;
use pce::app::{
    AppPeer, AppPolicyRow, AppWorkload, PolicyReason, WorkloadSummary, app_peers, app_policy,
    app_workload_filter, app_workloads, enforcement_word, label_hrefs, workload_in_app,
};
use pce::marks::{annotate_peers, partition};
use pce::models::{PolicyScope, Workload};
use pce::traffic::{Direction, DraftCounts, FocusDirection};
use serde::Serialize;

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
    /// Only flows sent this way: unicast, broadcast, multicast (repeatable or comma-separated;
    /// default all). Explorer excludes the other kinds on the destination side. workloader
    /// counts unicast only by default: add --transmission unicast to compare
    #[arg(long, value_enum, value_delimiter = ',')]
    pub transmission: Vec<TransmissionArg>,
    /// Match the active policy instead of the draft
    #[arg(long)]
    pub active: bool,
    /// Only this section (JSON: `app`, `direction` and it; --csv needs one)
    #[arg(long, value_enum)]
    pub section: Option<SectionArg>,
    /// Confirm an expensive query (window > 7d, max_results > 10000)
    #[arg(long)]
    pub yes: bool,
    /// Also each peer's flows the draft policy would allow or block (the PCE's Explorer draft
    /// view: per query one PUT on that query and more polls; changes no policy)
    #[arg(long)]
    pub draft: bool,
    /// Keep squelched flows (marks.toml) in the peers, marked `squelch`
    #[arg(long, conflicts_with = "no_marks")]
    pub show_squelched: bool,
    /// Ignore marks.toml: no mark fields, nothing left out
    #[arg(long)]
    pub no_marks: bool,
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
    fn units(&self) -> Option<&'static str> {
        Some(super::traffic::UNITS_GROUPS)
    }
    /// With draft decisions, a `draft` column (`DraftCounts::text`).
    fn headers(&self) -> Vec<&'static str> {
        let mut h = vec!["peer", "service", "decision", "flows", "conns", "bytes"];
        if self.draft() {
            h.push("draft");
        }
        if self.marked() {
            h.push("mark");
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let draft = self.draft();
        let marked = self.marked();
        self.0
            .iter()
            .map(|p| {
                let mut row = vec![
                    p.peer_text(),
                    p.service.clone(),
                    p.decision.as_str().to_string(),
                    p.flows.to_string(),
                    p.connections.to_string(),
                    super::output::bytes_text(p.bytes),
                ];
                if draft {
                    row.push(DraftCounts::text(p.draft.as_ref()));
                }
                if marked {
                    row.push(mark_cell(p.marks.as_ref()));
                }
                row
            })
            .collect()
    }
    /// The table's, with the draft counts as numbers, then `mark`.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        h.retain(|c| *c != "draft" && *c != "mark");
        if self.draft() {
            h.extend(super::traffic::DRAFT_COUNTS);
        }
        if self.marked() {
            h.push("mark");
        }
        h
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        let draft = self.draft();
        let marked = self.marked();
        self.0
            .iter()
            .zip(self.rows())
            .map(|(p, mut row)| {
                let mark = if marked { row.pop() } else { None };
                if draft {
                    row.pop();
                    row.extend(super::traffic::draft_cells(p.draft.as_ref()));
                }
                row.extend(mark);
                row
            })
            .collect()
    }
}

impl Peers<'_> {
    fn draft(&self) -> bool {
        self.0.iter().any(|p| p.draft.is_some())
    }

    fn marked(&self) -> bool {
        self.0.iter().any(|p| p.marks.is_some())
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

pub async fn run(ctx: &Context, args: &AppArgs) -> Result<(), CliError> {
    if ctx.format == OutputFormat::Csv && args.section.is_none() {
        return Err(CliError::Args(
            "--csv prints one table: add --section workloads|inbound|outbound|policy".into(),
        ));
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
        transmission: args.transmission.clone(),
        draft: args.draft,
        ..FilterArgs::default()
    };
    let focus = cli_focus(&filter, &keys, ByArg::Pairs)?
        .ok_or_else(|| CliError::Other("--app is set".into()))?;
    let Some(fetched) = fetch(ctx, &filter, Some(&focus)).await? else {
        return Ok(()); // a dry run; `app` has none, kept for completeness
    };
    let focus = focus.focus;
    let idx = fetched.index.clone();
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
    let d = focus.direction;
    // Marks: the peers aggregate the records kept; each side joins them.
    let cm = CliMarks::load(ctx, args.no_marks, args.show_squelched);
    let act = cm.active(args.draft);
    let part = act.as_ref().map(|a| partition(&fetched.flows, a, cm.show));
    let flows: &[pce::traffic::ResolvedFlow] = part.as_ref().map_or(&fetched.flows, |p| &p.kept);
    let side = |dir: Direction, skip: FocusDirection| -> (Vec<AppPeer>, u64) {
        if d == skip {
            return (Vec::new(), 0);
        }
        let agg = |x: &[pce::traffic::ResolvedFlow]| app_peers(x, &focus, dir);
        let mut rows = agg(flows);
        let gone = match (&part, &act) {
            (Some(p), Some(a)) => annotate_peers(&mut rows, &p.marked, a, agg),
            _ => 0,
        };
        (rows, gone)
    };
    let (inbound, gone_in) = side(Direction::In, FocusDirection::Out);
    let (outbound, gone_out) = side(Direction::Out, FocusDirection::In);
    let sq = squelched(
        part.as_ref(),
        act.as_ref(),
        |_, _| gone_in + gone_out,
        |x| focus.keeps(x),
    );
    let report = AppReport {
        app: focus.app_text(),
        direction: d,
        workloads: WorkloadsSection {
            truncated: wl.truncated,
            summary,
            rows,
        },
        inbound,
        outbound,
        policy: PolicySection {
            scope,
            truncated: rs.truncated,
            rows: app_policy(&rs.value, &hrefs, &idx),
        },
    };
    // `meta.truncated` is the traffic's; the workloads and the rulesets
    // say so in their sections.
    let mut meta = fetched.meta(ctx);
    cm.fill(&mut meta, act.as_ref(), sq);
    let all_hit = fetched.cache == "hit" && wl.hit && rs.hit;
    meta.cache = cache_label(all_hit, ctx.cache_mode);
    let s = &report.workloads.summary;
    let (inbound, outbound) = (Peers(&report.inbound), Peers(&report.outbound));
    let policy = Policy(&report.policy.rows);
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
        }
    }
}
