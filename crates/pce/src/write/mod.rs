//! Writes to the PCE. Two phases: `plan_*` functions (in the per-kind
//! modules) build a [`Plan`] without sending anything; [`Writer::execute`]
//! sends it exactly once, audits it, invalidates the cache and re-fetches
//! the object when the PCE answers with no body. A [`Writer`] exists only
//! through [`crate::Pce::writer`] with a [`WriteGuard`].
pub mod audit;
pub mod guard;
pub mod labels;
pub mod specs;

use crate::client::WriteMethod;
use crate::error::PceError;
use crate::{CacheMode, Pce};
use serde::Serialize;
use serde_json::Value;

pub use audit::{AuditEntry, AuditSink, JsonlAudit, MemoryAudit, NullAudit};
pub use guard::{WriteContext, WriteGuard, WriteRefused};
pub use labels::LabelDraft;
pub use specs::{InterfaceSpec, IpRangeSpec, PortSpec};

/// What kind of object a plan touches; decides the cache prefix and whether
/// the result waits for provisioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Label,
    IpList,
    Service,
    Workload,
}

impl ObjectKind {
    pub fn cache_prefix(self) -> &'static str {
        match self {
            ObjectKind::Label | ObjectKind::IpList | ObjectKind::Service => "objects:",
            ObjectKind::Workload => "workloads",
        }
    }

    /// Policy objects live in draft until provisioned.
    pub fn pending(self) -> bool {
        matches!(self, ObjectKind::IpList | ObjectKind::Service)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Label => "label",
            ObjectKind::IpList => "ip_list",
            ObjectKind::Service => "service",
            ObjectKind::Workload => "workload",
        }
    }
}

/// The exact request a plan would send. Printed by `--dry-run`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WriteRequest {
    pub method: WriteMethod,
    /// API path, e.g. `/orgs/5/labels/9` (same form as a PCE href).
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    /// One line for humans: `create label app=web`.
    pub summary: String,
}

#[derive(Debug, Clone)]
pub enum Plan {
    Send {
        request: WriteRequest,
        kind: ObjectKind,
    },
    /// Nothing to do (`--if-missing` found an exact match).
    Noop {
        existing_href: String,
        reason: String,
    },
}

