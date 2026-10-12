//! What every write command shares: the guard (profile `writes` + audit
//! log), the --dry-run / --yes contract, exact name resolution, and
//! the envelope data. Commands build a `Plan` with the library and hand it
//! to `run_plan`.
use super::context::Context;
use super::error::CliError;
use super::output::{TableRows, WriteMeta};
use crate::tokens::with_exclusion;
pub use crate::tokens::{ActorToken, ServiceToken, is_href};
pub(crate) use crate::tokens::{similar, with_similar};
use clap::Args;
use pce::Impact;
use pce::NameIndex;
use pce::models::{IpList, Label, LabelGroup, Service};
use pce::write::provision_counts;
use pce::write::{
    ActorKind, ActorRef, JsonlAudit, ObjectKind, Outcome, Plan, ServiceRef, WriteContext,
    WriteError, WriteGuard, WriteRefused, WriteRequest, Written, checked_href, checked_policy_href,
};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Args, Debug, Clone, Default)]
pub struct WriteFlags {
    /// Print the request that would be sent, send nothing
    #[arg(long, conflicts_with = "yes")]
    pub dry_run: bool,
    /// Send the request (required unless --dry-run)
    #[arg(long)]
    pub yes: bool,
}

pub fn check_flags(flags: &WriteFlags) -> Result<(), CliError> {
    if !flags.dry_run && !flags.yes {
        return Err(CliError::Args(
            "this command changes the PCE; re-run with --yes to send it, or --dry-run to see the request"
                .into(),
        ));
    }
    Ok(())
}

fn write_guard(ctx: &Context) -> Result<WriteGuard, CliError> {
    WriteGuard::check(ctx.config.writes).map_err(WriteError::from)?;
    let path = JsonlAudit::default_path().ok_or_else(|| {
        WriteError::from(WriteRefused::AuditUnwritable(
            "no $XDG_STATE_HOME or $HOME to place the audit log".into(),
        ))
    })?;
    let audit = JsonlAudit::new(path);
    let guard = WriteGuard::new(
        ctx.config.writes,
        Box::new(audit),
        WriteContext {
            profile: ctx.config.profile_name.clone(),
            host: ctx.config.host.clone(),
            org_id: ctx.config.org_id,
            source: "cli".into(),
        },
    )
    .map_err(WriteError::from)?;
    Ok(guard)
}

/// Guard first, then the flag check, before any request: an agent learns
/// about a profile without `writes = true` (exit 6) before a missing `--yes`
/// (exit 2).
pub fn prepare(ctx: &Context, flags: &WriteFlags) -> Result<WriteGuard, CliError> {
    let guard = write_guard(ctx)?;
    check_flags(flags)?;
    Ok(guard)
}

/// The `data` of a write command's envelope.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum WriteOutput {
    DryRun(WriteRequest),
    Written(Written),
    Skipped {
        skipped: bool,
        href: String,
        reason: String,
    },
    /// `policy provision --dry-run` and `policy revert --dry-run`: the
    /// request and its object counts.
    ProvisionDryRun {
        #[serde(flatten)]
        request: WriteRequest,
        counts: BTreeMap<String, usize>,
    },
    /// `policy provision --yes`: the new policy version and the counts.
    Provisioned {
        #[serde(flatten)]
        written: Written,
        counts: BTreeMap<String, usize>,
    },
    /// `policy revert --yes`: the PCE's answer and the counts.
    Reverted {
        #[serde(flatten)]
        written: Written,
        counts: BTreeMap<String, usize>,
    },
    /// A rule or deny-rule dry run with `--impact`: the request and the
    /// impact of what it would send.
    DryRunImpact {
        #[serde(flatten)]
        request: WriteRequest,
        impact: Impact,
    },
    /// A rule or deny-rule write sent with `--impact`: the result and the
    /// impact computed before the send.
    WrittenImpact {
        #[serde(flatten)]
        written: Written,
        impact: Impact,
    },
}

