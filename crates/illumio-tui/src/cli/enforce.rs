//! `illumio-tui enforce <app>` (#56): move one app's managed workloads to
//! enforcement once readiness allows it, watch what is blocked since,
//! and roll back exactly from the change record.
//!
//! - `--to full|selective`: readiness for the app (`readiness::evaluate`:
//!   one draft-evaluated query narrowed to the app, the cached reads), a
//!   refusal (exit 6) when the verdict does not allow it unless `--force`,
//!   then the bulk move (`pce::write::bulk`, every guard) and a change
//!   record (`pce::enforcement`).
//! - `--watch`: one Explorer query from the change time to now, the app
//!   on either side, blocked flows only; exit 7 when there are some.
//! - `--rollback`: one list read of the app's workloads past the cache,
//!   then each workload still as the move set it back to its `before`, one
//!   bulk request per `(mode, visibility)`; a rollback record.
use super::bulk::{
    BulkDryRun, ItemRows, PlanRows, RequestRows, StepRows, check_count, dry_run_data, exit_error,
    write_meta,
};
use super::context::{Context, cache_label};
use super::error::CliError;
use super::marked::CliMarks;
use super::output::{TableRows, Window};
use super::readiness::{self, Apps, Gaps, ReadinessArgs};
use super::write::{self, WriteFlags};
use clap::{ArgGroup, Args};
use jiff::Timestamp;
use pce::enforcement::{
    BlockedSince, ChangeRecord, ModeState, RecordOrigin, RecordReadiness, RollbackPlans,
    RollbackSkip, app_workloads, default_records_dir, newest_record, plan_rollback, probe_dir,
    read_record, rewrite_record, rollback_plans, rolled_back_by, watch_query, with_results,
    write_pending,
};
use pce::models::WorkloadFilter;
use pce::readiness::AppReadiness;
use pce::traffic::{TrafficQueryBuilder, app_labels, keys_or_default, parse_app, resolve_flows};
use pce::write::bulk::{BulkReport, ItemResult};
use pce::write::{
    BatchOrigin, BulkChange, BulkTarget, EnforcementMode, VisibilityLevel, plan_bulk,
};
use pce::{CacheMode, NameIndex};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

#[derive(Args, Debug, Clone, Default)]
#[command(group(ArgGroup::new("action").required(true).args(["to", "watch", "rollback"])))]
pub struct EnforceArgs {
    /// The app, e.g. shop/prod: one value per scope key
    pub app: String,
    /// Move the app's managed workloads to this mode (needs --dry-run or --yes)
    #[arg(long, value_parser = ["full", "selective"])]
    pub to: Option<String>,
    /// Also set this visibility level: flow_summary, flow_drops, flow_off, enhanced_data_collection
    #[arg(long, requires = "to")]
    pub visibility: Option<String>,
    /// The readiness window (default 7d; over 7d needs --yes)
    #[arg(long, requires = "to")]
    pub window: Option<String>,
    /// --to: move though readiness does not allow it (not ready, not evaluated);
    /// --rollback: roll back a rollback record (re-applies its move without readiness)
    #[arg(long, conflicts_with = "watch")]
    pub force: bool,
    /// The number of workloads that change (the dry run shows it); with --yes, needed for full
    /// or selective on more than one workload, or more than 25
    #[arg(long, conflicts_with = "watch")]
    pub confirm_count: Option<usize>,
    /// Row cap of the traffic query (default from profile; > 10000 needs --yes)
    #[arg(long, conflicts_with = "rollback")]
    pub max_results: Option<u32>,
    /// Ignore marks.toml (readiness: no flow accepted; watch: accept-blocked flows alert too)
    #[arg(long, conflicts_with = "rollback")]
    pub no_marks: bool,
    /// List the flows blocked since the change (read-only); exit 7 when there are some
    #[arg(long)]
    pub watch: bool,
    /// Watch from this change record (a path) or RFC 3339 time instead of the app's newest record
    #[arg(long, requires = "watch")]
    pub since: Option<String>,
    /// Restore each workload's mode and level from the change record (needs --dry-run or --yes)
    #[arg(long)]
    pub rollback: bool,
    /// The change record to roll back (default: the app's newest)
    #[arg(long, requires = "rollback")]
    pub record: Option<PathBuf>,
    #[command(flatten)]
    pub write: WriteFlags,
}

