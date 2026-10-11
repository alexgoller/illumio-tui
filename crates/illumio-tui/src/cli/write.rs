//! What every write command shares: the guard (profile + --allow-writes +
//! audit log), the --dry-run / --yes contract, exact name resolution, and
//! the envelope data. Commands build a `Plan` with the library and hand it
//! to `run_plan`.
use super::context::Context;
use super::error::CliError;
use super::output::{TableRows, WriteMeta};
use clap::Args;
use pce::models::{IpList, Label, LabelGroup, Service};
use pce::write::{
    ActorKind, ActorRef, JsonlAudit, Outcome, Plan, PortSpec, ServiceRef, WriteContext, WriteError,
    WriteGuard, WriteRefused, WriteRequest, Written, checked_href, checked_policy_href,
};
use serde::Serialize;

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
    WriteGuard::check(ctx.config.writes, ctx.allow_writes).map_err(WriteError::from)?;
    let path = JsonlAudit::default_path().ok_or_else(|| {
        WriteError::from(WriteRefused::AuditUnwritable(
            "no $XDG_STATE_HOME or $HOME to place the audit log".into(),
        ))
    })?;
    let audit = JsonlAudit::new(path);
    let guard = WriteGuard::new(
        ctx.config.writes,
        ctx.allow_writes,
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
/// about a missing `--allow-writes` (exit 6) before a missing `--yes` (exit 2).
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
}

fn compact(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

impl TableRows for WriteOutput {
    fn headers(&self) -> Vec<&'static str> {
        vec!["field", "value"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let row = |k: &str, v: String| vec![k.to_string(), v];
        match self {
            WriteOutput::DryRun(r) => vec![
                row("method", r.method.as_str().into()),
                row("path", r.path.clone()),
                row("summary", r.summary.clone()),
                row("body", r.body.as_ref().map(compact).unwrap_or_default()),
            ],
            WriteOutput::Written(w) => vec![
                row("href", w.href.clone()),
                row("pending", w.pending.to_string()),
                row("status", w.status.to_string()),
                row("dropped_fields", w.dropped_fields.join(", ")),
                row("warnings", w.warnings.join("; ")),
                row("object", compact(&w.object)),
            ],
            WriteOutput::Skipped {
                skipped,
                href,
                reason,
            } => vec![
                row("skipped", skipped.to_string()),
                row("href", href.clone()),
                row("reason", reason.clone()),
            ],
        }
    }
}

/// Dry run prints the request; otherwise the plan is executed once and the
/// result printed with `meta.write`.
pub async fn run_plan(
    ctx: &Context,
    flags: &WriteFlags,
    guard: &WriteGuard,
    plan: Plan,
) -> Result<(), CliError> {
    let (request, kind) = match plan {
        Plan::Noop {
            existing_href,
            reason,
        } => {
            return ctx.emit_write(
                &WriteOutput::Skipped {
                    skipped: true,
                    href: existing_href,
                    reason,
                },
                None,
            );
        }
        Plan::Send { request, kind } => (request, kind),
    };
    if flags.dry_run {
        return ctx.emit_write(&WriteOutput::DryRun(request), None);
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
            ctx.emit_write(&WriteOutput::Written(w), Some(meta))
        }
        Outcome::Skipped {
            existing_href,
            reason,
        } => ctx.emit_write(
            &WriteOutput::Skipped {
                skipped: true,
                href: existing_href,
                reason,
            },
            None,
        ),
    }
}

/// `key=value`, split on the first `=`; both sides trimmed and non-empty.
pub fn parse_kv(spec: &str) -> Result<(String, String), CliError> {
    let (k, v) = spec
        .split_once('=')
        .ok_or_else(|| CliError::Args(format!("expected key=value, got {spec:?}")))?;
    let (k, v) = (k.trim(), v.trim());
    if k.is_empty() || v.is_empty() {
        return Err(CliError::Args(format!(
            "expected key=value with both sides non-empty, got {spec:?}"
        )));
    }
    Ok((k.to_string(), v.to_string()))
}

pub fn is_href(s: &str) -> bool {
    s.trim_start().starts_with("/orgs/")
}

/// Up to five names that contain `needle` case-insensitively, for an
/// error that found no exact match.
pub(crate) fn similar<'a>(needle: &str, names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let needle = needle.to_lowercase();
    let mut out: Vec<String> = names
        .filter(|n| n.to_lowercase().contains(&needle))
        .map(str::to_string)
        .collect();
    out.sort();
    out.dedup();
    out.truncate(5);
    out
}

