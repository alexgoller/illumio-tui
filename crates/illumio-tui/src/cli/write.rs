//! What every write command shares: the guard (profile + --allow-writes +
//! audit log), the --dry-run / --yes contract, exact name resolution, and
//! the envelope data. Commands build a `Plan` with the library and hand it
//! to `run_plan`.
use super::context::Context;
use super::error::CliError;
use super::output::{TableRows, WriteMeta};
use clap::Args;
use pce::models::Label;
use pce::write::{
    JsonlAudit, Outcome, Plan, WriteContext, WriteError, WriteGuard, WriteRequest, Written,
};
use serde::Serialize;

#[derive(Args, Debug, Clone, Default)]
pub struct WriteFlags {
    /// Print the request that would be sent, send nothing
    #[arg(long)]
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
    let audit = JsonlAudit::new(JsonlAudit::default_path());
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
fn similar<'a>(needle: &str, names: impl Iterator<Item = &'a str>) -> Vec<String> {
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

fn with_similar(msg: String, similar: &[String]) -> String {
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

pub async fn resolve_ip_list(ctx: &Context, name: &str) -> Result<String, CliError> {
    if is_href(name) {
        return Ok(name.trim().to_string());
    }
    let lists = ctx.pce.ip_lists(ctx.cache_mode).await?.value;
    pick_one(
        "IP list",
        name.trim(),
        lists
            .iter()
            .filter(|l| not_pending_delete(l.update_type.as_deref()))
            .map(|l| (l.name.as_str(), l.href.as_str())),
    )
}

pub async fn resolve_service(ctx: &Context, name: &str) -> Result<String, CliError> {
    if is_href(name) {
        return Ok(name.trim().to_string());
    }
    let services = ctx.pce.services(ctx.cache_mode).await?.value;
    pick_one(
        "service",
        name.trim(),
        services
            .iter()
            .filter(|s| not_pending_delete(s.update_type.as_deref()))
            .map(|s| (s.name.as_str(), s.href.as_str())),
    )
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
}