pub async fn run(ctx: &Context, a: &EnforceArgs) -> Result<(), CliError> {
    if a.watch {
        watch(ctx, a).await
    } else if a.rollback {
        rollback(ctx, a).await
    } else {
        enforce(ctx, a).await
    }
}

fn records_dir() -> Result<PathBuf, CliError> {
    default_records_dir().ok_or_else(|| {
        CliError::Refused(
            "no $XDG_STATE_HOME or $HOME to place the change records; nothing sent".into(),
        )
    })
}

fn origin(ctx: &Context) -> RecordOrigin {
    RecordOrigin {
        profile: ctx.config.profile_name.clone(),
        pce: ctx.config.host.clone(),
        org_id: ctx.config.org_id,
    }
}

/// [`records_dir`], created and probed with a file written and removed:
/// before any request of a move or rollback, so a record that cannot be
/// written stops it with nothing sent.
fn writable_records_dir() -> Result<PathBuf, CliError> {
    let dir = records_dir()?;
    probe_dir(&dir).map_err(|e| {
        CliError::Refused(format!(
            "change records cannot be written in {}: {e}; nothing sent",
            dir.display()
        ))
    })?;
    Ok(dir)
}

/// Before the send: the record with every workload `pending`, so a send
/// that never finishes (a crash, a kill) still leaves one a rollback can
/// use. Refused (nothing sent) when it cannot be written.
fn record_before(dir: &Path, rec: &ChangeRecord) -> Result<PathBuf, CliError> {
    write_pending(dir, rec, Timestamp::now()).map_err(|e| {
        CliError::Refused(format!(
            "the change record could not be written in {}: {e}; nothing sent",
            dir.display()
        ))
    })
}

/// After the send: the record rewritten with the results. When that
/// fails, the whole record goes to stderr (keep it: a rollback needs it)
/// and the error is returned for after the report.
fn record_after(path: &Path, rec: &ChangeRecord, warnings: &mut Vec<String>) -> Option<CliError> {
    let e = rewrite_record(path, rec).err()?;
    let msg = format!(
        "CHANGE RECORD NOT UPDATED: {} still lists every workload as pending ({e}); the record with the results is on stderr: save it, a rollback needs it",
        path.display()
    );
    eprintln!("{msg}");
    eprintln!(
        "{}",
        serde_json::to_string_pretty(rec).unwrap_or_else(|e| format!("(unprintable: {e})"))
    );
    warnings.push(msg.clone());
    Some(CliError::Other(msg))
}

/// The app's newest move record made on this PCE and org, or the error
/// that names where none was found.
fn newest(ctx: &Context, dir: &Path, app: &str) -> Result<(PathBuf, ChangeRecord), CliError> {
    newest_record(dir, app, Some(&origin(ctx)))?.ok_or_else(|| {
        CliError::Args(format!(
            "no change record for {app} in {}; pass --record (or --since)",
            dir.display()
        ))
    })
}

/// Why a not-ready app is refused: the verdict, the gaps, the pending
/// changes.
fn blockers(r: &AppReadiness) -> String {
    let mut parts = Vec::new();
    let gaps = r.gaps.as_deref().unwrap_or_default();
    if r.coverage.total.would_block_flows > 0 {
        let shown: Vec<String> = gaps
            .iter()
            .take(10)
            .map(|g| format!("{} {} {}", g.direction.as_str(), g.peer_text, g.service))
            .collect();
        let more = gaps.len().saturating_sub(shown.len());
        parts.push(format!(
            "{} would-block flows ({} connections) in {} tuples: {}{}",
            r.coverage.total.would_block_flows,
            r.coverage.total.would_block,
            r.gap_count,
            shown.join(", "),
            if more > 0 {
                format!(", {more} more")
            } else {
                String::new()
            }
        ));
    }
    if r.pending.count > 0 {
        let names: Vec<String> = r
            .pending
            .changes
            .iter()
            .take(5)
            .map(|p| p.name.clone().unwrap_or_else(|| p.href.clone()))
            .collect();
        parts.push(format!(
            "{} pending changes ({})",
            r.pending.count,
            names.join(", ")
        ));
    }
    if parts.is_empty() {
        parts.push("no draft decision for its flows".into());
    }
    format!(
        "{} is not ready ({}): {}; nothing sent. Close the gaps (readiness --app {}) or pass --force",
        r.app,
        r.verdict.text(),
        parts.join("; "),
        r.app
    )
}

