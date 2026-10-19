//! `workloads enforce` and `workloads relabel`: one change to every
//! workload the list filters select. The selection is one list read (the
//! name index from the cache for labels); the plan is made locally; the
//! library's bulk executor sends it (one re-read past the cache, the label
//! creates, `bulk_update` chunks of 100, stopping after a failed chunk).
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{TableRows, WriteMeta};
use super::workloads::{WorkloadEnforceArgs, WorkloadRelabelArgs, WorkloadsArgs, select};
use super::write::{self, WriteFlags, parse_kv, similar, with_similar};
use pce::write::bulk::{BulkItem, BulkReport, BulkSkip, COUNT_ABOVE, StepStatus};
use pce::write::{
    BatchOrigin, BulkChange, BulkLabel, BulkTarget, EnforcementMode, LabelEdit, LabelRemoval,
    LabelSet, VisibilityLevel, WriteGuard, WriteRequest, plan_bulk,
};
use pce::{NameIndex, WriteMethod};
use serde::Serialize;

/// No filter and no `--all`: refused before any request, so a bare
/// command never reaches every workload by accident.
fn scope_check(select: &WorkloadsArgs, all: bool) -> Result<(), CliError> {
    if all || select.narrows() {
        return Ok(());
    }
    Err(CliError::Write(pce::write::WriteError::invalid(
        "no filter selects the workloads: pass filters (--label, --hostname, --enforcement, ...) or --all to change every workload; nothing sent",
    )))
}

pub async fn enforce(ctx: &Context, a: &WorkloadEnforceArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    scope_check(&a.select, a.all)?;
    let mode = EnforcementMode::parse(&a.mode)
        .ok_or_else(|| CliError::Args(format!("unknown enforcement mode {:?}", a.mode)))?;
    let visibility = a
        .visibility
        .as_deref()
        .map(|v| {
            VisibilityLevel::parse(v)
                .ok_or_else(|| CliError::Args(format!("unknown visibility level {v:?}")))
        })
        .transpose()?;
    let change = BulkChange::enforce(mode, visibility);
    send(
        ctx,
        &guard,
        &a.select,
        &a.write,
        change,
        a.confirm_count,
        &NameIndex::default(),
    )
    .await
}

/// `--set app=shop,env=-`, `--add-label app=shop`, `--remove-label env`
/// or `env=prod`: what to set (key, value) and what to remove.
type Specs = (Vec<(String, String)>, Vec<LabelRemoval>);

fn relabel_specs(a: &WorkloadRelabelArgs) -> Result<Specs, CliError> {
    let mut set = Vec::new();
    let mut remove = Vec::new();
    for s in a.set.iter().chain(&a.add_labels) {
        if s.trim().is_empty() {
            continue;
        }
        let (k, v) = parse_kv(s)?;
        if v == "-" {
            remove.push(LabelRemoval {
                key: k,
                value: None,
            });
        } else {
            set.push((k, v));
        }
    }
    for s in &a.remove_labels {
        remove.push(match s.split_once('=') {
            Some(_) => {
                let (k, v) = parse_kv(s)?;
                LabelRemoval {
                    key: k,
                    value: Some(v),
                }
            }
            None if s.trim().is_empty() => {
                return Err(CliError::Args("--remove-label needs a key".into()));
            }
            None => LabelRemoval {
                key: s.trim().to_string(),
                value: None,
            },
        });
    }
    if set.is_empty() && remove.is_empty() {
        return Err(CliError::Args(
            "nothing to change; pass --set key=value,key=-, --add-label or --remove-label".into(),
        ));
    }
    Ok((set, remove))
}

