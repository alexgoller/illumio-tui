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
use pce::write::bulk::{BulkItem, BulkReport, BulkSkip, StepStatus};
use pce::write::{
    BatchOrigin, BulkChange, BulkLabel, BulkTarget, EnforcementMode, LabelEdit, LabelRemoval,
    LabelSet, VisibilityLevel, WriteGuard, WriteRequest, plan_bulk,
};
use pce::{NameIndex, WriteMethod};
use serde::Serialize;

pub async fn enforce(ctx: &Context, a: &WorkloadEnforceArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
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
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
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
    /// The list read before the first write (the re-read).
    pub reads: u32,
    pub requests: Vec<WriteRequest>,
}

struct PlanRows<'a>(&'a BulkDryRun);

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

struct RequestRows<'a>(&'a [WriteRequest]);

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

struct StepRows<'a>(&'a BulkReport);

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

struct ItemRows<'a>(&'a BulkReport);

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
    if let Some(n) = confirm_count
        && n != count
    {
        return Err(CliError::Args(format!(
            "--confirm-count {n} does not match: {count} workloads would change; nothing sent"
        )));
    }
    if plan.needs_count() && flags.yes && confirm_count.is_none() {
        return Err(CliError::Args(format!(
            "{} on more than one workload: pass --confirm-count with the number of workloads that change (--dry-run shows it); nothing sent",
            change
                .enforcement
                .map_or("enforcement", EnforcementMode::as_str)
        )));
    }
    plan.confirmed = confirm_count;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let org = ctx.config.org_id;
    if flags.dry_run {
        let data = BulkDryRun {
            dry_run: true,
            change: change.text(),
            summary: plan.summary(),
            count,
            needs_confirm_count: plan.needs_count(),
            reads: plan.reads(),
            requests: plan.requests(org),
            workloads: plan.items.clone(),
            skipped: plan.skipped.clone(),
        };
        let mut meta = ctx.meta(cache_label(sel.hit, ctx.cache_mode), sel.truncated, None);
        meta.local_filter = sel.local_filter.clone();
        meta.warnings = Some(warnings).filter(|w| !w.is_empty());
        let summary = format!("{} · {}", data.summary, plan.cost_text());
        let requests = RequestRows(&data.requests);
        let rows = PlanRows(&data);
        return ctx.emit_report(
            &data,
            &[(summary, &rows), ("requests".to_string(), &requests)],
            &meta,
        );
    }
    let report = ctx
        .pce
        .writer(guard)
        .execute_bulk(plan, &mut |_, _| {})
        .await;
    let mut meta = ctx.meta("none", sel.truncated, None);
    meta.local_filter = sel.local_filter.clone();
    if let Some(last) = report.steps.iter().rev().find(|s| s.http_status.is_some()) {
        meta.write = Some(WriteMeta {
            method: WriteMethod::Put,
            path: format!("/orgs/{org}/workloads/bulk_update"),
            status: last.http_status.unwrap_or_default(),
            audit: guard.audit_location(),
        });
    }
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
    if report.stopped() {
        return Err(CliError::Partial(status));
    }
    Ok(())
}