/// `enforce --to`'s data.
#[derive(Debug, Serialize)]
pub struct EnforceOutput<'a> {
    pub app: &'a str,
    pub to: ModeState,
    pub dry_run: bool,
    /// Moved though readiness did not allow it.
    pub forced: bool,
    pub readiness: &'a AppReadiness,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<BulkDryRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bulk: Option<&'a BulkReport>,
    /// The change record written (none when nothing moved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<String>,
}

fn parse_target(a: &EnforceArgs) -> Result<(EnforcementMode, Option<VisibilityLevel>), CliError> {
    let to = a.to.as_deref().unwrap_or_default();
    let mode = EnforcementMode::parse(to)
        .filter(|m| m.enforces())
        .ok_or_else(|| CliError::Args(format!("--to {to:?}: full or selective")))?;
    let visibility = a
        .visibility
        .as_deref()
        .map(|v| {
            VisibilityLevel::parse(v)
                .ok_or_else(|| CliError::Args(format!("unknown visibility level {v:?}")))
        })
        .transpose()?;
    Ok((mode, visibility))
}

/// A cut workloads read is not the app: never sent.
fn refuse_truncated(
    truncated: bool,
    yes: bool,
    warnings: &mut Vec<String>,
) -> Result<(), CliError> {
    if !truncated {
        return Ok(());
    }
    if yes {
        return Err(CliError::Args(
            "the workloads read was truncated (the app has more workloads than were returned); nothing sent"
                .into(),
        ));
    }
    warnings.push("the workloads read was truncated: --yes refuses".into());
    Ok(())
}

