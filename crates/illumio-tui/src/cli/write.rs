//! What every write command shares: the guard (profile + --allow-writes +
//! audit log), the --dry-run / --yes contract, exact name resolution, and
//! the envelope data. Commands build a `Plan` with the library and hand it
//! to `run_plan`.
use super::context::Context;
use super::error::CliError;
use super::output::{TableRows, WriteMeta};
use clap::Args;
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

/// An href as given, or `key=value` resolved exactly through the name index.
pub async fn resolve_label(ctx: &Context, spec: &str) -> Result<String, CliError> {
    if is_href(spec) {
        return Ok(spec.trim().to_string());
    }
    let (k, v) = parse_kv(spec)?;
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    idx.label_href(&k, &v).map(str::to_string).ok_or_else(|| {
        let known = idx.values_for_key(&k);
        CliError::Args(if known.is_empty() {
            format!("unknown label {k}={v}; no labels have key {k}")
        } else {
            format!(
                "unknown label {k}={v}; known values for {k}: {}",
                known.join(", ")
            )
        })
    })
}

pub async fn resolve_labels(ctx: &Context, specs: &[String]) -> Result<Vec<String>, CliError> {
    let mut out = Vec::with_capacity(specs.len());
    for s in specs {
        out.push(resolve_label(ctx, s).await?);
    }
    Ok(out)
}

fn pick_one(kind: &str, name: &str, hrefs: Vec<&str>) -> Result<String, CliError> {
    match hrefs.len() {
        0 => Err(CliError::Args(format!("no {kind} named {name:?}"))),
        1 => Ok(hrefs[0].to_string()),
        _ => Err(CliError::Args(format!(
            "{name:?} names several {kind}s: {}; use the href",
            hrefs.join(", ")
        ))),
    }
}

pub async fn resolve_ip_list(ctx: &Context, name: &str) -> Result<String, CliError> {
    if is_href(name) {
        return Ok(name.trim().to_string());
    }
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let hrefs: Vec<&str> = idx
        .ip_lists_named(name.trim())
        .iter()
        .map(|l| l.href.as_str())
        .collect();
    pick_one("IP list", name.trim(), hrefs)
}

pub async fn resolve_service(ctx: &Context, name: &str) -> Result<String, CliError> {
    if is_href(name) {
        return Ok(name.trim().to_string());
    }
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let hrefs: Vec<&str> = idx
        .services_named(name.trim())
        .iter()
        .map(|s| s.href.as_str())
        .collect();
    pick_one("service", name.trim(), hrefs)
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