pub(crate) fn with_similar(msg: String, similar: &[String]) -> String {
    if similar.is_empty() {
        msg
    } else {
        format!("{msg}; similar: {}", similar.join(", "))
    }
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
/// (`resolve_actor`) is the checked path (Task 8 migrates the commands).
pub async fn resolve_ip_list(ctx: &Context, name: &str) -> Result<String, CliError> {
    if is_href(name) {
        return Ok(name.trim().to_string());
    }
    ip_list_in(&ctx.pce.ip_lists(ctx.cache_mode).await?.value, name)
}

/// Legacy: passes hrefs through unchecked; the token resolver
/// (`resolve_service_ref`) is the checked path (Task 8 migrates the commands).
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

const ACTOR_GRAMMAR: &str = "use key=value, label:key=value, group:NAME, iplist:NAME, workload:HOST, all, an href, or ! before a label, label group or IP list";

/// One `--provider`, `--consumer` or `--scope` token, parsed without any
/// request (grammar in the README's Writes section).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorToken {
    All,
    /// `key=value`, already split and re-joined on the first `=`.
    Label {
        spec: String,
        exclusion: bool,
    },
    LabelGroup {
        name: String,
        exclusion: bool,
    },
    IpList {
        name: String,
        exclusion: bool,
    },
    Workload {
        name: String,
    },
    Href {
        href: String,
        exclusion: bool,
    },
}

impl ActorToken {
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        let (exclusion, rest) = match t.strip_prefix('!') {
            Some(r) => (true, r.trim()),
            None => (false, t),
        };
        if rest.is_empty() {
            return Err(format!("{s:?} is empty; {ACTOR_GRAMMAR}"));
        }
        let named = |prefix: &str| -> Option<Result<String, String>> {
            rest.strip_prefix(prefix).map(|n| {
                let n = n.trim();
                if n.is_empty() {
                    Err(format!("{s:?}: missing name after {prefix}"))
                } else {
                    Ok(n.to_string())
                }
            })
        };
        if rest.eq_ignore_ascii_case("all") {
            return if exclusion {
                Err(format!("{s:?}: all cannot be excluded"))
            } else {
                Ok(Self::All)
            };
        }
        if is_href(rest) {
            return Ok(Self::Href {
                href: rest.to_string(),
                exclusion,
            });
        }
        if let Some(name) = named("group:") {
            return Ok(Self::LabelGroup {
                name: name?,
                exclusion,
            });
        }
        if let Some(name) = named("iplist:") {
            return Ok(Self::IpList {
                name: name?,
                exclusion,
            });
        }
        if let Some(name) = named("workload:") {
            if exclusion {
                return Err(format!(
                    "{s:?}: only labels, label groups and IP lists can be excluded"
                ));
            }
            return Ok(Self::Workload { name: name? });
        }
        let spec = rest.strip_prefix("label:").unwrap_or(rest);
        if spec != rest && is_href(spec) {
            // `label:<href>` is checked as a label href at resolution.
            return Ok(Self::Label {
                spec: spec.to_string(),
                exclusion,
            });
        }
        if spec.contains('=') {
            let (k, v) = parse_kv(spec).map_err(|e| e.to_string())?;
            return Ok(Self::Label {
                spec: format!("{k}={v}"),
                exclusion,
            });
        }
        Err(format!("{s:?} is not an actor; {ACTOR_GRAMMAR}"))
    }

    /// Scopes take labels and label groups; an href's kind is checked by
    /// the library without a request.
    fn scope_member(&self) -> bool {
        matches!(
            self,
            Self::Label { .. } | Self::LabelGroup { .. } | Self::Href { .. }
        )
    }
}

/// One `--service` token: an href, an inline port, or a service name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceToken {
    Href(String),
    Port(PortSpec),
    Name(String),
}