async fn enforce(ctx: &Context, a: &EnforceArgs) -> Result<(), CliError> {
    // The guard and the flags, the target and the records' place: before
    // any request.
    let guard = write::prepare(ctx, &a.write)?;
    let (mode, visibility) = parse_target(a)?;
    let dir = writable_records_dir()?;
    let rargs = ReadinessArgs {
        window: a.window.clone(),
        apps: vec![a.app.clone()],
        max_results: a.max_results,
        yes: a.write.yes,
        no_marks: a.no_marks,
    };
    let ev = readiness::evaluate(ctx, &rargs).await?;
    let r = ev
        .report
        .apps
        .first()
        .ok_or_else(|| CliError::Other(format!("readiness returned nothing for {}", a.app)))?;
    let allowed = r.verdict.allows_enforcement();
    if !allowed && !a.force {
        return Err(CliError::Refused(blockers(r)));
    }
    let mut warnings = ev.meta.warnings.clone().unwrap_or_default();
    if !allowed {
        warnings.push(format!(
            "--force: moving {} though readiness says {}",
            r.app,
            r.verdict.text()
        ));
    }
    refuse_truncated(ev.workloads.truncated, a.write.yes, &mut warnings)?;
    // The app's workloads, as readiness read and counted them: the
    // plan's selection.
    let keys = keys_or_default(&ctx.config.scope_keys);
    let none = NameIndex::default();
    let targets: Vec<BulkTarget> = app_workloads(&ev.workloads.value, &r.key, &keys, &ev.index)
        .into_iter()
        .map(|w| BulkTarget::of(w, &none))
        .collect();
    if targets.is_empty() {
        return Err(CliError::Args(format!(
            "{} has no workloads: nothing to move",
            r.app
        )));
    }
    let reread = WorkloadFilter {
        labels: ev
            .app_labels
            .iter()
            .map(|m| pce::app::label_hrefs(m))
            .collect(),
        ..WorkloadFilter::default()
    };
    let change = BulkChange::enforce(mode, visibility);
    let mut plan = plan_bulk(
        &targets,
        &change,
        reread,
        Some(BatchOrigin::of(guard.context())),
    )?;
    let org = ctx.config.org_id;
    let target = ModeState::target(mode, visibility);
    let mut meta = ev.meta.clone();
    // Nothing to change, or a dry run: the plan, nothing sent.
    if plan.items.is_empty() || a.write.dry_run {
        if !plan.items.is_empty() {
            check_count(&plan, false, a.confirm_count)?;
            plan.selection_read(ev.workloads_read_at, ev.workloads.hit);
        }
        let data = EnforceOutput {
            app: &r.app,
            to: target,
            dry_run: a.write.dry_run,
            forced: !allowed,
            readiness: r,
            plan: Some(dry_run_data(&plan, a.write.dry_run, org)),
            bulk: None,
            record: None,
        };
        meta.warnings = Some(warnings).filter(|w| !w.is_empty());
        let apps = Apps(std::slice::from_ref(r));
        let p = data.plan.as_ref().expect("a plan");
        let (rows, reqs) = (PlanRows(p), RequestRows(&p.requests));
        let summary = if plan.items.is_empty() {
            format!("{} · nothing sent", p.summary)
        } else {
            format!("{} · {}", p.summary, plan.cost_text())
        };
        return ctx.emit_report(
            &data,
            &[
                (format!("readiness · {}", r.app), &apps as &dyn TableRows),
                (summary, &rows),
                ("requests".into(), &reqs),
            ],
            &meta,
        );
    }
    check_count(&plan, a.write.yes, a.confirm_count)?;
    plan.confirmed = a.confirm_count;
    plan.selection_read(ev.workloads_read_at, ev.workloads.hit);
    let afters: Vec<_> = plan
        .items
        .iter()
        .map(|i| (i.clone(), target.clone()))
        .collect();
    let at = Timestamp::now();
    let mut rec =
        ChangeRecord::of_move(&r.app, &origin(ctx), at, Some(target.clone()), &afters, &[]);
    rec.readiness = Some(RecordReadiness::of(r, !allowed));
    let path = record_before(&dir, &rec)?;
    let report = ctx
        .pce
        .writer(&guard)
        .execute_bulk(plan, &mut |_, _| {})
        .await;
    let rec = with_results(&rec, &afters, &report.items);
    let record_err = record_after(&path, &rec, &mut warnings);
    let record = Some(path.display().to_string());
    let data = EnforceOutput {
        app: &r.app,
        to: target,
        dry_run: false,
        forced: !allowed,
        readiness: r,
        plan: None,
        bulk: Some(&report),
        record: record.clone(),
    };
    let mut meta = ctx.meta("none", ev.workloads.truncated, ev.meta.window.clone());
    meta.write = write_meta(&report, org, &guard);
    let dropped = report.dropped();
    if dropped > 0 {
        warnings.push(format!(
            "{dropped} workloads changed since readiness read them (or are gone): skipped, not overwritten"
        ));
    }
    meta.warnings = Some(warnings).filter(|w| !w.is_empty());
    let status = report.status_text();
    let apps = Apps(std::slice::from_ref(r));
    let steps = StepRows(&report);
    let items = ItemRows(&report);
    ctx.emit_report(
        &data,
        &[
            (format!("readiness · {}", r.app), &apps as &dyn TableRows),
            (
                format!(
                    "sent to {} · {status} · record {}",
                    report.target,
                    record.as_deref().unwrap_or("none")
                ),
                &steps,
            ),
            ("workloads".into(), &items),
        ],
        &meta,
    )?;
    // A record that could not be updated first: it is the louder fault.
    if let Some(e) = record_err {
        return Err(e);
    }
    if let Some(e) = exit_error(&report, status) {
        return Err(e);
    }
    Ok(())
}