fn compact(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

fn row(k: &str, v: String) -> Vec<String> {
    vec![k.to_string(), v]
}

fn request_rows(r: &WriteRequest) -> Vec<Vec<String>> {
    vec![
        row("method", r.method.as_str().into()),
        row("path", r.path.clone()),
        row("summary", r.summary.clone()),
        row("body", r.body.as_ref().map(compact).unwrap_or_default()),
    ]
}

fn written_rows(w: &Written) -> Vec<Vec<String>> {
    vec![
        row("href", w.href.clone()),
        row("pending", w.pending.to_string()),
        row("status", w.status.to_string()),
        row("dropped_fields", w.dropped_fields.join(", ")),
        row("warnings", w.warnings.join("; ")),
        row("object", compact(&w.object)),
    ]
}

/// `ip_lists=1, rule_sets=3`.
fn counts_text(counts: &BTreeMap<String, usize>) -> String {
    counts
        .iter()
        .map(|(k, n)| format!("{k}={n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `impact` and one `ip list` row per IP-list actor.
fn impact_rows(i: &Impact) -> Vec<Vec<String>> {
    let mut rows = vec![row("impact", i.summary())];
    rows.extend(i.ip_lists.iter().map(|l| row("ip list", l.line())));
    rows
}

impl TableRows for WriteOutput {
    fn headers(&self) -> Vec<&'static str> {
        vec!["field", "value"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        match self {
            WriteOutput::DryRun(r) => request_rows(r),
            WriteOutput::Written(w) => written_rows(w),
            WriteOutput::Skipped {
                skipped,
                href,
                reason,
            } => vec![
                row("skipped", skipped.to_string()),
                row("href", href.clone()),
                row("reason", reason.clone()),
            ],
            WriteOutput::ProvisionDryRun { request, counts } => {
                let mut rows = request_rows(request);
                rows.push(row("counts", counts_text(counts)));
                rows
            }
            WriteOutput::Provisioned { written, counts } => {
                let mut rows = vec![
                    row("provisioned", written.href.clone()),
                    row("counts", counts_text(counts)),
                ];
                rows.extend(written_rows(written));
                rows
            }
            WriteOutput::Reverted { written, counts } => {
                let mut rows = vec![row("reverted", counts_text(counts))];
                rows.extend(written_rows(written));
                rows
            }
            WriteOutput::DryRunImpact { request, impact } => {
                let mut rows = request_rows(request);
                rows.extend(impact_rows(impact));
                rows
            }
            WriteOutput::WrittenImpact { written, impact } => {
                let mut rows = written_rows(written);
                rows.extend(impact_rows(impact));
                rows
            }
        }
    }
}

/// What a write's output carries besides the request or the result.
enum Extra {
    None,
    /// `policy provision` and `policy revert`: object counts from
    /// `change_subset`.
    Counts,
    /// `--impact` on a rule write, and whether its reads were truncated.
    Impact(Impact, bool),
}

/// Dry run prints the request; otherwise the plan is executed once and the
/// result printed with `meta.write`.
pub async fn run_plan(
    ctx: &Context,
    flags: &WriteFlags,
    guard: &WriteGuard,
    plan: Plan,
) -> Result<(), CliError> {
    send(ctx, flags, guard, plan, Extra::None).await
}

/// [`run_plan`] for `policy provision` and `policy revert`: the dry run
/// and the result carry `counts` (object type to number of hrefs in
/// `change_subset`).
pub async fn run_provision_plan(
    ctx: &Context,
    flags: &WriteFlags,
    guard: &WriteGuard,
    plan: Plan,
) -> Result<(), CliError> {
    send(ctx, flags, guard, plan, Extra::Counts).await
}

/// [`run_plan`] for a rule or deny-rule write: with an `impact` (from
/// `--impact`, computed before anything is sent) the dry run and the result
/// carry it, and `meta.truncated` reports a truncated read behind it.
pub async fn run_rule_plan(
    ctx: &Context,
    flags: &WriteFlags,
    guard: &WriteGuard,
    plan: Plan,
    impact: Option<(Impact, bool)>,
) -> Result<(), CliError> {
    let extra = match impact {
        Some((impact, truncated)) => Extra::Impact(impact, truncated),
        None => Extra::None,
    };
    send(ctx, flags, guard, plan, extra).await
}

async fn send(
    ctx: &Context,
    flags: &WriteFlags,
    guard: &WriteGuard,
    plan: Plan,
    extra: Extra,
) -> Result<(), CliError> {
    let truncated = matches!(extra, Extra::Impact(_, true));
    let (request, kind) = match plan {
        Plan::Noop {
            existing_href,
            reason,
        } => {
            return ctx.emit_write_with(
                &WriteOutput::Skipped {
                    skipped: true,
                    href: existing_href,
                    reason,
                },
                None,
                truncated,
            );
        }
        Plan::Send { request, kind } => (request, kind),
    };
    let counts = matches!(extra, Extra::Counts)
        .then(|| provision_counts(request.body.as_ref().unwrap_or(&serde_json::Value::Null)));
    if flags.dry_run {
        let out = match (extra, counts) {
            (Extra::Impact(impact, _), _) => WriteOutput::DryRunImpact { request, impact },
            (_, Some(counts)) => WriteOutput::ProvisionDryRun { request, counts },
            _ => WriteOutput::DryRun(request),
        };
        return ctx.emit_write_with(&out, None, truncated);
    }
    let method = request.method;
    let path = request.path.clone();
    match ctx
        .pce
        .writer(guard)
        .execute(Plan::Send { request, kind })
        .await?
    {
        Outcome::Written(w) => {
            let meta = WriteMeta {
                method,
                path,
                status: w.status,
                audit: guard.audit_location(),
            };
            let out = match (extra, counts) {
                (Extra::Impact(impact, _), _) => WriteOutput::WrittenImpact { written: w, impact },
                (_, Some(counts)) if kind == ObjectKind::Revert => {
                    WriteOutput::Reverted { written: w, counts }
                }
                (_, Some(counts)) => WriteOutput::Provisioned { written: w, counts },
                _ => WriteOutput::Written(w),
            };
            ctx.emit_write_with(&out, Some(meta), truncated)
        }
        Outcome::Skipped {
            existing_href,
            reason,
        } => ctx.emit_write_with(
            &WriteOutput::Skipped {
                skipped: true,
                href: existing_href,
                reason,
            },
            None,
            truncated,
        ),
    }
}

/// `key=value`, split on the first `=`; both sides trimmed and non-empty.
pub fn parse_kv(spec: &str) -> Result<(String, String), CliError> {
    crate::tokens::split_kv(spec).map_err(CliError::Args)
}

/// An href as given, or `key=value` matched exactly against `labels`
/// (deleted labels excluded).
pub fn label_in(labels: &[Label], spec: &str) -> Result<String, CliError> {
    if is_href(spec) {
        return Ok(spec.trim().to_string());
    }
    let (k, v) = parse_kv(spec)?;
    let live = || labels.iter().filter(|l| !l.is_deleted() && l.key == k);
    if let Some(l) = live().find(|l| l.value == v) {
        return Ok(l.href.clone());
    }
    let mut known: Vec<&str> = live().map(|l| l.value.as_str()).collect();
    known.sort_unstable();
    known.dedup();
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
        &similar(&v, known.into_iter()),
    )))
}

/// Resolves every spec against one labels list, loaded (one GET) only when
/// a spec is not an href.
pub async fn resolve_labels(ctx: &Context, specs: &[String]) -> Result<Vec<String>, CliError> {
    let labels = if specs.iter().all(|s| is_href(s)) {
        Vec::new()
    } else {
        ctx.pce.labels(ctx.cache_mode).await?.value
    };
    specs.iter().map(|s| label_in(&labels, s)).collect()
}

pub async fn resolve_label(ctx: &Context, spec: &str) -> Result<String, CliError> {
    let mut hrefs = resolve_labels(ctx, &[spec.to_string()]).await?;
    Ok(hrefs.remove(0))
}

/// Exactly one live object named `name` (case-sensitive); draft objects
/// pending deletion do not count. None or several is an args error listing
/// candidates.
fn pick_one<'a>(
    kind: &str,
    name: &str,
    live: impl Iterator<Item = (&'a str, &'a str)> + Clone,
) -> Result<String, CliError> {
    let mut hrefs: Vec<&str> = live
        .clone()
        .filter(|(n, _)| *n == name)
        .map(|(_, h)| h)
        .collect();
    hrefs.sort_unstable();
    match hrefs.len() {
        0 => Err(CliError::Args(with_similar(
            format!("no {kind} named {name:?}"),
            &similar(name, live.map(|(n, _)| n)),
        ))),
        1 => Ok(hrefs[0].to_string()),
        _ => Err(CliError::Args(format!(
            "{name:?} names several {kind}s: {}; use the href",
            hrefs.join(", ")
        ))),
    }
}

