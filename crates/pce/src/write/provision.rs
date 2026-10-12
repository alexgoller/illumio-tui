//! Provisioning: `POST /orgs/N/sec_policy` with an explicit
//! `change_subset`, built from one fresh read of the pending list. A rule
//! is provisioned through its ruleset, so rule hrefs never appear.
use super::{
    ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_nested_href, checked_policy_href,
    parent_href,
};
use crate::client::WriteMethod;
use crate::models::PendingChange;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisionSubset {
    /// Every pending change, listed explicitly.
    All,
    /// These hrefs; each must be pending. A rule href means its ruleset.
    Hrefs(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionRequest {
    pub description: Option<String>,
    pub subset: ProvisionSubset,
}

/// Objects that live outside the draft policy and are never provisioned.
const NOT_POLICY: [&str; 2] = ["workloads", "labels"];

/// Children of a ruleset: provisioned through the ruleset itself.
const RULE_CHILDREN: [&str; 2] = ["sec_rules", "deny_rules"];

/// Object type → number of hrefs in the body's `change_subset`.
pub fn provision_counts(body: &Value) -> BTreeMap<String, usize> {
    body.get("change_subset")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_array().map(|a| (k.clone(), a.len())))
                .filter(|(_, n)| *n > 0)
                .collect()
        })
        .unwrap_or_default()
}

/// `ip_list` for `ip_lists`.
pub(super) fn singular(kind: &str) -> &str {
    kind.strip_suffix('s').unwrap_or(kind)
}

/// `provision 1 ip_list, 3 rule_sets`.
fn summary(counts: &BTreeMap<String, usize>) -> String {
    let parts: Vec<String> = counts
        .iter()
        .map(|(k, n)| {
            if *n == 1 {
                format!("1 {}", singular(k))
            } else {
                format!("{n} {k}")
            }
        })
        .collect();
    format!("provision {}", parts.join(", "))
}

/// `(object type, draft href)` for a pending or requested href. A rule or
/// deny rule maps to its ruleset; anything else is a draft policy object
/// grouped under its own type segment. Parsed by component, checked strictly.
/// An id-less singleton such as `/orgs/N/sec_policy/draft/firewall_settings`,
/// parsed by component: matching org, `draft` or `active` (rewritten to
/// draft), a lowercase type that is a policy object. `None` when the href
/// has another shape.
fn checked_singleton_href(
    trimmed: &str,
    org_id: u64,
) -> Option<Result<(String, String), WriteError>> {
    let parts: Vec<&str> = trimmed.split('/').collect();
    let ["", "orgs", org, "sec_policy", scope, kind] = parts.as_slice() else {
        return None;
    };
    let type_ok = !kind.is_empty()
        && !NOT_POLICY.contains(kind)
        && kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    let org_ok = !org.is_empty() && org.bytes().all(|b| b.is_ascii_digit());
    if !type_ok || !org_ok || !matches!(*scope, "draft" | "active") {
        return Some(Err(WriteError::invalid(format!(
            "{trimmed} is not a draft policy object href"
        ))));
    }
    if org.parse::<u64>().ok() != Some(org_id) {
        return Some(Err(WriteError::invalid(format!(
            "{trimmed} belongs to org {org}, profile is org {org_id}"
        ))));
    }
    Some(Ok((
        kind.to_string(),
        format!("/orgs/{org_id}/sec_policy/draft/{kind}"),
    )))
}

pub(super) fn provisionable(href: &str, org_id: u64) -> Result<(String, String), WriteError> {
    let trimmed = href.trim().trim_end_matches('/');
    if let Some(single) = checked_singleton_href(trimmed, org_id) {
        return single;
    }
    let parts: Vec<&str> = trimmed.split('/').collect();
    if let [_, _, _, _, _, _, _, child, _] = parts.as_slice()
        && RULE_CHILDREN.contains(child)
    {
        let nested = checked_nested_href(trimmed, "rule_sets", child, org_id)?;
        return Ok(("rule_sets".into(), parent_href(&nested)));
    }
    let kind = parts.get(5).copied().unwrap_or_default();
    if kind.is_empty()
        || NOT_POLICY.contains(&kind)
        || !kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
    {
        return Err(WriteError::invalid(format!(
            "{trimmed} is not a draft policy object href"
        )));
    }
    let checked = checked_policy_href(trimmed, kind, org_id)?;
    Ok((kind.to_string(), checked))
}