/// One `(mode, visibility)` of a rollback: its plan (dry run) or report.
#[derive(Debug, Serialize)]
pub struct RollbackStep {
    pub to: ModeState,
    pub plan: BulkDryRun,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bulk: Option<BulkReport>,
}

/// `enforce --rollback`'s data.
#[derive(Debug, Serialize)]
pub struct RollbackOutput {
    pub app: String,
    /// The record rolled back.
    pub record: String,
    pub dry_run: bool,
    /// Workloads restored (or, dry run, to restore).
    pub count: usize,
    pub needs_confirm_count: bool,
    /// Left as they are, and why (changed by someone else since, gone,
    /// not moved by that change).
    pub skipped: Vec<RollbackSkip>,
    pub groups: Vec<RollbackStep>,
    /// The rollback's own record (none when nothing moved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_record: Option<String>,
}

struct SkipRows<'a>(&'a [RollbackSkip]);

impl TableRows for SkipRows<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["workload", "skipped", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|s| vec![s.hostname.clone(), s.reason.clone(), s.href.clone()])
            .collect()
    }
}

/// A record names where it was made: a rollback goes only there.
fn same_place(ctx: &Context, rec: &ChangeRecord, path: &Path) -> Result<(), CliError> {
    let here = origin(ctx);
    if !rec.made_on(&here) {
        return Err(CliError::Refused(format!(
            "{} was made on {} (org {}); this profile writes to {} (org {}); nothing sent",
            path.display(),
            rec.pce,
            rec.org_id,
            here.pce,
            here.org_id
        )));
    }
    Ok(())
}