fn not_pending_delete(update_type: Option<&str>) -> bool {
    update_type != Some("delete")
}

fn ip_list_in(lists: &[IpList], name: &str) -> Result<String, CliError> {
    pick_one(
        "IP list",
        name.trim(),
        lists
            .iter()
            .filter(|l| not_pending_delete(l.update_type.as_deref()))
            .map(|l| (l.name.as_str(), l.href.as_str())),
    )
}

fn service_in(services: &[Service], name: &str) -> Result<String, CliError> {
    pick_one(
        "service",
        name.trim(),
        services
            .iter()
            .filter(|s| not_pending_delete(s.update_type.as_deref()))
            .map(|s| (s.name.as_str(), s.href.as_str())),
    )
}

fn label_group_in(groups: &[LabelGroup], name: &str) -> Result<String, CliError> {
    pick_one(
        "label group",
        name.trim(),
        groups
            .iter()
            .filter(|g| not_pending_delete(g.update_type.as_deref()))
            .map(|g| (g.name.as_str(), g.href.as_str())),
    )
}

/// Legacy: passes hrefs through unchecked; the token resolver
/// (`resolve_actor`) is the checked path.
pub async fn resolve_ip_list(ctx: &Context, name: &str) -> Result<String, CliError> {
    if is_href(name) {
        return Ok(name.trim().to_string());
    }
    ip_list_in(&ctx.pce.ip_lists(ctx.cache_mode).await?.value, name)
}