impl ServiceToken {
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        if t.is_empty() {
            return Err("empty service".into());
        }
        if is_href(t) {
            return Ok(Self::Href(t.to_string()));
        }
        if let Some(n) = t.strip_prefix("service:") {
            let n = n.trim();
            return if n.is_empty() {
                Err(format!("{s:?}: missing service name"))
            } else {
                Ok(Self::Name(n.to_string()))
            };
        }
        // A slash or a bare number can only mean a port: report the port
        // error, not "no service named".
        if t.contains('/') || t.bytes().all(|b| b.is_ascii_digit()) {
            return PortSpec::parse(t).map(Self::Port);
        }
        Ok(PortSpec::parse(t)
            .map(Self::Port)
            .unwrap_or_else(|_| Self::Name(t.to_string())))
    }
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
            let tokens: Vec<String> = spec
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect();
            if tokens.is_empty() {
                return Err(CliError::Args(
                    "--scope needs at least one label or label group; leave --scope out for a ruleset that applies everywhere"
                        .into(),
                ));
            }
            let parsed = parse_actors(&tokens)?;
            if parsed.iter().any(|t| !t.scope_member()) {
                return Err(CliError::Args(
                    "scopes take labels and label groups only".into(),
                ));
            }
            Ok(parsed)
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

fn with_exclusion(mut a: ActorRef, exclusion: bool) -> ActorRef {
    a.exclusion = exclusion;
    a
}

/// Tokens that carry an href are checked by the library's strict checkers
/// for the kind their prefix names, with no list consulted. `None` for
/// tokens that name an object (or `all`).
fn actor_from_href_token(token: &ActorToken, org_id: u64) -> Option<Result<ActorRef, CliError>> {
    let (kind, href, segment, policy, exclusion) = match token {
        ActorToken::Href { href, exclusion } => {
            return Some(
                ActorRef::from_href(href, org_id)
                    .map(|a| with_exclusion(a, *exclusion))
                    .map_err(CliError::from),
            );
        }
        ActorToken::Label { spec, exclusion } if is_href(spec) => {
            (ActorKind::Label, spec, "labels", false, *exclusion)
        }
        ActorToken::LabelGroup { name, exclusion } if is_href(name) => (
            ActorKind::LabelGroup,
            name,
            "label_groups",
            true,
            *exclusion,
        ),
        ActorToken::IpList { name, exclusion } if is_href(name) => {
            (ActorKind::IpList, name, "ip_lists", true, *exclusion)
        }
        ActorToken::Workload { name } if is_href(name) => {
            (ActorKind::Workload, name, "workloads", false, false)
        }
        _ => return None,
    };
    let checked = if policy {
        checked_policy_href(href, segment, org_id)
    } else {
        checked_href(href, segment, org_id)
    };
    Some(
        checked
            .map(|h| with_exclusion(ActorRef::new(kind, h), exclusion))
            .map_err(CliError::from),
    )
}

