//! Writes to the PCE. Two phases: `plan_*` functions (in the per-kind
//! modules) build a [`Plan`] without sending anything; [`Writer::execute`]
//! sends it exactly once, audits it, invalidates the cache and re-fetches
//! the object when the PCE answers with no body. A [`Writer`] exists only
//! through [`crate::Pce::writer`] with a [`WriteGuard`].
pub mod actors;
pub mod audit;
pub mod batch;
pub mod bulk;
pub mod guard;
pub mod ip_lists;
pub mod labels;
pub mod pairing_profiles;
pub mod provision;
pub mod revert;
pub mod rule_sets;
pub mod rules;
pub mod services;
pub mod specs;
pub mod workloads;

use crate::client::WriteMethod;
use crate::error::PceError;
use crate::{CacheMode, Pce};
use serde::Serialize;
use serde_json::Value;

pub use actors::{ActorKind, ActorRef, ServiceRef};
pub use audit::{AuditEntry, AuditSink, JsonlAudit, MemoryAudit, NullAudit};
pub use batch::{Batch, BatchInput, BatchOrigin, BatchStep, StepResult, step_placeholder};
pub use bulk::{
    BULK_CHUNK, BulkChange, BulkLabel, BulkPlan, BulkReport, BulkTarget, EnforcementMode,
    ItemStatus, LabelEdit, LabelSet, VisibilityLevel, plan_bulk,
};
pub use guard::{WriteContext, WriteGuard, WriteRefused};
pub use ip_lists::{IpListDraft, IpListPatch};
pub use labels::LabelDraft;
pub use pairing_profiles::{Limit, PairingProfileDraft, VenType};
pub use provision::{ProvisionRequest, ProvisionSubset, provision_counts};
pub use revert::{BACK_TO_ACTIVE, FATE_UNKNOWN, REMOVED, RevertRequest};
pub use rule_sets::{RuleSetDraft, RuleSetPatch};
pub use rules::{
    DenyRuleDraft, DenyRulePatch, INLINE_SERVICE, RuleDraft, RulePatch, apply_deny_rule_patch,
    apply_rule_patch,
};
pub use services::{ServiceDraft, ServicePatch};
pub use specs::{InterfaceSpec, IpRangeSpec, PortSpec};
pub use workloads::{LabelRemoval, LabelsPatch, WorkloadDraft, WorkloadPatch};

/// What kind of object a plan touches; decides the cache prefixes and
/// whether the result waits for provisioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Label,
    LabelGroup,
    IpList,
    Service,
    Workload,
    /// `PUT workloads/bulk_update`: many workloads in one request; the
    /// answer lists each one's outcome and names no object to re-fetch.
    WorkloadBulk,
    RuleSet,
    Rule,
    DenyRule,
    /// `POST sec_policy`: changes active policy and therefore every
    /// policy and object list the cache holds.
    Provision,
    /// `PUT sec_policy/delete`: reverts draft policy objects of any type
    /// to their active version.
    Revert,
    /// Not a policy object: applies at once.
    PairingProfile,
}