pub async fn relabel(ctx: &Context, a: &WorkloadRelabelArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    scope_check(&a.select, a.all)?;
    let (set, remove) = relabel_specs(a)?;
    // The shape first (a key set twice, set and removed): before any request.
    let shape = LabelEdit {
        set: set
            .iter()
            .map(|(k, v)| {
                LabelSet::Existing(BulkLabel {
                    href: String::new(),
                    key: k.clone(),
                    value: v.clone(),
                })
            })
            .collect(),
        remove: remove.clone(),
    };
    BulkChange::relabel(shape).validate()?;
    // The labels list alone (one read, cached for the selection's
    // `--label` too) resolves them and names the workloads' label keys.
    let labels = ctx.pce.labels(ctx.cache_mode).await?.value;
    let idx = NameIndex::new(&labels, &[], &[], &[]);
    let set = set
        .into_iter()
        .map(|(k, v)| match idx.label_href(&k, &v) {
            Some(href) => Ok(LabelSet::Existing(BulkLabel {
                href: href.to_string(),
                key: k,
                value: v,
            })),
            None => {
                let known = idx.values_for_key(&k);
                let msg = if known.is_empty() {
                    format!("unknown label {k}={v}; no labels have key {k}")
                } else {
                    format!(
                        "unknown label {k}={v}; known values for {k}: {}",
                        known.join(", ")
                    )
                };
                Err(CliError::Args(with_similar(
                    msg,
                    &similar(&v, known.iter().map(String::as_str)),
                )))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let change = BulkChange::relabel(LabelEdit { set, remove });
    send(ctx, &guard, &a.select, &a.write, change, None, &idx).await
}

/// `--dry-run`'s data: the plan, each workload's change and every request.
#[derive(Debug, Serialize)]
pub struct BulkDryRun {
    pub dry_run: bool,
    pub change: String,
    pub summary: String,
    /// The number of workloads that change (what `--confirm-count` takes).
    pub count: usize,
    /// `--yes` needs `--confirm-count <count>`.
    pub needs_confirm_count: bool,
    pub workloads: Vec<BulkItem>,
    pub skipped: Vec<BulkSkip>,
    /// The reads before the first write (the re-read: one list read, one
    /// GET per workload for a few, or none).
    pub reads: u32,
    /// What the re-read is (`1 list read`, `no re-read: …`).
    pub reread: String,
    pub requests: Vec<WriteRequest>,
}

pub(crate) struct PlanRows<'a>(pub &'a BulkDryRun);

impl TableRows for PlanRows<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["workload", "change", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let d = self.0;
        d.workloads
            .iter()
            .map(|i| vec![i.name.clone(), i.change.clone(), i.href.clone()])
            .chain(d.skipped.iter().map(|s| {
                vec![
                    s.name.clone(),
                    format!("skipped: {}", s.reason),
                    s.href.clone(),
                ]
            }))
            .collect()
    }
}

pub(crate) struct RequestRows<'a>(pub &'a [WriteRequest]);

impl TableRows for RequestRows<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["step", "method", "path", "summary"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let n = self.0.len();
        self.0
            .iter()
            .enumerate()
            .map(|(i, r)| {
                vec![
                    format!("{}/{n}", i + 1),
                    r.method.as_str().to_string(),
                    r.path.clone(),
                    r.summary.clone(),
                ]
            })
            .collect()
    }
    /// Each request's body, whole.
    fn notes(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|r| {
                r.body
                    .as_ref()
                    .map(|b| vec![serde_json::to_string(b).unwrap_or_default()])
                    .unwrap_or_default()
            })
            .collect()
    }
}

pub(crate) struct StepRows<'a>(pub &'a BulkReport);

impl TableRows for StepRows<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["step", "request", "status", "detail"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let n = self.0.steps.len();
        self.0
            .steps
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let status = match s.status {
                    StepStatus::Written => "written",
                    StepStatus::Partial => "partial",
                    StepStatus::Failed => "failed",
                    StepStatus::Unanswered => "not answered",
                    StepStatus::NotSent => "not sent",
                };
                let detail = s
                    .audit_failed
                    .as_ref()
                    .map(|a| format!("NOT AUDITED: {a}"))
                    .or_else(|| s.error.clone())
                    .or_else(|| s.href.clone())
                    .unwrap_or_default();
                vec![
                    format!("{}/{n}", i + 1),
                    s.summary.clone(),
                    status.to_string(),
                    detail,
                ]
            })
            .collect()
    }
}

pub(crate) struct ItemRows<'a>(pub &'a BulkReport);

impl TableRows for ItemRows<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["workload", "status", "change", "error", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .items
            .iter()
            .map(|i| {
                vec![
                    i.name.clone(),
                    i.status.as_str().to_string(),
                    i.change.clone(),
                    i.error.clone().unwrap_or_default(),
                    i.href.clone(),
                ]
            })
            .collect()
    }
}