impl Plan {
    pub fn request(&self) -> Option<&WriteRequest> {
        match self {
            Plan::Send { request, .. } => Some(request),
            Plan::Noop { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Written {
    pub href: String,
    /// True for policy objects: the change waits in draft for provisioning.
    pub pending: bool,
    pub status: u16,
    /// The object as the PCE returned it (re-fetched after a 204); `null`
    /// after a delete.
    pub object: Value,
    /// Keys sent with a value that came back empty or missing.
    pub dropped_fields: Vec<String>,
    /// Things the caller should know although the write succeeded.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Written(Written),
    Skipped {
        existing_href: String,
        reason: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error(transparent)]
    Refused(#[from] WriteRefused),
    #[error("{}", .0.join("; "))]
    Invalid(Vec<String>),
    /// The request was sent but the outcome is unknown (timeout, connection
    /// loss, or a gateway status). Never retried automatically.
    #[error("{0}; write may have applied; check the PCE before retrying")]
    Unanswered(PceError),
    #[error(transparent)]
    Pce(PceError),
    /// The write failed and its audit entry could not be written either.
    #[error("{write}; and the audit entry could not be written: {error}")]
    AuditFailed {
        write: Box<WriteError>,
        error: String,
    },
}

impl WriteError {
    /// Stable machine-readable name for CLI JSON errors.
    pub fn kind(&self) -> &'static str {
        match self {
            WriteError::Refused(_) => "write_refused",
            WriteError::Invalid(_) => "invalid",
            WriteError::Unanswered(_) => "write_unanswered",
            WriteError::Pce(e) => e.kind(),
            WriteError::AuditFailed { write, .. } => write.kind(),
        }
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        WriteError::Invalid(vec![msg.into()])
    }
}

impl From<PceError> for WriteError {
    fn from(e: PceError) -> Self {
        WriteError::Pce(e)
    }
}

/// `--if-missing`: skip the create when an exact match already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfMissing {
    Create,
    SkipExisting,
}

/// Rewrites an active policy href to its draft twin; refuses any other
/// `sec_policy` scope (draft is the only write target).
pub fn draft_href(href: &str) -> Result<String, WriteError> {
    const ACTIVE: &str = "/sec_policy/active/";
    const DRAFT: &str = "/sec_policy/draft/";
    if href.contains(ACTIVE) {
        return Ok(href.replacen(ACTIVE, DRAFT, 1));
    }
    if let Some(rest) = href.split_once("/sec_policy/").map(|(_, r)| r)
        && !rest.starts_with("draft/")
    {
        return Err(WriteError::invalid(format!(
            "{href}: only draft policy objects can be written"
        )));
    }
    Ok(href.to_string())
}

/// An href of the expected kind (`labels`, `ip_lists`, `services`,
/// `workloads`), rewritten to draft where that applies.
pub fn checked_href(href: &str, segment: &str) -> Result<String, WriteError> {
    let href = href.trim().trim_end_matches('/');
    if !href.starts_with("/orgs/") || !href.contains(&format!("/{segment}/")) {
        return Err(WriteError::invalid(format!(
            "{href:?} is not a {segment} href (expected /orgs/<org>/.../{segment}/<id>)"
        )));
    }
    draft_href(href)
}

/// A value counts as "present" for `dropped_fields` when it is not null,
/// not an empty array, object or string.
fn present(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn dropped_fields(sent: Option<&Value>, got: &Value) -> Vec<String> {
    let Some(Value::Object(sent)) = sent else {
        return Vec::new();
    };
    let Value::Object(got) = got else {
        return Vec::new();
    };
    sent.iter()
        .filter(|(_, v)| present(v))
        .filter(|(k, _)| !got.get(k.as_str()).is_some_and(present))
        .map(|(k, _)| k.clone())
        .collect()
}

/// The only thing that sends writes.
pub struct Writer<'a> {
    pce: &'a Pce,
    guard: &'a WriteGuard,
}

impl Pce {
    pub fn writer<'a>(&'a self, guard: &'a WriteGuard) -> Writer<'a> {
        Writer { pce: self, guard }
    }
}

impl<'a> Writer<'a> {
    pub fn pce(&self) -> &'a Pce {
        self.pce
    }

    pub fn guard(&self) -> &'a WriteGuard {
        self.guard
    }

    /// One uncached GET of a single object by href.
    pub async fn fetch(&self, href: &str) -> Result<Value, WriteError> {
        Ok(self.pce.client().get::<Value>(href, &[]).await?.body)
    }

    /// The cache mode for an `--if-missing` lookup: always fresh.
    pub fn bypass(&self) -> CacheMode {
        CacheMode::Bypass
    }

    pub async fn execute(&self, plan: Plan) -> Result<Outcome, WriteError> {
        let (request, kind) = match plan {
            Plan::Noop {
                existing_href,
                reason,
            } => {
                return Ok(Outcome::Skipped {
                    existing_href,
                    reason,
                });
            }
            Plan::Send { request, kind } => (request, kind),
        };
        let audit = self.guard.audit();
        audit
            .check_writable()
            .map_err(|e| WriteRefused::AuditUnwritable(format!("{}: {e}", audit.location())))?;

        let result = self
            .pce
            .client()
            .write(request.method, &request.path, request.body.as_ref())
            .await;

        let mut entry = self.entry(&request);
        let resp = match result {
            Ok(resp) => resp,
            Err(e) => {
                let unanswered = matches!(
                    e,
                    PceError::Timeout(_)
                        | PceError::Network(_)
                        | PceError::Decode(_)
                        | PceError::Server {
                            status: 502..=504,
                            ..
                        }
                );
                match &e {
                    PceError::Server { status, .. }
                    | PceError::Rejected { status, .. }
                    | PceError::Auth { status } => entry.status = Some(*status),
                    PceError::NotFound { .. } => entry.status = Some(404),
                    PceError::RateLimited { .. } => entry.status = Some(429),
                    _ => {}
                }
                entry.error = Some(e.to_string());
                if unanswered {
                    entry.phase = Some("sent".into());
                }
                let appended = audit.append(&entry);
                let err = if unanswered {
                    WriteError::Unanswered(e)
                } else {
                    WriteError::Pce(e)
                };
                return Err(match appended {
                    Ok(()) => err,
                    Err(a) => WriteError::AuditFailed {
                        write: Box::new(err),
                        error: format!("{}: {a}", audit.location()),
                    },
                });
            }
        };

        let body_href = resp
            .body
            .get("href")
            .and_then(Value::as_str)
            .map(str::to_string);
        let post_without_href = request.method == WriteMethod::Post && body_href.is_none();
        let href = body_href.unwrap_or_else(|| request.path.clone());
        entry.status = Some(resp.status);
        entry.href = Some(href.clone());
        let mut warnings = Vec::new();
        if let Err(e) = audit.append(&entry) {
            warnings.push(format!(
                "audit entry could not be written to {}: {e}",
                audit.location()
            ));
        }
        self.pce.invalidate(kind.cache_prefix());

        let object = if request.method == WriteMethod::Delete {
            Value::Null
        } else if post_without_href {
            // Re-fetching the collection path would load the whole list.
            warnings.push("response carried no href; object not re-fetched".into());
            resp.body
        } else if resp.body.is_null() {
            match self.fetch(&href).await {
                Ok(v) => v,
                Err(e) => {
                    warnings.push(format!("write applied; re-fetch of {href} failed: {e}"));
                    Value::Null
                }
            }
        } else {
            resp.body
        };
        let dropped = if request.method == WriteMethod::Delete {
            Vec::new()
        } else {
            dropped_fields(request.body.as_ref(), &object)
        };
        Ok(Outcome::Written(Written {
            href,
            pending: kind.pending(),
            status: resp.status,
            object,
            dropped_fields: dropped,
            warnings,
        }))
    }

    fn entry(&self, request: &WriteRequest) -> AuditEntry {
        let c = self.guard.context();
        AuditEntry {
            ts: jiff::Timestamp::now().to_string(),
            profile: c.profile.clone(),
            host: c.host.clone(),
            org_id: c.org_id,
            method: request.method,
            path: request.path.clone(),
            body: request.body.clone(),
            status: None,
            href: None,
            error: None,
            phase: None,
            source: c.source.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dropped_fields_flags_sent_values_that_came_back_empty() {
        let sent = json!({"name": "PG", "service_ports": [{"port": 1}], "description": ""});
        let got = json!({"name": "PG", "service_ports": [], "href": "/x"});
        assert_eq!(
            dropped_fields(Some(&sent), &got),
            vec!["service_ports".to_string()]
        );
        assert!(dropped_fields(None, &got).is_empty());
        assert!(dropped_fields(Some(&sent), &Value::Null).is_empty());
    }

    #[test]
    fn object_kind_prefixes_and_pending() {
        assert_eq!(ObjectKind::Label.cache_prefix(), "objects:");
        assert_eq!(ObjectKind::Workload.cache_prefix(), "workloads");
        assert!(ObjectKind::IpList.pending());
        assert!(!ObjectKind::Label.pending());
        assert!(!ObjectKind::Workload.pending());
    }
}