/// Legacy: passes hrefs through unchecked; the token resolver
/// (`resolve_service_ref`) is the checked path.
pub async fn resolve_service(ctx: &Context, name: &str) -> Result<String, CliError> {
    if is_href(name) {
        return Ok(name.trim().to_string());
    }
    service_in(&ctx.pce.services(ctx.cache_mode).await?.value, name)
}

/// An href as given, or an exact hostname/name match through
/// `Writer::find_workload` (one or two reads of at most 500 rows).
pub async fn resolve_workload(
    ctx: &Context,
    guard: &WriteGuard,
    target: &str,
) -> Result<String, CliError> {
    if is_href(target) {
        return Ok(checked_href(target, "workloads", ctx.config.org_id)?);
    }
    Ok(ctx.pce.writer(guard).find_workload(target).await?.href)
}

pub fn parse_actors(specs: &[String]) -> Result<Vec<ActorToken>, CliError> {
    specs
        .iter()
        .map(|s| ActorToken::parse(s).map_err(CliError::Args))
        .collect()
}

pub fn parse_services(specs: &[String]) -> Result<Vec<ServiceToken>, CliError> {
    specs
        .iter()
        .map(|s| ServiceToken::parse(s).map_err(CliError::Args))
        .collect()
}

/// Every `--scope` value: comma-separated labels and label groups.
pub fn parse_scopes(specs: &[String]) -> Result<Vec<Vec<ActorToken>>, CliError> {
    specs
        .iter()
        .map(|spec| {
            let scope = crate::tokens::parse_scope(spec).map_err(CliError::Args)?;
            if scope.is_empty() {
                return Err(CliError::Args(
                    "--scope needs at least one label or label group; leave --scope out for a ruleset that applies everywhere"
                        .into(),
                ));
            }
            Ok(scope)
        })
        .collect()
}