/// Selects (one list read), plans, checks the typed count, then prints the
/// plan (`--dry-run`) or sends it and prints the report. A send that did
/// not change everything it planned exits 4 after the report.
async fn send(
    ctx: &Context,
    guard: &WriteGuard,
    args: &WorkloadsArgs,
    flags: &WriteFlags,
    change: BulkChange,
    confirm_count: Option<usize>,
    idx: &NameIndex,
) -> Result<(), CliError> {
    let selected_at = std::time::Instant::now();
    let sel = select(ctx, args, ctx.cache_mode).await?;
    let mut warnings = sel.warnings.clone();
    // A cut selection is not what the filters mean: never sent.
    if sel.truncated {
        if flags.yes {
            return Err(CliError::Args(
                "the selection read was truncated (more workloads match than were returned): narrow the filters or raise --max-results; nothing sent"
                    .into(),
            ));
        }
        warnings.push(
            "the selection read was truncated: --yes refuses until the filters or --max-results take every match"
                .into(),
        );
    }
    let targets: Vec<BulkTarget> = sel
        .rows
        .iter()
        .map(|r| BulkTarget::of(&r.workload, idx))
        .collect();
    if targets.is_empty() {
        return Err(CliError::Args(
            "no workload matches the filters: nothing to change".into(),
        ));
    }
    let origin = Some(BatchOrigin::of(guard.context()));
    let mut plan = plan_bulk(&targets, &change, sel.filter.clone(), origin)?;
    let count = plan.items.len();
    let org = ctx.config.org_id;
    // Nothing to change: the summary, exit 0, nothing sent.
    if count == 0 {
        for w in &warnings {
            eprintln!("warning: {w}");
        }
        return emit_plan(ctx, &plan, flags.dry_run, &sel, warnings, org);
    }
    check_count(&plan, flags.yes, confirm_count)?;
    plan.confirmed = confirm_count;
    // Read past the cache moments ago in this very run: no re-read.
    plan.selection_read(selected_at, sel.hit);
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    if flags.dry_run {
        return emit_plan(ctx, &plan, true, &sel, warnings, org);
    }
    let report = ctx
        .pce
        .writer(guard)
        .execute_bulk(plan, &mut |_, _| {})
        .await;
    let mut meta = ctx.meta("none", sel.truncated, None);
    meta.local_filter = sel.local_filter.clone();
    meta.write = write_meta(&report, org, guard);
    let dropped = report.dropped();
    if dropped > 0 {
        warnings.push(format!(
            "{dropped} workloads changed since the selection (or are gone): skipped, not overwritten"
        ));
    }
    meta.warnings = Some(warnings).filter(|w| !w.is_empty());
    let status = report.status_text();
    let steps = StepRows(&report);
    let items = ItemRows(&report);
    ctx.emit_report(
        &report,
        &[
            (format!("sent to {} · {status}", report.target), &steps),
            ("workloads".to_string(), &items),
        ],
        &meta,
    )?;
    if let Some(e) = exit_error(&report, status) {
        return Err(e);
    }
    Ok(())
}

/// The typed count, before any write: `--confirm-count` must match the
/// plan's count, and `--yes` needs it when the plan does (full or
/// selective on more than one workload, or more than 25).
pub(crate) fn check_count(
    plan: &pce::write::BulkPlan,
    yes: bool,
    confirm_count: Option<usize>,
) -> Result<(), CliError> {
    let count = plan.items.len();
    if let Some(n) = confirm_count
        && n != count
    {
        return Err(CliError::Args(format!(
            "--confirm-count {n} does not match: {count} workloads would change; nothing sent"
        )));
    }
    if plan.needs_count() && yes && confirm_count.is_none() {
        // Why, without the count itself: the dry run shows it.
        let why = match plan.change.enforcement {
            Some(m) if m.enforces() => format!("{} on more than one workload", m.as_str()),
            _ => format!("more than {COUNT_ABOVE} workloads"),
        };
        return Err(CliError::Args(format!(
            "{why}: pass --confirm-count with the number of workloads that change (--dry-run shows it); nothing sent"
        )));
    }
    Ok(())
}

/// `--dry-run`'s data for `plan`.
pub(crate) fn dry_run_data(plan: &pce::write::BulkPlan, dry_run: bool, org: u64) -> BulkDryRun {
    BulkDryRun {
        dry_run,
        change: plan.change.text(),
        summary: plan.summary(),
        count: plan.items.len(),
        needs_confirm_count: plan.needs_count(),
        reads: plan.reads(),
        reread: plan.reread_text(),
        requests: plan.requests(org),
        workloads: plan.items.clone(),
        skipped: plan.skipped.clone(),
    }
}

/// The meta's `write`: the last answered bulk request.
pub(crate) fn write_meta(report: &BulkReport, org: u64, guard: &WriteGuard) -> Option<WriteMeta> {
    report
        .steps
        .iter()
        .rev()
        .find(|s| s.http_status.is_some())
        .map(|last| WriteMeta {
            method: WriteMethod::Put,
            path: format!("/orgs/{org}/workloads/bulk_update"),
            status: last.http_status.unwrap_or_default(),
            audit: guard.audit_location(),
        })
}