async fn rollback(ctx: &Context, a: &EnforceArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let app = parse_app(&a.app).join("/");
    let dir = writable_records_dir()?;
    let (path, rec) = match &a.record {
        Some(p) => (p.clone(), read_record(p)?),
        None => {
            let (p, rec) = newest(ctx, &dir, &app)?;
            if let Some(by) = rolled_back_by(&dir, &p)? {
                return Err(CliError::Refused(format!(
                    "{} was already rolled back by {}; pass --record {} to roll it back again; nothing sent",
                    p.display(),
                    by.display(),
                    p.display()
                )));
            }
            (p, rec)
        }
    };
    if rec.app != app {
        return Err(CliError::Args(format!(
            "{} is a record of {}, not {app}",
            path.display(),
            rec.app,
        )));
    }
    same_place(ctx, &rec, &path)?;
    if rec.is_rollback() && !a.force {
        return Err(CliError::Refused(format!(
            "{} is a rollback record: rolling it back moves the workloads again, without readiness; pass --force to do so; nothing sent",
            path.display()
        )));
    }
    // The app's workloads now: one list read past the cache.
    let keys = keys_or_default(&ctx.config.scope_keys);
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let labels = app_labels(&parse_app(&app), &keys, &idx).map_err(CliError::Args)?;
    let filter = WorkloadFilter {
        labels: vec![pce::app::label_hrefs(&labels)],
        ..WorkloadFilter::default()
    };
    let read_at = std::time::Instant::now();
    let wl = ctx.pce.workloads(&filter, CacheMode::Bypass).await?;
    let mut warnings = Vec::new();
    refuse_truncated(wl.truncated, a.write.yes, &mut warnings)?;
    let rp = plan_rollback(&rec, &wl.value);
    // The typed count covers the whole rollback.
    let RollbackPlans {
        plans,
        count,
        needs_count: needs,
    } = rollback_plans(&rp, &filter, Some(BatchOrigin::of(guard.context())))?;
    if let Some(n) = a.confirm_count
        && n != count
    {
        return Err(CliError::Args(format!(
            "--confirm-count {n} does not match: {count} workloads would be restored; nothing sent"
        )));
    }
    if needs && a.write.yes && a.confirm_count.is_none() {
        return Err(CliError::Args(
            "a rollback of more than one workload to full or selective, or of more than 25: pass --confirm-count with the number restored (--dry-run shows it); nothing sent".into(),
        ));
    }
    let org = ctx.config.org_id;
    let mut out = RollbackOutput {
        app: app.clone(),
        record: path.display().to_string(),
        dry_run: a.write.dry_run,
        count,
        needs_confirm_count: needs,
        skipped: rp.skipped.clone(),
        groups: Vec::new(),
        rollback_record: None,
    };
    let mut results: Vec<ItemResult> = Vec::new();
    let afters = pce::enforcement::afters(
        &plans
            .iter()
            .map(|(to, p)| (to.clone(), p))
            .collect::<Vec<_>>(),
    );
    let mut failed: Option<CliError> = None;
    let mut written = None;
    let at = Timestamp::now();
    let mut rb = ChangeRecord::of_move(&rec.app, &origin(ctx), at, None, &afters, &[]);
    rb.rollback_of = Some(path.display().to_string());
    // The rollback's record before its first send.
    let rb_path = if out.dry_run || count == 0 {
        None
    } else {
        Some(record_before(&dir, &rb)?)
    };
    for (to, mut plan) in plans {
        plan.confirmed = a.confirm_count.map(|_| plan.items.len());
        plan.selection_read(read_at, false);
        let data = dry_run_data(&plan, out.dry_run, org);
        let bulk = if out.dry_run || failed.is_some() {
            None
        } else {
            let report = ctx
                .pce
                .writer(&guard)
                .execute_bulk(plan, &mut |_, _| {})
                .await;
            results.extend(report.items.iter().cloned());
            if let Some(w) = write_meta(&report, org, &guard) {
                written = Some(w);
            }
            failed = exit_error(&report, report.status_text());
            Some(report)
        };
        out.groups.push(RollbackStep {
            to,
            plan: data,
            bulk,
        });
    }
    let mut record_err = None;
    if let Some(p) = rb_path {
        rb = with_results(&rb, &afters, &results);
        record_err = record_after(&p, &rb, &mut warnings);
        out.rollback_record = Some(p.display().to_string());
    }
    if !out.skipped.is_empty() {
        warnings.push(format!(
            "{} workloads left as they are (see skipped)",
            out.skipped.len()
        ));
    }
    let mut meta = ctx.meta("bypass", wl.truncated, None);
    meta.write = written;
    meta.warnings = Some(warnings).filter(|w| !w.is_empty());
    let skips = SkipRows(&out.skipped);
    let plan_rows: Vec<PlanRows> = out.groups.iter().map(|g| PlanRows(&g.plan)).collect();
    let step_rows: Vec<Option<StepRows>> = out
        .groups
        .iter()
        .map(|g| g.bulk.as_ref().map(StepRows))
        .collect();
    let mut tables: Vec<(String, &dyn TableRows)> = Vec::new();
    for ((g, p), s) in out.groups.iter().zip(&plan_rows).zip(&step_rows) {
        let title = match &g.bulk {
            Some(b) => format!("restore to {} · {}", g.to.text(), b.status_text()),
            None => format!("restore to {} · {}", g.to.text(), g.plan.summary),
        };
        tables.push((title, p));
        if let Some(s) = s {
            tables.push(("sent".into(), s));
        }
    }
    let skipped_title = format!(
        "rollback of {} · {} to restore · {} skipped{}",
        out.record,
        out.count,
        out.skipped.len(),
        out.rollback_record
            .as_deref()
            .map(|p| format!(" · record {p}"))
            .unwrap_or_default()
    );
    tables.insert(0, (skipped_title, &skips));
    ctx.emit_report(&out, &tables, &meta)?;
    if let Some(e) = record_err {
        return Err(e);
    }
    failed.map_or(Ok(()), Err)
}

/// `enforce --watch`'s data.
#[derive(Debug, Serialize)]
pub struct WatchOutput {
    pub app: String,
    /// The change time the query starts at.
    pub since: String,
    /// The record it came from (none for `--since <time>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<String>,
    #[serde(flatten)]
    pub blocked: BlockedSince,
}