/// The object lists one invocation resolves names against, each fetched
/// at most once and only when a token needs it. Holds the guard so
/// `workload:HOST` can use `Writer::find_workload`.
pub struct Lists<'a> {
    guard: &'a WriteGuard,
    labels: Option<Vec<Label>>,
    label_groups: Option<Vec<LabelGroup>>,
    ip_lists: Option<Vec<IpList>>,
    services: Option<Vec<Service>>,
}

impl<'a> Lists<'a> {
    pub fn new(guard: &'a WriteGuard) -> Self {
        Self {
            guard,
            labels: None,
            label_groups: None,
            ip_lists: None,
            services: None,
        }
    }

    async fn labels(&mut self, ctx: &Context) -> Result<&[Label], CliError> {
        if self.labels.is_none() {
            self.labels = Some(ctx.pce.labels(ctx.cache_mode).await?.value);
        }
        Ok(self.labels.as_deref().unwrap_or_default())
    }

    async fn label_groups(&mut self, ctx: &Context) -> Result<&[LabelGroup], CliError> {
        if self.label_groups.is_none() {
            self.label_groups = Some(ctx.pce.label_groups(ctx.cache_mode).await?.value);
        }
        Ok(self.label_groups.as_deref().unwrap_or_default())
    }

    async fn ip_lists(&mut self, ctx: &Context) -> Result<&[IpList], CliError> {
        if self.ip_lists.is_none() {
            self.ip_lists = Some(ctx.pce.ip_lists(ctx.cache_mode).await?.value);
        }
        Ok(self.ip_lists.as_deref().unwrap_or_default())
    }

    async fn services(&mut self, ctx: &Context) -> Result<&[Service], CliError> {
        if self.services.is_none() {
            self.services = Some(ctx.pce.services(ctx.cache_mode).await?.value);
        }
        Ok(self.services.as_deref().unwrap_or_default())
    }
}

/// A label group's href: an href checked without a request, or an exact
/// name from the one label-groups list.
pub async fn resolve_label_group(
    ctx: &Context,
    name: &str,
    lists: &mut Lists<'_>,
) -> Result<String, CliError> {
    if is_href(name) {
        return Ok(checked_policy_href(
            name,
            "label_groups",
            ctx.config.org_id,
        )?);
    }
    label_group_in(lists.label_groups(ctx).await?, name)
}

/// [`crate::tokens::actor_from_href_token`] with the CLI's error type.
fn actor_from_href_token(token: &ActorToken, org_id: u64) -> Option<Result<ActorRef, CliError>> {
    crate::tokens::actor_from_href_token(token, org_id).map(|r| r.map_err(CliError::from))
}

/// [`crate::tokens::service_from_href_token`] with the CLI's error type.
fn service_from_href_token(
    token: &ServiceToken,
    org_id: u64,
) -> Option<Result<ServiceRef, CliError>> {
    crate::tokens::service_from_href_token(token, org_id).map(|r| r.map_err(CliError::from))
}