/// The plan as `--dry-run` prints it (also a plan with nothing to change,
/// with or without `--yes`).
fn emit_plan(
    ctx: &Context,
    plan: &pce::write::BulkPlan,
    dry_run: bool,
    sel: &super::workloads::Selection,
    warnings: Vec<String>,
    org: u64,
) -> Result<(), CliError> {
    let data = dry_run_data(plan, dry_run, org);
    let mut meta = ctx.meta(cache_label(sel.hit, ctx.cache_mode), sel.truncated, None);
    meta.local_filter = sel.local_filter.clone();
    meta.warnings = Some(warnings).filter(|w| !w.is_empty());
    let summary = if plan.items.is_empty() {
        format!("{} · nothing sent", data.summary)
    } else {
        format!("{} · {}", data.summary, plan.cost_text())
    };
    let requests = RequestRows(&data.requests);
    let rows = PlanRows(&data);
    ctx.emit_report(
        &data,
        &[(summary, &rows), ("requests".to_string(), &requests)],
        &meta,
    )
}

/// The exit a sent bulk write ends with: none when it did everything (or
/// only skipped workloads the re-read found changed, some updated); else
/// the first reason's code: 2 invalid, 3 auth, 5 rate limited, 6 refused by
/// the write guard, 4 anything else (a failed request, an unknown outcome,
/// refused workloads, or nothing updated because every workload changed).
pub(crate) fn exit_error(report: &BulkReport, status: String) -> Option<CliError> {
    use pce::write::bulk::BulkFailure as F;
    let (code, kind) = match report.failure {
        Some(F::Invalid) => (2, "invalid"),
        Some(F::Refused) => (6, "write_refused"),
        Some(F::Auth) => (3, "auth"),
        Some(F::RateLimited) => (5, "rate_limited"),
        Some(F::Pce | F::Unanswered | F::Partial) => (4, "write_incomplete"),
        Some(F::AuditFailed) => (4, "audit_failed"),
        None if report.updated() == 0 && report.dropped() > 0 => (4, "write_skipped"),
        // A stopped send always names a reason; should one not, still 4.
        None if report.stopped() => (4, "write_incomplete"),
        None => return None,
    };
    Some(CliError::Bulk {
        message: status,
        code,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pce::write::bulk::{BulkFailure, ItemResult, ItemStatus};

    fn report(failure: Option<BulkFailure>, statuses: &[ItemStatus]) -> BulkReport {
        BulkReport {
            target: "t".into(),
            change: "c".into(),
            refused: None,
            reads: 1,
            reread_skipped: None,
            steps: Vec::new(),
            items: statuses
                .iter()
                .map(|s| ItemResult {
                    href: "/orgs/5/workloads/a".into(),
                    name: "a".into(),
                    change: String::new(),
                    status: *s,
                    error: None,
                })
                .collect(),
            labels_changed: false,
            labels_created: false,
            failure,
        }
    }

    fn exit(r: &BulkReport) -> Option<(i32, String)> {
        exit_error(r, "s".into()).map(|e| (e.exit_code(), e.kind()))
    }

    #[test]
    fn exits_follow_the_first_reason() {
        use ItemStatus as S;
        for (f, code, kind) in [
            (BulkFailure::Invalid, 2, "invalid"),
            (BulkFailure::Refused, 6, "write_refused"),
            (BulkFailure::Auth, 3, "auth"),
            (BulkFailure::RateLimited, 5, "rate_limited"),
            (BulkFailure::Pce, 4, "write_incomplete"),
            (BulkFailure::Unanswered, 4, "write_incomplete"),
            (BulkFailure::Partial, 4, "write_incomplete"),
            (BulkFailure::AuditFailed, 4, "audit_failed"),
        ] {
            assert_eq!(
                exit(&report(Some(f), &[S::NotSent])),
                Some((code, kind.to_string()))
            );
        }
        // Every workload changed since the selection: nothing updated.
        assert_eq!(
            exit(&report(None, &[S::Changed, S::Missing])),
            Some((4, "write_skipped".to_string()))
        );
        // Some updated, some changed: a warning only.
        assert_eq!(exit(&report(None, &[S::Updated, S::Changed])), None);
        assert_eq!(exit(&report(None, &[S::Updated])), None);
        // Backstop: stopped without a named reason is still 4.
        assert_eq!(
            exit(&report(None, &[S::Updated, S::NotSent])),
            Some((4, "write_incomplete".to_string()))
        );
    }
}