async fn watch(ctx: &Context, a: &EnforceArgs) -> Result<(), CliError> {
    let (since, record) = match &a.since {
        Some(s) => match s.parse::<Timestamp>() {
            Ok(t) => (t, None),
            Err(_) => {
                let p = PathBuf::from(s);
                let rec = read_record(&p)?;
                same_place(ctx, &rec, &p)?;
                if rec.app != parse_app(&a.app).join("/") {
                    return Err(CliError::Args(format!(
                        "{} is a record of {}, not {}",
                        p.display(),
                        rec.app,
                        a.app
                    )));
                }
                (record_at(&rec, &p)?, Some(p))
            }
        },
        None => {
            let app = parse_app(&a.app).join("/");
            let (p, rec) = newest(ctx, &records_dir()?, &app)?;
            (record_at(&rec, &p)?, Some(p))
        }
    };
    let now = Timestamp::now();
    let cap = a.max_results.unwrap_or(ctx.config.traffic_max_results);
    // The cost guard first: before any request.
    let probe = TrafficQueryBuilder::between(since, now)
        .max_results(cap)
        .build()?;
    if !a.write.yes
        && let Some(reason) = probe.needs_confirmation()
    {
        return Err(CliError::Args(format!(
            "expensive query: {reason}; re-run with --yes to confirm"
        )));
    }
    let keys = keys_or_default(&ctx.config.scope_keys);
    let app = parse_app(&a.app);
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let m = app_labels(&app, &keys, &idx).map_err(CliError::Args)?;
    let q = watch_query(since, now, m, cap)?;
    let r = ctx
        .pce
        .traffic(&q, ctx.cache_mode, &CancellationToken::new(), None)
        .await?;
    let flows = resolve_flows(&r.value.flows, &idx);
    let cm = CliMarks::load(ctx, a.no_marks, false);
    let act = cm.active(false);
    let blocked = pce::enforcement::blocked_since(&flows, &app, &keys, act.as_ref());
    let mut meta = ctx.meta(
        cache_label(r.hit, ctx.cache_mode),
        r.truncated,
        Some(Window {
            start: q.start_date.clone(),
            end: q.end_date.clone(),
        }),
    );
    meta.cap = Some(q.max_results);
    meta.queries = Some(1);
    meta.incomplete = Some(r.value.incomplete);
    cm.fill(&mut meta, act.as_ref(), None);
    if r.truncated {
        meta.warnings
            .get_or_insert_with(Vec::new)
            .push("the query hit its cap: more blocked flows may exist".into());
    }
    if r.value.incomplete {
        meta.warnings
            .get_or_insert_with(Vec::new)
            .push("a region did not answer: blocked flows may be missing".into());
    }
    let out = WatchOutput {
        app: app.join("/"),
        since: since.to_string(),
        record: record.map(|p| p.display().to_string()),
        blocked,
    };
    let tuples = Gaps(&out.blocked.tuples);
    ctx.emit_report(
        &out,
        &[(
            format!(
                "blocked since {} · {} · {} flows · {} connections{}",
                out.since,
                out.app,
                out.blocked.flows,
                out.blocked.connections,
                if out.blocked.accepted > 0 {
                    format!(" · {} accepted by marks", out.blocked.accepted)
                } else {
                    String::new()
                }
            ),
            &tuples,
        )],
        &meta,
    )?;
    // None found in a partial result is not "none": exit 4.
    if out.blocked.flows == 0 && (r.truncated || r.value.incomplete) {
        return Err(CliError::Incomplete(format!(
            "no blocked flows of {} found, but the result is {}: the answer is not complete",
            out.app,
            if r.truncated {
                "truncated (cap reached)"
            } else {
                "incomplete (a region did not answer)"
            }
        )));
    }
    if out.blocked.flows > 0 {
        return Err(CliError::Blocked(format!(
            "{} flows of {} blocked since {} ({} tuples)",
            out.blocked.flows,
            out.app,
            out.since,
            out.blocked.tuples.len()
        )));
    }
    Ok(())
}

fn record_at(rec: &ChangeRecord, p: &Path) -> Result<Timestamp, CliError> {
    rec.at_time()
        .ok_or_else(|| CliError::Args(format!("{}: unreadable time {:?}", p.display(), rec.at)))
}