pub async fn resolve_actor(
    ctx: &Context,
    token: &ActorToken,
    lists: &mut Lists<'_>,
) -> Result<ActorRef, CliError> {
    if let Some(done) = actor_from_href_token(token, ctx.config.org_id) {
        let actor = done?;
        actor.validate().map_err(CliError::Args)?;
        return Ok(actor);
    }
    let actor = match token {
        ActorToken::All => ActorRef::all(),
        ActorToken::Href { href, exclusion } => {
            with_exclusion(ActorRef::from_href(href, ctx.config.org_id)?, *exclusion)
        }
        ActorToken::Label { spec, exclusion } => {
            let href = label_in(lists.labels(ctx).await?, spec)?;
            with_exclusion(ActorRef::new(ActorKind::Label, href), *exclusion)
        }
        ActorToken::LabelGroup { name, exclusion } => {
            let href = resolve_label_group(ctx, name, lists).await?;
            with_exclusion(ActorRef::new(ActorKind::LabelGroup, href), *exclusion)
        }
        ActorToken::IpList { name, exclusion } => {
            let href = ip_list_in(lists.ip_lists(ctx).await?, name)?;
            with_exclusion(ActorRef::new(ActorKind::IpList, href), *exclusion)
        }
        ActorToken::Workload { name } => {
            let guard = lists.guard;
            ActorRef::new(
                ActorKind::Workload,
                resolve_workload(ctx, guard, name).await?,
            )
        }
    };
    actor.validate().map_err(CliError::Args)?;
    Ok(actor)
}

pub async fn resolve_actors(
    ctx: &Context,
    tokens: &[ActorToken],
    lists: &mut Lists<'_>,
) -> Result<Vec<ActorRef>, CliError> {
    let mut out = Vec::with_capacity(tokens.len());
    for t in tokens {
        out.push(resolve_actor(ctx, t, lists).await?);
    }
    Ok(out)
}

pub async fn resolve_scopes(
    ctx: &Context,
    scopes: &[Vec<ActorToken>],
    lists: &mut Lists<'_>,
) -> Result<Vec<Vec<ActorRef>>, CliError> {
    let mut out = Vec::with_capacity(scopes.len());
    for scope in scopes {
        out.push(resolve_actors(ctx, scope, lists).await?);
    }
    Ok(out)
}

pub async fn resolve_service_ref(
    ctx: &Context,
    token: &ServiceToken,
    lists: &mut Lists<'_>,
) -> Result<ServiceRef, CliError> {
    if let Some(done) = service_from_href_token(token, ctx.config.org_id) {
        return done;
    }
    match token {
        ServiceToken::Href(h) => Ok(ServiceRef::from_href(h, ctx.config.org_id)?),
        ServiceToken::Port(p) if matches!(p.proto, 1 | 58) => {
            let idx = NameIndex::new(&[], &[], &[], lists.services(ctx).await?);
            crate::tokens::icmp_service_ref(p, &idx)
                .expect("icmp is 1 or 58")
                .map_err(|e| CliError::from(WriteError::Invalid(vec![e])))
        }
        ServiceToken::Port(p) => Ok(ServiceRef::Port(p.clone())),
        ServiceToken::Name(n) => Ok(ServiceRef::Href(service_in(lists.services(ctx).await?, n)?)),
    }
}