fn service_from_href_token(
    token: &ServiceToken,
    org_id: u64,
) -> Option<Result<ServiceRef, CliError>> {
    match token {
        ServiceToken::Href(h) => Some(ServiceRef::from_href(h, org_id).map_err(CliError::from)),
        ServiceToken::Name(n) if is_href(n) => {
            Some(ServiceRef::from_href(n, org_id).map_err(CliError::from))
        }
        _ => None,
    }
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
    fn actor_tokens_parse_every_form() {
        let label = |spec: &str, exclusion| ActorToken::Label {
            spec: spec.into(),
            exclusion,
        };
        assert_eq!(
            ActorToken::parse("app=web").unwrap(),
            label("app=web", false)
        );
        assert_eq!(
            ActorToken::parse(" label:app = a=b ").unwrap(),
            label("app=a=b", false)
        );
        assert_eq!(
            ActorToken::parse("!env=prod").unwrap(),
            label("env=prod", true)
        );
        assert_eq!(
            ActorToken::parse("group:All prod envs").unwrap(),
            ActorToken::LabelGroup {
                name: "All prod envs".into(),
                exclusion: false
            }
        );
        assert_eq!(
            ActorToken::parse("!iplist:Corp").unwrap(),
            ActorToken::IpList {
                name: "Corp".into(),
                exclusion: true
            }
        );
        assert_eq!(
            ActorToken::parse("workload:web-1.lab").unwrap(),
            ActorToken::Workload {
                name: "web-1.lab".into()
            }
        );
        assert_eq!(ActorToken::parse("ALL").unwrap(), ActorToken::All);
        assert_eq!(
            ActorToken::parse("!/orgs/5/labels/1").unwrap(),
            ActorToken::Href {
                href: "/orgs/5/labels/1".into(),
                exclusion: true
            }
        );
    }

    #[test]
    fn actor_tokens_refuse_bad_input() {
        assert_eq!(
            ActorToken::parse("!all").unwrap_err(),
            r#""!all": all cannot be excluded"#
        );
        assert!(
            ActorToken::parse("!workload:x")
                .unwrap_err()
                .contains("only labels, label groups and IP lists can be excluded")
        );
        assert!(
            ActorToken::parse("group:")
                .unwrap_err()
                .contains("missing name")
        );
        assert!(
            ActorToken::parse("web")
                .unwrap_err()
                .contains("is not an actor")
        );
        assert!(
            ActorToken::parse("app=")
                .unwrap_err()
                .contains("expected key=value")
        );
        assert!(ActorToken::parse("!").unwrap_err().contains("is empty"));
    }

    #[test]
    fn service_tokens_split_ports_names_and_hrefs() {
        use pce::write::PortSpec;
        assert_eq!(
            ServiceToken::parse("/orgs/5/sec_policy/draft/services/31").unwrap(),
            ServiceToken::Href("/orgs/5/sec_policy/draft/services/31".into())
        );
        assert_eq!(
            ServiceToken::parse("443/tcp").unwrap(),
            ServiceToken::Port(PortSpec::parse("443/tcp").unwrap())
        );
        assert_eq!(
            ServiceToken::parse("8000-8100/udp").unwrap(),
            ServiceToken::Port(PortSpec::parse("8000-8100/udp").unwrap())
        );
        assert_eq!(
            ServiceToken::parse("icmp").unwrap(),
            ServiceToken::Port(PortSpec::parse("icmp").unwrap())
        );
        assert_eq!(
            ServiceToken::parse("HTTPS").unwrap(),
            ServiceToken::Name("HTTPS".into())
        );
        assert_eq!(
            ServiceToken::parse("service:tcp").unwrap(),
            ServiceToken::Name("tcp".into())
        );
        assert!(
            ServiceToken::parse("443")
                .unwrap_err()
                .contains("bare number is ambiguous")
        );
        assert!(
            ServiceToken::parse("443/tpc")
                .unwrap_err()
                .contains("unknown protocol")
        );
        assert!(ServiceToken::parse("service: ").is_err());
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

    #[test]
    fn prefixed_hrefs_resolve_through_the_strict_checkers_without_lists() {
        // The href paths are pure (no Context, no Lists, no request), so
        // these tests cannot fetch anything.
        let org = 5;
        let actor = |s: &str| {
            let tok = ActorToken::parse(s).unwrap();
            actor_from_href_token(&tok, org).expect("href token")
        };
        let a = actor("label:/orgs/5/labels/1").unwrap();
        assert_eq!(
            (a.kind, a.href.as_deref()),
            (ActorKind::Label, Some("/orgs/5/labels/1"))
        );
        assert!(actor("!label:/orgs/5/labels/1").unwrap().exclusion);
        let a = actor("group:/orgs/5/sec_policy/active/label_groups/2").unwrap();
        assert_eq!(a.kind, ActorKind::LabelGroup);
        assert_eq!(
            a.href.as_deref(),
            Some("/orgs/5/sec_policy/draft/label_groups/2")
        );
        let a = actor("!iplist:/orgs/5/sec_policy/draft/ip_lists/3").unwrap();
        assert_eq!(a.kind, ActorKind::IpList);
        assert!(a.exclusion);
        let a = actor("workload:/orgs/5/workloads/abc").unwrap();
        assert_eq!(a.kind, ActorKind::Workload);
        assert_eq!(a.href.as_deref(), Some("/orgs/5/workloads/abc"));
        assert!(
            actor("iplist:/orgs/5/labels/1")
                .unwrap_err()
                .to_string()
                .contains("ip_lists")
        );
        assert!(
            actor("group:/orgs/5/labels/1")
                .unwrap_err()
                .to_string()
                .contains("label_groups")
        );
        assert!(
            actor("workload:/orgs/5/labels/1")
                .unwrap_err()
                .to_string()
                .contains("workloads")
        );
        assert!(actor("label:/orgs/9/labels/1").is_err(), "wrong org");
        assert!(actor_from_href_token(&ActorToken::parse("app=web").unwrap(), org).is_none());
        assert!(actor_from_href_token(&ActorToken::parse("iplist:Corp").unwrap(), org).is_none());

        let svc = |s: &str| service_from_href_token(&ServiceToken::parse(s).unwrap(), org);
        assert_eq!(
            svc("service:/orgs/5/sec_policy/active/services/31")
                .unwrap()
                .unwrap(),
            ServiceRef::Href("/orgs/5/sec_policy/draft/services/31".into())
        );
        assert!(svc("service:/orgs/5/labels/1").unwrap().is_err());
        assert!(svc("HTTPS").is_none());
        assert!(svc("443/tcp").is_none());
    }
}