impl ObjectKind {
    /// The cache prefixes a write of this kind drops. Every kind drops
    /// `usage:` (the PCE's label usage flags): a workload, rule or group
    /// write changes what holds a label. Every write that can change how
    /// the draft policy decides a flow drops `draft:` (the draft traffic
    /// results): the draft objects, provision and revert, and a workload
    /// (its labels place it in scopes). A label write changes no decision.
    pub fn cache_prefixes(self) -> &'static [&'static str] {
        match self {
            ObjectKind::Label => &["objects:", "usage:"],
            ObjectKind::LabelGroup | ObjectKind::IpList | ObjectKind::Service => {
                &["objects:", "usage:", "draft:"]
            }
            ObjectKind::Workload => &["workloads", "usage:", "draft:"],
            // An enforcement change moves no workload between scopes; the
            // executor drops `usage:` and `draft:` after a label change.
            ObjectKind::WorkloadBulk => &["workloads"],
            ObjectKind::RuleSet | ObjectKind::Rule | ObjectKind::DenyRule => {
                &["policy:", "usage:", "draft:"]
            }
            ObjectKind::Provision | ObjectKind::Revert => {
                &["policy:", "objects:", "usage:", "draft:"]
            }
            // Its own list; a label's usage flags name pairing profiles.
            ObjectKind::PairingProfile => &["objects:pairing_profiles", "usage:"],
        }
    }

    /// Policy objects live in draft until provisioned.
    pub fn pending(self) -> bool {
        matches!(
            self,
            ObjectKind::LabelGroup
                | ObjectKind::IpList
                | ObjectKind::Service
                | ObjectKind::RuleSet
                | ObjectKind::Rule
                | ObjectKind::DenyRule
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Label => "label",
            ObjectKind::LabelGroup => "label_group",
            ObjectKind::IpList => "ip_list",
            ObjectKind::Service => "service",
            ObjectKind::Workload => "workload",
            ObjectKind::WorkloadBulk => "workload_bulk",
            ObjectKind::RuleSet => "rule_set",
            ObjectKind::Rule => "rule",
            ObjectKind::DenyRule => "deny_rule",
            ObjectKind::Provision => "provision",
            ObjectKind::Revert => "revert",
            ObjectKind::PairingProfile => "pairing_profile",
        }
    }

    /// Acts on the draft policy as a whole: the answer echoes none of the
    /// request's keys and names no object to re-fetch.
    pub fn whole_policy(self) -> bool {
        matches!(self, ObjectKind::Provision | ObjectKind::Revert)
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
    /// One line per object the request touches, for a request that names
    /// many (a revert: `ip_list "Corp" (removed)`); empty otherwise.
    /// Shown, never sent.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<String>,
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
    /// The write applied but its audit entry could not be written (why).
    /// Also among `warnings`; a batch stops on it.
    #[serde(skip)]
    pub audit_failed: Option<String>,
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

/// An href of the expected kind (`labels`, `ip_lists`, `services`,
/// `workloads`) in the profile's org, parsed strictly by component:
/// `/orgs/<org>/<segment>/<id>` or `/orgs/<org>/sec_policy/<scope>/<segment>/<id>`
/// where `<org>` is digits, `<id>` is `[A-Za-z0-9-]+` and `<scope>` is
/// `draft` or `active` (rewritten to draft: the only write target). Dot
/// segments, percent escapes, extra segments and foreign orgs are refused,
/// so URL normalisation can never turn the href into another object.
pub fn checked_href(href: &str, segment: &str, org_id: u64) -> Result<String, WriteError> {
    let href = href.trim().trim_end_matches('/');
    let bad = || {
        WriteError::invalid(format!(
            "{href:?} is not a {segment} href (expected /orgs/<org>/{segment}/<id> or /orgs/<org>/sec_policy/draft/{segment}/<id>)"
        ))
    };
    let parts: Vec<&str> = href.split('/').collect();
    let ["", "orgs", org, rest @ ..] = parts.as_slice() else {
        return Err(bad());
    };
    let (scope, kind, id) = match rest {
        [kind, id] => (None, *kind, *id),
        ["sec_policy", scope, kind, id] => (Some(*scope), *kind, *id),
        _ => return Err(bad()),
    };
    let id_ok = !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    let org_ok = !org.is_empty() && org.bytes().all(|b| b.is_ascii_digit());
    if kind != segment || !id_ok || !org_ok {
        return Err(bad());
    }
    let org: u64 = org.parse().map_err(|_| bad())?;
    if org != org_id {
        return Err(WriteError::invalid(format!(
            "{href} belongs to org {org}, profile is org {org_id}"
        )));
    }
    match scope {
        None => Ok(format!("/orgs/{org}/{segment}/{id}")),
        Some("draft" | "active") => Ok(format!("/orgs/{org}/sec_policy/draft/{segment}/{id}")),
        Some(_) => Err(WriteError::invalid(format!(
            "{href}: only draft policy objects can be written"
        ))),
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A rule or deny rule href inside a ruleset, parsed strictly by component:
/// `/orgs/<org>/sec_policy/<draft|active>/<parent>/<id>/<child>/<id>`, each
/// id `[A-Za-z0-9-]+`, `active` rewritten to `draft`, a foreign org refused.
pub fn checked_nested_href(
    href: &str,
    parent: &str,
    child: &str,
    org_id: u64,
) -> Result<String, WriteError> {
    let href = href.trim().trim_end_matches('/');
    let bad = || {
        WriteError::invalid(format!(
            "{href:?} is not a {child} href (expected /orgs/<org>/sec_policy/draft/{parent}/<id>/{child}/<id>)"
        ))
    };
    let parts: Vec<&str> = href.split('/').collect();
    let ["", "orgs", org, "sec_policy", scope, p, pid, c, cid] = parts.as_slice() else {
        return Err(bad());
    };
    let org_ok = !org.is_empty() && org.bytes().all(|b| b.is_ascii_digit());
    if *p != parent || *c != child || !valid_id(pid) || !valid_id(cid) || !org_ok {
        return Err(bad());
    }
    let org: u64 = org.parse().map_err(|_| bad())?;
    if org != org_id {
        return Err(WriteError::invalid(format!(
            "{href} belongs to org {org}, profile is org {org_id}"
        )));
    }
    match *scope {
        "draft" | "active" => Ok(format!(
            "/orgs/{org}/sec_policy/draft/{parent}/{pid}/{child}/{cid}"
        )),
        _ => Err(WriteError::invalid(format!(
            "{href}: only draft policy objects can be written"
        ))),
    }
}

/// [`checked_href`] for policy objects: the `/sec_policy/<scope>/` form
/// only. `checked_href` alone also accepts `/orgs/N/rule_sets/ID`, which
/// names no ruleset.
pub fn checked_policy_href(href: &str, segment: &str, org_id: u64) -> Result<String, WriteError> {
    let out = checked_href(href, segment, org_id)?;
    if out.split('/').nth(3) == Some("sec_policy") {
        Ok(out)
    } else {
        Err(WriteError::invalid(format!(
            "{:?} is not a {segment} href (expected /orgs/<org>/sec_policy/draft/{segment}/<id>)",
            href.trim()
        )))
    }
}

/// The ruleset href of a rule or deny rule href (the first seven
/// components). Call it on an href that passed [`checked_nested_href`].
pub fn parent_href(nested: &str) -> String {
    let parts: Vec<&str> = nested.trim().trim_end_matches('/').split('/').collect();
    parts[..parts.len().min(7)].join("/")
}

/// A fetched object decoded into its model; the error names the href.
pub(crate) fn decode<T: serde::de::DeserializeOwned>(
    href: &str,
    v: Value,
) -> Result<T, WriteError> {
    serde_json::from_value(v).map_err(|e| WriteError::Pce(PceError::Decode(format!("{href}: {e}"))))
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
        if kind == ObjectKind::WorkloadBulk {
            entry.items = Some(bulk::audit_items(request.body.as_ref(), &resp.body));
        }
        let mut warnings = Vec::new();
        let mut audit_failed = None;
        if let Err(e) = audit.append(&entry) {
            let why = format!(
                "audit entry could not be written to {}: {e}",
                audit.location()
            );
            warnings.push(why.clone());
            audit_failed = Some(why);
        }
        for prefix in kind.cache_prefixes() {
            self.pce.invalidate(prefix);
        }

        let object = if request.method == WriteMethod::Delete {
            Value::Null
        } else if post_without_href {
            // Re-fetching the collection path would load the whole list.
            warnings.push("response carried no href; object not re-fetched".into());
            resp.body
        } else if resp.body.is_null() && (kind.whole_policy() || kind == ObjectKind::WorkloadBulk) {
            // The path is an action (`sec_policy/delete`,
            // `workloads/bulk_update`), not an object.
            Value::Null
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
        // A provision answers with the new policy version, a revert with
        // little or nothing: neither echoes the request's keys.
        let dropped = if request.method == WriteMethod::Delete || kind.whole_policy() {
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
            audit_failed,
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
            items: None,
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
        assert_eq!(ObjectKind::Label.cache_prefixes(), &["objects:", "usage:"]);
        assert_eq!(
            ObjectKind::IpList.cache_prefixes(),
            &["objects:", "usage:", "draft:"]
        );
        assert_eq!(
            ObjectKind::Workload.cache_prefixes(),
            &["workloads", "usage:", "draft:"]
        );
        // A bulk enforcement change moves no workload between scopes; the
        // bulk executor drops `usage:` and `draft:` after a label change.
        assert_eq!(ObjectKind::WorkloadBulk.cache_prefixes(), &["workloads"]);
        assert!(!ObjectKind::WorkloadBulk.pending() && !ObjectKind::WorkloadBulk.whole_policy());
        assert_eq!(
            ObjectKind::RuleSet.cache_prefixes(),
            &["policy:", "usage:", "draft:"]
        );
        assert_eq!(
            ObjectKind::Rule.cache_prefixes(),
            &["policy:", "usage:", "draft:"]
        );
        assert_eq!(
            ObjectKind::DenyRule.cache_prefixes(),
            &["policy:", "usage:", "draft:"]
        );
        assert_eq!(
            ObjectKind::Provision.cache_prefixes(),
            &["policy:", "objects:", "usage:", "draft:"]
        );
        assert_eq!(
            ObjectKind::Revert.cache_prefixes(),
            &["policy:", "objects:", "usage:", "draft:"]
        );
        // Every pending kind can change a draft decision: it drops the
        // draft traffic results.
        for k in [
            ObjectKind::LabelGroup,
            ObjectKind::IpList,
            ObjectKind::Service,
            ObjectKind::RuleSet,
            ObjectKind::Rule,
            ObjectKind::DenyRule,
        ] {
            assert!(
                k.pending() && k.cache_prefixes().contains(&"draft:"),
                "{k:?}"
            );
        }
        // Label usage goes stale with any write: every kind drops it.
        for k in [
            ObjectKind::Label,
            ObjectKind::LabelGroup,
            ObjectKind::IpList,
            ObjectKind::Service,
            ObjectKind::Workload,
            ObjectKind::RuleSet,
            ObjectKind::Rule,
            ObjectKind::DenyRule,
            ObjectKind::Provision,
            ObjectKind::Revert,
            ObjectKind::PairingProfile,
        ] {
            assert!(k.cache_prefixes().contains(&"usage:"), "{k:?}");
        }
        assert!(ObjectKind::Revert.whole_policy() && ObjectKind::Provision.whole_policy());
        assert!(!ObjectKind::RuleSet.whole_policy());
        for k in [
            ObjectKind::IpList,
            ObjectKind::Service,
            ObjectKind::RuleSet,
            ObjectKind::Rule,
            ObjectKind::DenyRule,
        ] {
            assert!(k.pending(), "{k:?}");
        }
        for k in [
            ObjectKind::Label,
            ObjectKind::Workload,
            ObjectKind::Provision,
            ObjectKind::Revert,
        ] {
            assert!(!k.pending(), "{k:?}");
        }
        assert_eq!(ObjectKind::DenyRule.as_str(), "deny_rule");
        assert_eq!(ObjectKind::Provision.as_str(), "provision");
        assert_eq!(ObjectKind::Revert.as_str(), "revert");
        assert_eq!(
            ObjectKind::PairingProfile.cache_prefixes(),
            &["objects:pairing_profiles", "usage:"]
        );
        assert!(!ObjectKind::PairingProfile.pending());
        assert_eq!(ObjectKind::PairingProfile.as_str(), "pairing_profile");
        assert_eq!(
            serde_json::to_value(ObjectKind::RuleSet).unwrap(),
            json!("rule_set")
        );
    }

    fn nested_err(href: &str, child: &str) -> String {
        match checked_nested_href(href, "rule_sets", child, 5).unwrap_err() {
            WriteError::Invalid(m) => m.join("; "),
            other => panic!("{href}: expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn nested_hrefs_pass_and_active_is_rewritten() {
        assert_eq!(
            checked_nested_href(
                " /orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1/ ",
                "rule_sets",
                "sec_rules",
                5
            )
            .unwrap(),
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1"
        );
        assert_eq!(
            checked_nested_href(
                "/orgs/5/sec_policy/active/rule_sets/24206847997116627/deny_rules/0a-B",
                "rule_sets",
                "deny_rules",
                5
            )
            .unwrap(),
            "/orgs/5/sec_policy/draft/rule_sets/24206847997116627/deny_rules/0a-B"
        );
    }

    #[test]
    fn nested_other_scopes_are_refused() {
        for scope in ["42", "Draft", "", "pending"] {
            let m = nested_err(
                &format!("/orgs/5/sec_policy/{scope}/rule_sets/100/sec_rules/1"),
                "sec_rules",
            );
            assert!(!m.is_empty(), "{scope}");
        }
    }

    #[test]
    fn nested_dot_and_empty_segments_are_refused() {
        for href in [
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/..",
            "/orgs/5/sec_policy/draft/rule_sets/../sec_rules/1",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1/../2",
            "/orgs/5/sec_policy/draft/rule_sets/./100/sec_rules/1",
            "/orgs/5/sec_policy/draft/rule_sets//sec_rules/1",
            "/orgs/5/sec_policy/draft/rule_sets/100//1",
            "//orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/%2e%2e",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1_2",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1.2",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/",
        ] {
            let m = nested_err(href, "sec_rules");
            assert!(
                m.contains("expected /orgs/<org>/sec_policy/draft/rule_sets/<id>/sec_rules/<id>"),
                "{href}: {m}"
            );
        }
    }

    #[test]
    fn nested_wrong_parent_child_or_flat_is_refused() {
        nested_err(
            "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/7",
            "sec_rules",
        );
        nested_err(
            "/orgs/5/sec_policy/draft/ip_lists/100/sec_rules/1",
            "sec_rules",
        );
        let m = nested_err("/orgs/5/sec_policy/draft/rule_sets/100", "sec_rules");
        assert!(m.contains("is not a sec_rules href"), "{m}");
        // checked_href keeps refusing nested paths.
        assert!(
            checked_href(
                "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
                "rule_sets",
                5
            )
            .is_err()
        );
        assert!(
            checked_href(
                "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
                "sec_rules",
                5
            )
            .is_err()
        );
    }

    #[test]
    fn nested_foreign_org_is_refused_with_both_ids() {
        let m = nested_err(
            "/orgs/7/sec_policy/draft/rule_sets/100/sec_rules/1",
            "sec_rules",
        );
        assert!(m.contains("org 7") && m.contains("org 5"), "{m}");
    }

    #[test]
    fn parent_href_strips_the_rule_part() {
        assert_eq!(
            parent_href("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1"),
            "/orgs/5/sec_policy/draft/rule_sets/100"
        );
        assert_eq!(
            parent_href("/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8/"),
            "/orgs/5/sec_policy/draft/rule_sets/100"
        );
    }

    #[test]
    fn policy_hrefs_refuse_the_flat_form() {
        assert_eq!(
            checked_policy_href("/orgs/5/sec_policy/active/rule_sets/100", "rule_sets", 5).unwrap(),
            "/orgs/5/sec_policy/draft/rule_sets/100"
        );
        let err = checked_policy_href("/orgs/5/rule_sets/100", "rule_sets", 5).unwrap_err();
        assert!(
            err.to_string()
                .contains("expected /orgs/<org>/sec_policy/draft/rule_sets/<id>"),
            "{err}"
        );
    }
}