pub async fn resolve_services(
    ctx: &Context,
    tokens: &[ServiceToken],
    lists: &mut Lists<'_>,
) -> Result<Vec<ServiceRef>, CliError> {
    let mut out = Vec::with_capacity(tokens.len());
    for t in tokens {
        out.push(resolve_service_ref(ctx, t, lists).await?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_kv_splits_on_the_first_equals() {
        assert_eq!(parse_kv("app=a=b").unwrap(), ("app".into(), "a=b".into()));
        assert_eq!(
            parse_kv(" env = prod ").unwrap(),
            ("env".into(), "prod".into())
        );
        assert!(parse_kv("app").is_err());
        assert!(parse_kv("=x").is_err());
        assert!(parse_kv("app=").is_err());
        assert!(is_href("/orgs/5/labels/1"));
        assert!(!is_href("app=web"));
    }

    #[test]
    fn zero_matches_list_similar_names() {
        let live = [
            ("Corp-DMZ", "/a"),
            ("corp", "/b"),
            ("Other", "/c"),
            ("Corp", "/d"),
        ];
        let err = pick_one("IP list", "Corporate", live.iter().copied()).unwrap_err();
        assert_eq!(err.to_string(), r#"no IP list named "Corporate""#);
        let err = pick_one("IP list", "CORP-", live.iter().copied()).unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"no IP list named "CORP-"; similar: Corp-DMZ"#
        );
        let err = pick_one("IP list", "Cor", live.iter().copied()).unwrap_err();
        assert!(
            err.to_string().ends_with("similar: Corp, Corp-DMZ, corp"),
            "{err}"
        );
        assert_eq!(
            pick_one("IP list", "Corp", live.iter().copied()).unwrap(),
            "/d"
        );
        let many: Vec<(String, String)> = (0..8)
            .map(|i| (format!("web{i}"), format!("/{i}")))
            .collect();
        let err = pick_one(
            "service",
            "we",
            many.iter().map(|(n, h)| (n.as_str(), h.as_str())),
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .ends_with("similar: web0, web1, web2, web3, web4"),
            "at most five: {err}"
        );
    }

    #[test]
    fn write_output_renders_key_value_rows() {
        let out = WriteOutput::DryRun(WriteRequest {
            method: pce::WriteMethod::Post,
            path: "/orgs/5/labels".into(),
            body: Some(json!({"key": "app"})),
            summary: "create label app=web".into(),
            objects: Vec::new(),
        });
        assert_eq!(out.headers(), vec!["field", "value"]);
        let rows = out.rows();
        assert_eq!(rows[0], vec!["method", "POST"]);
        assert_eq!(rows[1], vec!["path", "/orgs/5/labels"]);
        assert_eq!(rows[2], vec!["summary", "create label app=web"]);
        assert_eq!(rows[3], vec!["body", r#"{"key":"app"}"#]);
        let out = WriteOutput::Skipped {
            skipped: true,
            href: "/x".into(),
            reason: "exists".into(),
        };
        assert_eq!(out.rows()[0], vec!["skipped", "true"]);
        let json = serde_json::to_value(&out).unwrap();
        assert_eq!(
            json,
            json!({"skipped": true, "href": "/x", "reason": "exists"})
        );
    }

    #[test]
    fn write_output_renders_written_rows() {
        let out = WriteOutput::Written(Written {
            href: "/orgs/5/labels/9".into(),
            pending: false,
            status: 201,
            object: json!({"key": "app"}),
            dropped_fields: vec![],
            warnings: vec!["w1".into()],
        });
        let rows = out.rows();
        assert_eq!(rows[0], vec!["href", "/orgs/5/labels/9"]);
        assert_eq!(rows[1], vec!["pending", "false"]);
        assert_eq!(rows[2], vec!["status", "201"]);
        assert_eq!(rows[3], vec!["dropped_fields", ""]);
        assert_eq!(rows[4], vec!["warnings", "w1"]);
        assert_eq!(rows[5], vec!["object", r#"{"key":"app"}"#]);
    }

    #[test]
    fn missing_yes_is_an_args_error_unless_dry_run() {
        assert!(
            check_flags(&WriteFlags {
                dry_run: false,
                yes: false
            })
            .is_err()
        );
        assert!(
            check_flags(&WriteFlags {
                dry_run: true,
                yes: false
            })
            .is_ok()
        );
        assert!(
            check_flags(&WriteFlags {
                dry_run: false,
                yes: true
            })
            .is_ok()
        );
    }

    #[test]
    fn scopes_take_labels_and_groups_only() {
        let scopes = parse_scopes(&["app=web, group:G".into(), "env=prod".into()]).unwrap();
        assert_eq!(scopes.len(), 2);
        assert_eq!(scopes[0].len(), 2);
        assert_eq!(
            parse_scopes(&["iplist:Corp".into()])
                .unwrap_err()
                .to_string(),
            "scopes take labels and label groups only"
        );
        assert!(
            parse_scopes(&[" , ".into()])
                .unwrap_err()
                .to_string()
                .contains("--scope needs at least one label or label group")
        );
        assert!(parse_actors(&["all".into(), "web".into()]).is_err());
        assert_eq!(
            parse_services(&["443/tcp".into(), "HTTPS".into()])
                .unwrap()
                .len(),
            2
        );
    }
}