/// True for a rule or deny rule href: it is provisioned (and reverted)
/// through its ruleset, so it is never the ruleset's own pending entry.
pub(super) fn is_rule_child(href: &str) -> bool {
    href.trim()
        .trim_end_matches('/')
        .split('/')
        .nth(7)
        .is_some_and(|c| RULE_CHILDREN.contains(&c))
}

/// What a provision or a revert covers, from one fresh read of the pending
/// list: the `(object type, draft href)` pairs chosen and the entries read.
pub(super) struct PendingSubset {
    pub chosen: BTreeSet<(String, String)>,
    pub pending: Vec<PendingChange>,
}

impl PendingSubset {
    /// `{"ip_lists": [{"href": ...}], ...}`: one list per object type.
    pub fn change_subset(&self) -> BTreeMap<String, Vec<Value>> {
        let mut subset: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for (kind, href) in &self.chosen {
            subset
                .entry(kind.clone())
                .or_default()
                .push(json!({ "href": href }));
        }
        subset
    }
}

/// `Noop` when nothing is pending: provisioning and reverting say the same.
pub(super) fn nothing_pending(w: &Writer<'_>) -> Plan {
    Plan::Noop {
        existing_href: w.pce().client().org_path("sec_policy/pending"),
        reason: "nothing pending".into(),
    }
}

impl Writer<'_> {
    /// Checks requested hrefs before any request, then reads the pending
    /// list once, past the cache. `All` lists every pending change and
    /// refuses the whole plan when one cannot be named; explicit hrefs must
    /// each be pending. `verb` names the operation in errors.
    pub(super) async fn pending_subset(
        &self,
        subset: &ProvisionSubset,
        verb: &str,
    ) -> Result<PendingSubset, WriteError> {
        let org = self.pce().client().org_id();
        let requested: Option<Vec<(String, String)>> = match subset {
            ProvisionSubset::All => None,
            ProvisionSubset::Hrefs(h) if h.is_empty() => {
                return Err(WriteError::invalid(format!(
                    "nothing to {verb}: name at least one href, or {verb} all"
                )));
            }
            ProvisionSubset::Hrefs(h) => Some(
                h.iter()
                    .map(|x| provisionable(x, org))
                    .collect::<Result<_, _>>()?,
            ),
        };
        let pending = self.pce().pending_changes(self.bypass()).await?.value;
        let mut pending_set: BTreeSet<(String, String)> = BTreeSet::new();
        for p in &pending {
            match provisionable(&p.href, org) {
                Ok(entry) => {
                    pending_set.insert(entry);
                }
                // `All` must mean all: an entry this version cannot
                // name refuses the whole plan.
                Err(e) if requested.is_none() => return Err(e),
                Err(_) => {}
            }
        }
        let chosen = match requested {
            None => pending_set,
            Some(list) => {
                if let Some((_, h)) = list.iter().find(|e| !pending_set.contains(*e)) {
                    return Err(WriteError::invalid(format!("{h} has no pending changes")));
                }
                list.into_iter().collect()
            }
        };
        Ok(PendingSubset { chosen, pending })
    }

    pub async fn plan_provision(&self, req: &ProvisionRequest) -> Result<Plan, WriteError> {
        let picked = self.pending_subset(&req.subset, "provision").await?;
        if picked.chosen.is_empty() {
            return Ok(nothing_pending(self));
        }
        let description = req
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .unwrap_or("illumio-tui provision");
        let body = json!({
            "update_description": description,
            "change_subset": picked.change_subset(),
        });
        let counts = provision_counts(&body);
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("sec_policy"),
                body: Some(body),
                summary: summary(&counts),
                objects: Vec::new(),
            },
            kind: ObjectKind::Provision,
        })
    }
}
